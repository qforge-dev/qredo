use crate::Finding;
use std::collections::BTreeMap;

/// `EX4012`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let max_lines: usize = params
        .get("max_line_count")
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    let ignore_comments = params.get("ignore_comments").is_some_and(|v| v == "true");
    let masked = prepared.masked();
    let masked_lines: Vec<&str> = masked.split('\n').collect();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in masked_lines.iter().enumerate() {
        for pos in word_positions(line, "quote") {
            if defines_quote(line, pos) || has_do_colon(&line[pos..]) {
                continue;
            }
            let Some(end_idx) = block_end(&masked_lines, idx, pos) else {
                continue;
            };
            // Lines strictly inside the block, mirroring Credo's
            // `max_line_no - quote_line` window without the closing `end`.
            let line_count = end_idx.saturating_sub(idx + 1);
            if line_count <= max_lines {
                continue;
            }
            let mut kept = line_count;
            if ignore_comments {
                kept = raw_lines[idx + 1..end_idx]
                    .iter()
                    .filter(|l| !is_comment_line(l))
                    .count();
            }
            if kept > max_lines {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    trigger_column(raw_lines[idx], "quote"),
                    "Avoid long quote blocks.",
                    "quote".to_owned(),
                ));
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// 0-based index of the `end` line closing the `quote` opener, if balanced.
fn block_end(lines: &[&str], start: usize, pos: usize) -> Option<usize> {
    let mut depth =
        count_opens(&lines[start][pos..]).saturating_sub(count_closes(&lines[start][pos..]));
    if depth == 0 {
        return None;
    }
    for (idx, line) in lines.iter().enumerate().skip(start + 1) {
        depth += count_opens(line);
        depth = depth.saturating_sub(count_closes(line));
        if depth == 0 {
            return Some(idx);
        }
    }
    None
}

/// Bare `do` openers plus `fn` (which closes with `end` but opens without `do`).
fn count_opens(line: &str) -> usize {
    count_word(line, "do", true) + count_word(line, "fn", false)
}

fn count_closes(line: &str) -> usize {
    count_word(line, "end", false)
}

/// Count `word` occurrences with identifier boundaries. For `do`, a trailing
/// colon (`do:`) is an inline keyword, not a block opener.
fn count_word(line: &str, word: &str, exclude_colon: bool) -> usize {
    word_positions(line, word)
        .iter()
        .filter(|p| !(exclude_colon && line[**p + word.len()..].starts_with(':')))
        .count()
}

/// Byte positions of `word` with identifier boundaries, excluding
/// `@`-/`:`-prefixed atoms/attributes and `.`-qualified access.
fn word_positions(line: &str, word: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut search = 0_usize;
    while search <= line.len() {
        let Some(rel) = line[search..].find(word) else {
            break;
        };
        let pos = search + rel;
        let prev_ok = line[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c) && c != ':' && c != '@' && c != '.');
        let next_ok = line[pos + word.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_word_char(c) && c != '?' && c != '!');
        if prev_ok && next_ok {
            out.push(pos);
        }
        search = pos + word.len();
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `quote` at `pos` is a `def`-family definition rather than a call.
fn defines_quote(line: &str, pos: usize) -> bool {
    let mut prev: Vec<char> = line[..pos].chars().collect();
    while prev.pop_if(|c| c.is_whitespace()).is_some() {}
    let word: String = prev
        .iter()
        .rev()
        .take_while(|c| c.is_alphanumeric() || **c == '_')
        .collect::<Vec<_>>()
        .iter()
        .rev()
        .map(|c| **c)
        .collect();
    matches!(word.as_str(), "def" | "defp" | "defmacro" | "defmacrop")
}

fn has_do_colon(rest: &str) -> bool {
    word_positions(rest, "do")
        .iter()
        .any(|pos| rest[pos + "do".len()..].starts_with(':'))
}

fn is_comment_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#')
}

/// Credo infers the column from the trigger: first occurrence flanked by
/// whitespace, a word boundary, or `(`/`)`/`,`. Byte math matches native
/// output on ASCII lines; char counting keeps multibyte lines panic-free.
fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let first = trigger.chars().next()?;
    let last = trigger.chars().next_back()?;
    let mut search = 0_usize;
    while search <= line.len() {
        let rel = line[search..].find(trigger)?;
        let pos = search + rel;
        let prev_ok = line[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || "() ,".contains(c) || boundary_between(c, first));
        let next_ok = line[pos + trigger.len()..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || "() ,".contains(c) || boundary_between(last, c));
        if prev_ok && next_ok {
            return Some(line[..pos].chars().count() + 1);
        }
        search = pos + trigger.len();
    }
    None
}

fn boundary_between(left: char, right: char) -> bool {
    is_word_char(left) != is_word_char(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("quote do\n  x\nend\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn ignored_comments_do_not_count() {
        use std::fmt::Write as _;
        let mut src = String::from(
            "defmodule M do\n  defmacro __using__(opts) do\n    quote do\n      def some_fun do\n",
        );
        for word in [
            "This", "is", "a", "rather", "long", "comment", "block", "...",
        ] {
            let _ = writeln!(src, "        # {word}");
        }
        src.push_str(
            "        some_stuff()\n      end\n\n      def some_fun do\n        some_stuff()\n      end\n    end\n  end\nend\n",
        );
        let mut params = BTreeMap::new();
        params.insert("max_line_count".to_owned(), "7".to_owned());
        params.insert("ignore_comments".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(&src), &params).is_empty());
    }
}
