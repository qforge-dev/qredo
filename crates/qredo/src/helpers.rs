//! Shared text scanning helpers for rule kernels.
//!
//! These operate on raw source text without a full Elixir parser. They provide
//! comment/string-aware scanning sufficient for single-file kernels with
//! default parameters. Full pipeline parity (scopes, priorities, suppression,
//! project aggregation) remains unsupported.

use std::collections::BTreeMap;

/// Get a string parameter with a default.
#[must_use]
pub fn param_str<'a>(params: &'a BTreeMap<String, String>, key: &str, default: &'a str) -> &'a str {
    params.get(key).map_or(default, String::as_str)
}

/// Get a boolean parameter with a default, following Elixir truthiness:
/// everything except `"false"` counts as true (`nil` params fail closed
/// at config load and never reach kernels).
#[must_use]
pub fn param_bool(params: &BTreeMap<String, String>, key: &str, default: bool) -> bool {
    params.get(key).map_or(default, |v| v != "false")
}

/// Get an integer parameter with a default.
#[must_use]
pub fn param_usize(params: &BTreeMap<String, String>, key: &str, default: usize) -> usize {
    params
        .get(key)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

/// Mask strings, chars, and comments with spaces (preserving newlines and
/// byte length for ASCII delimiters). This lets text checks ignore `;`, `,`,
/// `#`, TODO, etc. inside literals. It is an approximation: sigils, heredocs
/// with custom delimiters, and escapes are handled on a best-effort basis.
#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "single-pass scanner with string/heredoc/comment states; splitting would obscure byte-index invariants"
)]
#[must_use]
pub fn mask_strings_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0_usize;
    let mut heredoc: Option<String> = None;
    while i < bytes.len() {
        // Inside a heredoc: copy until closing delimiter line.
        if let Some(delim) = heredoc.clone() {
            if source[i..].starts_with(&delim) {
                for _ in 0..delim.len() {
                    out.push(bytes[i]);
                    i += 1;
                }
                heredoc = None;
                continue;
            }
            let b = bytes[i];
            if b == b'\n' {
                out.push(b);
            } else {
                out.push(b' ');
            }
            // Advance by UTF-8 char length.
            let ch_len = utf8_len(source, i);
            i += ch_len;
            continue;
        }
        // Line comment.
        if bytes[i] == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(b' ');
                i += utf8_len(source, i);
            }
            continue;
        }
        // Heredoc open `"""` or `'''`.
        if source[i..].starts_with("\"\"\"") || source[i..].starts_with("'''") {
            let delim = source[i..i + 3].to_owned();
            for _ in 0..3 {
                out.push(bytes[i]);
                i += 1;
            }
            heredoc = Some(delim);
            continue;
        }
        // Sigils: `~name<open>...<close>` with escapes, nesting bracket
        // pairs and triple-quoted heredoc forms (`~s"""..."""`).
        if bytes[i] == b'~' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_alphabetic() {
            let mut name_end = i + 1;
            while name_end < bytes.len() && bytes[name_end].is_ascii_alphanumeric() {
                name_end += 1;
            }
            if name_end < bytes.len() {
                let open = bytes[name_end];
                let triple = name_end + 2 < bytes.len()
                    && bytes[name_end + 1] == open
                    && bytes[name_end + 2] == open
                    && (open == b'"' || open == b'\'');
                // Copy `~name` verbatim so later scans still see it.
                for byte in &bytes[i..name_end] {
                    out.push(*byte);
                }
                i = name_end;
                if triple {
                    for _ in 0..3 {
                        out.push(bytes[i]);
                        i += 1;
                    }
                    mask_until_triple(source, bytes, &mut out, &mut i, open);
                } else if let Some(close) = sigil_close(open) {
                    out.push(open);
                    i += 1;
                    mask_until_sigil_end(source, bytes, &mut out, &mut i, (open, close));
                }
                continue;
            }
        }
        // Double-quoted string.
        if bytes[i] == b'"' {
            out.push(b'"');
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(b' ');
                    out.push(b' ');
                    i += escape_len(source, i);
                    continue;
                }
                if bytes[i] == b'"' {
                    out.push(b'"');
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' {
                    out.push(b'\n');
                    i += 1;
                } else {
                    out.push(b' ');
                    i += utf8_len(source, i);
                }
            }
            continue;
        }
        // Single-quoted charlist.
        if bytes[i] == b'\'' {
            out.push(b'\'');
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(b' ');
                    out.push(b' ');
                    i += escape_len(source, i);
                    continue;
                }
                if bytes[i] == b'\'' {
                    out.push(b'\'');
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' {
                    out.push(b'\n');
                    i += 1;
                } else {
                    out.push(b' ');
                    i += utf8_len(source, i);
                }
            }
            continue;
        }
        // `?x` char literal: mask both characters, but only where an expression
        // can start. A `?` directly after a name character is a trailing
        // `?`/`!` name suffix (e.g. `question?`), not a literal. A multibyte
        // predecessor conservatively counts as a literal start.
        if bytes[i] == b'?'
            && i + 1 < bytes.len()
            && bytes[i + 1] != b'\n'
            && bytes[i + 1] != b' '
            && (i == 0 || !is_name_byte(bytes[i - 1]))
        {
            out.push(b' ');
            out.push(b' ');
            i += escape_len(source, i);
            continue;
        }
        // Any other byte starts a code character: copy it whole so later
        // indexes stay on UTF-8 boundaries.
        let len = utf8_len(source, i);
        out.extend_from_slice(&bytes[i..i + len]);
        i += len;
    }
    // SAFETY: we only replaced content with spaces, preserving UTF-8 boundaries
    // via `utf8_len` steps and copying delimiter bytes verbatim.
    String::from_utf8(out).unwrap_or_else(|_| source.to_owned())
}

