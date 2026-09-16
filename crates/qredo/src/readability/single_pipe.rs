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
    let masked_string = prepared.masked();
    let masked: Vec<&str> = masked_string.split('\n').collect();
    let raw: Vec<&str> = source.split('\n').collect();
    let pipes = pipe_positions(masked_string);
    let chars: Vec<char> = masked_string.chars().collect();
    let bytes: Vec<usize> = masked_string.char_indices().map(|(byte, _)| byte).collect();
    let starts = line_starts(masked_string);
    let mut findings = Vec::new();
    let chains = group_pipe_chains(&chars, &bytes, &pipes);
    let lone: Vec<usize> = chains
        .iter()
        .filter(|chain| chain.len() == 1)
        .map(|chain| chain[0])
        .collect();
    for pipe in &lone {
        // Lone singles rejoin across balanced `fn` bodies: link-following
        // for chains like `pod |> update_in(..., fn ... end) |>
        // update_in(...)`, where the inner pipes group separately but the
        // outers form one chain.
        if joins_any_pipe(&chars, &bytes, &pipes, *pipe) {
            continue;
        }
        let (line_idx, pipe_at) = line_pipe_at(&starts, &masked, *pipe);
        if let Some(finding) = single_issue(&masked, &raw, line_idx, pipe_at, params) {
            findings.push(finding);
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// True when a lone pipe joins any other pipe by plain text
/// connectivity (link-following for chains spanning `fn` arguments whose
/// inner pipes group separately). Grouping splits conservatively at
/// block boundaries; a textual connection across the split means one
/// expression, hence one chain.
fn joins_any_pipe(chars: &[char], bytes: &[usize], pipes: &[usize], pipe: usize) -> bool {
    pipes.iter().any(|other| {
        *other != pipe && {
            let (first, second) = if pipe < *other {
                (pipe, *other)
            } else {
                (*other, pipe)
            };
            pipes_connected(chars, bytes, first, second)
        }
    })
}

/// Byte offsets of `|>` operators in a chain: consecutive pipes join when no
/// top-level statement boundary lies between them, so multiline arguments
/// (`[...]`, `(...)`, `fn...end`) no longer split AST chains.
fn group_pipe_chains(chars: &[char], bytes: &[usize], pipes: &[usize]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for pipe in pipes {
        let join = groups.last().and_then(|group| {
            let prev = *group.last()?;
            if pipes_connected(chars, bytes, prev, *pipe) {
                Some(())
            } else {
                None
            }
        });
        if join.is_some() {
            if let Some(group) = groups.last_mut() {
                group.push(*pipe);
            }
        } else {
            groups.push(vec![*pipe]);
        }
    }
    groups
}

/// Whether two consecutive `|>` byte offsets belong to one chain: the text
/// between them must not cross a depth-zero statement boundary.
fn pipes_connected(chars: &[char], bytes: &[usize], prev: usize, curr: usize) -> bool {
    pipes_connected_inner(chars, bytes, prev, curr)
}

fn pipes_connected_inner(chars: &[char], bytes: &[usize], prev: usize, curr: usize) -> bool {
    let mut start = char_index(bytes, prev) + 2;
    let end = char_index(bytes, curr);
    let mut depth = 0_usize;
    while start < end && start < chars.len() {
        match chars[start] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth > 0 {
                    depth -= 1;
                } else {
                    return false;
                }
            }
            '\n' if depth == 0 => {
                if newline_stops(chars, start) {
                    return false;
                }
            }
            '=' | ',' | ';' if depth == 0 => return false,
            _ if depth == 0 => {
                if word_ending_at_con(chars, start) {
                    return false;
                }
            }
            _ => {}
        }
        start += 1;
    }
    true
}

const CHAIN_STOP_WORDS: &[&str] = &[
    "do", "else", "if", "unless", "case", "cond", "with", "for", "try", "quote", "receive",
    "catch", "rescue", "after", "when", "in", "not", "and", "or", "end",
];

fn word_ending_at_con(chars: &[char], i: usize) -> bool {
    CHAIN_STOP_WORDS
        .iter()
        .any(|w| word_ending_at(chars, i, w.as_bytes()))
}

/// Whether `word` ends at char `i` with identifier boundaries.
fn word_ending_at(chars: &[char], i: usize, word: &[u8]) -> bool {
    if word.is_empty() || i + 1 < word.len() {
        return false;
    }
    if !(chars[i + 1 - word.len()..=i]
        .iter()
        .zip(word.iter())
        .all(|(got, want)| *got == *want as char))
    {
        return false;
    }
    if i + 1 == word.len() {
        return true;
    }
    let prev = chars[i - word.len()];
    if is_name_char(prev) || prev == '.' || prev == ':' || prev == '@' {
        return false;
    }
    chars.get(i + 1).is_none_or(|c| !is_name_char(*c))
}

