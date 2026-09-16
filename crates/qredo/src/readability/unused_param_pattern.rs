use crate::Finding;

/// `EX5032`: pattern matches in function parameters that are ignored.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, pair) in source.split('\n').zip(masked.split('\n')).enumerate() {
        let (_raw, masked_line) = pair;
        if let Some((open, close)) = def_args(masked_line) {
            // `open`/`close` sit on ASCII parens, hence char boundaries.
            collect_matches(masked_line, open, close, idx + 1, &mut findings);
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// Byte range of the `def`/`defp`/`defmacro` argument parens on one line.
fn def_args(line: &str) -> Option<(usize, usize)> {
    let trimmed = line.trim_start();
    let rest = strip_def_keyword(trimmed)?;
    let base = line.len() - rest.len();
    // `rest` starts after ASCII (`def` keyword plus indent), so `base`
    // is a char boundary; all later offsets stay within ASCII tokens.
    let bytes = rest.as_bytes();
    let mut name_start = 0_usize;
    while name_start < bytes.len() && bytes[name_start].is_ascii_whitespace() {
        name_start += 1;
    }
    let mut name_end = name_start;
    while name_end < bytes.len() && is_var_byte(bytes[name_end]) {
        name_end += 1;
    }
    if name_end == name_start {
        return None;
    }
    let mut open = name_end;
    while open < bytes.len() && bytes[open].is_ascii_whitespace() {
        open += 1;
    }
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let close = matching_paren(bytes, open)?;
    Some((base + open, base + close))
}

fn strip_def_keyword(trimmed: &str) -> Option<&str> {
    for keyword in ["defmacro", "defp", "def"] {
        if let Some(rest) = trimmed.strip_prefix(keyword) {
            // Slicing after an ASCII keyword is a char boundary.
            if rest
                .chars()
                .next()
                .is_none_or(|next| !is_var_byte_char(next))
            {
                return Some(rest);
            }
        }
    }
    None
}

/// Index of the paren closing the one at `open` (nesting-aware).
fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Report `= pattern` matches binding an ignored `_var` inside the args.
fn collect_matches(line: &str, open: usize, close: usize, line_no: usize, out: &mut Vec<Finding>) {
    let bytes = line.as_bytes();
    let mut i = open + 1;
    while i < close {
        if bytes[i] == b'='
            && is_match_operator(bytes, i, close)
            && let Some(finding) = ignored_match(bytes, i, close, line_no)
        {
            out.push(finding);
        }
        i += 1;
    }
}

/// A bare `=` match operator (not `==`, `=>`, `=~`, `!=`, `<=`, `>=`).
fn is_match_operator(bytes: &[u8], pos: usize, close: usize) -> bool {
    let prev = if pos > 0 { Some(bytes[pos - 1]) } else { None };
    let next = if pos + 1 < close {
        Some(bytes[pos + 1])
    } else {
        None
    };
    if prev.is_some_and(|byte| byte == b'=' || byte == b'!' || byte == b'<' || byte == b'>') {
        return false;
    }
    if next.is_some_and(|byte| byte == b'=' || byte == b'>' || byte == b'~') {
        return false;
    }
    true
}

/// Issue for `left = right` when either side is a bare ignored variable.
fn ignored_match(bytes: &[u8], equals: usize, close: usize, line_no: usize) -> Option<Finding> {
    if let Some((start, name)) = ident_before(bytes, equals)
        && is_ignored_var(&name)
        && var_start_ok(bytes, start)
    {
        return Some(issue(line_no, start, name));
    }
    if let Some((start, name)) = ident_after(bytes, equals, close)
        && is_ignored_var(&name)
        && var_end_ok(bytes, start + name.len())
    {
        return Some(issue(line_no, start, name));
    }
    None
}

fn issue(line_no: usize, start: usize, name: String) -> Finding {
    Finding::with_trigger(
        line_no,
        Some(start + 1),
        "Function parameter has a pattern match but is immediately ignored.",
        name,
    )
}

/// Identifier ending just before `pos` (skipping blanks); returns byte start.
fn ident_before(bytes: &[u8], pos: usize) -> Option<(usize, String)> {
    let mut end = pos;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_var_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    // ASCII-only scan, so `start..end` is a char boundary range.
    String::from_utf8(bytes[start..end].to_vec())
        .ok()
        .map(|name| (start, name))
}

/// Identifier starting just after `pos` (skipping blanks); returns byte start.
fn ident_after(bytes: &[u8], pos: usize, close: usize) -> Option<(usize, String)> {
    let mut start = pos + 1;
    while start < close && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = start;
    while end < close && is_var_byte(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    String::from_utf8(bytes[start..end].to_vec())
        .ok()
        .map(|name| (start, name))
}

/// A bare `_var` cannot follow `.`, `:`, `@`, or `&` (atom, field, capture).
fn var_start_ok(bytes: &[u8], start: usize) -> bool {
    if start == 0 {
        return true;
    }
    let prev = bytes[start - 1];
    prev != b'.' && prev != b':' && prev != b'@' && prev != b'&'
}

/// A bare `_var` cannot open a call (`_f(`) or a dot access (`_m.field`).
fn var_end_ok(bytes: &[u8], end: usize) -> bool {
    if end >= bytes.len() {
        return true;
    }
    bytes[end] != b'(' && bytes[end] != b'.'
}

fn is_ignored_var(name: &str) -> bool {
    name.starts_with('_')
}

fn is_var_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

fn is_var_byte_char(next: char) -> bool {
    next.is_alphanumeric() || next == '_' || next == '?' || next == '!'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("def foo(x), do: x\n")).is_empty());
    }
    #[test]
    fn reports_ignored_pattern() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "def foo(_x = %{a: a}), do: a\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_var_trigger_and_column() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  def some_function(%{} = _ignored) do\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(27));
        assert_eq!(
            findings[0].trigger,
            crate::Trigger::Text("_ignored".to_owned())
        );
    }
    #[test]
    fn flipped_reports_left_var() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "  def some_function(_ignored = %{}) do\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(21));
    }
    #[test]
    fn defmacro_is_checked() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "  defmacro some_macro(%{} = _ignored) do\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(29));
    }
    #[test]
    fn used_pattern_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "  def some_function(%{id: id} = user) do\n"
            ))
            .is_empty()
        );
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "  def another_function(user = %{id: id}) do\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn map_arrow_is_not_a_match() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("  def f(%{a: x}) do\n")).is_empty());
    }
}
