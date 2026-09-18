//! Project-level `UnusedVariableNames` consistency (EX1008).
//!
//! Per-file unused-variable votes (`anonymous` for bare `_`, `meaningful`
//! for `_name`) merge across the project; the majority wins (ties go to
//! `anonymous`, the smaller key) unless `force` overrides. Files holding the
//! other style get one issue per mismatching occurrence.
//!
//! Binding positions mirror the upstream collector: both sides of `=` and
//! `<-`, `def`/`defp`/`defmacro`/`defmacrop` head patterns (guards
//! included), the context argument of `test ... do`, and `->` clause
//! patterns. Call names, remote targets, `__`-prefixed specials, strings
//! and sigils never vote.

use std::collections::BTreeMap;

use super::{ProjectFile, ProjectIssue, majority};
use crate::helpers;

/// One unused-variable occurrence in binding position.
#[derive(Debug, Clone)]
pub(crate) struct Occurrence {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) kind: String,
    pub(crate) trigger: String,
    /// One vote per enclosing binding region: the upstream prewalk reduces
    /// every nested `=`/`<-`/head/context/`->` subtree, so an identifier in
    /// overlapping regions (like `%S{_a: _a} = _s` in a `def` head) votes
    /// more than once and can flip the majority.
    pub(crate) votes: usize,
}

/// Run the check over a file set.
pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let facts: Vec<crate::facts::Facts> = files
        .iter()
        .map(|file| {
            crate::ts_parser::parse(&file.source).map_or_else(crate::facts::Facts::empty, |tree| {
                crate::facts::extract(&tree, &file.source)
            })
        })
        .collect();
    let refs: Vec<&crate::facts::Facts> = facts.iter().collect();
    run_with_facts(files, &refs, params)
}

/// Run the check reusing prepare-phase facts: `facts` aligns with
/// `files`. The pipeline shares one parse and one walk per file instead
/// of walking trees per file.
pub(crate) fn run_with_facts(
    files: &[ProjectFile],
    facts: &[&crate::facts::Facts],
    params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_file = Vec::new();
    for (position, file) in files.iter().enumerate() {
        let found = match facts.get(position) {
            Some(facts) => collect_file(&file.source, facts),
            None => Vec::new(),
        };
        for vote in &found {
            *counts.entry(vote.kind.clone()).or_insert(0) += vote.votes;
        }
        per_file.push(found);
    }
    if counts.is_empty() {
        return Vec::new();
    }
    let Some(expected) = winner(&counts, params) else {
        return Vec::new();
    };
    emit_with_winner(&per_file, &expected)
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
    let force = force.strip_prefix(':').unwrap_or(force);
    let force = if force.is_empty() { None } else { Some(force) };
    majority(counts, force)
}

/// Per-file vote counts for `--stale` caching (weighted by enclosing
/// region count, mirroring the merge).
pub(crate) fn counts_of(found: &[Occurrence]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for vote in found {
        *counts.entry(vote.kind.clone()).or_insert(0) += vote.votes;
    }
    counts
}

/// Emit issues for one known winner over subset-relative per-file details.
/// Used by `--stale`; positions are subset-relative, callers remap `file`.
pub(crate) fn emit_with_winner(per_file: &[Vec<Occurrence>], winner: &str) -> Vec<ProjectIssue> {
    let mut issues = Vec::new();
    for (index, found) in per_file.iter().enumerate() {
        if !found.iter().any(|vote| vote.kind != winner) {
            continue;
        }
        for vote in mismatches(found, winner) {
            issues.push(ProjectIssue {
                file: index,
                line: Some(vote.line),
                column: Some(vote.column),
                trigger: vote.trigger.clone(),
                message: message_for(winner, &vote.trigger),
                severity: None,
            });
        }
    }
    issues
}

/// Mismatching occurrences in source order, deduplicated by location.
fn mismatches<'a>(found: &'a [Occurrence], expected: &str) -> Vec<&'a Occurrence> {
    let mut votes: Vec<&Occurrence> = found.iter().filter(|vote| vote.kind != expected).collect();
    votes.sort_by(|a, b| (a.line, a.column, &a.trigger).cmp(&(b.line, b.column, &b.trigger)));
    votes.dedup_by(|first, second| {
        first.line == second.line
            && first.column == second.column
            && first.trigger == second.trigger
    });
    votes
}

