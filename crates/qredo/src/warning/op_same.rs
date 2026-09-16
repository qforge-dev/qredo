use crate::Finding;

/// `EX5011`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        check_line(line, idx, &lines, &mut findings);
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

fn check_line(line: &str, idx: usize, lines: &[&str], findings: &mut Vec<Finding>) {
    let (body, off) = def_body(line);
    let mut pos = 0_usize;
    while pos < body.len() {
        let Some((op, start, end)) = match_op(body, pos) else {
            pos += body[pos..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        pos = end;
        let Some(lhs) = backward(body, start, &op) else {
            continue;
        };
        let Some(rhs) = forward(body, lines, idx, end, &op) else {
            continue;
        };
        if lhs != rhs {
            continue;
        }
        let message = match op.as_str() {
            "==" | ">=" | "<=" => "Comparison will always return true.",
            "!=" | ">" | "<" => "Comparison will always return false.",
            "/" => "Operation will always return 1.",
            _ => "Operation will always return 0.",
        };
        findings.push(Finding::with_trigger(
            idx + 1,
            Some(col_of(line, off + start)),
            message.to_owned(),
            op,
        ));
    }
}

/// Code after a `def`-head opener (`do`/`do:`) with its byte offset; operator
/// heads (`def a - a`) and `@spec` lines yield an empty body.
fn def_body(line: &str) -> (&str, usize) {
    let stripped = line.trim_start();
    let is_def = ["def ", "defp ", "defmacro ", "defmacrop "]
        .iter()
        .any(|kw| stripped.starts_with(kw));
    if stripped == "@spec" || stripped.starts_with("@spec ") || stripped.starts_with("@spec(") {
        return ("", 0);
    }
    if !is_def {
        return (line, 0);
    }
    if let Some(at) = line.find("do:") {
        return (&line[at + 3..], at + 3);
    }
    let mut search = 0_usize;
    while search < line.len() {
        let Some(rel) = line[search..].find("do") else {
            break;
        };
        let base = search + rel;
        search = base + 1;
        if base > 0
            && line[..base]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        if line[base + 2..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':')
        {
            continue;
        }
        return (&line[base + 2..], base + 2);
    }
    ("", 0)
}

/// Operator match at or after `pos`: `(op, byte_start, byte_end)`.
fn match_op(body: &str, pos: usize) -> Option<(String, usize, usize)> {
    let rest = &body[pos..];
    let chr = rest.chars().next()?;
    let next = rest.chars().nth(1);
    let third = rest.chars().nth(2);
    if let Some(op) = pair_op(chr, next, third) {
        return Some((op, pos, pos + 2));
    }
    single_op(body, pos, chr, next)
}

/// Two-character operators (`==`, `!=`, `>=`, `<=`); never `===`/`!==`.
fn pair_op(chr: char, next: Option<char>, third: Option<char>) -> Option<String> {
    match (chr, next, third) {
        ('=' | '!', Some('='), Some('=')) => None,
        ('=', Some('='), _) => Some("==".to_owned()),
        ('!', Some('='), _) => Some("!=".to_owned()),
        ('>', Some('='), _) => Some(">=".to_owned()),
        ('<', Some('='), _) => Some("<=".to_owned()),
        _ => None,
    }
}

/// Single-character operators with their exclusion guards.
fn single_op(
    body: &str,
    pos: usize,
    chr: char,
    next: Option<char>,
) -> Option<(String, usize, usize)> {
    match chr {
        '>' => {
            if next == Some('=') || next == Some('>') || prev_is(body, pos, &['=', '-', '<', '|']) {
                None
            } else {
                Some((">".to_owned(), pos, pos + 1))
            }
        }
        '<' => {
            if next == Some('=') || next == Some('-') || next == Some('<') || next == Some('>') {
                None
            } else {
                Some(("<".to_owned(), pos, pos + 1))
            }
        }
        '-' => {
            if next == Some('>') || next == Some('-') || prev_is(body, pos, &['<']) {
                None
            } else {
                Some(("-".to_owned(), pos, pos + 1))
            }
        }
        '/' => {
            if next == Some('/') || next == Some('=') {
                None
            } else {
                Some(("/".to_owned(), pos, pos + 1))
            }
        }
        _ => None,
    }
}

/// Whether the byte before `pos` is one of `chars`.
fn prev_is(body: &str, pos: usize, chars: &[char]) -> bool {
    pos > 0
        && body[..pos]
            .chars()
            .next_back()
            .is_some_and(|c| chars.contains(&c))
}

/// Left operand text (bare variable or module attribute only).
fn backward(body: &str, op_start: usize, op: &str) -> Option<String> {
    let wide = op == "==" || op == "!=" || op == ">" || op == "<" || op == ">=" || op == "<=";
    let mut pos = op_start;
    let mut operand = read_back(body, &mut pos)?;
    loop {
        skip_back(body, &mut pos);
        let Some(prev) = body[..pos].chars().next_back() else {
            break;
        };
        if prev == '+' || prev == '-' || prev == '*' || prev == '/' {
            if doubled(body, pos) {
                break;
            }
            pos -= 1;
            skip_back(body, &mut pos);
            let more = read_back(body, &mut pos)?;
            operand = format!("{more} {prev} {operand}");
        } else if wide && body[..pos].ends_with("|>") {
            pos -= 2;
            skip_back(body, &mut pos);
            let more = read_back(body, &mut pos)?;
            operand = format!("{more} |> {operand}");
        } else {
            break;
        }
    }
    shape_ok(&operand).then_some(operand)
}

/// Whether the operator char at `pos` is doubled (`++`, `--`).
fn doubled(body: &str, pos: usize) -> bool {
    let chr = body[..pos].chars().next_back();
    pos >= 2
        && body[..pos - 1]
            .chars()
            .next_back()
            .is_some_and(|prev| Some(prev) == chr)
}

/// Right operand text (bare variable or module attribute only).
fn forward(line: &str, lines: &[&str], idx: usize, op_end: usize, op: &str) -> Option<String> {
    let wide = op == "==" || op == "!=" || op == ">" || op == "<" || op == ">=" || op == "<=";
    let (text, mut pos) = skip_forward(line, lines, idx, op_end);
    let start = pos;
    let operand = read_ahead(text, &mut pos)?;
    let rest = skip_same_line(text, pos);
    let next = rest.chars().next();
    let extends = match next {
        Some('(' | '[' | '.' | '*' | '/') => true,
        Some('+' | '-') => wide,
        _ => false,
    };
    if extends || start >= pos {
        return None;
    }
    shape_ok(&operand).then_some(operand)
}

/// Skip whitespace forward, crossing into later lines; returns text and pos.
fn skip_forward<'a>(
    line: &'a str,
    lines: &[&'a str],
    idx: usize,
    op_end: usize,
) -> (&'a str, usize) {
    let rest = line[op_end..].trim_start();
    if !rest.is_empty() {
        let pos = line.len() - rest.len();
        return (line, pos);
    }
    for text in &lines[idx + 1..] {
        if !text.trim().is_empty() {
            let pos = text.len() - text.trim_start().len();
            return (text, pos);
        }
    }
    (line, line.len())
}

/// Skip spaces/tabs on the same text (no line crossing).
fn skip_same_line(text: &str, pos: usize) -> &str {
    text[pos..].trim_start_matches([' ', '\t'])
}

/// Read one operand token backward; `pos` moves to its start.
fn read_back(body: &str, pos: &mut usize) -> Option<String> {
    skip_back(body, pos);
    let end = *pos;
    while *pos > 0
        && body[..*pos]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
    {
        *pos -= body[..*pos].chars().next_back().map_or(1, char::len_utf8);
    }
    if end == *pos {
        return None;
    }
    let mut token = body[*pos..end].to_owned();
    // Dotted attribute path (`@a.b`).
    while body[..*pos].ends_with('.') {
        *pos -= 1;
        let mut dot = *pos;
        let name = read_back_name(body, &mut dot)?;
        *pos = dot;
        token = format!("{name}.{token}");
    }
    if body[..*pos].ends_with('@') {
        *pos -= 1;
        token = format!("@{token}");
    } else if prev_is(body, *pos, &['!', '~']) {
        return None;
    }
    Some(token)
}

/// Read a plain name run backward for dotted paths.
fn read_back_name(body: &str, pos: &mut usize) -> Option<String> {
    let end = *pos;
    while *pos > 0
        && body[..*pos]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
    {
        *pos -= body[..*pos].chars().next_back().map_or(1, char::len_utf8);
    }
    if end == *pos {
        return None;
    }
    Some(body[*pos..end].to_owned())
}

/// Skip spaces/tabs backward (same line).
fn skip_back(body: &str, pos: &mut usize) {
    while *pos > 0
        && body[..*pos]
            .chars()
            .next_back()
            .is_some_and(|c| c == ' ' || c == '\t')
    {
        *pos -= 1;
    }
}

/// Read one operand token forward; `pos` moves past it.
fn read_ahead(text: &str, pos: &mut usize) -> Option<String> {
    let mut at = *pos;
    if text[at..].starts_with('@') {
        at += 1;
    }
    let start = at;
    while text[at..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
    {
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    if at == start {
        return None;
    }
    let mut token = text[*pos..at].to_owned();
    *pos = at;
    if token.starts_with('@') {
        while text[*pos..].starts_with('.') {
            *pos += 1;
            let name_start = *pos;
            while text[*pos..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
            {
                *pos += text[*pos..].chars().next().map_or(1, char::len_utf8);
            }
            if *pos == name_start {
                return None;
            }
            token = format!("{token}.{}", &text[name_start..*pos]);
        }
    }
    Some(token)
}

/// Whether the operand is a bare variable or module attribute.
fn shape_ok(operand: &str) -> bool {
    if operand == "true" || operand == "false" || operand == "nil" || operand == "_" {
        return false;
    }
    if let Some(attr) = operand.strip_prefix('@') {
        return !attr.is_empty()
            && attr
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '?' || c == '!');
    }
    let mut chars = operand.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
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
        assert!(check_prepared(&crate::batch::Prepared::lazy("x + y\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert!(!check_prepared(&crate::batch::Prepared::lazy("x - x\n")).is_empty());
    }
    #[test]
    fn wider_rhs_is_clean() {
        let src = "defmodule CredoSampleModule do\n  use ExUnit.Case\n\n  def some_fun do\n    assert x == x + 2\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
}
