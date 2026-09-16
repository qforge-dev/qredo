use crate::Finding;
use std::collections::BTreeMap;

/// `EX5006`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    _params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        check_line(line, idx + 1, &mut findings);
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

fn check_line(line: &str, line_no: usize, findings: &mut Vec<Finding>) {
    let mut search = 0_usize;
    while search < line.len() {
        let Some(rel) = line[search..].find("IO.inspect") else {
            break;
        };
        let base = search + rel;
        search = base + 1;
        let (start, trigger) = if line[..base].ends_with("Elixir.") && dot_before_ok(line, base) {
            (base - "Elixir.".len(), "Elixir.IO.inspect")
        } else {
            (base, "IO.inspect")
        };
        if !before_ok(line, start) || !after_ok(line, base + "IO.inspect".len()) {
            continue;
        }
        if arg_count(line, base + "IO.inspect".len()) >= 3 {
            continue;
        }
        findings.push(Finding::with_trigger(
            line_no,
            Some(col_of(line, start)),
            "There should be no calls to `IO.inspect/1`.",
            trigger.to_owned(),
        ));
    }
}

/// The char before the module must not continue another name.
fn before_ok(line: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    !line[..start]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// `Elixir.` must itself start at a name boundary.
fn dot_before_ok(line: &str, base: usize) -> bool {
    before_ok(line, base - "Elixir.".len())
}

/// `inspect` must not continue into a longer name.
fn after_ok(line: &str, end: usize) -> bool {
    if end >= line.len() {
        return true;
    }
    !line[end..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

/// Number of call arguments (`< 3` triggers). Bare `IO.inspect` (piped) has
/// zero; space-separated arguments run to the next pipe or line end.
fn arg_count(line: &str, end: usize) -> usize {
    let rest = line[end..].trim_start();
    if let Some(inner) = rest.strip_prefix('(') {
        return paren_arity(inner);
    }
    let segment = pipe_segment(rest);
    if segment.trim().is_empty() {
        return 0;
    }
    split_args(&segment).len()
}

/// Text up to the next top-level pipe (or the line end).
fn pipe_segment(rest: &str) -> String {
    let mut depth = 0_usize;
    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(chr) = chars.next() {
        if chr == '|' && chars.peek() == Some(&'>') && depth == 0 {
            break;
        }
        match chr {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        out.push(chr);
    }
    out
}

/// Arity of a parenthesised argument list (without the opening paren).
fn paren_arity(rest: &str) -> usize {
    let mut depth = 1_usize;
    let mut commas = 0_usize;
    let mut empty = true;
    for chr in rest.chars() {
        match chr {
            '(' | '[' | '{' => {
                depth += 1;
                empty = false;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                empty = false;
            }
            ',' if depth == 1 => commas += 1,
            ' ' | '\t' => {}
            _ => empty = false,
        }
    }
    if empty { 0 } else { commas + 1 }
}

/// Split top-level comma-separated arguments.
fn split_args(segment: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (idx, chr) in segment.char_indices() {
        match chr {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&segment[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(&segment[start..]);
    parts
}

/// Column (1-based, characters) of the byte offset (which must be a boundary).
fn col_of(line: &str, byte_pos: usize) -> usize {
    line[..byte_pos].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("x = 1\n"), &BTreeMap::new()).is_empty()
        );
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("IO.inspect(x)\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn three_args_are_clean() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1) do\n    IO.inspect(:stderr, parameter1, [])\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
}
