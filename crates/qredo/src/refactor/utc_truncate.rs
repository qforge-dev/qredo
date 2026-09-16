use crate::Finding;

/// `EX4032`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let starts = line_starts(masked);
    let mut findings = Vec::new();
    for module in ["DateTime", "NaiveDateTime"] {
        let truncate = format!("{module}.truncate");
        let utc_now = format!("{module}.utc_now");
        for pos in name_positions(masked, &truncate) {
            if call_form_applies(masked, pos, &truncate, &utc_now)
                || pipe_form_applies(masked, pos, &utc_now)
            {
                let (line_no, line) = line_of(&starts, &lines, pos);
                findings.push(Finding::with_trigger(
                    line_no,
                    trigger_column(line, &truncate),
                    format!(
                        "Pass time unit to `{module}.utc_now` instead of composing with `{module}.truncate/2`."
                    ),
                    truncate.clone(),
                ));
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// `Mod.truncate(<first arg>, ...)` where the first argument is
/// `Mod.utc_now(...)` or pipes into it.
fn call_form_applies(masked: &str, pos: usize, truncate: &str, utc_now: &str) -> bool {
    let mut rest = skip_ws(masked, pos + truncate.len());
    if !rest.starts_with('(') {
        return false;
    }
    rest = &rest[1..];
    let Some(inner) = balanced_inner(rest, '(', ')') else {
        return false;
    };
    let first = split_top_level(inner, ',').first().copied().unwrap_or("");
    let trimmed = first.trim();
    if starts_with_name(trimmed, utc_now) {
        return true;
    }
    let segments = split_pipes(trimmed);
    segments.len() > 1
        && segments
            .last()
            .is_some_and(|last| starts_with_name(last.trim(), utc_now))
}

/// `... |> Mod.truncate(...)` where the piped value is a `Mod.utc_now(...)` call.
fn pipe_form_applies(masked: &str, pos: usize, utc_now: &str) -> bool {
    let before = skip_ws_back(masked, pos);
    if !before.ends_with("|>") {
        return false;
    }
    let mut lhs = skip_ws_back(masked, before.len() - "|>".len());
    if lhs.ends_with(')') {
        let Some(open) = match_paren_back(lhs, '(', ')') else {
            return false;
        };
        lhs = skip_ws_back(masked, open);
    }
    ends_with_name(lhs, utc_now)
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

/// Byte offsets of `name` with identifier boundaries on both sides.
fn name_positions(masked: &str, name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut search = 0_usize;
    while let Some(rel) = masked[search..].find(name) {
        let pos = search + rel;
        let before_ok = masked[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !is_name_char(c));
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

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '?' || c == '!'
}

/// Text inside balanced `open`/`close` starting just after the opener.
/// Returns the inner slice (without the closing delimiter).
fn balanced_inner(rest: &str, open: char, close: char) -> Option<&str> {
    let mut depth = 0_usize;
    let mut end = None;
    for (byte, c) in rest.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                end = Some(byte);
                break;
            }
            depth -= 1;
        }
    }
    end.map(|byte| &rest[..byte])
}

/// Byte offset of the opener matching a closer at the end of `text`.
fn match_paren_back(text: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0_usize;
    let mut bytes: Vec<(usize, char)> = text.char_indices().collect();
    while let Some((byte, c)) = bytes.pop() {
        if c == close {
            depth += 1;
        } else if c == open {
            depth -= 1;
            if depth == 0 {
                return Some(byte);
            }
        }
    }
    None
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
                let next = bytes.get(i + 2).copied().unwrap_or(text.len());
                start = next;
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

/// Split on `sep` at bracket depth zero (`|` splits `|>` pipes; the
/// second `>` stays attached to the next segment and is trimmed later).
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

fn skip_ws(masked: &str, mut pos: usize) -> &str {
    while let Some(c) = masked[pos..].chars().next() {
        if c.is_whitespace() {
            pos += c.len_utf8();
        } else {
            break;
        }
    }
    &masked[pos..]
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

/// Credo's trigger column: first occurrence flanked by whitespace,
/// word boundaries, or `(`/`)`/`,`.
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
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("DateTime.utc_now(:second)\n")).is_empty()
        );
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "DateTime.utc_now() |> DateTime.truncate(:second)\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_multiline_pipe() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  def f do\n    DateTime.utc_now()\n    |>\n    DateTime.truncate(:second)\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 5);
        assert_eq!(findings[0].column, Some(5));
    }
    #[test]
    fn naive_reports_only_naive_trigger() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  def f do\n    NaiveDateTime.truncate(NaiveDateTime.utc_now(), :second)\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Pass time unit to `NaiveDateTime.utc_now` instead of composing with `NaiveDateTime.truncate/2`."
        );
    }
}
