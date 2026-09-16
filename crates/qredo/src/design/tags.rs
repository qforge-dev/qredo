use crate::{Finding, Trigger, helpers};
use std::collections::BTreeMap;

const DOC_ATTRS: [&str; 3] = ["@doc", "@moduledoc", "@shortdoc"];

/// Find `tag` in `#` comments: `#` at the start or not preceded by `?`,
/// then optional whitespace, the tag case-insensitively with optional colon,
/// then more text. Returns the trimmed match starting at `#`.
fn match_comment_tag(full: &str, tag: &str) -> Option<String> {
    let positions: Vec<(usize, char)> = full.char_indices().collect();
    let mut index = 0_usize;
    while index < positions.len() {
        let (byte, character) = positions[index];
        if character == '#' && (index == 0 || positions[index - 1].1 != '?') {
            let rest = &full[byte + 1..];
            let stripped = rest.trim_start();
            // Char-based prefix: byte slicing panics on multibyte comments.
            let mut chars = stripped.chars();
            let prefix: String = chars.by_ref().take(tag.len()).collect();
            if prefix.len() == tag.len() && prefix.eq_ignore_ascii_case(tag) {
                let rest: String = chars.collect();
                let after = rest.strip_prefix(':').unwrap_or(&rest);
                if !after.trim_start().is_empty() {
                    return Some(full[byte..].trim().to_owned());
                }
            }
        }
        index += 1;
    }
    None
}

/// Whether attribute `string` holds the tag: optional leading whitespace,
/// the tag case-insensitively with optional colon, then more text.
fn match_doc_tag(string: &str, tag: &str) -> bool {
    let stripped = string.trim_start();
    if stripped.len() < tag.len() || !stripped[..tag.len()].eq_ignore_ascii_case(tag) {
        return false;
    }
    let after = stripped[tag.len()..]
        .strip_prefix(':')
        .unwrap_or(&stripped[tag.len()..]);
    !after.trim_start().is_empty()
}

/// Extract a string literal starting at `rest` (`"..."`, `'...'`, heredoc).
/// Returns the raw inner text (no escape processing), the last line, and the
/// char offset just past the closing quote for single-line literals (`None`
/// for heredocs).
fn read_literal(
    lines: &[&str],
    first: usize,
    rest: &str,
) -> Option<(String, usize, Option<usize>)> {
    if rest.starts_with("\"\"\"") || rest.starts_with("'''") {
        let delimiter = &rest[..3];
        return read_heredoc(lines, first, delimiter).map(|(value, last)| (value, last, None));
    }
    read_quoted(rest).map(|(value, end)| (value, first, Some(end)))
}

/// Read a heredoc opening on `lines[first]`; returns the Elixir value
/// approximation and the closing line.
fn read_heredoc(lines: &[&str], first: usize, delimiter: &str) -> Option<(String, usize)> {
    let opener = lines[first];
    let body = opener
        .find(delimiter)
        .map_or("", |at| opener[at + delimiter.len()..].trim_end());
    let mut content: Vec<String> = vec![body.to_owned()];
    let mut line = first + 1;
    let mut closing_indent = 0_usize;
    let mut closed = false;
    while line < lines.len() {
        if lines[line].trim() == delimiter {
            closing_indent = lines[line]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            closed = true;
            break;
        }
        content.push(lines[line].to_owned());
        line += 1;
    }
    if !closed {
        return None;
    }
    // Elixir strips the closing delimiter's indentation from heredoc lines.
    let mut dedented: Vec<String> = content
        .iter()
        .map(|content_line| {
            let mut stripped = content_line.as_str();
            for _ in 0..closing_indent {
                if let Some(rest) = stripped.strip_prefix([' ', '\t']) {
                    stripped = rest;
                } else {
                    break;
                }
            }
            stripped.to_owned()
        })
        .collect();
    // Elixir strips the newline immediately following the opener.
    if dedented.first().is_some_and(String::is_empty) {
        dedented.remove(0);
    }
    let mut value = dedented.join("\n");
    value.push('\n');
    Some((value, line))
}

