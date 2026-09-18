use crate::{Finding, Trigger};
use std::collections::BTreeMap;

/// `EX5031`: the result of a call to a configured module's functions has
/// to be used. Format: `{module, functions}` or `{module, functions,
/// issue_message}`; `functions` is a list of names or `:all`.
///
/// Without configured `modules`, the kernel is conservative (no findings).
/// Unused-ness follows `Credo.Check.Warning.UnusedFunctionReturnHelper`.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let configs = parse_modules(params);
    if configs.is_empty() {
        return Vec::new();
    }
    let facts = prepared.facts();
    let Some(tree) = prepared.tree() else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    // First matching config wins per call, mirroring the single-walk
    // `find`; later configs skip already-reported spans.
    let mut reported: Vec<(u32, u32)> = Vec::new();
    for config in &configs {
        let module = config.parts.join(".");
        let funs: Option<Vec<&str>> = config
            .funs
            .as_ref()
            .map(|names| names.iter().map(String::as_str).collect());
        let filter = super::unused_return::CandidateFilter {
            module: &module,
            funs: funs.as_deref(),
            ignored: &[],
            atoms: true,
        };
        for (start, end, _) in super::unused_return::candidate_spans_for(source, facts, &filter) {
            if reported.contains(&(start, end)) {
                continue;
            }
            let Some(call) = super::unused_return::recover_call(tree, start, end) else {
                continue;
            };
            if !is_unused(call, source) {
                continue;
            }
            reported.push((start, end));
            let (line, column) = super::unused_return::line_col(source, alias_start(&call));
            let trigger = super::unused_return::dot_text(source, &call).unwrap_or(&config.name);
            findings.push(Finding {
                line,
                column: Some(column),
                message: config.message(),
                trigger: Trigger::Text(trigger.to_owned()),
                severity: None,
            });
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// Module-side start byte of a recovered candidate call.
fn alias_start(call: &tree_sitter::Node<'_>) -> usize {
    let mut cursor = call.walk();
    call.children(&mut cursor)
        .find(|child| child.kind() == "dot")
        .and_then(|dot| {
            let mut inner = dot.walk();
            dot.children(&mut inner).find(tree_sitter::Node::is_named)
        })
        .map_or(call.start_byte(), |left| left.start_byte())
}

/// One `{module, functions, message?}` entry: `parts` is the dotted module
/// path, `funs` is `None` for `:all`, `message` overrides the default.
struct ModuleConfig {
    name: String,
    parts: Vec<String>,
    funs: Option<Vec<String>>,
    message: Option<String>,
}

impl ModuleConfig {
    fn message(&self) -> String {
        self.message.clone().unwrap_or_else(|| {
            format!(
                "There should be no unused return values for `{}` functions.",
                self.name
            )
        })
    }
}

/// Parse the `modules` param: compact JSON array of `[mod, funs]` or
/// `[mod, funs, message]` entries; corpus atoms arrive colon-marked
/// (`:Elixir.Map`, `:all`, `:get`), matched colon-tolerantly.
fn parse_modules(params: &BTreeMap<String, String>) -> Vec<ModuleConfig> {
    let Some(raw) = params.get("modules") else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in &items {
        let Some(entry) = item
            .as_array()
            .or_else(|| item.get("tuple").and_then(serde_json::Value::as_array))
        else {
            continue;
        };
        if entry.len() < 2 || entry.len() > 3 {
            continue;
        }
        let (Some(module), funs, message) = (
            entry[0].as_str(),
            entry[1].clone(),
            entry.get(2).and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        let bare = module.trim_start_matches(':');
        let name = bare.strip_prefix("Elixir.").unwrap_or(bare).to_owned();
        let parts = name.split('.').map(str::to_owned).collect();
        let funs = match funs {
            serde_json::Value::String(all) if all.trim_start_matches(':') == "all" => None,
            serde_json::Value::Array(names) => Some(
                names
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(|fun| fun.trim_start_matches(':').to_owned())
                    .collect(),
            ),
            _ => continue,
        };
        out.push(ModuleConfig {
            name,
            parts,
            funs,
            message: message.map(str::to_owned),
        });
    }
    out
}

/// `def`/`defp`/`defmacro` head name when `node` defines a function.
fn def_head(node: &tree_sitter::Node<'_>, source: &str) -> Option<String> {
    let head = node.child(0)?;
    if head.kind() != "identifier" {
        return None;
    }
    match node_text(source, &head) {
        Some("def" | "defp" | "defmacro") => node_text(source, &head).map(str::to_owned),
        _ => None,
    }
}

/// One upward step in the discard walk.
enum Step<'tree> {
    Up(tree_sitter::Node<'tree>),
    Done(bool),
}

/// Verdict for a `call` parent: def-head calls, control heads, arguments.
fn call_step<'tree>(
    parent: &tree_sitter::Node<'tree>,
    current: &tree_sitter::Node<'tree>,
    start: usize,
    source: &str,
) -> Step<'tree> {
    if def_head(parent, source).is_some() {
        if inside_arguments(parent, current) {
            return Step::Done(false);
        }
        return Step::Done(!def_tail_contains(parent, start));
    }
    if let Some(head) = call_head(parent, source) {
        if matches!(head.as_str(), "if" | "unless" | "case" | "for" | "quote") {
            if inside_arguments(parent, current) {
                return Step::Done(false);
            }
            return Step::Up(*parent);
        }
        if head == "try" && after_contains(parent, start) {
            return Step::Done(true);
        }
    }
    if inside_arguments(parent, current) {
        return Step::Done(false);
    }
    Step::Up(*parent)
}

/// Verdict for a block parent.
fn block_step<'tree>(
    parent: &tree_sitter::Node<'tree>,
    current: &tree_sitter::Node<'tree>,
) -> Step<'tree> {
    let verified = match parent.kind() {
        "do_block" => is_block_key(current) || is_last_statement(parent, current),
        "body" => is_last_statement(parent, current),
        _ => current.kind() == "stab_clause" || is_last_statement(parent, current),
    };
    if verified {
        Step::Up(*parent)
    } else {
        Step::Done(true)
    }
}

