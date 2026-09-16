use crate::Finding;

/// A `@spec`/`@callback` argument list with per-line positions.
struct SpecArgs {
    start_line: usize,
    text: String,
    line_offsets: Vec<usize>,
}

/// `EX3037`: `@spec`/`@callback` params should have names (`name :: type`).
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for span in spec_arg_spans(&lines) {
        for (arg, arg_line) in split_args(&span) {
            check_type(&arg, arg_line, &lines, &mut findings);
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Argument-list spans of `@spec`/`@callback` declarations with `::` returns.
fn spec_arg_spans(lines: &[&str]) -> Vec<SpecArgs> {
    let mut out = Vec::new();
    let mut idx = 0_usize;
    while idx < lines.len() {
        // `@spec`/`@callback` substrings are necessary for the attribute.
        if !lines[idx].contains("@spec") && !lines[idx].contains("@callback") {
            idx += 1;
            continue;
        }
        let chars: Vec<char> = lines[idx].chars().collect();
        let at = find_attr(&chars);
        let Some(name_end) = at else {
            idx += 1;
            continue;
        };
        let Some(open) = find_call_open(&chars, name_end) else {
            idx += 1;
            continue;
        };
        let Some((text, end_line, end_pos, offsets)) = read_balanced(lines, idx, open) else {
            idx += 1;
            continue;
        };
        if has_return(lines, end_line, end_pos) {
            out.push(SpecArgs {
                start_line: idx,
                text,
                line_offsets: offsets,
            });
        }
        idx += 1;
    }
    out
}

/// Offset of `@spec`/`@callback` name end, if the line declares one.
fn find_attr(chars: &[char]) -> Option<usize> {
    const ATTRS: [&[u8]; 2] = [b"spec", b"callback"];
    let mut i = 0_usize;
    while i < chars.len() {
        for attr in ATTRS {
            if chars.len() >= i + attr.len()
                && chars[i..i + attr.len()]
                    .iter()
                    .zip(attr.iter())
                    .all(|(got, want)| *got == *want as char)
                && i > 0
                && chars[i - 1] == '@'
                && (i + attr.len() >= chars.len()
                    || chars[i + attr.len()].is_whitespace()
                    || chars[i + attr.len()] == '(')
            {
                let mut end = i + attr.len();
                while end < chars.len() && chars[end] == ' ' {
                    end += 1;
                }
                let mut name_end = end;
                while name_end < chars.len()
                    && (chars[name_end].is_alphanumeric() || chars[name_end] == '_')
                {
                    name_end += 1;
                }
                if name_end > end {
                    return Some(name_end);
                }
            }
        }
        i += 1;
    }
    None
}

/// Offset of the `(` opening the argument list, if on this line.
fn find_call_open(chars: &[char], mut i: usize) -> Option<usize> {
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    if i < chars.len() && chars[i] == '(' {
        Some(i)
    } else {
        None
    }
}

/// Text inside the balanced parens at (`line_idx`, `open`), spanning lines.
/// Returns `(inner_text, end_line, end_pos_after_close, line_start_offsets)`.
fn read_balanced(
    lines: &[&str],
    line_idx: usize,
    open: usize,
) -> Option<(String, usize, usize, Vec<usize>)> {
    let mut text = String::new();
    let mut offsets = vec![0_usize];
    let mut depth = 0_usize;
    let mut idx = line_idx;
    let mut pos = open;
    let mut started = false;
    loop {
        if idx >= lines.len() {
            return None;
        }
        let chars: Vec<char> = lines[idx].chars().collect();
        while pos < chars.len() {
            let c = chars[pos];
            if matches!(c, '(' | '[' | '{') {
                depth += 1;
                if c == '(' && !started {
                    started = true;
                    pos += 1;
                    continue;
                }
            }
            if matches!(c, ')' | ']' | '}') {
                if depth == 1 && c == ')' {
                    return Some((text, idx, pos + 1, offsets));
                }
                depth = depth.saturating_sub(1);
            }
            if started {
                if c == '\n' {
                    text.push('\n');
                    offsets.push(text.len());
                } else {
                    text.push(c);
                }
            }
            pos += 1;
        }
        idx += 1;
        pos = 0;
        if started {
            text.push('\n');
            offsets.push(text.len());
        }
    }
}

/// True when a `::` return follows the argument list.
fn has_return(lines: &[&str], end_line: usize, end_pos: usize) -> bool {
    let mut text = String::new();
    for (offset, line) in lines[end_line..].iter().enumerate().take(6) {
        let chars: Vec<char> = line.chars().collect();
        let from = if offset == 0 { end_pos } else { 0 };
        if from < chars.len() {
            text.push_str(&chars[from..].iter().collect::<String>());
        }
        text.push('\n');
    }
    text.contains("::")
}

/// Top-level comma-separated arguments with their 0-based line numbers.
fn split_args(span: &SpecArgs) -> Vec<(String, usize)> {
    let chars: Vec<char> = span.text.chars().collect();
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    let mut i = 0_usize;
    while i <= chars.len() {
        let boundary = i == chars.len() || (chars[i] == ',' && depth == 0);
        if !boundary {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
            i += 1;
            continue;
        }
        let arg: String = chars[start..i].iter().collect();
        out.push((arg, line_of(span, start)));
        start = i + 1;
        i += 1;
    }
    out
}

/// 0-based line number of a text offset within the span.
fn line_of(span: &SpecArgs, offset: usize) -> usize {
    let mut line = 0_usize;
    for (n, start) in span.line_offsets.iter().enumerate() {
        if *start <= offset {
            line = n;
        }
    }
    span.start_line + line
}

/// Check one argument type expression for missing parameter names.
fn check_type(arg: &str, arg_line: usize, lines: &[&str], findings: &mut Vec<Finding>) {
    let stripped = strip_parens(arg);
    if stripped.trim().is_empty() {
        return;
    }
    if let Some(arrow) = find_top(&stripped, "->") {
        for part in split_top(&stripped[..arrow], ',') {
            check_type(&part, arg_line, lines, findings);
        }
        return;
    }
    if find_top(&stripped, "::").is_some() {
        // Named `name :: type` (unions on the right stay covered).
        return;
    }
    if find_top(&stripped, "|").is_some() {
        for part in split_top(&stripped, '|') {
            check_type(&part, arg_line, lines, findings);
        }
        return;
    }
    let trimmed = stripped.trim();
    if trimmed.starts_with('%') || trimmed.starts_with("<<") {
        // Maps, structs and bitstrings need no parameter names.
        return;
    }
    if let Some(colon) = find_single_colon(&stripped)
        && !trimmed.starts_with(':')
    {
        // Keyword `key: type` checks the type side.
        check_type(&stripped[colon + 1..], arg_line, lines, findings);
        return;
    }
    if let Some(bracketed) = strip_brackets(trimmed) {
        for part in split_top(&bracketed, ',') {
            check_type(&part, arg_line, lines, findings);
        }
        return;
    }
    if let Some((dotted, call_args)) = split_call(trimmed) {
        if call_args.trim().is_empty() {
            report(arg_line, lines, &dotted, findings);
        } else {
            for part in split_top(&call_args, ',') {
                check_type(&part, arg_line, lines, findings);
            }
        }
        return;
    }
    if is_bare_name(trimmed) {
        report(arg_line, lines, trimmed, findings);
    }
}

/// Report one unnamed parameter; the column is the first
/// boundary-delimited trigger occurrence on its line.
fn report(arg_line: usize, lines: &[&str], trigger: &str, findings: &mut Vec<Finding>) {
    let column = lines
        .get(arg_line)
        .and_then(|line| trigger_column(line, trigger))
        .unwrap_or(1);
    findings.push(Finding::with_trigger(
        arg_line + 1,
        Some(column),
        "Spec parameter is missing a name. Use `name :: type` syntax.",
        trigger.to_owned(),
    ));
}

/// Strip whole-span outer parens (`(A)` -> `A`, `map()` untouched).
fn strip_parens(text: &str) -> String {
    let mut current = text.trim();
    loop {
        let chars: Vec<char> = current.chars().collect();
        if chars.first() != Some(&'(') {
            return current.to_owned();
        }
        match closing_at(&chars) {
            Some(end) if end == chars.len() - 1 => {
                // ASCII `(`/`)` delimiters keep byte offsets on boundaries.
                current = current[1..current.len() - 1].trim();
            }
            _ => return current.to_owned(),
        }
    }
}

/// Index of the closer matching the opening bracket at zero.
fn closing_at(chars: &[char]) -> Option<usize> {
    let mut depth = 0_usize;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip whole-span `[...]` or `{...}`; `None` when not wrapped.
fn strip_brackets(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let (open, close) = match chars.first() {
        Some('[') => ('[', ']'),
        Some('{') => ('{', '}'),
        _ => return None,
    };
    let _ = (open, close);
    let mut depth = 0_usize;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if i == chars.len() - 1 {
                        return Some(chars[1..chars.len() - 1].iter().collect());
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a dotted/local call into `(trigger, args_text)`.
fn split_call(text: &str) -> Option<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0_usize;
    while i < chars.len()
        && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '?' | '!'))
    {
        i += 1;
    }
    if i == 0 || i >= chars.len() || chars[i] != '(' {
        return None;
    }
    let head: String = chars[..i].iter().collect();
    if head.is_empty() || head.starts_with('.') || head.ends_with('.') || head.contains("..") {
        return None;
    }
    let mut depth = 0_usize;
    let mut j = i;
    while j < chars.len() {
        match chars[j] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j != chars.len() - 1 {
        // Trailing text after the call (operators): not a plain call.
        let rest: String = chars[j + 1..].iter().collect();
        if !rest.trim().is_empty() {
            return None;
        }
    }
    let args: String = chars[i + 1..j].iter().collect();
    if head.contains('.') {
        let trigger = head.split('.').map(str::trim).collect::<Vec<_>>().join(".");
        Some((trigger, args))
    } else if args.trim().is_empty() {
        Some((format!("{head}()"), args))
    } else {
        Some((head, args))
    }
}

/// Bare variable/type names (excluding literals, atoms and numbers).
fn is_bare_name(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if ["nil", "true", "false"].contains(&trimmed) {
        return false;
    }
    if trimmed.starts_with([':', '"', '\'', '?', '%', '<', '@', '.']) {
        return false;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return false;
    }
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) if first.is_alphabetic() || first == '_' => {}
        _ => return false,
    }
    trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

/// Offset of a top-level multi-char operator (`->`, `::`, `|`).
fn find_top(text: &str, op: &str) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let pattern: Vec<char> = op.chars().collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && chars[i..].starts_with(&pattern) {
            if op == "|" {
                return Some(i);
            }
            if op == "->" && is_arrow(&chars, i) {
                return Some(i);
            }
            if op == "::"
                && chars.get(i + 2) != Some(&':')
                && chars.get(i.wrapping_sub(1)) != Some(&':')
            {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn is_arrow(chars: &[char], i: usize) -> bool {
    chars.get(i) == Some(&'-') && chars.get(i + 1) == Some(&'>') && chars.get(i + 2) != Some(&'>')
}

/// Offset of a top-level single `:` (excluding `::`); for `key: type`.
fn find_single_colon(text: &str) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ':' if depth == 0
                && chars.get(i + 1) != Some(&':')
                && chars.get(i.wrapping_sub(1)) != Some(&':') =>
            {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split on a top-level single-char separator.
fn split_top(text: &str, sep: char) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    let mut i = 0_usize;
    while i <= chars.len() {
        if i == chars.len() || (chars[i] == sep && depth == 0) {
            out.push(chars[start..i].iter().collect());
            start = i + 1;
        } else {
            match chars[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        i += 1;
    }
    out
}

/// First boundary-delimited trigger occurrence in `line` (1-based char col).
fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let pattern = format!(
        r"(\s|\b|\(|\)|,)({})(\s|\b|\(|\)|,)",
        regex::escape(trigger)
    );
    let re = regex::Regex::new(&pattern).ok()?;
    let inner = re.captures(line)?.get(2)?;
    Some(line[..inner.start()].chars().count() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_spec_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "@spec foo(x :: integer) :: integer\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_unnamed() {
        let src = "@spec foo(integer) :: integer\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 1);
    }
    #[test]
    fn reports_each_unnamed_argument() {
        let src = "@spec create_user(map(), String.t()) :: {:ok, term()}\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].column, Some(19));
        assert_eq!(findings[1].column, Some(26));
    }
    #[test]
    fn decoy_substrings_do_not_scan() {
        // "special", "callbacks" and "despec" carry gated substrings.
        let src = "special = 1\ncallbacks = 2\ndespec = 3\n@spec foo(integer) :: integer\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 1);
    }
}
