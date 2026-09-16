//! Project-level `TabsOrSpaces` consistency (EX1007).
//!
//! Per-file indentation votes merge across the project: a line starting with
//! two spaces votes `spaces`, a line starting with a tab votes `tabs`. The
//! majority wins (an explicit `force` param overrides) and non-matching
//! indented lines in other files become issues. Columns are backfilled from
//! the trigger with the same boundary-sensitive search Credo applies when an
//! issue carries a trigger but no column.

use std::collections::BTreeMap;

use super::{ProjectFile, ProjectIssue, majority};
use crate::helpers;

/// Run the check over a file set.
pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_file: Vec<BTreeMap<String, usize>> = Vec::new();
    for file in files {
        let votes = collect(&file.source);
        for (kind, count) in &votes {
            *counts.entry(kind.clone()).or_insert(0) += count;
        }
        per_file.push(votes);
    }
    if counts.is_empty() {
        return Vec::new();
    }
    let force = normalized_force(params);
    let Some(expected) = majority(&counts, force.as_deref()) else {
        return Vec::new();
    };
    let (message, trigger) = issue_text(&expected);
    let pattern = format!(
        r"(\s|\b|\(|\)|,)({})(\s|\b|\(|\)|,)",
        regex::escape(&trigger)
    );
    let Some(expression) = regex::Regex::new(&pattern).ok() else {
        return Vec::new();
    };
    let mut issues = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let unexpected = per_file[index].keys().any(|kind| kind != &expected);
        if !unexpected {
            continue;
        }
        for (line_no, line) in file.source.split('\n').enumerate() {
            if indentation(line).is_some_and(|kind| kind != expected) {
                issues.push(ProjectIssue {
                    file: index,
                    line: Some(line_no + 1),
                    column: backfilled_column(line, &expression),
                    trigger: trigger.clone(),
                    message: message.clone(),
                    severity: None,
                });
            }
        }
    }
    issues
}

/// Per-file votes over `\n`-split lines.
fn collect(source: &str) -> BTreeMap<String, usize> {
    let mut votes: BTreeMap<String, usize> = BTreeMap::new();
    for line in source.split('\n') {
        if let Some(kind) = indentation(line) {
            *votes.entry(kind.to_owned()).or_insert(0) += 1;
        }
    }
    votes
}

/// Two leading spaces vote `spaces`, a leading tab votes `tabs`.
fn indentation(line: &str) -> Option<&str> {
    if line.starts_with("  ") {
        Some("spaces")
    } else if line.starts_with('\t') {
        Some("tabs")
    } else {
        None
    }
}

/// Explicit `force` param wins; the gate strips `:atom` colons already.
fn normalized_force(params: &BTreeMap<String, String>) -> Option<String> {
    let bare = helpers::param_str(params, "force", "").trim_start_matches(':');
    if bare == "spaces" || bare == "tabs" {
        Some(bare.to_owned())
    } else {
        None
    }
}

/// Message and trigger for the winning style.
fn issue_text(expected: &str) -> (String, String) {
    match expected {
        "spaces" => (
            "File is using tabs while most of the files use spaces for indentation.".to_owned(),
            "\t".to_owned(),
        ),
        _ => (
            "File is using spaces while most of the files use tabs for indentation.".to_owned(),
            " ".to_owned(),
        ),
    }
}

