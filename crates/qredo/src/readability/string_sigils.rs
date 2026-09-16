use crate::Finding;
use std::collections::BTreeMap;

/// A double-quoted string literal with its start line and unescaped value.
struct StringToken {
    line: usize,
    value: String,
}

/// `EX3027`: prefer sigils for strings containing many quotes.
pub(crate) fn check(source: &str, params: &BTreeMap<String, String>) -> Vec<Finding> {
    let limit = match params.get("maximum_allowed_quotes") {
        Some(raw) => raw.parse().unwrap_or(3),
        None => 3,
    };
    let mut findings = Vec::new();
    for token in string_tokens(source) {
        if quote_count(&token.value) > limit {
            findings.push(Finding {
                line: token.line,
                column: None,
                message: format!(
                    "More than {limit} quotes found inside string literal, consider using a sigil instead."
                ),
                trigger: crate::Trigger::Text(token.value),
                        severity: None,
});
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

fn quote_count(value: &str) -> usize {
    value.chars().filter(|c| *c == '"').count()
}

/// Double-quoted `:string` tokens: skips heredocs, charlists, sigils,
/// quoted atoms, comments and interpolated strings (non-binary upstream).
fn string_tokens(source: &str) -> Vec<StringToken> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut out = Vec::new();
    let (mut idx, mut pos) = (0_usize, 0_usize);
    while idx < lines.len() {
        match scan_line(&lines, idx, pos, &mut out) {
            LineStep::Done => {
                idx += 1;
                pos = 0;
            }
            LineStep::Jump(next_line, next_pos) => {
                idx = next_line;
                pos = next_pos;
            }
        }
    }
    out
}

enum LineStep {
    Done,
    Jump(usize, usize),
}

/// Scan one line for string tokens from `start`; returns where to continue.
fn scan_line(lines: &[&str], idx: usize, start: usize, out: &mut Vec<StringToken>) -> LineStep {
    let chars: Vec<char> = lines[idx].chars().collect();
    let mut pos = start;
    while pos < chars.len() {
        match scan_char(lines, idx, &chars, pos, out) {
            CharStep::Break => return LineStep::Done,
            CharStep::Jump(next_line, next_pos) => return LineStep::Jump(next_line, next_pos),
            CharStep::Advance(next_pos) => pos = next_pos,
        }
    }
    LineStep::Done
}

enum CharStep {
    Break,
    Jump(usize, usize),
    Advance(usize),
}

/// Handle the character at `pos`: strings, atoms, sigils and literals.
fn scan_char(
    lines: &[&str],
    idx: usize,
    chars: &[char],
    pos: usize,
    out: &mut Vec<StringToken>,
) -> CharStep {
    let c = chars[pos];
    if c == '#' {
        return CharStep::Break;
    }
    if c == '\'' {
        if is_triple(chars, pos, '\'') {
            let (next_line, next_pos) = skip_heredoc(lines, idx, pos + 3, '\'');
            return CharStep::Jump(next_line, next_pos);
        }
        return CharStep::Advance(skip_quoted(chars, pos, '\''));
    }
    if c == '"' {
        if is_triple(chars, pos, '"') {
            let (next_line, next_pos) = skip_heredoc(lines, idx, pos + 3, '"');
            return CharStep::Jump(next_line, next_pos);
        }
        return consume_read(read_string(lines, idx, pos), idx, out);
    }
    if c == ':' && chars.get(pos + 1) == Some(&'"') {
        // Quoted atom (`:"..."`): same spans, never a string token.
        let mut ignored = Vec::new();
        return consume_read(read_string(lines, idx, pos + 1), idx, &mut ignored);
    }
    if c == '~'
        && chars.get(pos + 1).is_some_and(char::is_ascii_alphabetic)
        && let Some(end) = skip_sigil(lines, idx, pos)
    {
        if end.0 != idx {
            return CharStep::Jump(end.0, end.1);
        }
        return CharStep::Advance(end.1);
    }
    if c == '?' && is_char_literal(chars, pos) {
        return CharStep::Advance(pos + 2);
    }
    CharStep::Advance(pos + 1)
}

/// Advance past a string read, collecting plain-string tokens.
fn consume_read(read: StringRead, idx: usize, out: &mut Vec<StringToken>) -> CharStep {
    match read {
        StringRead::Token(token, next_line, next_pos) => {
            out.push(token);
            jump(idx, next_line, next_pos)
        }
        StringRead::Skip(next_line, next_pos) => jump(idx, next_line, next_pos),
    }
}

fn jump(idx: usize, next_line: usize, next_pos: usize) -> CharStep {
    if next_line == idx {
        CharStep::Advance(next_pos)
    } else {
        CharStep::Jump(next_line, next_pos)
    }
}

enum StringRead {
    Token(StringToken, usize, usize),
    Skip(usize, usize),
}

/// Read the string opening at (`line_idx`, `quote_pos`); reports plain
/// strings and skips interpolated or unterminated ones.
fn read_string(lines: &[&str], line_idx: usize, quote_pos: usize) -> StringRead {
    let mut raw = String::new();
    let mut idx = line_idx;
    let mut pos = quote_pos + 1;
    loop {
        let chars: Vec<char> = lines[idx].chars().collect();
        while pos < chars.len() {
            let c = chars[pos];
            if c == '\\' && pos + 1 < chars.len() {
                let next = chars[pos + 1];
                raw.push('\\');
                raw.push(next);
                pos += 2;
                continue;
            }
            if c == '"' {
                let cooked = unescape(&raw);
                if has_interpolation(&raw) {
                    return StringRead::Skip(idx, pos + 1);
                }
                return StringRead::Token(
                    StringToken {
                        line: line_idx + 1,
                        value: cooked,
                    },
                    idx,
                    pos + 1,
                );
            }
            if c == '\n' {
                break;
            }
            raw.push(c);
            pos += 1;
        }
        // Continued on the next line only through an escaped newline.
        if raw.ends_with('\\') {
            idx += 1;
            if idx >= lines.len() {
                return StringRead::Skip(idx.saturating_sub(1), chars.len());
            }
            raw.pop();
            pos = 0;
            continue;
        }
        return StringRead::Skip(idx, chars.len());
    }
}

/// Unescape the common escapes; other escapes keep their character.
fn unescape(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0_usize;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            }
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Unescaped `#{` starts an interpolation: the token is not plain binary.
fn has_interpolation(raw: &str) -> bool {
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '#' && chars.get(i + 1) == Some(&'{') {
            return true;
        }
        i += 1;
    }
    false
}

fn is_triple(chars: &[char], pos: usize, quote: char) -> bool {
    chars.get(pos + 1) == Some(&quote) && chars.get(pos + 2) == Some(&quote)
}

/// Skip a `"""`/`'''` heredoc; returns the position after the closing run.
fn skip_heredoc(lines: &[&str], line_idx: usize, pos: usize, quote: char) -> (usize, usize) {
    let mut idx = line_idx;
    let mut start = pos;
    loop {
        let chars: Vec<char> = lines[idx].chars().collect();
        let mut p = start;
        while p < chars.len() {
            if chars[p] == quote
                && chars.get(p + 1) == Some(&quote)
                && chars.get(p + 2) == Some(&quote)
                && !is_escaped(&chars, p)
            {
                return (idx, p + 3);
            }
            p += 1;
        }
        idx += 1;
        if idx >= lines.len() {
            return (lines.len().saturating_sub(1), 0);
        }
        start = 0;
    }
}

fn is_escaped(chars: &[char], pos: usize) -> bool {
    let mut backslashes = 0_usize;
    let mut i = pos;
    while i > 0 && chars[i - 1] == '\\' {
        backslashes += 1;
        i -= 1;
    }
    backslashes % 2 == 1
}

/// Skip a `'...'` charlist span on one line; returns the position after it.
fn skip_quoted(chars: &[char], pos: usize, quote: char) -> usize {
    let mut i = pos + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// `?x` char literal: `?` starting an expression, not a name suffix.
fn is_char_literal(chars: &[char], pos: usize) -> bool {
    if chars.get(pos) != Some(&'?') {
        return false;
    }
    let prev_ok = pos == 0
        || !(chars[pos - 1].is_alphanumeric() || matches!(chars[pos - 1], '_' | '?' | '!'));
    let next_ok = chars
        .get(pos + 1)
        .is_some_and(|next| *next != '\n' && *next != ' ');
    prev_ok && next_ok && chars.len() > pos + 1
}

/// End position `(line, char_pos)` just past the sigil at (`line_idx`, `pos`).
fn skip_sigil(lines: &[&str], line_idx: usize, pos: usize) -> Option<(usize, usize)> {
    let first: Vec<char> = lines[line_idx].chars().collect();
    let mut name_end = pos + 1;
    while name_end < first.len() && first[name_end].is_ascii_alphanumeric() {
        name_end += 1;
    }
    let open = *first.get(name_end)?;
    if (open == '"' || open == '\'')
        && first.get(name_end + 1) == Some(&open)
        && first.get(name_end + 2) == Some(&open)
    {
        return skip_triple_sigil(lines, line_idx, name_end + 3, open);
    }
    let close = match open {
        '/' | '|' | '"' | '\'' => open,
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => return None,
    };
    skip_bracket_sigil(lines, line_idx, name_end + 1, open, close)
}

/// Triple-quoted heredoc sigil (`~s"""..."""`).
fn skip_triple_sigil(
    lines: &[&str],
    line_idx: usize,
    start: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let mut idx = line_idx;
    let mut from = start;
    loop {
        let chars: Vec<char> = lines[idx].chars().collect();
        let mut p = from;
        while p < chars.len() {
            if chars[p] == quote
                && chars.get(p + 1) == Some(&quote)
                && chars.get(p + 2) == Some(&quote)
            {
                return Some((idx, p + 3));
            }
            p += 1;
        }
        idx += 1;
        if idx >= lines.len() {
            return None;
        }
        from = 0;
    }
}

/// Single-span sigil with nesting bracket pairs.
fn skip_bracket_sigil(
    lines: &[&str],
    line_idx: usize,
    start: usize,
    open: char,
    close: char,
) -> Option<(usize, usize)> {
    let nested = open != close;
    let mut depth = 0_usize;
    let mut idx = line_idx;
    let mut p = start;
    loop {
        let chars: Vec<char> = lines[idx].chars().collect();
        while p < chars.len() {
            if chars[p] == '\\' {
                p += 2;
                continue;
            }
            if nested && chars[p] == open {
                depth += 1;
            } else if chars[p] == close {
                if depth > 0 {
                    depth -= 1;
                } else {
                    return Some((idx, p + 1));
                }
            }
            p += 1;
        }
        idx += 1;
        if idx >= lines.len() {
            return None;
        }
        p = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn few_quotes_is_clean() {
        assert!(check("x = \"a\\\"b\"\n", &BTreeMap::new()).is_empty());
    }
    #[test]
    fn reports_many_quotes() {
        let src = "x = \"a\\\"b\\\"c\\\"d\\\"e\"\n";
        assert_eq!(check(src, &BTreeMap::new()).len(), 1);
    }
    #[test]
    fn message_names_quote_count_and_trigger_is_value() {
        let src = "defmodule M do\n  @m \"f\\\"\\\"b\\\"\\\"\"\nend\n";
        let findings = check(src, &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "More than 3 quotes found inside string literal, consider using a sigil instead."
        );
        assert_eq!(findings[0].column, None);
    }
}
