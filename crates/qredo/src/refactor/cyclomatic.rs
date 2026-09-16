use crate::Finding;
use std::collections::BTreeMap;

/// `EX4006`: cyclomatic complexity approximation.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let max_c: usize = params
        .get("max_complexity")
        .and_then(|v| v.parse().ok())
        .unwrap_or(9);
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let depths = line_depths(&lines);
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !is_def(trimmed) {
            continue;
        }
        let Some(name) = def_name(trimmed) else {
            continue;
        };
        if name == "__using__" {
            continue;
        }
        let span = def_span(&lines, &depths, idx);
        let complexity = 1 + span_complexity(&lines, &depths, &span);
        if complexity > max_c {
            findings.push(
                Finding::with_trigger(
                    idx + 1,
                    credo_column(line, &name),
                    format!(
                        "Function is too complex (cyclomatic complexity is {complexity}, max is {max_c})."
                    ),
                    name,
                )
                .with_severity(crate::check_meta::severity_count(complexity, max_c)),
            );
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

fn is_def(trimmed: &str) -> bool {
    ["def ", "defp ", "defmacro "]
        .iter()
        .any(|op| trimmed.starts_with(op))
}

fn def_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.split_whitespace().nth(1)?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Depth before each line: block `do`/`fn` open, `end` closes.
fn line_depths(lines: &[&str]) -> Vec<usize> {
    let mut depths = Vec::with_capacity(lines.len());
    let mut depth = 0_usize;
    for line in lines {
        depths.push(depth);
        depth += count_word(line, "do") + count_word(line, "fn");
        depth = depth.saturating_sub(count_word(line, "end"));
    }
    depths
}

/// Lines of the function body (single line for `, do:` definitions).
fn def_span(lines: &[&str], depths: &[usize], from: usize) -> Vec<usize> {
    let base = depths[from];
    let mut span = vec![from];
    if depth_after(lines, depths, from) == base {
        return span;
    }
    for idx in from + 1..lines.len() {
        span.push(idx);
        if depth_after(lines, depths, idx) == base {
            break;
        }
    }
    span
}

fn depth_after(lines: &[&str], depths: &[usize], idx: usize) -> usize {
    let line = lines[idx];
    (depths[idx] + count_word(line, "do") + count_word(line, "fn"))
        .saturating_sub(count_word(line, "end"))
}

fn span_complexity(lines: &[&str], depths: &[usize], span: &[usize]) -> usize {
    let mut total = 0_usize;
    for word in ["if", "unless", "for", "try"] {
        total += span
            .iter()
            .map(|idx| count_keyword(lines[*idx], word))
            .sum::<usize>();
    }
    total += count_operator(lines, span, "&&");
    total += count_operator(lines, span, "||");
    for word in ["and", "or"] {
        total += span
            .iter()
            .map(|idx| count_keyword(lines[*idx], word))
            .sum::<usize>();
    }
    for word in ["case", "cond"] {
        total += case_clauses(lines, depths, span, word);
    }
    total
}

fn count_operator(lines: &[&str], span: &[usize], op: &str) -> usize {
    span.iter().map(|idx| lines[*idx].matches(op).count()).sum()
}

/// `->` clauses inside each `case`/`cond` block in the span: only the
/// block's own lines count (up to the depth drop at its `end`), so
/// sibling blocks at the same depth never share clauses.
fn case_clauses(lines: &[&str], depths: &[usize], span: &[usize], word: &str) -> usize {
    let mut total = 0_usize;
    for idx in span {
        if count_keyword(lines[*idx], word) == 0 {
            continue;
        }
        let level = depth_after(lines, depths, *idx);
        // Own block: lines from the keyword until the depth drops back
        // below the body level at the block's `end`.
        let mut block_end = *idx;
        for inner in span {
            if *inner <= *idx {
                continue;
            }
            if depths[*inner] < level {
                break;
            }
            block_end = *inner;
        }
        for inner in span {
            if *inner >= *idx && *inner <= block_end && depths[*inner] == level {
                total += clause_arrows(lines[*inner]);
            }
        }
    }
    total
}

/// `->` occurrences opening `case`/`cond` clauses: `fn` stabs
/// (`fn x -> ... end`) at the same depth never count.
fn clause_arrows(line: &str) -> usize {
    let mut count = 0_usize;
    let mut from = 0_usize;
    while let Some(rel) = line[from..].find("->") {
        let pos = from + rel;
        if !is_fn_arrow(line, pos) {
            count += 1;
        }
        from = pos + 2;
    }
    count
}

/// True when the `->` at byte `pos` opens a `fn` stab: the nearest
/// block word before it on the line is `fn` rather than `do`/`end`.
fn is_fn_arrow(line: &str, pos: usize) -> bool {
    let mut best: Option<(usize, &str)> = None;
    for word in ["fn", "do", "end"] {
        for hit in word_positions(line, word) {
            if hit < pos && best.is_none_or(|(at, _)| hit >= at) {
                best = Some((hit, word));
            }
        }
    }
    matches!(best, Some((_, "fn")))
}

/// Byte offsets of whole-word occurrences (excluding `do:` keywords).
fn word_positions(line: &str, needle: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0_usize;
    while idx + needle.len() <= bytes.len() {
        if line.get(idx..).is_some_and(|rest| rest.starts_with(needle))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + needle.len())
            && prev_allows(bytes, idx)
            && bytes.get(idx + needle.len()) != Some(&b':')
        {
            out.push(idx);
            idx += needle.len();
        } else {
            idx += 1;
        }
    }
    out
}