/// Length in bytes of the two-character escape starting at byte `i`
/// (backslash plus one character, which may be multibyte). Masking pushes two
/// spaces for the two characters, preserving char alignment.
fn escape_len(source: &str, i: usize) -> usize {
    1 + utf8_len(source, i + 1)
}

/// Matching closer for a sigil opener, if it opens a sigil span.
fn sigil_close(open: u8) -> Option<u8> {
    match open {
        b'/' | b'|' | b'"' | b'\'' => Some(open),
        b'(' => Some(b')'),
        b'[' => Some(b']'),
        b'{' => Some(b'}'),
        b'<' => Some(b'>'),
        _ => None,
    }
}

/// Mask a triple-quoted sigil body; the closing triple is copied verbatim.
fn mask_until_triple(source: &str, bytes: &[u8], out: &mut Vec<u8>, i: &mut usize, delimiter: u8) {
    while *i < bytes.len() {
        if bytes[*i] == b'\\' && *i + 1 < bytes.len() {
            out.push(b' ');
            out.push(b' ');
            *i += escape_len(source, *i);
            continue;
        }
        if *i + 2 < bytes.len()
            && bytes[*i] == delimiter
            && bytes[*i + 1] == delimiter
            && bytes[*i + 2] == delimiter
        {
            for _ in 0..3 {
                out.push(bytes[*i]);
                *i += 1;
            }
            return;
        }
        if bytes[*i] == b'\n' {
            out.push(b'\n');
            *i += 1;
        } else {
            out.push(b' ');
            *i += utf8_len(source, *i);
        }
    }
}

/// Mask a sigil body; brackets nest, other delimiters end at the first
/// unescaped closer. The closer is copied verbatim.
fn mask_until_sigil_end(
    source: &str,
    bytes: &[u8],
    out: &mut Vec<u8>,
    i: &mut usize,
    delimiters: (u8, u8),
) {
    let (open, close) = delimiters;
    let nested = open != close;
    let mut depth = 0_usize;
    while *i < bytes.len() {
        if bytes[*i] == b'\\' && *i + 1 < bytes.len() {
            out.push(b' ');
            out.push(b' ');
            *i += escape_len(source, *i);
            continue;
        }
        if nested && bytes[*i] == open {
            depth += 1;
            out.push(b' ');
            *i += 1;
            continue;
        }
        if bytes[*i] == close {
            if nested && depth > 0 {
                depth -= 1;
                out.push(b' ');
                *i += 1;
                continue;
            }
            out.push(close);
            *i += 1;
            return;
        }
        if bytes[*i] == b'\n' {
            out.push(b'\n');
            *i += 1;
        } else {
            out.push(b' ');
            *i += utf8_len(source, *i);
        }
    }
}

fn utf8_len(source: &str, idx: usize) -> usize {
    source[idx..].chars().next().map_or(1, char::len_utf8)
}

/// Byte index just past the sigil starting at `i`, if `i` opens one.
/// Returns `None` for operators (`~>`) and unclosed spans.
fn skip_sigil(source: &str, bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'~') {
        return None;
    }
    let mut name_end = i + 1;
    if name_end >= bytes.len() || !bytes[name_end].is_ascii_alphabetic() {
        return None;
    }
    while name_end < bytes.len() && bytes[name_end].is_ascii_alphanumeric() {
        name_end += 1;
    }
    let open = *bytes.get(name_end)?;
    if open == b'"' || open == b'\'' {
        let triple =
            bytes.get(name_end + 1) == Some(&open) && bytes.get(name_end + 2) == Some(&open);
        if triple {
            return skip_sigil_triple(bytes, name_end + 3, open);
        }
    }
    let close = sigil_close(open)?;
    skip_sigil_span(source, bytes, name_end + 1, open, close)
}

