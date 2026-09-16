use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3023`: single-element pipelines should be function calls.
///
/// Groups `|>` operators into chains across continuation lines; a chain with
/// exactly one pipe reports unless its left side is an allowed shape
/// (`allow_blocks`, `allow_lists`, `allow_maps`, `allow_0_arity_functions`).
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let masked: Vec<&str> = prepared.masked().split('\n').collect();
    let raw: Vec<&str> = source.split('\n').collect();
    let mut findings = Vec::new();
    for group in group_pipe_lines(&masked) {
        let total: usize = group
            .iter()
            .map(|line| masked[*line].matches("|>").count())
            .sum();
        if total != 1 {
            continue;
        }
        let line_idx = group[0];
        let pipe_at = masked[line_idx].find("|>").unwrap_or(0);
        if let Some(finding) = single_issue(&masked, &raw, line_idx, pipe_at, params) {
            findings.push(finding);
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// Lines holding `|>` clustered into chains: neighbours join across blank
/// lines when the lower starts with `|>` or the upper ends with one.
fn group_pipe_lines(masked: &[&str]) -> Vec<Vec<usize>> {
    let pipe_lines: Vec<usize> = masked
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("|>"))
        .map(|(idx, _)| idx)
        .collect();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for line_idx in pipe_lines {
        let join = groups.last().and_then(|group| {
            let prev = *group.last()?;
            if masked[prev + 1..line_idx]
                .iter()
                .all(|mid| mid.trim().is_empty())
                && (masked[line_idx].trim_start().starts_with("|>")
                    || masked[prev].trim_end().ends_with("|>"))
            {
                Some(())
            } else {
                None
            }
        });
        if join.is_some() {
            if let Some(group) = groups.last_mut() {
                group.push(line_idx);
            }
        } else {
            groups.push(vec![line_idx]);
        }
    }
    groups
}

/// Issue for a lone pipe, unless params allow its left-hand shape.
fn single_issue(
    masked: &[&str],
    raw: &[&str],
    line_idx: usize,
    pipe_at: usize,
    params: &BTreeMap<String, String>,
) -> Option<Finding> {
    let line = masked[line_idx];
    // `pipe_at` is an ASCII match offset: slicing is boundary-safe.
    let (head, _) = line.split_at(pipe_at);
    if head.trim().is_empty() {
        return piped_base_issue(masked, raw, line_idx, params);
    }
    if has_block_shape(head) {
        if helpers::param_bool(params, "allow_blocks", true) {
            return None;
        }
        return Some(report(raw, line_idx));
    }
    plain_issue(head, raw, line_idx, params)
}

/// Issue for a pipe starting its line: classify the base line above.
fn piped_base_issue(
    masked: &[&str],
    raw: &[&str],
    line_idx: usize,
    params: &BTreeMap<String, String>,
) -> Option<Finding> {
    let left = base_line(masked, line_idx)?;
    if left.contains("|>") {
        return None;
    }
    if left.trim() == "end"
        && matches!(block_opener(masked, line_idx)?, Opener::Do)
        && helpers::param_bool(params, "allow_blocks", true)
    {
        return None;
    }
    plain_issue(&left, raw, line_idx, params)
}

/// Issue for a plain left-hand side unless params allow its literal shape.
fn plain_issue(
    left: &str,
    raw: &[&str],
    line_idx: usize,
    params: &BTreeMap<String, String>,
) -> Option<Finding> {
    let trimmed = left.trim();
    if is_pure_list(trimmed) && helpers::param_bool(params, "allow_lists", false) {
        return None;
    }
    if is_pure_map(trimmed) && helpers::param_bool(params, "allow_maps", false) {
        return None;
    }
    if is_zero_arity(trimmed) && helpers::param_bool(params, "allow_0_arity_functions", false) {
        return None;
    }
    Some(report(raw, line_idx))
}

fn report(raw: &[&str], line_idx: usize) -> Finding {
    Finding::with_trigger(
        line_idx + 1,
        derive_column(raw.get(line_idx).copied().unwrap_or(""), "|>"),
        "Use a function call when a pipeline is only one function long.",
        "|>",
    )
}

/// Nearest non-blank line above a pipe-leading chain line.
fn base_line(masked: &[&str], line_idx: usize) -> Option<String> {
    let mut cursor = line_idx;
    while cursor > 0 {
        cursor -= 1;
        if !masked[cursor].trim().is_empty() {
            return Some(masked[cursor].to_owned());
        }
    }
    None
}

#[derive(PartialEq, Eq)]
enum Opener {
    Do,
    Fn,
}

/// Whether the `end` closing a piped block opened `do` or `fn`.
fn block_opener(masked: &[&str], line_idx: usize) -> Option<Opener> {
    let mut pending = 0_i32;
    let mut cursor = line_idx;
    loop {
        for (_, word) in block_words(masked[cursor]).iter().rev() {
            match *word {
                "end" => pending += 1,
                _ => {
                    if pending > 0 {
                        pending -= 1;
                    } else {
                        return Some(if *word == "do" {
                            Opener::Do
                        } else {
                            Opener::Fn
                        });
                    }
                }
            }
        }
        if cursor == 0 {
            break;
        }
        cursor -= 1;
    }
    None
}

/// `do` (not `do:`), `fn`, and `end` words on one masked line, left to right.
fn block_words(line: &str) -> Vec<(usize, &'static str)> {
    let mut words = Vec::new();
    for word in ["do", "fn", "end"] {
        for (pos, _) in line.match_indices(word) {
            if keyword_at(line, pos, word)
                && !(word == "do" && line[pos + word.len()..].starts_with(':'))
            {
                words.push((pos, word));
            }
        }
    }
    words.sort_unstable();
    words
}

/// Same-line block shape such as `for x <- y do x end` before the pipe.
fn has_block_shape(left: &str) -> bool {
    let Some(do_at) = word_after(left, 0, "do") else {
        return false;
    };
    word_after(left, do_at, "end").is_some()
}

/// Offset past word `word` at/after `from`, with strict boundaries.
fn word_after(left: &str, from: usize, word: &str) -> Option<usize> {
    for (pos, _) in left.match_indices(word) {
        if pos < from {
            continue;
        }
        if keyword_at(left, pos, word)
            && !(word == "do" && left[pos + word.len()..].starts_with(':'))
        {
            return Some(pos + word.len());
        }
    }
    None
}

/// `[1, 2]` (or `'abc'`): a bracket-matched literal with nothing after it.
fn is_pure_list(trimmed: &str) -> bool {
    if trimmed.starts_with('\'') {
        return trimmed.len() > 1 && trimmed.ends_with('\'');
    }
    if !trimmed.starts_with('[') {
        return false;
    }
    matching_bracket(trimmed, 0).is_some_and(|close| trimmed[close + 1..].trim().is_empty())
}

/// `%{a: 1}` or `%Name{}`: brace-matched literal with nothing after it.
fn is_pure_map(trimmed: &str) -> bool {
    if !trimmed.starts_with('%') {
        return false;
    }
    let Some(open) = trimmed.find('{') else {
        return false;
    };
    matching_bracket(trimmed, open).is_some_and(|close| trimmed[close + 1..].trim().is_empty())
}

/// Byte offset of the bracket closing the opener at `open`.
fn matching_bracket(text: &str, open: usize) -> Option<usize> {
    // ASCII delimiters only: every index stays a char boundary.
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
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

/// Call with empty parens: `foo()`, `Mod.fun()`, `fun.()`.
fn is_zero_arity(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    if bytes.last() != Some(&b')') || bytes.len() < 3 {
        return false;
    }
    let mut idx = bytes.len() - 1;
    loop {
        if idx == 0 {
            return false;
        }
        idx -= 1;
        if bytes[idx] != b' ' && bytes[idx] != b'\t' {
            break;
        }
    }
    bytes[idx] == b'('
}

fn keyword_at(line: &str, pos: usize, word: &str) -> bool {
    let bytes = line.as_bytes();
    if pos > 0 {
        let prev = bytes[pos - 1];
        if prev.is_ascii_alphanumeric()
            || prev == b'_'
            || prev == b'?'
            || prev == b'!'
            || prev == b':'
            || prev == b'@'
            || prev == b'.'
            || prev == b'&'
        {
            return false;
        }
    }
    // `pos` comes from `match_indices` and `word` is ASCII.
    if line[pos + word.len()..]
        .chars()
        .next()
        .is_some_and(is_name_char)
    {
        return false;
    }
    true
}

fn is_name_char(next: char) -> bool {
    next.is_alphanumeric() || next == '_' || next == '?' || next == '!'
}

/// Mirror of `Credo.SourceFile.column/3`: first trigger occurrence flanked by
/// whitespace, parens, commas, or word boundaries (byte-based, 1-based).
fn derive_column(line: &str, trigger: &str) -> Option<usize> {
    if trigger.is_empty() {
        return None;
    }
    let bytes = line.as_bytes();
    let first = trigger.as_bytes()[0];
    let last = trigger.as_bytes()[trigger.len() - 1];
    for (pos, _) in line.match_indices(trigger) {
        if boundary_before(bytes, pos, first) && boundary_after(bytes, pos + trigger.len(), last) {
            return Some(pos + 1);
        }
    }
    None
}

fn boundary_before(bytes: &[u8], pos: usize, first: u8) -> bool {
    if pos == 0 {
        return is_word_byte(first);
    }
    let prev = bytes[pos - 1];
    is_delim_byte(prev) || (is_word_byte(prev) != is_word_byte(first))
}

fn boundary_after(bytes: &[u8], end: usize, last: u8) -> bool {
    if end >= bytes.len() {
        return is_word_byte(last);
    }
    let next = bytes[end];
    is_delim_byte(next) || (is_word_byte(last) != is_word_byte(next))
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_delim_byte(byte: u8) -> bool {
    byte.is_ascii_whitespace() || byte == b'(' || byte == b')' || byte == b','
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_pipe_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("foo(x)\n"), &BTreeMap::new()).is_empty()
        );
    }
    #[test]
    fn reports_single_pipe() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("x |> foo()\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn chained_pipes_are_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x |> foo() |> bar()\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn two_singles_both_report() {
        let src = "defmodule M do\n  use ExUnit.Case\n\n  def some_fun do\n    some_val |> do_something\n    some_other_val\n    |> do_something\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].line, 5);
        assert_eq!(findings[0].column, Some(14));
        assert_eq!(findings[1].line, 7);
        assert_eq!(findings[1].column, Some(5));
    }
    #[test]
    fn zero_arity_param_skips_calls() {
        let mut params = BTreeMap::new();
        params.insert("allow_0_arity_functions".to_owned(), "true".to_owned());
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("foo() |> bar()\n"), &params).is_empty()
        );
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(":foo |> bar()\n"), &params).len(),
            1
        );
    }
    #[test]
    fn block_end_base_reports_when_disallowed() {
        let mut params = BTreeMap::new();
        params.insert("allow_blocks".to_owned(), "false".to_owned());
        let src = "defmodule M do\n  def f do\n    for x <- y do\n      x\n    end\n    |> Enum.reverse()\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 6);
    }
    #[test]
    fn block_end_base_skips_by_default() {
        let src = "defmodule M do\n  def f do\n    for x <- y do\n      x\n    end\n    |> Enum.reverse()\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn map_and_list_params() {
        let mut maps = BTreeMap::new();
        maps.insert("allow_maps".to_owned(), "true".to_owned());
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("%{a: 1}\n|> Map.put(:c, 3)\n"),
                &maps
            )
            .is_empty()
        );
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("%{a: 1}\n|> Map.put(:c, 3)\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
        let mut lists = BTreeMap::new();
        lists.insert("allow_lists".to_owned(), "true".to_owned());
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("[1, 2]\n|> Enum.sum()\n"),
                &lists
            )
            .is_empty()
        );
    }
}
