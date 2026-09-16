use crate::Finding;

/// `EX5021`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    run(prepared)
}

const CHECKED_MODULE: &str = "Path";
const ISSUE_MESSAGE: &str = "There should be no unused return values for `Path` functions.";
const RETURNING_FUNS: Option<&[&str]> = None;

/// Verified use (`false`) or unused return value (`true`) for one candidate.
fn run(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let facts = prepared.facts();
    let Some(tree) = prepared.tree() else {
        return Vec::new();
    };
    let bytes = source.as_bytes();
    let lines: Vec<&str> = source.split('\n').collect();
    let filter = super::unused_return::CandidateFilter {
        module: CHECKED_MODULE,
        funs: RETURNING_FUNS,
        ignored: &[],
        atoms: false,
    };
    let mut findings: Vec<Finding> =
        super::unused_return::candidate_spans_for(source, facts, &filter)
            .into_iter()
            .filter_map(|(start, end, fun)| {
                super::unused_return::recover_call(tree, start, end).map(|call| (call, fun))
            })
            .filter(|(call, _)| is_unused(*call, bytes))
            .map(|(call, fun)| {
                let (line, column) = line_column(&lines, call.start_position());
                Finding::with_trigger(
                    line,
                    Some(column),
                    ISSUE_MESSAGE,
                    format!("{CHECKED_MODULE}.{fun}"),
                )
            })
            .collect();
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Decision {
    Used,
    Unused,
    Defer,
}

/// Walk from the candidate toward the root, mirroring the reference
/// verify rules: assignments, pipe targets, condition heads and call
/// arguments mark the value used; non-final block statements discard it.
/// Containment is always tested against the original candidate.
fn is_unused(call: tree_sitter::Node, bytes: &[u8]) -> bool {
    let mut current = call.parent();
    while let Some(parent) = current {
        match decide(parent, call, bytes) {
            Decision::Used => return false,
            Decision::Unused => return true,
            Decision::Defer => {}
        }
        current = parent.parent();
    }
    false
}

fn decide(parent: tree_sitter::Node, child: tree_sitter::Node, bytes: &[u8]) -> Decision {
    match parent.kind() {
        "call" => decide_call(parent, child, bytes),
        "binary_operator" => decide_binary(parent, child, bytes),
        "unary_operator" | "access_call" => Decision::Used,
        "do_block" | "else_block" | "after_block" | "body" => decide_sequence(parent, child),
        _ => Decision::Defer,
    }
}

/// Remote/local calls use their arguments (including `do` sections); only
/// the reference condition/loop heads, `try`, definitions and module
/// scaffolding behave differently.
fn decide_call(parent: tree_sitter::Node, child: tree_sitter::Node, bytes: &[u8]) -> Decision {
    let name = call_name(parent, bytes);
    match name {
        Some("def" | "defp" | "defmacro") => decide_def(parent, child, bytes),
        Some("if" | "unless" | "case" | "for" | "quote") => {
            if head_contains(parent, child) {
                Decision::Used
            } else {
                Decision::Defer
            }
        }
        Some("try") => {
            if section_contains(parent, "after_block", child) {
                Decision::Unused
            } else {
                Decision::Defer
            }
        }
        Some(
            "defmodule" | "defprotocol" | "defimpl" | "defguard" | "defguardp" | "defdelegate",
        ) => Decision::Defer,
        _ => {
            if field_contains(parent, "target", child) {
                Decision::Defer
            } else {
                Decision::Used
            }
        }
    }
}

/// `|>` uses every segment but the last; every other operator consumes its
/// operands exactly like the reference generic-call rule.
fn decide_binary(parent: tree_sitter::Node, child: tree_sitter::Node, bytes: &[u8]) -> Decision {
    if is_pipe(parent, bytes) {
        if field_contains(parent, "right", child) {
            Decision::Defer
        } else {
            Decision::Used
        }
    } else {
        Decision::Used
    }
}

/// Statement sequences keep only the final statement alive; branch lists of
/// `->` clauses are transparent like the reference `->`/`fn` rules, and so
/// is the surrounding sequence for values judged by their own section block.
/// Comments are separate tokens in the reference AST, so they never count
/// as statements here either.
fn decide_sequence(parent: tree_sitter::Node, child: tree_sitter::Node) -> Decision {
    let named = code_children(parent);
    if named.is_empty() || named.iter().all(|item| item.kind() == "stab_clause") {
        return Decision::Defer;
    }
    if named
        .iter()
        .any(|item| is_section(item.kind()) && contains(*item, child))
    {
        return Decision::Defer;
    }
    let statements: Vec<_> = named
        .iter()
        .filter(|item| !is_section(item.kind()))
        .collect();
    match statements.last() {
        Some(last) if contains(**last, child) => Decision::Defer,
        _ => Decision::Unused,
    }
}

/// Named children excluding comments (the reference AST has no comment nodes).
fn code_children(parent: tree_sitter::Node) -> Vec<tree_sitter::Node> {
    let mut cursor = parent.walk();
    parent
        .named_children(&mut cursor)
        .filter(|item| item.kind() != "comment")
        .collect()
}

fn is_section(kind: &str) -> bool {
    matches!(
        kind,
        "else_block" | "rescue_block" | "catch_block" | "after_block"
    )
}

/// The reference verifies a definition when the candidate feeds the final
/// statement of its `do`, `rescue` or `catch` section.
fn decide_def(parent: tree_sitter::Node, child: tree_sitter::Node, bytes: &[u8]) -> Decision {
    for section in ["do_block", "rescue_block", "catch_block"] {
        if section_last_uses(parent, section, child) {
            return Decision::Used;
        }
    }
    if keyword_value_uses(parent, child, bytes) {
        return Decision::Used;
    }
    Decision::Unused
}

fn section_last_uses(parent: tree_sitter::Node, section: &str, child: tree_sitter::Node) -> bool {
    if section == "do_block" {
        let Some(block) = child_kind(parent, "do_block") else {
            return false;
        };
        return block_last_uses(block, child);
    }
    let Some(item) = call_section(parent, section) else {
        return false;
    };
    let named = code_children(item);
    let Some(last) = named.last() else {
        return false;
    };
    if last.kind() == "stab_clause" {
        stab_body_last_contains(*last, child)
    } else {
        contains(*last, child)
    }
}

/// Last statement of a `do` section, ignoring trailing section blocks.
fn block_last_uses(block: tree_sitter::Node, child: tree_sitter::Node) -> bool {
    let statements: Vec<_> = code_children(block)
        .into_iter()
        .filter(|item| !is_section(item.kind()))
        .collect();
    match statements.last() {
        Some(last) if last.kind() == "stab_clause" => stab_body_last_contains(*last, child),
        Some(last) => contains(*last, child),
        None => false,
    }
}

fn stab_body_last_contains(stab: tree_sitter::Node, child: tree_sitter::Node) -> bool {
    let Some(body) = stab.child_by_field_name("right") else {
        return contains(stab, child);
    };
    match code_children(body).last() {
        Some(last) => contains(*last, child),
        None => false,
    }
}

/// Single-expression `def f, do: value` style definitions.
fn keyword_value_uses(parent: tree_sitter::Node, child: tree_sitter::Node, bytes: &[u8]) -> bool {
    let Some(args) = child_kind(parent, "arguments") else {
        return false;
    };
    let Some(keywords) = child_kind(args, "keywords") else {
        return false;
    };
    for pair in code_children(keywords) {
        let key = pair
            .child_by_field_name("key")
            .map(|key| keyword_key(key, bytes));
        if matches!(key, Some("do" | "rescue" | "catch"))
            && pair
                .child_by_field_name("value")
                .is_some_and(|value| contains(value, child))
        {
            return true;
        }
    }
    false
}

/// Call name for plain `name(...)` calls; remote/dot targets yield `None`.
fn call_name<'a>(call: tree_sitter::Node, bytes: &'a [u8]) -> Option<&'a str> {
    let target = call.child_by_field_name("target")?;
    if target.kind() != "identifier" {
        return None;
    }
    Some(node_text(target, bytes))
}