/// Every unused-variable occurrence holding a binding position, with
/// one vote per containing binding region. Exposed for `--stale` so
/// unchanged files skip parsing entirely.
pub(crate) fn collect_file(source: &str, facts: &crate::facts::Facts) -> Vec<Occurrence> {
    let starts = line_starts(source);
    let mut votes = Vec::new();
    for ident in &facts.var_idents {
        let count = facts
            .bind_regions
            .iter()
            .filter(|(start, end)| *start <= ident.start && ident.end <= *end)
            .count();
        if count == 0 {
            continue;
        }
        let Some(trigger) = source.get(ident.start as usize..ident.end as usize) else {
            continue;
        };
        let (line, column) = line_col(&starts, source, ident.start as usize);
        votes.push(Occurrence {
            line,
            column,
            kind: match ident.kind {
                crate::facts::VarKind::Anonymous => "anonymous".to_owned(),
                crate::facts::VarKind::Meaningful => "meaningful".to_owned(),
            },
            trigger: trigger.to_owned(),
            votes: count,
        });
    }
    votes
}

/// Message for one mismatching trigger under the winning strategy.
fn message_for(expected: &str, trigger: &str) -> String {
    if expected == "meaningful" {
        format!(
            "Unused variables should be named consistently. It seems your strategy is to give them meaningful names (eg. `_foo`) but `{trigger}` does not follow that convention."
        )
    } else {
        format!(
            "Unused variables should be named consistently. It seems your strategy is to name them anonymously (ie. `_`) but `{trigger}` does not follow that convention."
        )
    }
}

/// 1-based `(line, column)` with character-based columns.
fn line_col(starts: &[usize], source: &str, byte: usize) -> (usize, usize) {
    let row = starts
        .partition_point(|start| *start <= byte)
        .saturating_sub(1);
    let line_start = starts.get(row).copied().unwrap_or(0);
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |rel| line_start + rel);
    let column = source
        .get(line_start..byte.min(line_end))
        .map_or(1, |prefix| prefix.chars().count() + 1);
    (row + 1, column)
}