/// Byte index past a triple-quoted sigil body starting after the delimiter.
fn skip_sigil_triple(bytes: &[u8], mut i: usize, delimiter: u8) -> Option<usize> {
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if i + 2 < bytes.len()
            && bytes[i] == delimiter
            && bytes[i + 1] == delimiter
            && bytes[i + 2] == delimiter
        {
            return Some(i + 3);
        }
        i += 1;
    }
    None
}

/// Byte index past a sigil body; brackets nest, other delimiters end at the
/// first unescaped closer.
fn skip_sigil_span(source: &str, bytes: &[u8], mut i: usize, open: u8, close: u8) -> Option<usize> {
    let nested = open != close;
    let mut depth = 0_usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += escape_len(source, i);
            continue;
        }
        if nested && bytes[i] == open {
            depth += 1;
            i += 1;
            continue;
        }
        if bytes[i] == close {
            if nested && depth > 0 {
                depth -= 1;
                i += 1;
                continue;
            }
            return Some(i + 1);
        }
        // Step by characters so multibyte content cannot desync the scan.
        i += utf8_len(source, i);
    }
    None
}

/// Advance `(i, line_no, col_no)` past the byte range `i..end`.
fn advance_past(
    source: &str,
    i: usize,
    end: usize,
    line_no: usize,
    col_no: usize,
) -> (usize, usize, usize) {
    let skipped = &source[i..end];
    let newlines = skipped.matches('\n').count();
    if newlines == 0 {
        (end, line_no, col_no + skipped.chars().count())
    } else {
        let col = skipped
            .rsplit('\n')
            .next()
            .map_or(1, |last| last.chars().count() + 1);
        (end, line_no + newlines, col)
    }
}

/// Byte index just past the `?x` char literal at `i`, if `i` starts one.
/// A `?` after a name character is a suffix, not a literal.
fn skip_char_literal(source: &str, bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'?') {
        return None;
    }
    if i + 1 >= bytes.len() || bytes[i + 1] == b'\n' || bytes[i + 1] == b' ' {
        return None;
    }
    if i > 0 && is_name_byte(bytes[i - 1]) {
        return None;
    }
    Some(i + escape_len(source, i))
}

/// Byte index of the `\n` ending the comment opened by `#` at `i`
/// (or the input end).
fn comment_end(bytes: &[u8], i: usize) -> usize {
    let mut end = i + 1;
    while end < bytes.len() && bytes[end] != b'\n' {
        end += 1;
    }
    end
}

/// Extract `#` comments as `(line_no, column_1based, text_after_hash)`.
#[allow(
    clippy::too_many_lines,
    reason = "comment scanner tracks heredoc/string states; split would duplicate state machine"
)]
#[must_use]
pub fn comments(source: &str) -> Vec<(usize, usize, String)> {
    let mut result = Vec::new();
    let mut line_no = 1_usize;
    let mut col_no = 1_usize;
    let bytes = source.as_bytes();
    let mut i = 0_usize;
    let mut in_double = false;
    let mut in_single = false;
    let mut in_heredoc: Option<String> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line_no += 1;
            col_no = 1;
            i += 1;
            in_double = false;
            in_single = false;
            continue;
        }
        if let Some(delim) = in_heredoc.clone() {
            if source[i..].starts_with(&delim) {
                i += delim.len();
                col_no += delim.len();
                in_heredoc = None;
                continue;
            }
            i += utf8_len(source, i);
            col_no += 1;
            continue;
        }
        if !in_double && !in_single {
            if source[i..].starts_with("\"\"\"") || source[i..].starts_with("'''") {
                in_heredoc = Some(source[i..i + 3].to_owned());
                i += 3;
                col_no += 3;
                continue;
            }
            if b == b'"' {
                in_double = true;
                i += 1;
                col_no += 1;
                continue;
            }
            if b == b'\'' {
                in_single = true;
                i += 1;
                col_no += 1;
                continue;
            }
            if let Some(end) = skip_sigil(source, bytes, i) {
                (i, line_no, col_no) = advance_past(source, i, end, line_no, col_no);
                continue;
            }
            // `?x` char literals (e.g. `?"`) must not open phantom strings.
            // Like the text mask, a `?` after a name character is a suffix.
            if let Some(end) = skip_char_literal(source, bytes, i) {
                i = end;
                col_no += 2;
                continue;
            }
            if b == b'#' {
                // Comment to end of line.
                let start_col = col_no;
                let end = comment_end(bytes, i);
                result.push((line_no, start_col, source[i + 1..end].to_owned()));
                i = end;
                continue;
            }
        } else {
            if b == b'\\' && i + 1 < bytes.len() {
                i += escape_len(source, i);
                col_no += 2;
                continue;
            }
            if in_double && b == b'"' {
                in_double = false;
            } else if in_single && b == b'\'' {
                in_single = false;
            }
        }
        i += utf8_len(source, i);
        col_no += 1;
    }
    result
}

