use crate::{Finding, helpers};

/// `EX2003`: `@tag :skip` without a preceding comment.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let raw: Vec<&str> = source.split('\n').collect();
    let commented: std::collections::BTreeSet<usize> = helpers::comments(source)
        .iter()
        .map(|(line, _, _)| *line)
        .collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if find_tag_skip(line).is_some() && !commented.contains(&idx) {
            // One issue per tag, matching the AST walk over each `@` node.
            let count = count_tag_skips(line);
            for _ in 0..count {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    fallback_column(raw[idx], "@tag :skip"),
                    "Tests tagged to be skipped should have a comment preceding the `@tag :skip`.",
                    "@tag :skip".to_owned(),
                ));
            }
        }
    }
    findings
}

/// Byte index of a `@tag :skip` attribute value on the line, if present.
fn find_tag_skip(line: &str) -> Option<usize> {
    tag_skip_at(line, 0)
}

/// Number of `@tag :skip` attributes on the line.
fn count_tag_skips(line: &str) -> usize {
    let mut count = 0_usize;
    let mut search = 0_usize;
    while let Some(base) = tag_skip_at(line, search) {
        count += 1;
        search = base + 1;
    }
    count
}

fn tag_skip_at(line: &str, from: usize) -> Option<usize> {
    let mut search = from;
    while let Some(rel) = line[search..].find("@tag") {
        let base = search + rel;
        if before_ok(line, base) && value_is_skip(line, base + "@tag".len()) {
            return Some(base);
        }
        search = base + 1;
    }
    None
}

fn before_ok(line: &str, base: usize) -> bool {
    if base == 0 {
        return true;
    }
    let prev = line[..base].chars().next_back().unwrap_or(' ');
    !(prev.is_alphanumeric() || prev == '_' || prev == '@')
}

/// True when `@tag` is followed by a `:skip` value (not `@tags`, `:skipped`).
fn value_is_skip(line: &str, end: usize) -> bool {
    let chars: Vec<char> = line[end..].chars().collect();
    let mut idx = 0_usize;
    while idx < chars.len() && (chars[idx] == ' ' || chars[idx] == '\t') {
        idx += 1;
    }
    if idx < chars.len() && chars[idx] == '(' {
        idx += 1;
        while idx < chars.len() && (chars[idx] == ' ' || chars[idx] == '\t') {
            idx += 1;
        }
    }
    let tail: String = chars[idx..].iter().collect();
    if !tail.starts_with(":skip") {
        return false;
    }
    match tail[":skip".len()..].chars().next() {
        Some(next) => next.is_whitespace() || next == ')' || next == ']' || next == '#',
        None => true,
    }
}

/// Native column fallback: first `trigger` occurrence with Credo's boundary
/// rule (`SourceFile.column/3`), as a 1-based byte column.
fn fallback_column(line: &str, trigger: &str) -> Option<usize> {
    let first = trigger.chars().next()?;
    let last = trigger.chars().next_back()?;
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find(trigger) {
        let base = search + rel;
        if before_col_ok(line, base, first) && after_col_ok(line, base + trigger.len(), last) {
            return Some(base + 1);
        }
        search = base + 1;
    }
    None
}

fn before_col_ok(line: &str, base: usize, first: char) -> bool {
    if base == 0 {
        return first.is_alphanumeric() || first == '_';
    }
    let prev = line[..base].chars().next_back().unwrap_or(' ');
    prev.is_whitespace() || "()[],".contains(prev) || is_word(prev) != is_word(first)
}

fn after_col_ok(line: &str, end: usize, last: char) -> bool {
    if end >= line.len() {
        return last.is_alphanumeric() || last == '_';
    }
    let next = line[end..].chars().next().unwrap_or(' ');
    next.is_whitespace() || "()[],".contains(next) || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Single-source entry: the same lazy parse the old `check` built.
    fn check(src: &str) -> Vec<Finding> {
        check_prepared(&crate::batch::Prepared::lazy(src))
    }

    #[test]
    fn commented_skip_is_clean() {
        let src = "# reason\n@tag :skip\ntest \"x\", do: 1\n";
        assert!(check(src).is_empty());
    }
    #[test]
    fn reports_bare_skip() {
        assert_eq!(check("@tag :skip\ntest \"x\", do: 1\n").len(), 1);
    }
    #[test]
    fn reports_with_exact_trigger_and_column() {
        let src = "defmodule CredoSampleModuleTest do\n  alias ExUnit.Case\n\n  @tag :skip\n  test \"foo\" do\n    :ok\n  end\nend\n";
        assert_eq!(
            check(src),
            vec![Finding::with_trigger(
                4,
                Some(3),
                "Tests tagged to be skipped should have a comment preceding the `@tag :skip`.",
                "@tag :skip",
            )]
        );
    }
    #[test]
    fn trailing_comment_after_tag_still_reports() {
        // A comment after the tag (not on the previous line) does not excuse it.
        let src = "defmodule T do\n  @tag :skip\n  # another comment, shouldn't matter\n  test \"foo2\" do\n  end\nend\n";
        assert_eq!(check(src).len(), 1);
    }
}