fn count_keyword(line: &str, needle: &str) -> usize {
    if needle == "do" {
        return count_bare_do(line);
    }
    count_word(line, needle)
}

fn count_word(line: &str, needle: &str) -> usize {
    let bytes = line.as_bytes();
    let mut count = 0_usize;
    let mut idx = 0_usize;
    while idx + needle.len() <= bytes.len() {
        if line.get(idx..).is_some_and(|rest| rest.starts_with(needle))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + needle.len())
            && prev_allows(bytes, idx)
            && bytes.get(idx + needle.len()) != Some(&b':')
        {
            count += 1;
            idx += needle.len();
        } else {
            idx += 1;
        }
    }
    count
}

/// `do` keywords that open a block (`do:` keyword arguments do not).
fn count_bare_do(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut count = 0_usize;
    let mut idx = 0_usize;
    while idx + 2 <= bytes.len() {
        if line.get(idx..).is_some_and(|rest| rest.starts_with("do"))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + 2)
            && bytes.get(idx + 2) != Some(&b':')
        {
            count += 1;
            idx += 2;
        } else {
            idx += 1;
        }
    }
    count
}

fn word_boundary(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 || idx >= bytes.len() {
        return true;
    }
    !is_name_byte(bytes[idx]) || !is_name_byte(bytes[idx - 1])
}

fn prev_allows(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    prev != b'@' && prev != b':' && prev != b'.'
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

/// Credo `SourceFile.column/3`: 1-based column of `trigger` when surrounded by
/// whitespace, parens, commas or word boundaries; `None` otherwise.
fn credo_column(line: &str, trigger: &str) -> Option<usize> {
    if trigger.is_empty() {
        return None;
    }
    let lchars: Vec<char> = line.chars().collect();
    let tchars: Vec<char> = trigger.chars().collect();
    if lchars.len() < tchars.len() {
        return None;
    }
    for idx in 0..=lchars.len() - tchars.len() {
        if lchars[idx..idx + tchars.len()] != tchars[..] {
            continue;
        }
        let before_ok = if idx == 0 {
            is_word(tchars[0])
        } else {
            before_ok(lchars[idx - 1], tchars[0])
        };
        let after = idx + tchars.len();
        let after_ok = if after == lchars.len() {
            is_word(tchars[tchars.len() - 1])
        } else {
            after_ok(tchars[tchars.len() - 1], lchars[after])
        };
        if before_ok && after_ok {
            return Some(idx + 1);
        }
    }
    None
}

fn before_ok(prev: char, first: char) -> bool {
    prev.is_whitespace()
        || prev == '('
        || prev == ')'
        || prev == ','
        || is_word(prev) != is_word(first)
}

fn after_ok(last: char, next: char) -> bool {
    next.is_whitespace()
        || next == '('
        || next == ')'
        || next == ','
        || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simple_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo(x), do: x\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_complex() {
        let mut src = String::from("def foo(x) do\n");
        for _ in 0..12 {
            src.push_str("  if x, do: 1\n");
        }
        src.push_str("end\n");
        assert!(!check_prepared(&crate::batch::Prepared::lazy(&src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn reports_trigger_column_like_upstream() {
        // EX4006.upstream.expected-2: column points at the function name.
        let src = "def some_function do\n  if x == 0, do: x = 1\nend\n";
        let params: BTreeMap<String, String> = [("max_complexity".to_owned(), "1".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (1, Some(5)));
        assert_eq!(
            findings[0].message,
            "Function is too complex (cyclomatic complexity is 2, max is 1)."
        );
    }
    #[test]
    fn fn_stabs_do_not_count_as_clauses() {
        // Native reference: complexity 7 (def 1 + case 5 + `and` 1);
        // `fn x -> ... end` stabs at clause depth are not case clauses.
        let src = "defmodule M do\n  def max_depth(ast, depth) do\n    case ast do\n      {node, _, args} when node in @branch_nodes and is_list(args) ->\n        1 + Enum.reduce(args, depth, fn arg, inner -> max(inner, max_depth(arg, depth)) end)\n      {_, _, args} when is_list(args) ->\n        Enum.reduce(args, depth, fn arg, inner -> max(inner, max_depth(arg, depth)) end)\n      {key, value} when is_atom(key) ->\n        max_depth(value, depth)\n      list when is_list(list) ->\n        Enum.reduce(list, depth, fn item, inner -> max(inner, max_depth(item, depth)) end)\n      _leaf ->\n        depth\n    end\n  end\nend\n";
        let params: BTreeMap<String, String> = [("max_complexity".to_owned(), "8".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
    #[test]
    fn sibling_case_blocks_do_not_share_clauses() {
        // Native reference: complexity 8 (def 1 + cond 3 + case 2 + case
        // 2); same-depth `->` past a block's `end` must not count.
        let src = "defmodule M do\n  def f(ref) do\n    cond do\n      a?(ref) ->\n        case g(ref) do\n          nil -> :e\n          x -> {:ok, x}\n        end\n      b?(ref) ->\n        case h(ref) do\n          nil -> :e\n          x -> {:ok, x}\n        end\n      true -> :err\n    end\n  end\nend\n";
        let params: BTreeMap<String, String> = [("max_complexity".to_owned(), "7".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Function is too complex (cyclomatic complexity is 8, max is 7)."
        );
    }
}
