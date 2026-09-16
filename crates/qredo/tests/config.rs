//! Config-context checks (EX2006/EX2007/EX2008).
//!
//! These checks evaluate validated `.credo.exs` data and registered config
//! comments — never raw Elixir config, which would require a BEAM. Corpora
//! carry their validated configs inline (`exec_validated_config`) or setup
//! markers (`setup`), transcribed from the upstream exec-based tests.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compatibility/cases")
}

#[derive(Deserialize)]
struct ConfigEntry {
    id: String,
    origin: String,
    source: String,
    #[serde(default)]
    params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    findings: Vec<ConfigFinding>,
    #[serde(default)]
    excluded_reason: Option<String>,
    #[serde(default)]
    exec_validated_config: Option<serde_json::Value>,
    #[serde(default)]
    setup: Option<String>,
}

#[derive(Deserialize)]
struct ConfigFinding {
    line: Option<usize>,
    column: Option<usize>,
    trigger: String,
    message: String,
}

fn load(exid: &str) -> Vec<ConfigEntry> {
    let text = std::fs::read_to_string(cases_dir().join(format!("{exid}.json")))
        .expect("corpus file readable");
    serde_json::from_str(&text).expect("valid JSON")
}

fn finding_key(finding: &ConfigFinding) -> String {
    format!(
        "{}|{}|{}|{}",
        finding.line.unwrap_or(0),
        finding.column.unwrap_or(0),
        finding.trigger,
        finding.message
    )
}

#[test]
fn ex2007_missing_checks_match() {
    let mut failures = Vec::new();
    for entry in load("EX2007") {
        if entry.excluded_reason.is_some() {
            continue;
        }
        assert!(!entry.origin.is_empty(), "{}", entry.id);
        let compare_to = entry
            .params
            .get("compare_to")
            .and_then(|value| match value {
                serde_json::Value::String(text) => {
                    Some(text.strip_prefix(':').unwrap_or(text).to_owned())
                }
                _ => None,
            })
            .unwrap_or_else(|| "credo_checks".to_owned());
        let messages =
            qredo::config_checks::missing_checks(entry.exec_validated_config.as_ref(), &compare_to);
        let mut actual: Vec<String> = messages;
        let mut expected: Vec<String> = entry
            .findings
            .iter()
            .map(|finding| finding.message.clone())
            .collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            failures.push(format!(
                "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
                entry.id
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn ex2008_deprecated_config_match() {
    let mut failures = Vec::new();
    for entry in load("EX2008") {
        if entry.excluded_reason.is_some() {
            continue;
        }
        assert!(!entry.origin.is_empty(), "{}", entry.id);
        let messages =
            qredo::config_checks::deprecated_config(entry.exec_validated_config.as_ref());
        let mut actual: Vec<String> = messages;
        let mut expected: Vec<String> = entry
            .findings
            .iter()
            .map(|finding| finding.message.clone())
            .collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            failures.push(format!(
                "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
                entry.id
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn ex2006_redundant_comments_match() {
    let mut failures = Vec::new();
    for entry in load("EX2006") {
        if entry.excluded_reason.is_some() {
            continue;
        }
        assert!(!entry.origin.is_empty(), "{}", entry.id);
        let registered = entry.setup.as_deref() == Some("with_config_comments");
        let actual = qredo::config_checks::redundant_comments(&entry.source, registered, &[]);
        let mut actual_keys: Vec<String> = actual
            .iter()
            .map(|finding| {
                finding_key(&ConfigFinding {
                    line: Some(finding.line),
                    column: finding.column,
                    trigger: finding.trigger.clone(),
                    message: finding.message.clone(),
                })
            })
            .collect();
        let mut expected: Vec<String> = entry
            .findings
            .iter()
            .map(|finding| {
                finding_key(&ConfigFinding {
                    line: finding.line,
                    column: finding.column,
                    trigger: finding.trigger.clone(),
                    message: finding.message.clone(),
                })
            })
            .collect();
        actual_keys.sort();
        expected.sort();
        if actual_keys != expected {
            failures.push(format!(
                "{}: mismatch\n  actual:   {actual_keys:?}\n  expected: {expected:?}",
                entry.id
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
