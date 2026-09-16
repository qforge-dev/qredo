use crate::Finding;
use std::collections::BTreeMap;

/// `EX4022`: perceived complexity (case clauses weigh 0.3, message uses CC).
///
/// Ports `Credo.Check.Refactor.PerceivedComplexity`: per-function score is
/// `round(1 + double-condition ops + cond clauses + 0.3 * case clauses)`;
/// `defmacro __using__` is exempt. Upstream ships zero test cases, so the
/// contracts below are verified against native `run` probes on the pinned
/// checkout (see commit message), not transcribed upstream assertions.
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
        let complexity = span_complexity(&lines, &depths, &span);
        if complexity > max_c {
            findings.push(
                Finding::with_trigger(
                    idx + 1,
                    credo_column(line, &name),
                    format!("Function is too complex (CC is {complexity}, max is {max_c})."),
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

/// Rounded score: double-condition ops and `cond` clauses add 1 each,
/// `case` clauses add 0.3 each (upstream `@op_complexity_map`). Tenths
/// arithmetic matches Elixir `round/1` for these non-negative scores.
fn span_complexity(lines: &[&str], depths: &[usize], span: &[usize]) -> usize {
    let mut base = 1_usize;
    for word in ["if", "unless", "for", "try"] {
        base += span
            .iter()
            .map(|idx| count_keyword(lines[*idx], word))
            .sum::<usize>();
    }
    base += count_operator(lines, span, "&&");
    base += count_operator(lines, span, "||");
    for word in ["and", "or"] {
        base += span
            .iter()
            .map(|idx| count_keyword(lines[*idx], word))
            .sum::<usize>();
    }
    base += case_clauses(lines, depths, span, "cond");
    let tenths = 10_usize
        .saturating_mul(base)
        .saturating_add(3_usize.saturating_mul(case_clauses(lines, depths, span, "case")));
    (tenths + 5) / 10
}

fn count_operator(lines: &[&str], span: &[usize], op: &str) -> usize {
    span.iter().map(|idx| lines[*idx].matches(op).count()).sum()
}

/// `->` clauses directly inside each `case`/`cond` block in the span.
fn case_clauses(lines: &[&str], depths: &[usize], span: &[usize], word: &str) -> usize {
    let mut total = 0_usize;
    for idx in span {
        if count_keyword(lines[*idx], word) == 0 {
            continue;
        }
        let level = depth_after(lines, depths, *idx);
        for inner in span {
            if *inner >= *idx && depths[*inner] == level {
                total += lines[*inner].matches("->").count();
            }
        }
    }
    total
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
    fn nine_ifs_violate_with_cc_message() {
        // EX4022.nine-ifs: def(1) + 9 ifs = 10 > 9; native reports
        // line 2 col 6 trigger foo with CC message.
        let mut src = String::from("defmodule M do\n  def foo(x) do\n");
        for var in ["a", "b", "c", "d", "e", "f", "g", "h", "i"] {
            src.push_str("    if ");
            src.push_str(var);
            src.push_str(", do: 1\n");
        }
        src.push_str("  end\nend\n");
        let findings = check_prepared(&crate::batch::Prepared::lazy(&src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (2, Some(7)));
        assert_eq!(
            findings[0].message,
            "Function is too complex (CC is 10, max is 9)."
        );
    }

    #[test]
    fn eight_ifs_stay_clean_at_default_max() {
        // EX4022.eight-ifs: 1 + 8 = 9 is not > 9.
        let mut src = String::from("defmodule M do\n  def foo(x) do\n");
        for var in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            src.push_str("    if ");
            src.push_str(var);
            src.push_str(", do: 1\n");
        }
        src.push_str("  end\nend\n");
        assert!(check_prepared(&crate::batch::Prepared::lazy(&src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn custom_max_triggers_earlier() {
        // EX4022.custom-max: native reports CC 3 > max 2 at 2:6 shape.
        let src = "defmodule M do\n  def foo(x) do\n    if a, do: 1\n    if b, do: 2\n  end\nend\n";
        let params: BTreeMap<String, String> = [("max_complexity".to_owned(), "2".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Function is too complex (CC is 3, max is 2)."
        );
    }

    #[test]
    fn using_macro_is_exempt() {
        // EX4022.using-exempt: native reports nothing even at max 1.
        let src = "defmodule M do\n  defmacro __using__(_) do\n    if a, do: 1\n    if b, do: 2\n    if c, do: 3\n  end\nend\n";
        let params: BTreeMap<String, String> = [("max_complexity".to_owned(), "1".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn case_clauses_weigh_point_three() {
        // EX4022.case-weight: 3-clause case scores round(1 + 0.9) = 2,
        // clean at max 2 where cyclomatic (1 + 3 = 4) would violate.
        let src = "defmodule M do\n  def foo(x) do\n    case x do\n      1 -> :a\n      2 -> :b\n      3 -> :c\n    end\n  end\nend\n";
        let params: BTreeMap<String, String> = [("max_complexity".to_owned(), "2".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
        let cyclo =
            super::super::cyclomatic::check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(cyclo.len(), 1);
    }
}