/// Byte that can continue an Elixir identifier (ASCII subset).
fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

/// Find function/macro definitions `def name`, `defp name`, etc.
/// Returns `(line_no, column_1based, name)`.
#[must_use]
pub fn def_names(source: &str) -> Vec<(usize, usize, String)> {
    let masked = mask_strings_comments(source);
    let mut out = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        for op in [
            "def ",
            "defp ",
            "defmacro ",
            "defmacrop ",
            "defguard ",
            "defguardp ",
            "defdelegate ",
        ] {
            let mut search = 0_usize;
            while let Some(pos) = line[search..].find(op) {
                let name_start = search + pos + op.len();
                let rest = &line[name_start..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
                    .collect();
                if !name.is_empty() && name != "unquote" {
                    out.push((idx + 1, name_start + 1, name.clone()));
                }
                search = name_start + name.len().max(1);
                if search >= line.len() {
                    break;
                }
            }
        }
    }
    out
}

/// Find module definitions `defmodule Name`.
#[must_use]
pub fn module_names(source: &str) -> Vec<(usize, usize, String)> {
    let masked = mask_strings_comments(source);
    let mut out = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let mut search = 0_usize;
        while let Some(pos) = line[search..].find("defmodule ") {
            let name_start = search + pos + "defmodule ".len();
            let rest = &line[name_start..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if !name.is_empty() {
                out.push((idx + 1, name_start + 1, name.clone()));
            }
            search = name_start + name.len().max(1);
            if search >= line.len() {
                break;
            }
        }
    }
    out
}

/// Find variable usages/assignments heuristically: identifiers starting with
/// lowercase or underscore that are not keywords/atoms.
#[must_use]
#[allow(clippy::too_many_lines, reason = "token scanner with column tracking")]
pub fn variable_tokens(source: &str) -> Vec<(usize, usize, String)> {
    let masked = mask_strings_comments(source);
    let keywords = [
        "def",
        "defp",
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
        "after",
    ];
    let mut out = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut col = 0_usize;
        while col < chars.len() {
            let c = chars[col];
            if c == ':' || c == '@' || c.is_ascii_uppercase() {
                col += 1;
                continue;
            }
            if c.is_ascii_lowercase() || c == '_' {
                let start = col;
                let mut end = col;
                while end < chars.len()
                    && (chars[end].is_alphanumeric()
                        || chars[end] == '_'
                        || chars[end] == '?'
                        || chars[end] == '!')
                {
                    end += 1;
                }
                let name: String = chars[start..end].iter().collect();
                // Skip field access `.name` and calls already handled.
                let prev = if start > 0 {
                    Some(chars[start - 1])
                } else {
                    None
                };
                if prev != Some('.') && prev != Some(':') && !keywords.contains(&name.as_str()) {
                    out.push((idx + 1, start + 1, name));
                }
                col = end;
            } else {
                col += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_survives_multibyte_char_literal() {
        // Previously panicked: byte-stepping split `μ`.
        let masked = mask_strings_comments("x = ?μ\n");
        assert_eq!(masked.chars().count(), "x = ?μ\n".chars().count());
    }

    #[test]
    fn mask_survives_multibyte_escape() {
        let masked = mask_strings_comments("x = \"a\\μb\"\n");
        assert_eq!(masked.chars().count(), "x = \"a\\μb\"\n".chars().count());
    }

    #[test]
    fn mask_keeps_code_char_columns() {
        let masked = mask_strings_comments("λ = 1\n");
        assert!(masked.contains('λ'));
    }

    #[test]
    fn comments_survive_multibyte_escapes() {
        let found = comments("x = 1 # μ \\μ end\n");
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn comments_ignore_sigil_contents() {
        let found = comments("    x = ~s{also: # not comment}\n    y = 1 # real\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 2);
    }

    #[test]
    fn comments_follow_char_literals() {
        let found = comments("    ?\" # real\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, 8);
    }

    #[test]
    fn char_hash_is_not_a_comment() {
        assert!(comments("x = ?#\n").is_empty());
    }

    fn bool_params(value: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("flag".to_owned(), value.to_owned())])
    }

    #[test]
    fn bool_params_follow_elixir_truthiness() {
        // Upstream `if flag do`: everything except `false`/`nil` is truthy.
        // `nil` params fail closed at config load and never reach kernels.
        assert!(param_bool(&bool_params("true"), "flag", false));
        assert!(param_bool(&bool_params("yes"), "flag", false));
        assert!(param_bool(&bool_params("0"), "flag", false));
        assert!(!param_bool(&bool_params("false"), "flag", true));
    }

    #[test]
    fn missing_bool_params_use_the_default() {
        assert!(param_bool(&BTreeMap::new(), "flag", true));
        assert!(!param_bool(&BTreeMap::new(), "flag", false));
    }
}
