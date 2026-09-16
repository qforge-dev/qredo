use crate::Finding;

const KEYWORDS: [&str; 34] = [
    "def",
    "defp",
    "defmacro",
    "defmacrop",
    "defguard",
    "defguardp",
    "defmodule",
    "do",
    "end",
    "if",
    "unless",
    "case",
    "cond",
    "with",
    "for",
    "fn",
    "in",
    "not",
    "and",
    "or",
    "true",
    "false",
    "nil",
    "when",
    "else",
    "try",
    "rescue",
    "catch",
    "after",
    "quote",
    "unquote",
    "super",
    "receive",
    "require",
];

const SPECIAL_VARS: [&str; 4] = ["__CALLER__", "__DIR__", "__ENV__", "__MODULE__"];

/// `EX3031`: variable names must be `snake_case`.
///
/// Only binding positions are checked: `=`/`<-`/`->` left-hand sides and
/// `def`/`defp` heads (exactly two plain arguments check the first one,
/// guarded heads check every argument), mirroring the upstream walk.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let line_no = idx + 1;
        collect_def_head(line, line_no, &mut findings);
        for (lhs, _) in lhs_segments(line, '=') {
            collect_vars(&lhs, line, line_no, &mut findings);
        }
        for (lhs, _) in lhs_segments(line, '<') {
            collect_vars(&lhs, line, line_no, &mut findings);
        }
        for (lhs, _) in lhs_segments(line, '-') {
            collect_vars(&lhs, line, line_no, &mut findings);
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Text before each binding operator on the line. `kind` selects `=`, `<-`
/// or `->`; neighbouring characters disqualify comparisons and map arrows.
fn lhs_segments(line: &str, kind: char) -> Vec<(String, usize)> {
    // Each binding operator carries its own substring; lines without it
    // cannot bind through this operator.
    let needle = match kind {
        '=' => "=",
        '<' => "<-",
        '-' => "->",
        _ => return Vec::new(),
    };
    if !line.contains(needle) {
        return Vec::new();
    }
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < chars.len() {
        let is_binding = match kind {
            '=' => chars[i] == '=' && !is_comparison(&chars, i),
            '<' => chars[i] == '<' && chars.get(i + 1) == Some(&'-'),
            '-' => chars[i] == '-' && chars.get(i + 1) == Some(&'>'),
            _ => false,
        };
        if is_binding {
            let lhs: String = chars[..i].iter().collect();
            out.push((lhs, i));
        }
        i += 1;
    }
    out
}

/// True when the `=` at `i` belongs to `==`, `!=`, `<=`, `>=`, `=>`, `=~`.
fn is_comparison(chars: &[char], i: usize) -> bool {
    let prev = if i > 0 { Some(chars[i - 1]) } else { None };
    let next = chars.get(i + 1).copied();
    matches!(prev, Some('=' | '!' | '<' | '>')) || matches!(next, Some('=' | '>' | '~'))
}

/// Scan `lhs` for variable occurrences and report non-snake ones.
/// Columns follow the upstream lookup: first boundary-delimited trigger
/// match in the full line, falling back to the occurrence itself.
fn collect_vars(lhs: &str, line: &str, line_no: usize, findings: &mut Vec<Finding>) {
    for (start, name) in var_occurrences(lhs) {
        if KEYWORDS.contains(&name.as_str()) || SPECIAL_VARS.contains(&name.as_str()) {
            continue;
        }
        if is_snake_case(&name) || is_no_case(&name) {
            continue;
        }
        let column = trigger_column(line, &name).unwrap_or(start + 1);
        findings.push(Finding::with_trigger(
            line_no,
            Some(column),
            "Variable names should be written in snake_case.",
            name,
        ));
    }
}

/// `def`/`defp` heads: two plain arguments check the first pattern,
/// guarded heads check every argument pattern.
fn collect_def_head(line: &str, line_no: usize, findings: &mut Vec<Finding>) {
    // Both operators start with "def".
    if !line.contains("def") {
        return;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        let op = if match_word(&chars, i, b"defp") {
            Some(4)
        } else if match_word(&chars, i, b"def") {
            Some(3)
        } else {
            None
        };
        if let Some(len) = op {
            let head = head_text(&chars, i + len);
            check_def_args(&head, line, line_no, findings);
            i += len;
        } else {
            i += 1;
        }
    }
}

/// Head text after `def`/`defp`, cut before a top-level `do`.
fn head_text(chars: &[char], mut i: usize) -> String {
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    let start = i;
    let mut depth = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 {
            let rest: String = chars[i..].iter().collect();
            if rest.starts_with(" do") || rest.starts_with(", do") {
                break;
            }
        }
        i += 1;
    }
    chars[start..i].iter().collect()
}