/// Byte offsets where each 1-based line starts (`starts[0]` is zero).
fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    starts.extend(source.match_indices('\n').map(|(byte, _)| byte + 1));
    starts
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
        let files = vec![
            file("defmodule M do\n  def f(_, _, _) do\n  end\nend\n"),
            file("defmodule M do\nend\n"),
        ];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn anonymous_majority_reports_meaningful() {
        let files = vec![
            file("defmodule M do\n  def f(_, _, _) do\n  end\nend\n"),
            file(
                "defmodule M do\n  def g(list) do\n    Enum.map(list, fn _item -> 1 end)\n  end\nend\n",
            ),
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 1);
        assert_eq!(issues[0].trigger, "_item");
    }

    #[test]
    fn tie_breaks_toward_anonymous_and_blames_meaningful_file() {
        // Key space is `anonymous` (bare `_`) / `meaningful` (`_name`);
        // smallest key `anonymous` wins a 1-1 tie, so the file holding the
        // non-smallest `meaningful` vote (file 1) is blamed.
        let files = vec![
            file("defmodule M do\n  def f(_) do\n  end\nend\n"),
            file(
                "defmodule N do\n  def g(list) do\n    Enum.map(list, fn _item -> 1 end)\n  end\nend\n",
            ),
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 1);
        assert_eq!(issues[0].trigger, "_item");
    }

    #[test]
    fn def_names_never_vote() {
        let files = vec![
            file("defmodule M do\n  def _weird(var1, var2) do\n  end\nend\n"),
            file("defmodule M do\nend\n"),
        ];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn special_variables_never_vote() {
        let files = vec![file(
            "defmodule M do\n  defp a do\n    _ = __MODULE__\n  end\nend\n",
        )];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn force_anonymous_reports_every_meaningful() {
        let mut params = BTreeMap::new();
        params.insert("force".to_owned(), "anonymous".to_owned());
        let files = vec![file(
            "defmodule M do\n  def f(name, _) do\n    case name do\n      \"foo\" <> _name -> 1\n    end\n  end\nend\n",
        )];
        let issues = run(&files, &params);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].trigger, "_name");
    }

    #[test]
    fn shared_facts_match_fresh_parses() {
        // The pipeline shares prepare-phase facts instead of re-walking;
        // both paths must report identically.
        let sources = [
            "defmodule M do\n  def f(_, _, _) do\n  end\nend\n",
            "defmodule M do\n  def g(list) do\n    Enum.map(list, fn _item -> 1 end)\n  end\nend\n",
        ];
        let files: Vec<ProjectFile> = sources
            .iter()
            .map(|source| ProjectFile {
                filename: "case.ex".to_owned(),
                source: (*source).to_owned(),
            })
            .collect();
        let expected = run(&files, &BTreeMap::new());
        assert_eq!(expected.len(), 1);
        let prepared: Vec<crate::batch::Prepared<'_>> = sources
            .iter()
            .map(|source| crate::batch::Prepared::eager(source))
            .collect();
        let facts: Vec<&crate::facts::Facts> =
            prepared.iter().map(crate::batch::Prepared::facts).collect();
        assert_eq!(run_with_facts(&files, &facts, &BTreeMap::new()), expected);
    }

    #[test]
    fn nested_match_counts_once_per_region() {
        // Oracle: `%S{_a: _a} = _s` in a `def` head votes twice per
        // variable (head scan plus `=` scan), so meaningful wins 4-3.
        let files = vec![file(
            "defmodule M do\n  def f(%S{_a: _a} = _s) do\n    {_a, _s}\n  end\n  def g(_, _, _) do\n  end\nend\n",
        )];
        let issues = run(&files, &BTreeMap::new());
        let triggers: Vec<&str> = issues.iter().map(|issue| issue.trigger.as_str()).collect();
        assert_eq!(triggers, vec!["_"; 3]);
    }

    #[test]
    fn corpus_groups_match_oracle() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/compatibility/cases/EX1008.json"
        ))
        .expect("EX1008 corpus readable");
        let entries: Vec<Entry> = serde_json::from_str(&text).expect("EX1008 corpus valid");
        let mut subgroups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            if entry.excluded_reason.is_some() {
                continue;
            }
            let group = entry.group.clone().unwrap_or_else(|| entry.id.clone());
            let params = serde_json::to_string(&entry.params).unwrap_or_default();
            subgroups.entry((group, params)).or_default().push(index);
        }
        assert!(!subgroups.is_empty());
        let mut failures = Vec::new();
        for indexes in subgroups.values() {
            check_subgroup(&entries, indexes, &mut failures);
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[derive(serde::Deserialize)]
    struct Entry {
        id: String,
        #[serde(default)]
        group: Option<String>,
        #[serde(default)]
        params: BTreeMap<String, serde_json::Value>,
        source: String,
        #[serde(default)]
        findings: Vec<Expected>,
        #[serde(default)]
        excluded_reason: Option<serde_json::Value>,
    }

    #[derive(serde::Deserialize)]
    struct Expected {
        line: Option<usize>,
        column: Option<usize>,
        trigger: String,
        message: String,
    }

    fn check_subgroup(entries: &[Entry], indexes: &[usize], failures: &mut Vec<String>) {
        let params: BTreeMap<String, String> = entries[indexes[0]]
            .params
            .iter()
            .map(|(key, value)| (key.clone(), param_string(value)))
            .collect();
        let files: Vec<ProjectFile> = indexes
            .iter()
            .map(|index| ProjectFile {
                filename: entries[*index].id.clone(),
                source: entries[*index].source.clone(),
            })
            .collect();
        let issues = run(&files, &params);
        for (position, index) in indexes.iter().enumerate() {
            let mut actual: Vec<String> = issues
                .iter()
                .filter(|issue| issue.file == position)
                .map(issue_key)
                .collect();
            let mut expected: Vec<String> =
                entries[*index].findings.iter().map(expected_key).collect();
            actual.sort();
            expected.sort();
            if actual != expected {
                failures.push(format!(
                    "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
                    entries[*index].id
                ));
            }
        }
    }

    fn issue_key(issue: &ProjectIssue) -> String {
        finding_key(issue.line, issue.column, &issue.trigger, &issue.message)
    }

    fn expected_key(finding: &Expected) -> String {
        finding_key(
            finding.line,
            finding.column,
            &finding.trigger,
            &finding.message,
        )
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

    fn param_string(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Bool(flag) => flag.to_string(),
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::String(text) => text.strip_prefix(':').unwrap_or(text).to_owned(),
            array_or_object => serde_json::to_string(array_or_object).unwrap_or_default(),
        }
    }
}
