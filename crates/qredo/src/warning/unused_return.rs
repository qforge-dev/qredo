use crate::{Finding, Trigger};

/// Shared unused-return engine behind `EX5023` (`String`) and `EX5024`
/// (`Tuple`): flags calls whose value is discarded. The two checks were
/// identical copies; this module keeps one engine with fused traversal.
/// Unused-call findings for one dotted module name over shared facts.
pub(crate) fn check_module(prepared: &crate::batch::Prepared<'_>, target: &str) -> Vec<Finding> {
    let source = prepared.source();
    let facts = prepared.facts();
    let Some(tree) = prepared.tree() else {
        return Vec::new();
    };
    let filter = CandidateFilter {
        module: target,
        funs: None,
        ignored: &[],
        atoms: true,
    };
    let mut findings: Vec<Finding> = candidate_spans_for(source, facts, &filter)
        .into_iter()
        .filter_map(|(start, end, _)| recover_call(tree, start, end))
        .filter(|call| is_unused(*call, source))
        .map(|call| {
            let (line, column) = line_col(source, dot_start(&call));
            let trigger = dot_text(source, &call).unwrap_or(target);
            Finding {
                line,
                column: Some(column),
                message: format!(
                    "There should be no unused return values for `{target}` functions."
                ),
                trigger: Trigger::Text(trigger.to_owned()),
                severity: None,
            }
        })
        .collect();
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// Module/function filter for candidate discovery.
pub(crate) struct CandidateFilter<'a> {
    pub module: &'a str,
    pub funs: Option<&'a [&'a str]>,
    pub ignored: &'a [String],
    pub atoms: bool,
}

/// Candidate call spans for one filter: remote calls with matching module
/// text inside definitions, plus the function name. Alias receivers always
/// qualify; atom receivers qualify only with `atoms` (colon trimmed).
/// Shared by the per-module unused checks; each runs its own discard
/// engine over the recovered nodes.
pub(crate) fn candidate_spans_for(
    source: &str,
    facts: &crate::facts::Facts,
    filter: &CandidateFilter<'_>,
) -> Vec<(u32, u32, String)> {
    let defs: Vec<(usize, usize)> = facts
        .def_ranges
        .iter()
        .map(|(start, end)| (*start as usize, *end as usize))
        .collect();
    let mut spans = Vec::new();
    for call in &facts.calls {
        let Some(crate::facts::HeadFact::Remote {
            mod_start,
            mod_end,
            mod_kind,
            fun_start,
            fun_end,
        }) = &call.head
        else {
            continue;
        };
        let module = match mod_kind {
            crate::facts::ModKind::Alias => slice(source, *mod_start, *mod_end),
            crate::facts::ModKind::Atom if filter.atoms => {
                slice(source, *mod_start, *mod_end).map(|text| text.trim_start_matches(':'))
            }
            _ => None,
        };
        let (Some(module), Some(fun)) = (module, slice(source, *fun_start, *fun_end)) else {
            continue;
        };
        if module != filter.module {
            continue;
        }
        if let Some(funs) = filter.funs
            && !funs.contains(&fun)
        {
            continue;
        }
        if filter.ignored.iter().any(|name| name == fun) {
            continue;
        }
        if within(&defs, call.start as usize) {
            spans.push((call.start, call.end, fun.to_owned()));
        }
    }
    spans
}

/// The call node for a candidate span: deepest covering node ascended to
/// the recorded call span. Always terminates: the span came from a call.
pub(crate) fn recover_call(
    tree: &tree_sitter::Tree,
    start: u32,
    end: u32,
) -> Option<tree_sitter::Node<'_>> {
    let mut node = tree
        .root_node()
        .descendant_for_byte_range(start as usize, end as usize)?;
    loop {
        if node.kind() == "call"
            && node.start_byte() == start as usize
            && node.end_byte() == end as usize
        {
            return Some(node);
        }
        node = node.parent()?;
    }
}

/// Start byte of the call's dot target (trigger position).
pub(crate) fn dot_start(call: &tree_sitter::Node<'_>) -> usize {
    let mut cursor = call.walk();
    call.children(&mut cursor)
        .find(|child| child.kind() == "dot")
        .map_or(call.start_byte(), |dot| dot.start_byte())
}

/// Text of the call's dot target for the trigger.
pub(crate) fn dot_text<'src>(source: &'src str, call: &tree_sitter::Node<'_>) -> Option<&'src str> {
    let mut cursor = call.walk();
    call.children(&mut cursor)
        .find(|child| child.kind() == "dot")
        .and_then(|dot| source.get(dot.start_byte()..dot.end_byte()))
}

/// Source slice for fact spans; `None` on invalid boundaries.
fn slice(source: &str, start: u32, end: u32) -> Option<&str> {
    source.get(start as usize..end as usize)
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

/// True when `byte` lies in one of `ranges`.
fn within(ranges: &[(usize, usize)], byte: usize) -> bool {
    ranges
        .iter()
        .any(|(start, end)| *start <= byte && byte < *end)
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
        // `cond` verifies everything reaching it (native reference: 0
        // issues for conditions and single-expression bodies even in a
        // discarded cond); multi-statement non-tail bodies falsify at
        // their inner block before arriving here.
        if head == "cond" {
            return Step::Done(false);
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
    // Branch lists of `->` clauses are transparent like the reference
    // `->`/`fn` rules: neither the conditions nor single-expression
    // bodies decide use on their own.
    if only_stabs(parent) {
        return Step::Up(*parent);
    }
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

/// True when every named child is a `->` clause (comments excluded).
fn only_stabs(parent: &tree_sitter::Node<'_>) -> bool {
    let mut cursor = parent.walk();
    let items: Vec<_> = parent
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
        .collect();
    !items.is_empty() && items.iter().all(|item| item.kind() == "stab_clause")
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
            // Map/struct/dot values are consumed (upstream verifies every
            // non-list/tuple call node, including the `.` operator's
            // function argument: `String.trim(x).()` uses the result);
            // lists and tuples stay transparent so tail position decides.
            "unary_operator" | "map" | "dot" => return false,
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

/// 1-based `(line, column)` with the column counted in characters.
pub(crate) fn line_col(source: &str, byte: usize) -> (usize, usize) {
    let before = source.get(..byte).unwrap_or("");
    let line = before.bytes().filter(|&byte| byte == b'\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

/// Source slice for a node; `None` on invalid boundaries.
fn node_text<'src>(source: &'src str, node: &tree_sitter::Node<'_>) -> Option<&'src str> {
    source.get(node.start_byte()..node.end_byte())
}
