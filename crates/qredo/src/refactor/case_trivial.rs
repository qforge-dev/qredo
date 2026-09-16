use crate::{Finding, Trigger};

/// `EX4004`: `case` with only `true`/`false` branches.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let raw: Vec<&str> = source.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("case ")
            && let (t, f, other) = count_branches(&lines, idx)
            && t == 1
            && f == 1
            && other == 0
        {
            findings.push(Finding {
                line: idx + 1,
                column: fallback_column(raw[idx], "cond"),
                message: "Case statements should not only contain `true` and `false`.".to_owned(),
                trigger: Trigger::Text("cond".to_owned()),
                severity: None,
            });
        }
    }
    findings
}

/// Count `true ->`, `false ->` and other branches in a `case` body, starting
/// on the `case` line itself (one-liner bodies included). Nested blocks only
/// contribute at their own keyword depth.
fn count_branches(lines: &[&str], case_idx: usize) -> (usize, usize, usize) {
    let mut body = String::new();
    for (offset, line) in lines[case_idx..].iter().enumerate() {
        if offset == 0 {
            body.push_str(case_tail(line));
        } else {
            body.push('\n');
            body.push_str(line);
        }
    }
    scan_body(&body)
}

/// Text after the `case` line's body-opening `do` (empty when absent).
fn case_tail(line: &str) -> &str {
    let chars: Vec<char> = line.chars().collect();
    let mut depth = 0_usize;
    let mut idx = 0_usize;
    while idx < chars.len() {
        match chars[idx] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && word_at(&chars, idx, "do") && do_opens(&chars, idx) => {
                return byte_tail(line, &chars, idx + "do".len());
            }
            _ => {}
        }
        idx += 1;
    }
    ""
}

/// String tail from the `idx`-th char.
fn byte_tail<'a>(line: &'a str, chars: &[char], idx: usize) -> &'a str {
    let base: usize = chars[..idx.min(chars.len())]
        .iter()
        .map(|c| c.len_utf8())
        .sum();
    &line[base.min(line.len())..]
}

/// True for a whole word at `idx` (identifier boundaries both sides).
fn word_at(chars: &[char], idx: usize, word: &str) -> bool {
    let text: String = chars[idx..].iter().take(word.len()).collect();
    if text != word {
        return false;
    }
    if idx > 0 && is_name(chars[idx - 1]) {
        return false;
    }
    !chars.get(idx + word.len()).is_some_and(|c| is_name(*c))
}

fn is_name(char: char) -> bool {
    char.is_alphanumeric() || char == '_' || char == '?' || char == '!'
}

/// Scan a `case` body for branch heads, stopping at its closing `end`.
fn scan_body(body: &str) -> (usize, usize, usize) {
    let chars: Vec<char> = body.chars().collect();
    let (mut t, mut f, mut other) = (0, 0, 0);
    let mut depth = 0_usize;
    let mut kw = 0_usize;
    let mut idx = 0_usize;
    while idx < chars.len() {
        match chars[idx] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '-' if depth == 0 && chars.get(idx + 1) == Some(&'>') => {
                if kw == 0 {
                    let pattern = branch_pattern(&chars, idx);
                    if pattern == "true" {
                        t += 1;
                    } else if pattern == "false" {
                        f += 1;
                    } else {
                        other += 1;
                    }
                }
                idx += 1;
            }
            _ if depth == 0 && word_at(&chars, idx, "do") && do_opens(&chars, idx) => {
                kw += 1;
            }
            _ if depth == 0 && word_at(&chars, idx, "fn") && value_start(&chars, idx) => {
                kw += 1;
            }
            _ if depth == 0 && word_at(&chars, idx, "end") && value_start(&chars, idx) => {
                if kw == 0 {
                    break;
                }
                kw = kw.saturating_sub(1);
            }
            _ => {}
        }
        idx += 1;
    }
    (t, f, other)
}

/// Branch pattern text before a `->` at `idx` (current line segment).
fn branch_pattern(chars: &[char], idx: usize) -> String {
    let mut start = idx;
    while start > 0 && chars[start - 1] != '\n' && chars[start - 1] != ';' {
        start -= 1;
    }
    let mut pattern: String = chars[start..idx].iter().collect();
    pattern = pattern.trim().to_owned();
    while pattern.starts_with('(') && pattern.ends_with(')') && pattern.len() > 2 {
        pattern = pattern[1..pattern.len() - 1].trim().to_owned();
    }
    pattern
}

/// True when `do` at `idx` opens a block (`do:` keywords take no `end`).
fn do_opens(chars: &[char], idx: usize) -> bool {
    if !value_start(chars, idx) {
        return false;
    }
    chars.get(idx + "do".len()) != Some(&':')
}

/// True when a keyword at `idx` is real code (not an atom, field or capture).
fn value_start(chars: &[char], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = chars[idx - 1];
    !(prev == ':' || prev == '.' || prev == '@' || prev == '&')
}

/// Native column fallback: first `trigger` occurrence with Credo's boundary
/// rule (`SourceFile.column/3`), as a 1-based byte column.
fn fallback_column(line: &str, trigger: &str) -> Option<usize> {
    let first = trigger.chars().next()?;
    let last = trigger.chars().next_back()?;
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find(trigger) {
        let base = search + rel;
        if before_ok(line, base, first) && after_ok(line, base + trigger.len(), last) {
            return Some(base + 1);
        }
        search = base + 1;
    }
    None
}

fn before_ok(line: &str, base: usize, first: char) -> bool {
    if base == 0 {
        return first.is_alphanumeric() || first == '_';
    }
    let prev = line[..base].chars().next_back().unwrap_or(' ');
    prev.is_whitespace() || "()[],".contains(prev) || is_word(prev) != is_word(first)
}

fn after_ok(line: &str, end: usize, last: char) -> bool {
    if end >= line.len() {
        return last.is_alphanumeric() || last == '_';
    }
    let next = line[end..].chars().next().unwrap_or(' ');
    next.is_whitespace() || "()[],".contains(next) || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Trigger;
    #[test]
    fn varied_case_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "case x do\n  1 -> :a\n  _ -> :b\nend\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_trivial() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "case x do\n  true -> :a\n  false -> :b\nend\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_single_line_body() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule T do\n  def f(x) do\n    case x do true -> 1; false -> 2 end\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
        assert_eq!(findings[0].column, None);
    }
    #[test]
    fn reports_cond_trigger_without_column() {
        let src = "defmodule Credo.Sample.Module do\n  def some_function(p1, p2, p3, p4, p5, p6) do\n    case some_value do\n      true -> :one\n      false -> :three\n    end\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src)),
            vec![Finding {
                line: 3,
                column: None,
                message: "Case statements should not only contain `true` and `false`.".to_owned(),
                trigger: Trigger::Text("cond".to_owned()),
                severity: None,
            }]
        );
    }
}