/// Read a single-line `"..."`/`'...'` literal at the start of `rest`.
/// Returns the value and the char offset just past the closing quote.
fn read_quoted(rest: &str) -> Option<(String, usize)> {
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut value = String::new();
    let mut escaped = false;
    let mut offset = 1_usize;
    for character in rest[1..].chars() {
        offset += character.len_utf8();
        if escaped {
            value.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote {
            return Some((value, offset));
        } else {
            value.push(character);
        }
    }
    None
}

/// Column backfill replicating `Credo.Check.add_column_if_missing`: locate the
/// trigger in its line with a boundary-sensitive search. `None` when absent.
fn backfilled_column(line: &str, trigger: &str) -> Option<usize> {
    let pattern = format!(
        r"(\s|\b|\(|\)|,)({})(\s|\b|\(|\)|,)",
        regex::escape(trigger)
    );
    let expression = regex::Regex::new(&pattern).ok()?;
    let captures = expression.captures(line)?;
    let matched = captures.get(2)?;
    Some(line[..matched.start()].chars().count() + 1)
}

fn find_tags(source: &str, tag: &str, include_doc: bool) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (line_no, col, body) in helpers::comments(source) {
        let full = format!("#{body}");
        if let Some(trigger) = match_comment_tag(&full, tag) {
            findings.push(Finding::with_trigger(
                line_no,
                Some(col),
                format!("Found a {tag} tag in a comment: {trigger}"),
                trigger,
            ));
        }
    }
    if include_doc {
        findings.extend(find_doc_tags(source, tag));
    }
    // Canonical ascending kernel order (native `run/2` accumulation order is
    // an internal artifact; presentation sorts by line/column).
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// Tag matches inside `@doc`/`@moduledoc`/`@shortdoc` string attributes.
fn find_doc_tags(source: &str, tag: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let lines: Vec<&str> = source.split('\n').collect();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(attr) = DOC_ATTRS.iter().find(|attr| {
            trimmed.starts_with(*attr) && trimmed[attr.len()..].starts_with(char::is_whitespace)
        }) else {
            continue;
        };
        let at_col = line.find('@').map_or(1, |col| col + 1);
        let rest = trimmed[attr.len()..].trim_start();
        if rest.starts_with('~') {
            continue;
        }
        let Some((value, _last, tail)) = read_literal(&lines, idx, rest) else {
            continue;
        };
        // A single-line literal must stand alone (trailing comments aside);
        // `@doc "x" <> y` is not a plain doc string in the AST either.
        if let Some(end) = tail {
            let after = rest[end..].trim_start();
            if !after.is_empty() && !after.starts_with('#') {
                continue;
            }
        }
        if !match_doc_tag(&value, tag) {
            continue;
        }
        if value.contains('\n') {
            findings.push(Finding::with_trigger(
                idx + 2,
                Some(at_col),
                format!("Found a {tag} tag in a comment: {}", value.trim_end()),
                value.trim_end().to_owned(),
            ));
        } else {
            let trigger = value.trim_end().to_owned();
            findings.push(Finding {
                line: idx + 1,
                column: backfilled_column(lines[idx], &trigger),
                message: format!("Found a {tag} tag in a comment: {trigger}"),
                trigger: Trigger::Text(trigger),
                severity: None,
            });
        }
    }
    findings
}

/// `EX2005`
pub(crate) fn check_todo(source: &str, params: &BTreeMap<String, String>) -> Vec<Finding> {
    let include_doc = helpers::param_bool(params, "include_doc", true);
    find_tags(source, "TODO", include_doc)
}

/// `EX2004`
pub(crate) fn check_fixme(source: &str, params: &BTreeMap<String, String>) -> Vec<Finding> {
    let include_doc = helpers::param_bool(params, "include_doc", true);
    find_tags(source, "FIXME", include_doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_has_no_tags() {
        assert!(check_todo("x = 1\n", &BTreeMap::new()).is_empty());
        assert!(check_fixme("x = 1\n", &BTreeMap::new()).is_empty());
    }
    #[test]
    fn reports_todo() {
        assert_eq!(check_todo("# TODO: fix me\n", &BTreeMap::new()).len(), 1);
    }
    #[test]
    fn reports_fixme() {
        assert_eq!(check_fixme("# FIXME: broken\n", &BTreeMap::new()).len(), 1);
    }
    #[test]
    fn comment_column_points_at_hash() {
        let findings = check_todo("  # TODO: fix me\n", &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(3));
    }
    #[test]
    fn matches_case_insensitive_with_full_comment_trigger() {
        let findings = check_fixme("  # fixme blah blah\n", &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        match &findings[0].trigger {
            crate::Trigger::Text(trigger) => assert_eq!(trigger, "# fixme blah blah"),
            crate::Trigger::NoTrigger => panic!("expected text trigger"),
        }
    }
    #[test]
    fn single_line_doc_backfills_column() {
        let src = "defmodule M do\n  @doc \"TODO: fix me\"\nend\n";
        let findings = check_todo(src, &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(9));
    }
    #[test]
    fn heredoc_doc_points_past_attribute() {
        let src = "defmodule M do\n  @moduledoc \"\"\"\n  TODO: fix me\n  \"\"\"\nend\n";
        let findings = check_todo(src, &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
        assert_eq!(findings[0].column, Some(3));
    }
    #[test]
    fn doc_ignored_when_excluded() {
        let src = "defmodule M do\n  @doc \"TODO: fix me\"\nend\n";
        let mut params = BTreeMap::new();
        params.insert("include_doc".to_owned(), "false".to_owned());
        assert!(check_todo(src, &params).is_empty());
    }
    #[test]
    fn fixme_doc_ignored_when_excluded() {
        let src = "defmodule M do\n  @doc \"FIXME: broken\"\nend\n";
        assert_eq!(check_fixme(src, &BTreeMap::new()).len(), 1);
        let mut params = BTreeMap::new();
        params.insert("include_doc".to_owned(), "false".to_owned());
        assert!(check_fixme(src, &params).is_empty());
    }
}