/// Whether a newline at char `i` ends the previous statement.
fn newline_stops(chars: &[char], i: usize) -> bool {
    // A pipe opener on the next line continues the expression, even after a
    // bare word (`do_something\n    |> ...` is one chain).
    if let Some((c, at)) = next_non_ws(chars, i)
        && c == '|'
        && chars.get(at + 1) == Some(&'>')
    {
        return false;
    }
    if let Some(c) = prev_non_ws(chars, i) {
        if is_continuation_char(c) {
            return false;
        }
        if is_name_char(c) {
            if !prev_word_is(chars, i, b"end") {
                return true;
            }
        } else if !(c == ')' || c == ']' || c == '}' || c == '"' || c == '\'') {
            return true;
        }
    } else {
        return true;
    }
    match next_non_ws(chars, i) {
        None => true,
        Some((c, at)) => {
            if !is_continuation_start(c) {
                return true;
            }
            c == '-' && chars.get(at + 1) == Some(&'>')
        }
    }
}

fn is_continuation_char(c: char) -> bool {
    matches!(
        c,
        '|' | '>' | '<' | '=' | '+' | '*' | '/' | '&' | '.' | '~' | '^' | ',' | '(' | '[' | '{'
    )
}

fn is_continuation_start(c: char) -> bool {
    matches!(
        c,
        '|' | '+' | '-' | '*' | '/' | '>' | '<' | '&' | '.' | '~' | '^'
    )
}

fn prev_non_ws(chars: &[char], i: usize) -> Option<char> {
    let mut j = i;
    while j > 0 {
        j -= 1;
        if !chars[j].is_whitespace() {
            return Some(chars[j]);
        }
    }
    None
}

fn next_non_ws(chars: &[char], i: usize) -> Option<(char, usize)> {
    let mut j = i + 1;
    while j < chars.len() {
        if !chars[j].is_whitespace() {
            return Some((chars[j], j));
        }
        j += 1;
    }
    None
}

fn prev_word_is(chars: &[char], i: usize, word: &[u8]) -> bool {
    let mut j = i;
    while j > 0 && chars[j - 1].is_whitespace() {
        j -= 1;
    }
    j >= word.len()
        && chars[j - word.len()..j]
            .iter()
            .zip(word.iter())
            .all(|(got, want)| *got == *want as char)
        && (j < word.len() + 1 || !is_name_char(chars[j - word.len() - 1]))
}

fn char_index(bytes: &[usize], pos: usize) -> usize {
    bytes.partition_point(|byte| *byte < pos)
}

fn pipe_positions(masked: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut search = 0_usize;
    while let Some(rel) = masked[search..].find("|>") {
        out.push(search + rel);
        search += rel + "|>".len();
    }
    out
}

fn line_starts(masked: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    for (byte, c) in masked.char_indices() {
        if c == '\n' {
            starts.push(byte + 1);
        }
    }
    starts
}

/// Line index and in-line byte offset of a `|>` byte offset.
fn line_pipe_at(starts: &[usize], masked: &[&str], pipe: usize) -> (usize, usize) {
    let line_idx = starts
        .partition_point(|start| *start <= pipe)
        .saturating_sub(1);
    let pipe_at = pipe - starts.get(line_idx).copied().unwrap_or(0);
    let _ = masked.get(line_idx);
    (line_idx, pipe_at)
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
    #[test]
    fn multiline_args_chain_is_clean() {
        let src = "def f(q, attrs) do\n  q\n  |> cast(attrs, [\n    :a,\n    :b\n  ])\n  |> validate_required([:a])\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn fn_block_chain_is_clean() {
        let src = "def g(headers) do\n  headers\n  |> Enum.reject(fn {name, _value} ->\n    name in [\"x\"]\n  end)\n  |> Enum.map(fn {name, value} -> name end)\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn inner_fn_pipes_do_not_merge_outward() {
        // `pod |> update_in(..., fn ... end) |> update_in(...)`: both
        // outers form one chain when the block holds no pipes.
        let src = "def trust(pod, secret) do\n  pod\n  |> update_in([\"a\"], fn c ->\n    Enum.map(c, fn x -> x end)\n  end)\n  |> update_in([\"b\"], &(&1 ++ [secret]))\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn pipes_inside_fn_body_stay_inner() {
        // Inner chain (2 pipes) and outer chain (2 pipes) are each clean;
        // the inner pipes must not join the outer chain into singles.
        let src = "def trust(pod) do\n  pod\n  |> update_in([\"a\"], fn c ->\n    c\n    |> Enum.map(fn x -> x end)\n    |> Enum.filter(fn x -> x end)\n  end)\n  |> update_in([\"b\"], pod)\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn true_single_after_chain_still_reports() {
        let src = "def f(q, attrs) do\n  q\n  |> cast(attrs, [\n    :a\n  ])\n  |> validate_required([:a])\nend\n\ndef h(x) do\n  x |> foo()\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 10);
    }
}
