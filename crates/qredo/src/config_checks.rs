//! Config-context evaluation for EX2006/EX2007/EX2008.
//!
//! These checks evaluate validated `.credo.exs` data and registered config
//! comments — never raw Elixir config, which would require a BEAM. Validated
//! configs arrive as data (see the `exec_validated_config` corpus fields);
//! a missing config means "no information" and yields no findings, never
//! clean credit by confusion.

use crate::config_data::{ENABLED_STANDARD_CHECKS, STANDARD_CHECKS};
use crate::suppression::config_comments;

/// Finding for a redundant config comment.
pub struct RedundantFinding {
    /// Comment's own line.
    pub line: usize,
    /// Column of the comment hash, if located.
    pub column: Option<usize>,
    /// Always `"# credo:"`.
    pub trigger: String,
    /// Always the redundancy message.
    pub message: String,
}

/// Names in a validated `checks` list or `{enabled, disabled}` map.
fn configured_names(config: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    let mut push_entry = |entry: &serde_json::Value| {
        if let Some(check) = entry.get("check").and_then(serde_json::Value::as_str) {
            names.push(check.to_owned());
        }
    };
    match config.get("checks") {
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                push_entry(item);
            }
        }
        Some(serde_json::Value::Object(_)) => {
            for key in ["enabled", "disabled"] {
                if let Some(items) = config["checks"]
                    .get(key)
                    .and_then(serde_json::Value::as_array)
                {
                    for item in items {
                        push_entry(item);
                    }
                }
            }
        }
        _ => {}
    }
    names
}

/// Relevant check sets for `compare_to` values.
fn relevant_checks(compare_to: &str, config: &serde_json::Value) -> Vec<String> {
    match compare_to {
        ":all" | "all" => configured_names(config),
        ":credo_checks_enabled_by_default" | "credo_checks_enabled_by_default" => {
            ENABLED_STANDARD_CHECKS
                .iter()
                .map(ToString::to_string)
                .collect()
        }
        _ => STANDARD_CHECKS.iter().map(ToString::to_string).collect(),
    }
}

/// EX2007 messages for checks missing from a validated config.
/// `None` config yields no findings.
#[must_use]
pub fn missing_checks(config: Option<&serde_json::Value>, compare_to: &str) -> Vec<String> {
    let Some(config) = config else {
        return Vec::new();
    };
    let configured = configured_names(config);
    relevant_checks(compare_to, config)
        .into_iter()
        .filter(|check| !configured.iter().any(|name| name == check))
        .map(|check| format!("Check `{check}` missing in config: enable or disable it explicitly."))
        .collect()
}

/// EX2008 messages for deprecated config shapes.
/// `None` config yields no findings.
#[must_use]
pub fn deprecated_config(config: Option<&serde_json::Value>) -> Vec<String> {
    let Some(config) = config else {
        return Vec::new();
    };
    match config.get("checks") {
        Some(serde_json::Value::Array(_)) => vec![
            "Using a list for `:checks` in Credo's config is deprecated, use a map instead."
                .to_owned(),
        ],
        Some(serde_json::Value::Object(_)) => {
            let mut messages = Vec::new();
            for key in ["enabled", "disabled"] {
                if let Some(items) = config["checks"]
                    .get(key)
                    .and_then(serde_json::Value::as_array)
                {
                    for item in items {
                        let disabled = item
                            .get("params")
                            .is_some_and(|params| params == &serde_json::Value::Bool(false));
                        if disabled
                            && let Some(check) =
                                item.get("check").and_then(serde_json::Value::as_str)
                        {
                            messages.push(format!(
                                "Using `false` for deactivating check `{check}` in Credo's config is deprecated, move them to `:disabled` instead."
                            ));
                        }
                    }
                }
            }
            messages
        }
        _ => Vec::new(),
    }
}

/// EX2006 comment lines redundant against `issues` (`(check, line)` same-file
/// pairs). Unregistered sources yield nothing; every registered comment
/// ignored by no issue is reported at its own line.
#[must_use]
pub fn redundant_comments(
    source: &str,
    registered: bool,
    issues: &[(String, usize)],
) -> Vec<RedundantFinding> {
    if !registered {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for comment in config_comments(source) {
        let ignored = issues
            .iter()
            .any(|(check, line)| comment.ignores(check, *line));
        if !ignored {
            findings.push(RedundantFinding {
                line: comment.line_no,
                column: Some(comment.column),
                trigger: "# credo:".to_owned(),
                message: "This config comment does not ignore any issue.".to_owned(),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_means_no_findings() {
        assert!(missing_checks(None, "credo_checks").is_empty());
        assert!(deprecated_config(None).is_empty());
        assert!(redundant_comments("x = 1\n", true, &[]).is_empty());
        assert!(
            !redundant_comments("# credo:disable-for-this-file\nx = 1\n", true, &[]).is_empty()
        );
    }

    #[test]
    fn standard_inventory_counts_hold() {
        assert_eq!(STANDARD_CHECKS.len(), 115);
        assert_eq!(ENABLED_STANDARD_CHECKS.len(), 77);
        assert!(STANDARD_CHECKS.contains(&"Credo.Check.Readability.TrailingBlankLine"));
        for check in ENABLED_STANDARD_CHECKS {
            assert!(
                STANDARD_CHECKS.contains(check),
                "enabled {check} not standard"
            );
        }
    }
}