/// Check argument patterns of one `def` head segment.
fn check_def_args(head: &str, line: &str, line_no: usize, findings: &mut Vec<Finding>) {
    let has_guard = split_guard(head).is_some();
    let args_text = fn_args_text(head);
    let Some(args) = args_text else { return };
    if has_guard {
        collect_vars(&args, line, line_no, findings);
    } else if let Some(first) = first_of_two_args(&args) {
        collect_vars(&first, line, line_no, findings);
    }
}

/// First pattern of a two-argument head; other arities are unchecked upstream.
fn first_of_two_args(args: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let chars: Vec<char> = args.chars().collect();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(chars[start..i].iter().collect());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last: String = chars[start..].iter().collect();
    if last.trim().is_empty() && parts.is_empty() {
        return None;
    }
    parts.push(last);
    if parts.len() == 2 {
        Some(parts.remove(0))
    } else {
        None
    }
}

/// Split `head` into `(before_when, has_guard)`.
fn split_guard(head: &str) -> Option<(&str, bool)> {
    let chars: Vec<char> = head.chars().collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 {
            let rest: String = chars[i..].iter().collect();
            if rest.starts_with(" when ") {
                return Some((&head[..head.len() - rest.len()], true));
            }
        }
        i += 1;
    }
    None
}

/// Argument list text of a `name(args)` head, or empty string for bare names.
fn fn_args_text(head: &str) -> Option<String> {
    let open = head.find('(')?;
    let chars: Vec<char> = head.chars().collect();
    let start = head[..open].chars().count();
    let mut depth = 0_usize;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(chars[start + 1..i].iter().collect());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// `(char_column_0based, name)` for each variable-like token: lowercase or
/// underscore start, not field access, atom, attribute, call or key.
fn var_occurrences(lhs: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = lhs.chars().collect();
    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < chars.len() {
        let c = chars[i];
        if c == ':' || c == '@' {
            i = skip_name(&chars, i + 1);
            continue;
        }
        if c.is_ascii_lowercase() || c == '_' {
            let start = i;
            let end = skip_name(&chars, i);
            let name: String = chars[start..end].iter().collect();
            let prev = if start > 0 {
                Some(chars[start - 1])
            } else {
                None
            };
            let next = chars.get(end).copied();
            // A lowercase run continuing an identifier (`MapSet` after `M`)
            // is not a variable occurrence.
            let continues_identifier = prev.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if !continues_identifier
                && !matches!(prev, Some('.' | ':'))
                && !matches!(next, Some('(' | ':'))
            {
                out.push((start, name));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn skip_name(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '?' | '!')) {
        i += 1;
    }
    i
}

/// ASCII word match with identifier boundaries, allocation-free.
fn match_word(chars: &[char], pos: usize, word: &[u8]) -> bool {
    chars.len() >= pos + word.len()
        && chars[pos..pos + word.len()]
            .iter()
            .zip(word.iter())
            .all(|(got, want)| *got == *want as char)
        && (pos == 0 || !(chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_'))
        && chars
            .get(pos + word.len())
            .is_none_or(|c| !c.is_alphanumeric() && *c != '_' && *c != '?')
}

/// Credo `Name.snake_case?/1`.
fn is_snake_case(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '?' | '!'))
}

/// Credo `Name.no_case?/1`: no letters or digits at all.
fn is_no_case(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(char::is_alphanumeric)
}

/// First boundary-delimited trigger occurrence in `line` (1-based char col).
fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let pattern = format!(
        r"(\s|\b|\(|\)|,)({})(\s|\b|\(|\)|,)",
        regex::escape(trigger)
    );
    let re = regex::Regex::new(&pattern).ok()?;
    let caps = re.captures(line)?;
    let inner = caps.get(2)?;
    Some(line[..inner.start()].chars().count() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snake_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("my_var = 1\n")).is_empty());
    }
    #[test]
    fn alias_suffix_is_not_a_variable() {
        // `MapSet` contributed "apSet" before identifier-continuation
        // filtering; found via pipeline comparison on labqoat-web.
        let src = "Enum.reduce_while(ms, {:ok, []}, fn spec, acc ->\n  MapSet.new()\nend)\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_camel() {
        assert!(!check_prepared(&crate::batch::Prepared::lazy("myVar = 1\n")).is_empty());
    }
    #[test]
    fn rhs_use_is_not_a_binding() {
        // `someValue` is only used on the RHS here; the LHS binds `other`.
        let src = "other = someValue + 1\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 0);
    }

    #[test]
    fn guarded_def_checks_every_argument() {
        let src = "def f(oneParam) when is_integer(oneParam), do: :ok\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(7));
    }
    #[test]
    fn operator_substrings_without_binding_are_clean() {
        // "==", "<div" and "a->b" carry gated substrings but bind nothing.
        let src = "x == 1\nhtml = \"<div>\"\nmap = %{a->b}\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
}
