use crate::Finding;
use std::collections::BTreeMap;

/// `EX3001`: avoid `alias Foo, as: Bar` unless the module is ignored.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let ignored = ignored_modules(params);
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, pair) in source.split('\n').zip(masked.split('\n')).enumerate() {
        let (raw, masked_line) = pair;
        for target in as_targets(masked_line) {
            if !ignored.iter().any(|known| known == &target) {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    derive_column(raw, "as:"),
                    "Avoid using the `:as` option with `alias`.",
                    "as:",
                ));
            }
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// Aliased module names carrying an `as:` option on one masked line.
fn as_targets(line: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for (pos, _) in line.match_indices("alias") {
        if !keyword_at(line, pos, "alias") || !alias_call_at(line, pos) {
            continue;
        }
        if let Some(target) = as_target(&line[pos + "alias".len()..]) {
            targets.push(target);
        }
    }
    targets
}

/// Module text between `alias` and its `, as:` option on one line.
fn as_target(rest: &str) -> Option<String> {
    let bytes = rest.as_bytes();
    let mut i = 0_usize;
    while i < bytes.len() {
        if bytes[i] == b',' {
            let mut end = i + 1;
            while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t') {
                end += 1;
            }
            // `i` is ASCII `,` and `end` advanced over ASCII spaces only,
            // so both are char boundaries.
            if rest[end..].starts_with("as:") {
                return Some(rest[..i].trim().to_owned());
            }
        }
        i += 1;
    }
    None
}

fn keyword_at(line: &str, pos: usize, word: &str) -> bool {
    let bytes = line.as_bytes();
    if pos > 0 {
        let prev = bytes[pos - 1];
        // Atoms (`:alias`), attributes, captures, and field access are not
        // alias calls.
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
    line[pos + word.len()..]
        .chars()
        .next()
        .is_none_or(|next| !is_name_char(next))
}

/// An `alias` call opens an uppercase target, `__MODULE__`, or parens.
/// A bare `alias` variable (`alias = 1`) never carries an `as:` option.
fn alias_call_at(line: &str, pos: usize) -> bool {
    // `pos` comes from `match_indices` and `"alias"` is ASCII.
    line[pos + "alias".len()..]
        .chars()
        .find(|next| !next.is_whitespace())
        .is_some_and(|next| next.is_ascii_uppercase() || next == '_' || next == '(')
}

/// Modules listed in the `ignore` param (compact JSON array), normalized like
/// `Credo.Code.Name.full/1` by dropping a leading `Elixir.` namespace.
fn ignored_modules(params: &BTreeMap<String, String>) -> Vec<String> {
    let Some(raw) = params.get("ignore") else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .map(|name| normalize_module(&name))
        .collect()
}

fn normalize_module(name: &str) -> String {
    name.strip_prefix(':')
        .unwrap_or(name)
        .strip_prefix("Elixir.")
        .unwrap_or(name.strip_prefix(':').unwrap_or(name))
        .to_owned()
}

fn is_name_char(next: char) -> bool {
    next.is_alphanumeric() || next == '_'
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
    fn clean_alias_has_no_findings() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("alias Foo.Bar\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn reports_as_option() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("alias Foo.Bar, as: Baz\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn respects_ignore_param() {
        let mut params = BTreeMap::new();
        params.insert("ignore".to_owned(), "[\"App.Module1\"]".to_owned());
        let src = "defmodule Test do\n  alias App.Module1, as: M1\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn ignore_strips_elixir_namespace() {
        let mut params = BTreeMap::new();
        params.insert("ignore".to_owned(), "[\"Elixir.App.Module1\"]".to_owned());
        let src = "defmodule Test do\n  alias App.Module1, as: M1\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn non_ignored_module_still_reported() {
        let mut params = BTreeMap::new();
        params.insert("ignore".to_owned(), "[\"App.Other\"]".to_owned());
        let src = "defmodule Test do\n  alias App.Module1, as: M1\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(22));
    }
}
