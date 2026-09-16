use crate::{Finding, helpers};
use std::collections::BTreeMap;

const DEFAULT_MESSAGE: &str = "Pass an `:async` boolean option to `use` a test case module.";
const MISSING_COMMENT_MESSAGE: &str =
    "Tests with `async: false` need a comment explaining why they can't be run asynchronously.";

/// `EX4031`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let force_comment = helpers::param_bool(params, "force_comment_on_explicit_false", false);
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let raw: Vec<&str> = source.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        for pos in use_positions(line) {
            match uses_case_async(&lines, idx, pos) {
                Async::Absent => findings.push(Finding::with_trigger(
                    idx + 1,
                    trigger_column(line, "use"),
                    DEFAULT_MESSAGE,
                    "use".to_owned(),
                )),
                Async::ExplicitFalse => {
                    if force_comment && !is_comment_line(raw.get(idx.wrapping_sub(1))) {
                        findings.push(Finding::with_trigger(
                            idx + 1,
                            trigger_column(line, "use"),
                            MISSING_COMMENT_MESSAGE,
                            "use".to_owned(),
                        ));
                    }
                }
                Async::Present => {}
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Whether the previous line is entirely a `#` comment, mirroring upstream's
/// `line_content =~ ~r/^\s*#.*$/` check on the line above the `use`.
fn is_comment_line(line: Option<&&str>) -> bool {
    line.is_some_and(|text| {
        let trimmed = text.trim_start();
        trimmed.starts_with('#')
    })
}

fn uses_case_async(lines: &[&str], idx: usize, pos: usize) -> Async {
    let after_use = skip_inline_ws(&lines[idx][pos + 3..]);
    let Some((last, after_mod)) = parse_module_path(after_use) else {
        return Async::Present;
    };
    if !last.ends_with("Case") {
        return Async::Present;
    }
    let mut window = after_mod.to_owned();
    let mut next = idx;
    while window.trim_end().ends_with(',') && next + 1 < lines.len() && next - idx < 8 {
        next += 1;
        window.push('\n');
        window.push_str(lines[next]);
    }
    async_option(&window)
}

#[derive(PartialEq, Eq)]
enum Async {
    Absent,
    ExplicitFalse,
    Present,
}

fn async_option(window: &str) -> Async {
    let chars: Vec<char> = window.chars().collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => {
                depth += 1;
                i += 1;
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            'a' if depth == 0 && is_word_at(&chars, i, "async") => {
                let mut j = i + "async".len();
                while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t' || chars[j] == '\n') {
                    j += 1;
                }
                if chars.get(j) != Some(&':') {
                    i += 1;
                    continue;
                }
                let mut k = j + 1;
                while k < chars.len() && (chars[k] == ' ' || chars[k] == '\t' || chars[k] == '\n') {
                    k += 1;
                }
                if is_word_at(&chars, k, "false") {
                    return Async::ExplicitFalse;
                }
                return Async::Present;
            }
            _ => {
                i += 1;
            }
        }
    }
    Async::Absent
}

