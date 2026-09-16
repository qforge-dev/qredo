use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3029`: no trailing whitespace; `ignore_strings: true` (default) skips
/// lines inside heredocs/strings.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let ignore_strings = helpers::param_bool(params, "ignore_strings", true);
    let masked = prepared.masked();
    let mut findings = Vec::new();
    // Masked rows collected once: indexing per line keeps this linear
    // instead of re-splitting the whole source on every row.
    let mask_lines: Vec<&str> = masked.split('\n').collect();
    for (idx, line) in source.split('\n').enumerate() {
        let line_no = idx + 1;
        // Skip lines that are entirely inside a masked string when ignoring.
        let mask_line = mask_lines.get(idx).copied().unwrap_or("");
        // Trailing horizontal whitespace: space or tab.
        let trimmed_end = line.trim_end_matches([' ', '\t']);
        if trimmed_end.len() == line.len() {
            continue;
        }
        // `\r` (CR in CRLF) is not trailing whitespace for this check.
        let trailing: String = line[trimmed_end.len()..]
            .chars()
            .filter(|c| *c == ' ' || *c == '\t')
            .collect();
        if trailing.is_empty() {
            continue;
        }
        // Empty/whitespace-only lines are not reported (see #1235 fixture).
        if trimmed_end.is_empty() {
            continue;
        }
        if ignore_strings {
            // If the visible (non-string) content is empty, the line is
            // string/heredoc content; skip.
            let visible = mask_line.trim_end_matches([' ', '\t']);
            if visible.is_empty() {
                continue;
            }
            // If trailing whitespace is inside a string literal on this line,
            // the masked line won't show it; only report when masked line
            // also has trailing whitespace.
            let mask_trimmed = mask_line.trim_end_matches([' ', '\t']);
            if mask_trimmed.len() == mask_line.len() {
                continue;
            }
        }
        let column = trimmed_end.chars().count() + 1;
        findings.push(Finding::with_trigger(
            line_no,
            Some(column),
            "There should be no trailing white-space at the end of a line.",
            trailing,
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn defaults() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn clean_module_has_no_findings() {
        let src = "defmodule CredoSampleModule do\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &defaults()).is_empty());
    }

    #[test]
    fn reports_trailing_spaces() {
        let src = "defmodule CredoSampleModule do\n@test true   \nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &defaults());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, Some(11));
        assert_eq!(
            findings[0].message,
            "There should be no trailing white-space at the end of a line."
        );
    }

    #[test]
    fn ignores_heredoc_content_by_default() {
        let src = "defmodule M do\n  @doc '''\n  Foo  \n  Bar\n  '''\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &defaults()).is_empty());
    }

    #[test]
    fn checks_heredoc_content_when_disabled() {
        let src = "defmodule M do\n  @doc '''\n  Foo  \n  Bar\n  '''\nend\n";
        let mut params = BTreeMap::new();
        params.insert("ignore_strings".to_owned(), "false".to_owned());
        assert!(!check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
}