/// True when the candidate call's value is discarded: walk up the tree;
/// only tail positions and value-consuming parents verify the call.
fn is_unused(call: tree_sitter::Node<'_>, source: &str) -> bool {
    let start = call.start_byte();
    let mut current = call;
    loop {
        let Some(parent) = current.parent() else {
            return false;
        };
        match parent.kind() {
            "call" => match call_step(&parent, &current, start, source) {
                Step::Up(node) => current = node,
                Step::Done(unused) => return unused,
            },
            "binary_operator" => {
                if operator_of(&parent, source) == "|>" && inside_right(&parent, &current) {
                    current = parent;
                } else {
                    return false;
                }
            }
            // Map/struct literals consume their values (upstream verifies
            // every non-list/tuple call node); lists and tuples stay
            // transparent so tail position decides.
            "unary_operator" | "map" => return false,
            "do_block" | "body" | "else_block" | "rescue_block" | "catch_block" | "after_block" => {
                match block_step(&parent, &current) {
                    Step::Up(node) => current = node,
                    Step::Done(unused) => return unused,
                }
            }
            _ => current = parent,
        }
    }
}

/// Head identifier of a `call` node, if any.
fn call_head(node: &tree_sitter::Node<'_>, source: &str) -> Option<String> {
    let head = node.child(0)?;
    if head.kind() != "identifier" {
        return None;
    }
    node_text(source, &head).map(str::to_owned)
}

/// True when `node` sits inside the `arguments` child of `call`.
fn inside_arguments(call: &tree_sitter::Node<'_>, node: &tree_sitter::Node<'_>) -> bool {
    let mut cursor = call.walk();
    call.children(&mut cursor)
        .filter(|child| child.kind() == "arguments")
        .any(|arguments| {
            arguments.start_byte() <= node.start_byte() && node.end_byte() <= arguments.end_byte()
        })
}

/// Operator text of a `binary_operator` node.
fn operator_of<'src>(node: &tree_sitter::Node<'_>, source: &'src str) -> &'src str {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| !child.is_named())
        .and_then(|operator| node_text(source, &operator))
        .unwrap_or("")
}

/// True when `node` is inside the right-hand side of a pipe.
fn inside_right(parent: &tree_sitter::Node<'_>, node: &tree_sitter::Node<'_>) -> bool {
    let mut cursor = parent.walk();
    let mut named = parent
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named);
    let (Some(_left), Some(right)) = (named.next(), named.last()) else {
        return false;
    };
    right.start_byte() <= node.start_byte()
}

/// True when `node` is a `rescue`/`catch`/`after`/`else` block container.
fn is_block_key(node: &tree_sitter::Node<'_>) -> bool {
    matches!(
        node.kind(),
        "else_block" | "rescue_block" | "catch_block" | "after_block"
    )
}

/// True when `node` is the last statement-like child of a sequence
/// (`do_block`, stab `body`, or block container).
fn is_last_statement(parent: &tree_sitter::Node<'_>, node: &tree_sitter::Node<'_>) -> bool {
    statements(parent).is_some_and(|last| last.id() == node.id())
}

