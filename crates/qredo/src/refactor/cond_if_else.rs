use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX4033`: `if/else` that should be `cond`.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let allow_one_liners = helpers::param_bool(params, "allow_one_liners", false);
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        for start in if_positions(line) {
            if inline_else(line, start) {
                if !allow_one_liners {
                    findings.push(issue(idx + 1, line));
                }
            } else if has_block_else(&lines, idx) {
                findings.push(issue(idx + 1, line));
            }
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

fn issue(line_no: usize, line: &str) -> Finding {
    Finding::with_trigger(
        line_no,
        credo_column(line, "if"),
        "Consider using `cond` instead of `if/else`.",
        "if".to_owned(),
    )
}

/// Byte offsets of `if` keywords (not `@if`, atoms, or parts of words).
fn if_positions(line: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let bytes = line.as_bytes();
    let mut idx = 0_usize;
    while idx + 2 <= bytes.len() {
        if line.get(idx..).is_some_and(|rest| rest.starts_with("if"))
            && boundary_at(bytes, idx)
            && boundary_at(bytes, idx + 2)
            && prev_allows(bytes, idx)
        {
            positions.push(idx);
            idx += 2;
        } else {
            idx += 1;
        }
    }
    positions
}

fn boundary_at(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 || idx >= bytes.len() {
        return true;
    }
    !is_name_byte(bytes[idx]) || !is_name_byte(bytes[idx - 1])
}

fn prev_allows(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    prev != b'@' && prev != b':' && prev != b'.'
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

/// Inline `else:` keyword arguments on the same line after `if`.
fn inline_else(line: &str, start: usize) -> bool {
    line.get(start..)
        .is_some_and(|rest| has_word(rest, "else:"))
}

fn has_word(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut idx = 0_usize;
    while idx + needle.len() <= bytes.len() {
        if haystack
            .get(idx..)
            .is_some_and(|rest| rest.starts_with(needle))
            && boundary_at(bytes, idx)
            && boundary_at(bytes, idx + needle.len())
        {
            return true;
        }
        idx += 1;
    }
    false
}

/// Block `else` before the matching `end`, tracking nested blocks in order.
fn has_block_else(lines: &[&str], from: usize) -> bool {
    let mut depth = 0_usize;
    for line in &lines[from + 1..] {
        for token in block_tokens(line) {
            match token {
                BlockToken::Open => depth += 1,
                BlockToken::Else if depth == 0 => return true,
                BlockToken::End if depth == 0 => return false,
                BlockToken::End => depth -= 1,
                BlockToken::Else | BlockToken::ElseColon => {}
            }
        }
    }
    false
}

#[derive(PartialEq, Eq)]
enum BlockToken {
    Open,
    Else,
    ElseColon,
    End,
}

/// `do`/`fn`/`else`/`end` keywords in source order.
fn block_tokens(line: &str) -> Vec<BlockToken> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut idx = 0_usize;
    while idx < bytes.len() {
        if let Some((token, width)) = token_at(line, bytes, idx) {
            tokens.push(token);
            idx += width;
        } else {
            idx += 1;
        }
    }
    tokens
}

fn token_at(line: &str, bytes: &[u8], idx: usize) -> Option<(BlockToken, usize)> {
    for (word, token) in [
        ("else:", BlockToken::ElseColon),
        ("else", BlockToken::Else),
        ("end", BlockToken::End),
        ("do", BlockToken::Open),
        ("fn", BlockToken::Open),
    ] {
        if line.get(idx..).is_some_and(|rest| rest.starts_with(word))
            && boundary_at(bytes, idx)
            && boundary_at(bytes, idx + word.len())
            && (word != "do" || bytes.get(idx + 2) != Some(&b':'))
        {
            let width = word.len();
            return Some((token, width));
        }
    }
    None
}

/// Credo `SourceFile.column/3`: 1-based column of `trigger` when surrounded by
/// whitespace, parens, commas or word boundaries; `None` otherwise.
fn credo_column(line: &str, trigger: &str) -> Option<usize> {
    if trigger.is_empty() {
        return None;
    }
    let lchars: Vec<char> = line.chars().collect();
    let tchars: Vec<char> = trigger.chars().collect();
    if lchars.len() < tchars.len() {
        return None;
    }
    for idx in 0..=lchars.len() - tchars.len() {
        if lchars[idx..idx + tchars.len()] != tchars[..] {
            continue;
        }
        let before_ok = if idx == 0 {
            is_word(tchars[0])
        } else {
            before_ok(lchars[idx - 1], tchars[0])
        };
        let after = idx + tchars.len();
        let after_ok = if after == lchars.len() {
            is_word(tchars[tchars.len() - 1])
        } else {
            after_ok(tchars[tchars.len() - 1], lchars[after])
        };
        if before_ok && after_ok {
            return Some(idx + 1);
        }
    }
    None
}

fn before_ok(prev: char, first: char) -> bool {
    prev.is_whitespace()
        || prev == '('
        || prev == ')'
        || prev == ','
        || is_word(prev) != is_word(first)
}

fn after_ok(last: char, next: char) -> bool {
    next.is_whitespace()
        || next == '('
        || next == ')'
        || next == ','
        || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_if_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("if x, do: y\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_if_else() {
        let src = "if x do\n  y\nelse\n  z\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn reports_inline_if_else() {
        // EX4033.upstream.violation-inline-disallowed.
        let src = "defmodule CredoSampleModule do\n  def some_fun do\n    if allowed?, do: :ok, else: :error\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
    }
    #[test]
    fn reports_nested_assignment_if_else() {
        // EX4033.upstream.violation-nested: `if` is not at the line start.
        let src = "defmodule CredoSampleModule do\n  def some_fun do\n    result = if condition do\n      :yes\n    else\n      :no\n    end\n    result\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(14)));
    }
    #[test]
    fn same_line_fn_end_does_not_close() {
        // An `end` closing a same-line `fn` must not end the `else` search.
        let src = "defmodule M do\n  def f do\n    if x do\n      y = Enum.map(z, fn a -> a end)\n    else\n      w\n    end\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
    }
    #[test]
    fn allow_one_liners_suppresses_inline_only() {
        let inline =
            "defmodule M do\n  def f do\n    if allowed?, do: :ok, else: :error\n  end\nend\n";
        let block = "if x do\n  y\nelse\n  z\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(inline), &BTreeMap::new()).len(),
            1
        );
        let mut params = BTreeMap::new();
        params.insert("allow_one_liners".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(inline), &params).is_empty());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(block), &params).len(),
            1
        );
    }
}
