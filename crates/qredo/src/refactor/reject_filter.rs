use crate::Finding;

const FIRST: &str = "Enum.reject";
const SECOND: &str = "Enum.filter";
const MESSAGE: &str = "One `Enum.filter/2` is more efficient than `Enum.reject/2 |> Enum.filter/2`";

/// `EX4025`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let starts = line_starts(masked);
    let mut findings = Vec::new();
    for pipe in pipe_positions(masked) {
        if piped_second(masked, pipe) && fed_by_first(masked, pipe) {
            push_issue(&mut findings, &starts, &lines, pipe);
        }
    }
    for second in name_positions(masked, SECOND) {
        if nested_first_is_first(masked, second) {
            push_issue(&mut findings, &starts, &lines, second);
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

fn push_issue(findings: &mut Vec<Finding>, starts: &[usize], lines: &[&str], pos: usize) {
    let (line_no, line) = line_of(starts, lines, pos);
    findings.push(Finding::with_trigger(
        line_no,
        trigger_column(line, "|>"),
        MESSAGE,
        "|>".to_owned(),
    ));
}

/// Whether the `|>` feeds its value into a `SECOND(...)` call.
fn piped_second(masked: &str, pipe: usize) -> bool {
    let mut rest = skip_inline_ws(&masked[pipe + "|>".len()..]);
    if rest.starts_with('\n') {
        // `|>` ends its line, so its call continues on a later line.
        rest = skip_ws(rest);
    }
    if !starts_with_name(rest, SECOND) {
        return false;
    }
    let after = skip_inline_ws(&rest[SECOND.len()..]);
    if after.starts_with('(') {
        return true;
    }
    matches!(
        after.chars().next(),
        None | Some('|' | ')' | ',' | ']' | '}')
    ) || after.starts_with('\n')
}

/// Whether the value piped in is a `FIRST(...)` call.
fn fed_by_first(masked: &str, pipe: usize) -> bool {
    let before = skip_ws_back(masked, pipe);
    if before.ends_with(')') {
        let Some(open) = match_paren_back(before) else {
            return false;
        };
        ends_with_name(skip_ws_back(masked, open), FIRST)
    } else {
        ends_with_name(before, FIRST)
    }
}

/// Whether a `SECOND(...)` call takes a `FIRST` call (or pipe into one)
/// as its first argument.
fn nested_first_is_first(masked: &str, second: usize) -> bool {
    let rest = skip_ws(&masked[second + SECOND.len()..]);
    if !rest.starts_with('(') {
        return false;
    }
    let Some(inner) = balanced_inner(&rest[1..]) else {
        return false;
    };
    let first = split_top_level(inner, ',').first().copied().unwrap_or("");
    let last = split_pipes(strip_parens(first.trim()))
        .last()
        .copied()
        .unwrap_or("")
        .trim()
        .to_owned();
    starts_with_name(&last, FIRST)
}

fn starts_with_name(text: &str, name: &str) -> bool {
    text.starts_with(name)
        && text[name.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_name_char(c))
}

fn ends_with_name(text: &str, name: &str) -> bool {
    text.ends_with(name)
        && text[..text.len() - name.len()]
            .chars()
            .next_back()
            .is_none_or(|c| !is_name_char(c) && c != '.')
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '?' || c == '!'
}

/// Byte offsets of `|>` operators.
fn pipe_positions(masked: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut search = 0_usize;
    while let Some(rel) = masked[search..].find("|>") {
        out.push(search + rel);
        search += rel + "|>".len();
    }
    out
}

/// Byte offsets of remote `name` calls with identifier boundaries.
fn name_positions(masked: &str, name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut search = 0_usize;
    while let Some(rel) = masked[search..].find(name) {
        let pos = search + rel;
        let before_ok = masked[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !is_name_char(c) && c != '.');
        let after_ok = masked[pos + name.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_name_char(c));
        if before_ok && after_ok {
            out.push(pos);
        }
        search = pos + 1;
    }
    out
}

/// Text inside balanced parens starting just after the opener.
fn balanced_inner(rest: &str) -> Option<&str> {
    let mut depth = 0_usize;
    for (byte, c) in rest.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    if c == ')' {
                        return Some(&rest[..byte]);
                    }
                    return None;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Byte offset of the opener matching the closer ending `text`.
fn match_paren_back(text: &str) -> Option<usize> {
    let mut depth = 0_usize;
    let mut pairs: Vec<(usize, char)> = text.char_indices().collect();
    while let Some((byte, c)) = pairs.pop() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(byte);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(text: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (byte, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                parts.push(&text[start..byte]);
                start = byte + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// Split on top-level `|>` pipe operators.
fn split_pipes(text: &str) -> Vec<&str> {
    let chars: Vec<char> = text.chars().collect();
    let bytes: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '|' if depth == 0 && chars.get(i + 1) == Some(&'>') => {
                parts.push(&text[start..bytes[i]]);
                start = bytes.get(i + 2).copied().unwrap_or(text.len());
                i += 2;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&text[start..]);
    parts
}

/// Remove redundant surrounding parentheses.
fn strip_parens(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim();
        if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
            return trimmed;
        }
        let inner = &trimmed[1..trimmed.len() - 1];
        if balanced(inner) {
            text = inner;
        } else {
            return trimmed;
        }
    }
}

fn balanced(text: &str) -> bool {
    let mut depth = 0_usize;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    depth == 0
}

fn skip_inline_ws(text: &str) -> &str {
    let mut end = 0_usize;
    for (byte, c) in text.char_indices() {
        if c == ' ' || c == '\t' {
            end = byte + c.len_utf8();
        } else {
            break;
        }
    }
    &text[end..]
}

fn skip_ws(text: &str) -> &str {
    let mut end = 0_usize;
    for (byte, c) in text.char_indices() {
        if c.is_whitespace() {
            end = byte + c.len_utf8();
        } else {
            break;
        }
    }
    &text[end..]
}

fn skip_ws_back(masked: &str, mut end: usize) -> &str {
    while let Some(c) = masked[..end].chars().next_back() {
        if c.is_whitespace() {
            end -= c.len_utf8();
        } else {
            break;
        }
    }
    &masked[..end]
}

fn line_starts(masked: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    for (byte, c) in masked.char_indices() {
        if c == '\n' {
            starts.push(byte + 1);
        }
    }
    starts
}

fn line_of<'a>(starts: &[usize], lines: &[&'a str], pos: usize) -> (usize, &'a str) {
    let mut line_no = 1_usize;
    for (i, start) in starts.iter().enumerate() {
        if *start <= pos {
            line_no = i + 1;
        } else {
            break;
        }
    }
    (line_no, lines.get(line_no - 1).copied().unwrap_or(""))
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find(trigger) {
        let pos = search + rel;
        if column_boundary_before(line, pos, trigger) && column_boundary_after(line, pos, trigger) {
            return Some(line[..pos].chars().count() + 1);
        }
        search = pos + 1;
    }
    None
}

fn column_boundary_before(line: &str, pos: usize, trigger: &str) -> bool {
    let first = trigger.chars().next();
    match line[..pos].chars().next_back() {
        None => first.is_some_and(is_word_char),
        Some(c) => {
            c.is_whitespace() || c == '(' || c == ')' || c == ',' || boundary_flip(Some(c), first)
        }
    }
}

fn column_boundary_after(line: &str, pos: usize, trigger: &str) -> bool {
    let last = trigger.chars().next_back();
    match line[pos + trigger.len()..].chars().next() {
        None => last.is_some_and(is_word_char),
        Some(c) => {
            c.is_whitespace() || c == '(' || c == ')' || c == ',' || boundary_flip(last, Some(c))
        }
    }
}

fn boundary_flip(left: Option<char>, right: Option<char>) -> bool {
    match (left, right) {
        (Some(l), Some(r)) => is_word_char(l) != is_word_char(r),
        (Some(l), None) => is_word_char(l),
        (None, Some(r)) => is_word_char(r),
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("Enum.filter(x, & &1)\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "x |> Enum.reject(& &1) |> Enum.filter(& &2)\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_piped_pair() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule Credo.Sample.Module do\n  def some_function(p1, p2, p3, p4, p5) do\n    [\"a\", \"b\", \"c\"]\n    |> Enum.reject(&String.contains?(&1, \"x\"))\n    |> Enum.filter(&String.contains?(&1, \"a\"))\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 5);
        assert_eq!(findings[0].column, Some(5));
        assert_eq!(
            findings[0].message,
            "One `Enum.filter/2` is more efficient than `Enum.reject/2 |> Enum.filter/2`"
        );
    }
    #[test]
    fn reports_nested_pair() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule Credo.Sample.Module do\n  def some_function(p1, p2, p3, p4, p5, p6) do\n    Enum.filter(Enum.reject([:a, :b, :c], &String.contains?(&1, \"x\")), &String.contains?(&1, \"a\"))\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
        assert_eq!(findings[0].column, None);
    }
}