/// Column backfill mirroring `Credo.SourceFile.column/3`: the first trigger
/// occurrence in the boundary-sensitive search, byte offset plus one.
fn backfilled_column(line: &str, expression: &regex::Regex) -> Option<usize> {
    expression.captures(line)?.get(2).map(|hit| hit.start() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../../compatibility/cases/EX1007.json");

    #[derive(serde::Deserialize)]
    struct Entry {
        id: String,
        source: String,
        #[serde(default)]
        group: Option<String>,
        #[serde(default)]
        params: BTreeMap<String, serde_json::Value>,
        #[serde(default)]
        findings: Vec<ExpectedFinding>,
        #[serde(default)]
        excluded_reason: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct ExpectedFinding {
        line: Option<usize>,
        column: Option<usize>,
        trigger: String,
        message: String,
    }

    fn param_string(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Bool(flag) => flag.to_string(),
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::String(text) => text.strip_prefix(':').unwrap_or(text).to_owned(),
            array_or_object => serde_json::to_string(array_or_object).unwrap_or_default(),
        }
    }

    fn finding_key(
        line: Option<usize>,
        column: Option<usize>,
        trigger: &str,
        message: &str,
    ) -> String {
        format!(
            "{}|{}|{trigger}|{message}",
            line.unwrap_or(0),
            column.unwrap_or(0)
        )
    }

    fn source_of(id: &str) -> String {
        let parsed: Vec<Entry> = serde_json::from_str(CASES).expect("valid corpus JSON");
        parsed
            .iter()
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("missing corpus entry {id}"))
            .source
            .clone()
    }

    #[test]
    fn oracle_pinned_corpus_counts() {
        // Guards the refute entries against vacuous passes from under-counting.
        let cases = [
            ("EX1007.upstream.tabs-only", [("tabs", 4)]),
            ("EX1007.upstream.spaces-only.file1", [("spaces", 5)]),
            ("EX1007.upstream.spaces-only.file2", [("spaces", 6)]),
            ("EX1007.upstream.mixed-indentation.file1", [("tabs", 4)]),
        ];
        for (id, expected) in cases {
            let expected: BTreeMap<String, usize> = expected
                .into_iter()
                .map(|(kind, count)| (kind.to_owned(), count))
                .collect();
            assert_eq!(collect(&source_of(id)), expected, "counts for {id}");
        }
    }

    #[test]
    fn corpus_projects_match_upstream_findings() {
        let failures = check_corpus();
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    fn check_corpus() -> Vec<String> {
        let parsed: Vec<Entry> = serde_json::from_str(CASES).expect("valid corpus JSON");
        let mut subgroups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
        for (index, entry) in parsed.iter().enumerate() {
            if entry.excluded_reason.is_some() {
                continue;
            }
            let group = entry.group.clone().unwrap_or_else(|| entry.id.clone());
            let params = serde_json::to_string(&entry.params).unwrap_or_default();
            subgroups.entry((group, params)).or_default().push(index);
        }
        assert!(!subgroups.is_empty(), "corpus must gate something");
        let mut failures = Vec::new();
        for indexes in subgroups.values() {
            failures.extend(check_subgroup(&parsed, indexes));
        }
        failures
    }

    fn check_subgroup(parsed: &[Entry], indexes: &[usize]) -> Vec<String> {
        let first = &parsed[indexes[0]];
        let params: BTreeMap<String, String> = first
            .params
            .iter()
            .map(|(key, value)| (key.clone(), param_string(value)))
            .collect();
        let files: Vec<ProjectFile> = indexes
            .iter()
            .map(|index| ProjectFile {
                filename: parsed[*index].id.clone(),
                source: parsed[*index].source.clone(),
            })
            .collect();
        let issues = run(&files, &params);
        let mut failures = Vec::new();
        for (position, index) in indexes.iter().enumerate() {
            compare_entry(parsed, &issues, position, *index, &mut failures);
        }
        failures
    }

    fn compare_entry(
        parsed: &[Entry],
        issues: &[ProjectIssue],
        position: usize,
        index: usize,
        failures: &mut Vec<String>,
    ) {
        let mut actual: Vec<String> = issues
            .iter()
            .filter(|issue| issue.file == position)
            .map(|issue| finding_key(issue.line, issue.column, &issue.trigger, &issue.message))
            .collect();
        let mut expected: Vec<String> = parsed[index]
            .findings
            .iter()
            .map(|finding| {
                finding_key(
                    finding.line,
                    finding.column,
                    &finding.trigger,
                    &finding.message,
                )
            })
            .collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            failures.push(format!(
                "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
                parsed[index].id
            ));
        }
    }
}
