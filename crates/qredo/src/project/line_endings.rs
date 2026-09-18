//! Project-level `LineEndings` consistency (EX1002).
//!
//! Per-file line-ending votes merge across the project; the majority wins and
//! files holding the other style get one issue at their first divergent line.
//! Mirrors the upstream collector exactly, including dropping the last line
//! segment for votes (see upstream #965) while still scanning it for issues.

use std::collections::BTreeMap;

use super::{ProjectFile, ProjectIssue, majority};
use crate::helpers;

/// Run the check over a file set.
pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_file: Vec<BTreeMap<String, usize>> = Vec::new();
    for file in files {
        let votes = collect_file(&file.source);
        for (ending, count) in &votes {
            *counts.entry(ending.clone()).or_insert(0) += count;
        }
        per_file.push(votes);
    }
    if counts.is_empty() {
        return Vec::new();
    }
    let Some(expected) = winner(&counts, params) else {
        return Vec::new();
    };
    emit_with_winner(files, &per_file, &expected)
}

/// Majority winner under the `force` override, if any votes exist.
/// Exposed for `--stale` cache validation (same normalization as `run`).
pub(crate) fn winner(
    counts: &BTreeMap<String, usize>,
    params: &BTreeMap<String, String>,
) -> Option<String> {
    if counts.is_empty() {
        return None;
    }
    let force = helpers::param_str(params, "force", "");
    let force = if force.is_empty() { None } else { Some(force) };
    majority(counts, force)
}

/// Emit issues for one known winner without recomputing the majority.
/// Used by `--stale` to rebuild from cached per-file votes.
pub(crate) fn emit_with_winner(
    files: &[ProjectFile],
    per_file: &[BTreeMap<String, usize>],
    winner: &str,
) -> Vec<ProjectIssue> {
    let mut issues = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let Some(votes) = per_file.get(index) else {
            continue;
        };
        let unexpected = votes.keys().any(|ending| ending != winner);
        if !unexpected {
            continue;
        }
        if let Some(line) = first_divergent_line(&file.source, winner) {
            let (message, trigger) = issue_text(winner);
            issues.push(ProjectIssue {
                file: index,
                line: Some(line),
                column: None,
                trigger,
                message,
                severity: None,
            });
        }
    }
    issues
}

/// Per-file votes over all line segments except the last.
pub(crate) fn collect_file(source: &str) -> BTreeMap<String, usize> {
    let mut votes: BTreeMap<String, usize> = BTreeMap::new();
    let lines: Vec<&str> = source.split('\n').collect();
    let lines = lines.get(..lines.len().saturating_sub(1)).unwrap_or(&[]);
    for line in lines {
        *votes.entry(ending(line).to_owned()).or_insert(0) += 1;
    }
    votes
}

/// Line ending of one segment: a trailing `\r` means windows.
fn ending(line: &str) -> &str {
    if line.ends_with('\r') {
        "windows"
    } else {
        "unix"
    }
}

/// First 1-based line whose ending differs, scanning every segment.
fn first_divergent_line(source: &str, expected: &str) -> Option<usize> {
    source
        .split('\n')
        .enumerate()
        .find(|(_, line)| ending(line) != expected)
        .map(|(idx, _)| idx + 1)
}

/// Message and trigger for the winning style.
fn issue_text(expected: &str) -> (String, String) {
    match expected {
        "unix" => (
            "File is using windows line endings while most of the files use unix line endings."
                .to_owned(),
            "\r\n".to_owned(),
        ),
        _ => (
            "File is using unix line endings while most of the files use windows line endings."
                .to_owned(),
            "\n".to_owned(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(source: &str) -> ProjectFile {
        ProjectFile {
            filename: "a.ex".to_owned(),
            source: source.to_owned(),
        }
    }

    #[test]
    fn consistent_project_is_clean() {
        let files = vec![file("a\nb\n"), file("c\nd\n")];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn minority_file_reports_first_divergent_line() {
        let files = vec![file("a\nb\n"), file("a\r\nb\r\n")];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 1);
        assert_eq!(issues[0].line, Some(1));
    }

    #[test]
    fn tie_breaks_toward_unix_and_blames_windows_file() {
        // Equal votes (3 unix vs 3 windows): the tie resolves to the
        // smallest key ("unix"), so the windows file is blamed.
        let files = vec![file("a\nb\nc\n"), file("a\r\nb\r\nc\r\n")];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 1);
        assert_eq!(issues[0].line, Some(1));
    }
}