fn field_contains(parent: tree_sitter::Node, field: &str, child: tree_sitter::Node) -> bool {
    parent
        .child_by_field_name(field)
        .is_some_and(|slot| contains(slot, child))
}

fn section_contains(parent: tree_sitter::Node, section: &str, child: tree_sitter::Node) -> bool {
    call_section(parent, section).is_some_and(|item| contains(item, child))
}

/// Section blocks (`else`/`rescue`/...) hang off the call's `do` block.
fn call_section<'a>(parent: tree_sitter::Node<'a>, section: &str) -> Option<tree_sitter::Node<'a>> {
    child_kind(child_kind(parent, "do_block")?, section)
}

fn child_kind<'a>(parent: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
    code_children(parent)
        .into_iter()
        .find(|item| item.kind() == kind)
}

/// Reference condition/loop head: positional `arguments`, excluding trailing
/// keyword options (`do:`/`into:` live in the last argument).
fn head_contains(parent: tree_sitter::Node, child: tree_sitter::Node) -> bool {
    let Some(args) = child_kind(parent, "arguments") else {
        return false;
    };
    code_children(args)
        .into_iter()
        .filter(|item| item.kind() != "keywords")
        .any(|item| contains(item, child))
}

fn contains(outer: tree_sitter::Node, inner: tree_sitter::Node) -> bool {
    outer.start_byte() <= inner.start_byte() && inner.end_byte() <= outer.end_byte()
}

/// Whether `node` is a `|>` pipeline step.
fn is_pipe(node: tree_sitter::Node, bytes: &[u8]) -> bool {
    node.child_by_field_name("operator")
        .is_some_and(|op| node_text(op, bytes) == "|>")
}

/// Keyword key text without decorations (`keys:`, trailing space).
fn keyword_key<'a>(key: tree_sitter::Node, bytes: &'a [u8]) -> &'a str {
    node_text(key, bytes).trim().trim_matches(':')
}

fn node_text<'a>(node: tree_sitter::Node, bytes: &'a [u8]) -> &'a str {
    bytes
        .get(node.start_byte()..node.end_byte())
        .and_then(|slot| std::str::from_utf8(slot).ok())
        .unwrap_or("")
}

/// 1-based `(line, column)` with character-based columns.
fn line_column(lines: &[&str], point: tree_sitter::Point) -> (usize, usize) {
    let text = lines.get(point.row).copied().unwrap_or("");
    let column = text
        .get(..point.column.min(text.len()))
        .map_or(1, |prefix| prefix.chars().count() + 1);
    (point.row + 1, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = Path.join(a, b)\n")).is_empty());
    }
    #[test]
    fn reports() {
        let source = "defmodule M do\n  def f(y) do\n    Path.join(a, b)\n\n    y\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(source)).len(),
            1
        );
    }
    #[test]
    fn piped_result_is_clean() {
        let source = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    Path.join(parameter1)\n    |> some_where\n\n    parameter1\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(source)).is_empty());
    }
    #[test]
    fn buried_call_reports_with_column() {
        let source = "defmodule CredoSampleModule do\n  defp print_issue(issue) do\n    if issue.column do\n      IO.puts \".\"\n    else\n      [:this_goes_nowhere, Path.join(w, \",\")]\n    end\n\n    IO.puts\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(source));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 6);
        assert_eq!(findings[0].column, Some(28));
    }
}