/// Last statement-like child: named children excluding block keywords,
/// sub-blocks, comments, and (for `do_block`) the `do`/`end` markers.
fn statements<'tree>(parent: &tree_sitter::Node<'tree>) -> Option<tree_sitter::Node<'tree>> {
    let mut cursor = parent.walk();
    parent
        .children(&mut cursor)
        .filter(|child| {
            child.is_named()
                && !is_block_key(child)
                && !matches!(
                    child.kind(),
                    "do" | "end" | "else" | "rescue" | "catch" | "after" | "comment"
                )
        })
        .last()
}

/// True when the `try` call's `after` block contains `byte`.
fn after_contains(try_call: &tree_sitter::Node<'_>, byte: usize) -> bool {
    let mut cursor = try_call.walk();
    try_call
        .children(&mut cursor)
        .filter(|child| child.kind() == "do_block")
        .flat_map(|block| {
            let mut inner = block.walk();
            block
                .children(&mut inner)
                .filter(|child| child.kind() == "after_block")
                .collect::<Vec<_>>()
        })
        .any(|after| after.start_byte() <= byte && byte < after.end_byte())
}

/// True when the candidate sits in the `do`, `rescue`, or `catch` tail of
/// a `def` node.
fn def_tail_contains(def: &tree_sitter::Node<'_>, byte: usize) -> bool {
    let mut cursor = def.walk();
    def.children(&mut cursor)
        .filter(|child| child.kind() == "do_block")
        .any(|block| {
            contains(statements(&block).as_ref(), byte)
                || ["rescue_block", "catch_block"].iter().any(|kind| {
                    let mut inner = block.walk();
                    block
                        .children(&mut inner)
                        .filter(|child| child.kind() == *kind)
                        .any(|sub| contains(block_tail(&sub).as_ref(), byte))
                })
        })
}

/// Tail call of a `rescue`/`catch` container: last stab body statement,
/// or the last direct statement when there are no stabs.
fn block_tail<'tree>(block: &tree_sitter::Node<'tree>) -> Option<tree_sitter::Node<'tree>> {
    let mut cursor = block.walk();
    let stabs = block
        .children(&mut cursor)
        .filter(|child| child.kind() == "stab_clause");
    if let Some(last) = stabs.last() {
        let mut inner = last.walk();
        return last
            .children(&mut inner)
            .filter(|child| child.kind() == "body")
            .find_map(|body| statements(&body));
    }
    statements(block)
}

/// True when `node` spans `byte`.
fn contains(node: Option<&tree_sitter::Node<'_>>, byte: usize) -> bool {
    node.is_some_and(|inner| inner.start_byte() <= byte && byte < inner.end_byte())
}

/// Source slice for a node; `None` on invalid boundaries.
fn node_text<'src>(source: &'src str, node: &tree_sitter::Node<'_>) -> Option<&'src str> {
    source.get(node.start_byte()..node.end_byte())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params(raw: &str) -> BTreeMap<String, String> {
        let mut params = BTreeMap::new();
        params.insert("modules".to_owned(), raw.to_owned());
        params
    }
    #[test]
    fn conservative_empty() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("x = 1\n"), &BTreeMap::new()).is_empty()
        );
    }
    #[test]
    fn reports_configured_violation_with_custom_message() {
        let params = params(
            r#"[["Elixir.Map",["get","take"],"My special issue message"],["Elixir.Keywords",["get","fetch"]]]"#,
        );
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    x = parameter1 + parameter2\n\n    Map.take(parameter1, x)\n\n    parameter1\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (5, Some(5)));
        assert_eq!(findings[0].message, "My special issue message");
        assert_eq!(findings[0].trigger, Trigger::Text("Map.take".to_owned()));
    }
    #[test]
    fn reports_module_only_config_with_default_message() {
        let params = params(r#"[["Elixir.MyModule","all"]]"#);
        let src = "defmodule CredoSampleModule do\n  def some_function(p) do\n    MyModule.transform(p)\n    :ok\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "There should be no unused return values for `MyModule` functions."
        );
    }
    #[test]
    fn accepts_config_parser_tuple_encoding() {
        let params = params(r#"[{"tuple":["Elixir.MyModule","all"]}]"#);
        let src = "defmodule M do\n  def f(p) do\n    MyModule.transform(p)\n    :ok\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params).len(),
            1
        );
    }
    #[test]
    fn respects_function_lists() {
        let params = params(r#"[["Elixir.Map",["get","fetch"]]]"#);
        let src = "defmodule M do\n  def f(p) do\n    Map.values(p)\n    :ok\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn broken_source_stays_clean() {
        let params = params(r#"[["Elixir.Map","all"]]"#);
        assert!(check_prepared(&crate::batch::Prepared::lazy("def foo( do\n"), &params).is_empty());
    }
}