/// Byte offsets of `use` keywords (word-bounded, not field/atom access).
fn use_positions(line: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find("use") {
        let pos = search + rel;
        if keyword_before(line, pos) && keyword_after(&line[pos + 3..]) {
            out.push(pos);
        }
        search = pos + 3;
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '?' || c == '!'
}

fn keyword_before(line: &str, pos: usize) -> bool {
    match line[..pos].chars().next_back() {
        None => true,
        Some(c) => !is_name_char(c) && c != '.' && c != ':' && c != '@',
    }
}

fn keyword_after(rest: &str) -> bool {
    rest.chars().next().is_none_or(|c| !is_name_char(c))
}

fn skip_inline_ws(text: &str) -> &str {
    let mut end = 0_usize;
    for (byte, c) in text.char_indices() {
        if c == ' ' || c == '\t' {
            end = byte + c.len_utf8();
        } else {
            break;
        }
    }
    &text[end..]
}

/// Parse a dotted module path; return its last segment and the remainder.
fn parse_module_path(text: &str) -> Option<(String, &str)> {
    let mut rest = text;
    let mut last: String;
    loop {
        let mut len = 0_usize;
        for (byte, c) in rest.char_indices() {
            if c.is_alphanumeric() || c == '_' || c == '?' || c == '!' {
                len = byte + c.len_utf8();
            } else {
                break;
            }
        }
        if len == 0 {
            return None;
        }
        last = rest[..len].to_owned();
        rest = &rest[len..];
        if rest.starts_with('.') {
            rest = &rest[1..];
        } else {
            return Some((last, rest));
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_word_at(chars: &[char], i: usize, word: &str) -> bool {
    let w: Vec<char> = word.chars().collect();
    if chars.len() < i + w.len() || chars[i..i + w.len()] != w[..] {
        return false;
    }
    let before_ok = i == 0 || (!is_name_char(chars[i - 1]) && chars[i - 1] != '.');
    let after_ok = chars.get(i + w.len()).is_none_or(|c| !is_name_char(*c));
    before_ok && after_ok
}

/// Credo's trigger column: first occurrence flanked by
/// whitespace, word boundaries, or `(`/`)`/`,`. `None` when absent
/// (e.g. a trigger at the very start of a line).
fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find(trigger) {
        let pos = search + rel;
        if column_boundary_before(line, pos, trigger) && column_boundary_after(line, pos, trigger) {
            return Some(line[..pos].chars().count() + 1);
        }
        search = pos + 1;
    }
    None
}

fn column_boundary_before(line: &str, pos: usize, trigger: &str) -> bool {
    let first = trigger.chars().next();
    match line[..pos].chars().next_back() {
        None => first.is_some_and(is_word_char),
        Some(c) => {
            c.is_whitespace() || c == '(' || c == ')' || c == ',' || boundary_flip(Some(c), first)
        }
    }
}

fn column_boundary_after(line: &str, pos: usize, trigger: &str) -> bool {
    let last = trigger.chars().next_back();
    match line[pos + trigger.len()..].chars().next() {
        None => last.is_some_and(is_word_char),
        Some(c) => {
            c.is_whitespace() || c == '(' || c == ')' || c == ',' || boundary_flip(last, Some(c))
        }
    }
}

fn boundary_flip(left: Option<char>, right: Option<char>) -> bool {
    match (left, right) {
        (Some(l), Some(r)) => is_word_char(l) != is_word_char(r),
        (Some(l), None) => is_word_char(l),
        (None, Some(r)) => is_word_char(r),
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    fn forced() -> BTreeMap<String, String> {
        BTreeMap::from([(
            "force_comment_on_explicit_false".to_owned(),
            "true".to_owned(),
        )])
    }
    #[test]
    fn async_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("use ExUnit.Case, async: true\n"),
                &params()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_missing_async() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("use ExUnit.Case\n"),
                &params()
            )
            .len(),
            1
        );
    }
    #[test]
    fn reports_case_module_with_other_opts() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(
                "defmodule ThingTest do\n  use MyApp.DataCase, bite_strength: :xtreme\nend\n",
            ),
            &params(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, Some(3));
        assert_eq!(
            findings[0].message,
            "Pass an `:async` boolean option to `use` a test case module."
        );
    }
    #[test]
    fn ignores_non_case_module_without_opts() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("defmodule FooTest do\n  use SomeModule\nend\n"),
                &params()
            )
            .is_empty()
        );
    }
    #[test]
    fn allows_explicit_async_false_by_default() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy(
                    "defmodule BlahTest do\n  use MyApp.DataCase, async: false\nend\n"
                ),
                &params()
            )
            .is_empty()
        );
    }
    #[test]
    fn allows_async_from_call() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("defmodule FooTest do\n  use MyApp.DataCase, async: Application.compile_env(:my_app, :async, true)\nend\n"),&params())
        .is_empty());
    }
    #[test]
    fn forced_explicit_false_without_comment_reports() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(
                "defmodule FooTest do\n  use MyApp.DataCase, async: false\nend\n",
            ),
            &forced(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(
            findings[0].message,
            "Tests with `async: false` need a comment explaining why they can't be run asynchronously."
        );
    }
    #[test]
    fn forced_explicit_false_with_comment_is_clean() {
        let src = "defmodule FooTest do\n  # why\n  use MyApp.DataCase, async: false\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &forced()).is_empty());
    }
    #[test]
    fn forced_non_literal_async_is_clean() {
        let src =
            "defmodule FooTest do\n  @async false\n  use MyApp.DataCase, async: @async\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &forced()).is_empty());
    }
}
