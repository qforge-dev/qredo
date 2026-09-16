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
            match if_else_kind(&lines, idx, start) {
                IfElse::Keyword => {
                    if !allow_one_liners {
                        findings.push(issue(idx + 1, line));
                    }
                }
                IfElse::Block => findings.push(issue(idx + 1, line)),
                IfElse::None => {}
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

/// How the `if` at a site carries its `else`, if at all.
#[derive(PartialEq, Eq)]
enum IfElse {
    /// Same-line or continued keyword `else:` (no `:end`; one-liner class
    /// that `allow_one_liners` suppresses).
    Keyword,
    /// Bare `do..else..end` block (always reported).
    Block,
    /// No `else` belonging to this `if`.
    None,
}

/// Classify the `if` starting at byte `start` on line `from`: scan tokens
/// from the `if` across continuation lines and let the first decisive token
/// at nesting zero win. A bare `do` there opens this `if`'s own block (its
/// body then decides); an `else:` there is a keyword else; a bare
/// `else`/`end` first belongs to an outer construct, so an `else`-less
/// `if` never adopts it.
fn if_else_kind(lines: &[&str], from: usize, start: usize) -> IfElse {
    let mut nest = 0_usize;
    let mut brackets = 0_usize;
    // An `else:` directly inside the `if`'s own call parens still belongs
    // to the `if` (`if(v, do: 1, else: 2)` reports). Only the first
    // substantive token decides: a leading `(` opens the call args, while
    // any other opener (`if f(else: 1)`) owns deeper colons itself.
    // Residual: `if(else: 1)`-as-condition would miscount; no such shape
    // occurs outside pathological input.
    let paren_first = first_substantive_is_paren(lines[from], start + 2);
    let mut idx = from;
    let mut pos = start + 2;
    loop {
        let line = lines[idx];
        for (token, end) in block_tokens_at(line, pos) {
            match token {
                BlockToken::Open => {
                    if nest == 0 && brackets == 0 {
                        if block_body_has_else(lines, idx, end) {
                            return IfElse::Block;
                        }
                        return IfElse::None;
                    }
                    nest += 1;
                }
                BlockToken::ElseColon if nest == 0 && brackets == 0 => {
                    return IfElse::Keyword;
                }
                BlockToken::ElseColon if nest == 0 && brackets == 1 && paren_first => {
                    return IfElse::Keyword;
                }
                BlockToken::Else | BlockToken::End if nest == 0 => return IfElse::None,
                BlockToken::End => nest -= 1,
                BlockToken::OpenBracket => brackets += 1,
                BlockToken::CloseBracket => brackets = brackets.saturating_sub(1),
                BlockToken::Else | BlockToken::ElseColon => {}
            }
        }
        if idx + 1 >= lines.len() {
            return IfElse::None;
        }
        // Follow only while the statement continues: a trailing comma,
        // opener, operator or `and`/`or`/`not`/`in`, or an unclosed nest.
        let tail = if idx == from { &line[start..] } else { line };
        if nest == 0 && brackets == 0 && !line_continues(tail) {
            return IfElse::None;
        }
        idx += 1;
        pos = 0;
    }
}

/// True when the first substantive character after an `if` on its line
/// opens the call parens (`if(` or `if (`).
fn first_substantive_is_paren(line: &str, from: usize) -> bool {
    line.get(from..)
        .is_some_and(|rest| rest.trim_start().starts_with('('))
}

/// True when a bare `else` precedes the matching `end` after the `if`'s own
/// `do` (which ends at byte `open_end` on line `from`).
fn block_body_has_else(lines: &[&str], from: usize, open_end: usize) -> bool {
    let mut depth = 0_usize;
    for (offset, line) in lines[from..].iter().enumerate() {
        let pos = if offset == 0 { open_end } else { 0 };
        for (token, _) in block_tokens_at(line, pos) {
            match token {
                BlockToken::Open => depth += 1,
                BlockToken::Else if depth == 0 => return true,
                BlockToken::End if depth == 0 => return false,
                BlockToken::End => depth -= 1,
                BlockToken::Else
                | BlockToken::ElseColon
                | BlockToken::OpenBracket
                | BlockToken::CloseBracket => {}
            }
        }
    }
    false
}

/// True when the text keeps the statement going on the next line.
fn line_continues(text: &str) -> bool {
    let trimmed = text.trim_end();
    let Some(last) = trimmed.chars().next_back() else {
        return false;
    };
    if matches!(last, ',' | '(' | '[' | '{') {
        return true;
    }
    if last.is_alphanumeric() || last == '_' {
        let word: String = trimmed
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return matches!(word.as_str(), "and" | "or" | "not" | "in");
    }
    matches!(
        last,
        '+' | '-' | '*' | '/' | '=' | '<' | '>' | '!' | '&' | '|' | '\\' | '.' | '^' | '~' | ':'
    )
}

#[derive(PartialEq, Eq)]
enum BlockToken {
    Open,
    Else,
    ElseColon,
    End,
    OpenBracket,
    CloseBracket,
}

/// `(token, byte end offset)` for `do`/`fn`/`else`/`end` keywords and
/// brackets in source order, starting at byte `from`.
fn block_tokens_at(line: &str, from: usize) -> Vec<(BlockToken, usize)> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut idx = from.min(bytes.len());
    while idx < bytes.len() {
        if let Some((token, width)) = token_at(line, bytes, idx) {
            tokens.push((token, idx + width));
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
    let bracket = match bytes[idx] {
        b'(' | b'[' | b'{' => BlockToken::OpenBracket,
        b')' | b']' | b'}' => BlockToken::CloseBracket,
        _ => return None,
    };
    Some((bracket, 1))
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
    fn keyword_if_inside_own_call_parens_reports() {
        // Both labqoat shapes: `, do: if(` and guard-prefixed heads.
        for source in [
            "def f(v), do: if(v, do: 1, else: 2)\n",
            "defp scalar(value) when is_boolean(value), do: if(value, do: \"TRUE\", else: \"FALSE\")\n",
            "defp choice(value, choices),\n  do: if(value in choices, do: {:ok, value}, else: {:error, :invalid_filter})\n",
        ] {
            assert_eq!(
                check_prepared(&crate::batch::Prepared::lazy(source), &BTreeMap::new()).len(),
                1,
                "{source:?}"
            );
        }
    }
    #[test]
    fn nested_call_keyword_else_stays_clean() {
        // The `else:` belongs to the inner call, not the `if`.
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def f(x), do: if(x, do: g(else: 1))\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
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
    fn reports_split_keyword_if_else() {
        // Triage C-B: `else:` on the lines below the `if`.
        let src = "def c(attrs) do\n  value =\n    if Map.has_key?(attrs, \"fields\"),\n      do: Map.get(attrs, \"fields\"),\n      else: Map.get(attrs, :fields, :default)\n\n  value\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
    }
    #[test]
    fn reports_split_condition_keyword_if_else() {
        // Triage C-B variant: condition spans lines with `or`, `do:`/`else:` later.
        let src = "def f(d) do\n  if ready?(d) or\n       active?(d.id),\n    do: start(d),\n    else: sync(d)\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (2, Some(3)));
    }
    #[test]
    fn reports_block_if_with_multiline_condition() {
        // Triage C-C: bare `do` on the condition's continuation line.
        let src = "def f(a, b) do\n  if a == b and\n       c?(a, b) do\n    :ok\n  else\n    :error\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (2, Some(3)));
    }
    #[test]
    fn reports_block_if_with_fn_in_condition() {
        // Triage C-C variant (runs.ex:2164): `fn..end` inside the condition.
        let src = "def f(entries) do\n  if Enum.all?(entries, fn entry ->\n       ready?(entry)\n     end) do\n    :ok\n  else\n    :error\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (2, Some(3)));
    }
    #[test]
    fn inner_keyword_if_without_else_is_clean() {
        // Triage C-D: the inner `if` must not adopt the outer block's `else`.
        let src =
            "def nest(a, b) do\n  if a do\n    if b, do: :x\n    :y\n  else\n    :z\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (2, Some(3)));
    }
    #[test]
    fn keyword_if_inside_with_else_is_clean() {
        // Triage C-D: `with..else` is not the keyword `if`'s `else`.
        let src = "def withfp(x) do\n  with {:ok, y} <- g(x) do\n    if y, do: y\n  else\n    :error\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert!(findings.is_empty());
    }
    #[test]
    fn later_block_does_not_lend_its_do() {
        // A keyword `if` without `else` followed by an unrelated block `if/else`.
        let src = "def f(x, a) do\n  if x, do: :x\n  if a do\n    :y\n  else\n    :z\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(3)));
    }
    #[test]
    fn allow_one_liners_suppresses_split_keyword_else() {
        // Split keyword `if/else` is still keyword form (no `:end` upstream).
        let src = "def c(attrs) do\n  value =\n    if Map.has_key?(attrs, \"fields\"),\n      do: Map.get(attrs, \"fields\"),\n      else: Map.get(attrs, :fields, :default)\n\n  value\nend\n";
        let mut params = BTreeMap::new();
        params.insert("allow_one_liners".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
        let block = "def f(a, b) do\n  if a == b and\n       c?(a, b) do\n    :ok\n  else\n    :error\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(block), &params).len(),
            1
        );
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
