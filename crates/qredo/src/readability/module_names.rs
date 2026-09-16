use crate::Finding;
use std::collections::BTreeMap;

/// `EX3010`: module names must be `PascalCase`.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let ignored = parse_ignore(params);
    let mut findings = Vec::new();
    for (line, col, dotted) in module_entries(prepared.masked()) {
        if dotted.split('.').all(is_pascal_case) {
            continue;
        }
        if is_ignored(&ignored, &dotted) {
            continue;
        }
        findings.push(Finding::with_trigger(
            line,
            Some(col),
            "Module names should be written in PascalCase.",
            dotted,
        ));
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Credo `Name.pascal_case?/1` applied per segment.
fn is_pascal_case(segment: &str) -> bool {
    let mut chars = segment.chars();
    match chars.next() {
        Some(first) if first.is_ascii_uppercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric())
}

enum IgnorePattern {
    Exact(String),
    Regex(regex::Regex),
}

/// `ignore` arrives as compact JSON: strings, `:atom` strings, or
/// `{"regex": source}` objects.
fn parse_ignore(params: &BTreeMap<String, String>) -> Vec<IgnorePattern> {
    let Some(raw) = params.get("ignore") else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let items = match &value {
        serde_json::Value::Array(items) => items.clone(),
        single => vec![single.clone()],
    };
    let mut out = Vec::new();
    for item in items {
        match &item {
            serde_json::Value::String(text) => {
                // Nested atoms keep their colon in compact JSON; Elixir atoms
                // also carry the historical `Elixir.` prefix.
                let atom = text.strip_prefix(':').unwrap_or(text);
                let name = atom.strip_prefix("Elixir.").unwrap_or(atom).to_owned();
                out.push(IgnorePattern::Exact(name));
            }
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(pattern)) = map.get("regex")
                    && let Ok(compiled) = regex::Regex::new(pattern)
                {
                    out.push(IgnorePattern::Regex(compiled));
                }
            }
            _ => {}
        }
    }
    out
}

fn is_ignored(patterns: &[IgnorePattern], module: &str) -> bool {
    patterns.iter().any(|pattern| match pattern {
        IgnorePattern::Exact(name) => name == module,
        IgnorePattern::Regex(re) => re.is_match(module),
    })
}

/// `(line, char-column, dotted-name)` for `defmodule` aliases.
/// Non-alias heads (`unquote(...)`, vars) are skipped like the AST walk.
fn module_entries(masked: &str) -> Vec<(usize, usize, String)> {
    const PREFIX: &[u8] = b"defmodule ";
    let mut out = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        // The prefix carries the match; lines without it cannot match.
        if !line.contains("defmodule") {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut pos = 0_usize;
        while pos < chars.len() {
            if match_prefix(&chars, pos, PREFIX) {
                let name_start = pos + PREFIX.len();
                let mut name_end = name_start;
                while name_end < chars.len()
                    && (chars[name_end].is_alphanumeric() || matches!(chars[name_end], '_' | '.'))
                {
                    name_end += 1;
                }
                let name: String = chars[name_start..name_end].iter().collect();
                let is_alias = name.chars().next().is_some_and(|c| c.is_ascii_uppercase());
                if is_alias && !name.is_empty() {
                    out.push((idx + 1, name_start + 1, name));
                }
                pos = name_end.max(pos + 1);
            } else {
                pos += 1;
            }
        }
    }
    out
}

/// ASCII-literal prefix with identifier boundary, allocation-free.
fn match_prefix(chars: &[char], pos: usize, prefix: &[u8]) -> bool {
    chars.len() >= pos + prefix.len()
        && chars[pos..pos + prefix.len()]
            .iter()
            .zip(prefix.iter())
            .all(|(got, want)| *got == *want as char)
        && (pos == 0 || !(chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("defmodule Foo.Bar do\nend\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn reports_snake_case_module() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("defmodule Credo_SampleModule do\nend\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }

    #[test]
    fn ignored_module_is_clean() {
        let mut params = BTreeMap::new();
        params.insert("ignore".to_owned(), "[\"Sample_Module\"]".to_owned());
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("defmodule Sample_Module do\nend\n"),
                &params
            )
            .is_empty()
        );
    }
    #[test]
    fn defmodule_substrings_do_not_match() {
        // "my_defmodule_x" carries the gated substring but no keyword.
        let src = "my_defmodule_x = 1\ndefmodule Foo do\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
}
