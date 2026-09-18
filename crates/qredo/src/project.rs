//! Multi-file project evaluation for consistency checks.
//!
//! Mirrors `Credo.Check.Consistency.Collector.find_issues/5`: per-file match
//! counts merge across the project, the most frequent match wins (ties go to
//! the smallest key; an explicit `force` param overrides), and files holding
//! any other match are formatted into issues. Counts, not wall time, are the
//! work signal; each file is collected once per run.

use std::collections::BTreeMap;

use crate::check_meta::{FileMeta, IssueError, build_issue};
use crate::issue::Issue;
use crate::pipeline::GeneralParams;
use crate::{Finding, Trigger};

mod collect_duplicated;
mod collect_exception_names;
mod collect_multi_alias;
mod collect_param_pattern;
mod collect_space_around_ops;
mod collect_space_in_parens;
mod collect_tabs_or_spaces;
mod collect_unused_var_names;
mod line_endings;

/// One project file under evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    /// Identity for debugging and issue filenames.
    pub filename: String,
    pub source: String,
}

/// One project-level finding with its owning file index.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectIssue {
    pub file: usize,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub trigger: String,
    pub message: String,
    /// `Severity.compute` value (`DuplicatedCode` sets it; `None` means
    /// the upstream default of `1`).
    pub severity: Option<f64>,
}

/// A rule without a project implementation.
#[derive(Debug, PartialEq, Eq)]
pub struct UnsupportedRule(pub String);

/// Run a consistency check over a whole file set.
///
/// # Errors
/// Returns `UnsupportedRule` for checks without a project implementation.
pub fn run_project_check(
    rule: &str,
    files: &[ProjectFile],
    params: &BTreeMap<String, String>,
) -> Result<Vec<ProjectIssue>, UnsupportedRule> {
    match rule {
        "Credo.Check.Consistency.LineEndings" => Ok(line_endings::run(files, params)),
        "Credo.Check.Design.DuplicatedCode" => Ok(collect_duplicated::run(files, params)),
        "Credo.Check.Consistency.ExceptionNames" => Ok(collect_exception_names::run(files, params)),
        "Credo.Check.Consistency.MultiAliasImportRequireUse" => {
            Ok(collect_multi_alias::run(files, params))
        }
        "Credo.Check.Consistency.ParameterPatternMatching" => {
            Ok(collect_param_pattern::run(files, params))
        }
        "Credo.Check.Consistency.SpaceAroundOperators" => {
            Ok(collect_space_around_ops::run(files, params))
        }
        "Credo.Check.Consistency.SpaceInParentheses" => {
            Ok(collect_space_in_parens::run(files, params))
        }
        "Credo.Check.Consistency.TabsOrSpaces" => Ok(collect_tabs_or_spaces::run(files, params)),
        "Credo.Check.Consistency.UnusedVariableNames" => {
            Ok(collect_unused_var_names::run(files, params))
        }
        _ => Err(UnsupportedRule(rule.to_owned())),
    }
}

/// Run a consistency check reusing prepare-phase trees: `trees` aligns
/// with `files`, `None` entries parse normally. Collectors migrate to
/// shared trees one measured check at a time; unmigrated checks ignore
/// the trees and behave exactly like [`run_project_check`]. `facts`
/// aligns the same way for collectors migrated to shared facts.
///
/// # Errors
/// Returns `UnsupportedRule` for checks without a project implementation.
pub fn run_project_check_with_trees(
    rule: &str,
    files: &[ProjectFile],
    trees: &[Option<&tree_sitter::Tree>],
    facts: &[&crate::facts::Facts],
    params: &BTreeMap<String, String>,
) -> Result<Vec<ProjectIssue>, UnsupportedRule> {
    match rule {
        "Credo.Check.Consistency.ExceptionNames" => Ok(collect_exception_names::run_with_facts(
            files, facts, params,
        )),
        "Credo.Check.Consistency.MultiAliasImportRequireUse" => {
            Ok(collect_multi_alias::run_with_facts(files, facts, params))
        }
        "Credo.Check.Consistency.UnusedVariableNames" => Ok(
            collect_unused_var_names::run_with_facts(files, facts, params),
        ),
        "Credo.Check.Design.DuplicatedCode" => {
            Ok(collect_duplicated::run_with_trees(files, trees, params))
        }
        _ => run_project_check(rule, files, params),
    }
}

/// Run a project check and build full issues with per-file metadata.
///
/// Returns `(file index, issue)` pairs; file indices follow `files` order
/// for suppression and sorting downstream.
///
/// Consumed by pipeline wiring (stage 7); unit-tested here.
///
/// # Errors
/// Returns [`IssueError`] for unknown rules or invalid priority params.
#[allow(dead_code, reason = "consumed by pipeline wiring in stage 7")]
pub fn run_project_issues(
    rule: &str,
    files: &[ProjectFile],
    params: &BTreeMap<String, String>,
    general: &GeneralParams,
) -> Result<Vec<(usize, Issue)>, IssueError> {
    let found = run_project_check(rule, files, params).map_err(|error| IssueError(error.0))?;
    let metas: Vec<FileMeta> = files
        .iter()
        .map(|file| FileMeta::collect(&file.filename, &file.source))
        .collect();
    let refs: Vec<&FileMeta> = metas.iter().collect();
    build_project_issues(rule, &found, params, general, &refs)
}

/// Build full issues for project findings against caller-provided
/// per-file metadata (positions follow `found` file indices).
///
/// # Errors
/// Returns [`IssueError`] for unknown rules or invalid priority params.
pub(crate) fn build_project_issues(
    rule: &str,
    found: &[ProjectIssue],
    params: &BTreeMap<String, String>,
    general: &GeneralParams,
    metas: &[&FileMeta],
) -> Result<Vec<(usize, Issue)>, IssueError> {
    let mut out = Vec::new();
    for issue in found {
        let Some(meta) = metas.get(issue.file) else {
            return Err(IssueError(format!(
                "project issue for unknown file index {}",
                issue.file
            )));
        };
        let finding = project_finding(issue);
        out.push((
            issue.file,
            build_issue(rule, finding, params, general, meta)?,
        ));
    }
    Ok(out)
}

/// Project finding as a kernel finding: empty triggers stay text (native
/// passes `""` through column backfill), only the sentinel is triggerless.
pub(crate) fn project_finding(issue: &ProjectIssue) -> Finding {
    Finding {
        line: issue.line.unwrap_or(0),
        column: issue.column,
        message: issue.message.clone(),
        trigger: match issue.trigger.as_str() {
            "no_trigger" => Trigger::NoTrigger,
            text => Trigger::Text(text.to_owned()),
        },
        severity: issue.severity,
    }
}

/// Per-check fresh-file details for `--stale` rebuilds (in-memory only,
/// never serialized). Counts go to disk; details stay alive just long
/// enough to emit fresh-file issues under the merged global winner.
#[derive(Debug)]
pub(crate) enum StaleDetails {
    /// `TabsOrSpaces`, `LineEndings`: counts are the details.
    Counts(Vec<BTreeMap<String, usize>>),
    ParamPattern(Vec<Vec<collect_param_pattern::ParamMatch>>),
    SpaceInParens(Vec<BTreeMap<String, Vec<collect_space_in_parens::Loc>>>),
    SpaceAroundOps(Vec<Vec<collect_space_around_ops::OpVote>>),
    ExceptionNames(Vec<Vec<collect_exception_names::Exception>>),
    MultiAlias(Vec<collect_multi_alias::ModuleVotes>),
    UnusedVarNames(Vec<Vec<collect_unused_var_names::Occurrence>>),
}

/// Collect per-file votes for fresh files under `--stale`.
///
/// Returns the merged fresh counts plus the in-memory details needed to
/// emit fresh-file issues once the global winner is known. `facts` aligns
/// with `files` for the `Facts`-backed checks (callers parse fresh files
/// once and share). Returns `None` for checks without incremental support
/// (`DuplicatedCode`, unknown rules): callers fail open to a full run.
#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "8-arm per-check dispatch mirroring run_project_check; each arm is one collect call"
)]
pub(crate) fn stale_collect(
    rule: &str,
    files: &[ProjectFile],
    facts: &[&crate::facts::Facts],
) -> Option<(BTreeMap<String, usize>, StaleDetails)> {
    match rule {
        "Credo.Check.Consistency.TabsOrSpaces" => {
            let mut counts = BTreeMap::new();
            let mut per_file = Vec::with_capacity(files.len());
            for file in files {
                let votes = collect_tabs_or_spaces::collect_file(&file.source);
                for (kind, count) in &votes {
                    *counts.entry(kind.clone()).or_insert(0) += count;
                }
                per_file.push(votes);
            }
            Some((counts, StaleDetails::Counts(per_file)))
        }
        "Credo.Check.Consistency.LineEndings" => {
            let mut counts = BTreeMap::new();
            let mut per_file = Vec::with_capacity(files.len());
            for file in files {
                let votes = line_endings::collect_file(&file.source);
                for (kind, count) in &votes {
                    *counts.entry(kind.clone()).or_insert(0) += count;
                }
                per_file.push(votes);
            }
            Some((counts, StaleDetails::Counts(per_file)))
        }
        "Credo.Check.Consistency.ParameterPatternMatching" => {
            let mut counts = BTreeMap::new();
            let mut per_file = Vec::with_capacity(files.len());
            for file in files {
                let found = collect_param_pattern::collect_file(&file.source);
                for (kind, count) in collect_param_pattern::counts_of(&found) {
                    *counts.entry(kind).or_insert(0) += count;
                }
                per_file.push(found);
            }
            Some((counts, StaleDetails::ParamPattern(per_file)))
        }
        "Credo.Check.Consistency.SpaceInParentheses" => {
            let mut counts = BTreeMap::new();
            let mut per_file = Vec::with_capacity(files.len());
            for file in files {
                let votes = collect_space_in_parens::collect_file(&file.source);
                for (kind, count) in collect_space_in_parens::counts_of(&votes) {
                    *counts.entry(kind).or_insert(0) += count;
                }
                per_file.push(votes);
            }
            Some((counts, StaleDetails::SpaceInParens(per_file)))
        }
        "Credo.Check.Consistency.SpaceAroundOperators" => {
            let mut counts = BTreeMap::new();
            let mut per_file = Vec::with_capacity(files.len());
            for file in files {
                let votes = collect_space_around_ops::collect_file(&file.source);
                for (kind, count) in collect_space_around_ops::counts_of(&votes) {
                    *counts.entry(kind).or_insert(0) += count;
                }
                per_file.push(votes);
            }
            Some((counts, StaleDetails::SpaceAroundOps(per_file)))
        }
        "Credo.Check.Consistency.ExceptionNames" => {
            let mut counts = BTreeMap::new();
            let mut per_file = Vec::with_capacity(files.len());
            for (position, file) in files.iter().enumerate() {
                let found = match facts.get(position) {
                    Some(facts) => collect_exception_names::collect_file(&file.source, facts),
                    None => Vec::new(),
                };
                for (kind, count) in collect_exception_names::counts_of(&found) {
                    *counts.entry(kind).or_insert(0) += count;
                }
                per_file.push(found);
            }
            Some((counts, StaleDetails::ExceptionNames(per_file)))
        }
        "Credo.Check.Consistency.MultiAliasImportRequireUse" => {
            let mut counts = BTreeMap::new();
            let mut per_file = Vec::with_capacity(files.len());
            for (position, file) in files.iter().enumerate() {
                let modules = match facts.get(position) {
                    Some(facts) => collect_multi_alias::collect_file(&file.source, facts),
                    None => BTreeMap::new(),
                };
                let stats = collect_multi_alias::counts_of(&modules);
                for (kind, count) in &stats {
                    *counts.entry(kind.clone()).or_insert(0) += count;
                }
                per_file.push((stats, modules));
            }
            Some((counts, StaleDetails::MultiAlias(per_file)))
        }
        "Credo.Check.Consistency.UnusedVariableNames" => {
            let mut counts = BTreeMap::new();
            let mut per_file = Vec::with_capacity(files.len());
            for (position, file) in files.iter().enumerate() {
                let found = match facts.get(position) {
                    Some(facts) => collect_unused_var_names::collect_file(&file.source, facts),
                    None => Vec::new(),
                };
                for (kind, count) in collect_unused_var_names::counts_of(&found) {
                    *counts.entry(kind).or_insert(0) += count;
                }
                per_file.push(found);
            }
            Some((counts, StaleDetails::UnusedVarNames(per_file)))
        }
        _ => None,
    }
}

/// Emit fresh-file issues under one known global winner. Positions in the
/// returned issues are subset-relative; callers remap `file` to global
/// indices. Returns `None` for checks without incremental support.
pub(crate) fn stale_emit(
    rule: &str,
    files: &[ProjectFile],
    details: &StaleDetails,
    winner: &str,
    params: &BTreeMap<String, String>,
) -> Option<Vec<ProjectIssue>> {
    match (rule, details) {
        ("Credo.Check.Consistency.TabsOrSpaces", StaleDetails::Counts(per_file)) => Some(
            collect_tabs_or_spaces::emit_with_winner(files, per_file, winner),
        ),
        ("Credo.Check.Consistency.LineEndings", StaleDetails::Counts(per_file)) => {
            Some(line_endings::emit_with_winner(files, per_file, winner))
        }
        (
            "Credo.Check.Consistency.ParameterPatternMatching",
            StaleDetails::ParamPattern(per_file),
        ) => Some(collect_param_pattern::emit_with_winner(
            files, per_file, winner,
        )),
        ("Credo.Check.Consistency.SpaceInParentheses", StaleDetails::SpaceInParens(per_file)) => {
            Some(collect_space_in_parens::emit_with_winner(
                per_file, winner, params,
            ))
        }
        (
            "Credo.Check.Consistency.SpaceAroundOperators",
            StaleDetails::SpaceAroundOps(per_file),
        ) => Some(collect_space_around_ops::emit_with_winner(
            files, per_file, winner, params,
        )),
        ("Credo.Check.Consistency.ExceptionNames", StaleDetails::ExceptionNames(per_file)) => Some(
            collect_exception_names::emit_with_winner(files, per_file, winner),
        ),
        (
            "Credo.Check.Consistency.MultiAliasImportRequireUse",
            StaleDetails::MultiAlias(per_file),
        ) => Some(collect_multi_alias::emit_with_winner(
            files, per_file, winner,
        )),
        ("Credo.Check.Consistency.UnusedVariableNames", StaleDetails::UnusedVarNames(per_file)) => {
            Some(collect_unused_var_names::emit_with_winner(per_file, winner))
        }
        _ => None,
    }
}

/// Majority winner for one project check under `--stale`, using the same
/// force normalization and suppression as its `run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StaleWinner {
    /// Supported check with its merged winner (`None` = no votes).
    Known(Option<String>),
    /// No incremental support (`DuplicatedCode`, unknown rules).
    Unsupported,
}

/// Majority winner for one project check under `--stale`, using the same
/// force normalization and suppression as its `run`. Returns
/// [`StaleWinner::Unsupported`] for checks without incremental support.
pub(crate) fn stale_winner(
    rule: &str,
    counts: &BTreeMap<String, usize>,
    params: &BTreeMap<String, String>,
) -> StaleWinner {
    match rule {
        "Credo.Check.Consistency.TabsOrSpaces" => {
            StaleWinner::Known(collect_tabs_or_spaces::winner(counts, params))
        }
        "Credo.Check.Consistency.LineEndings" => {
            StaleWinner::Known(line_endings::winner(counts, params))
        }
        "Credo.Check.Consistency.ParameterPatternMatching" => {
            StaleWinner::Known(collect_param_pattern::winner(counts, params))
        }
        "Credo.Check.Consistency.SpaceInParentheses" => {
            StaleWinner::Known(collect_space_in_parens::winner(counts, params))
        }
        "Credo.Check.Consistency.SpaceAroundOperators" => {
            StaleWinner::Known(collect_space_around_ops::winner(counts, params))
        }
        "Credo.Check.Consistency.ExceptionNames" => {
            StaleWinner::Known(collect_exception_names::winner(counts, params))
        }
        "Credo.Check.Consistency.MultiAliasImportRequireUse" => {
            StaleWinner::Known(collect_multi_alias::winner(counts, params))
        }
        "Credo.Check.Consistency.UnusedVariableNames" => {
            StaleWinner::Known(collect_unused_var_names::winner(counts, params))
        }
        _ => StaleWinner::Unsupported,
    }
}

/// Per-file vote counts by collector, re-exported for `--stale` cache
/// saves so the disk view shares each collector's merge accounting.
pub(crate) fn collect_param_pattern_counts(
    found: &[collect_param_pattern::ParamMatch],
) -> BTreeMap<String, usize> {
    collect_param_pattern::counts_of(found)
}

/// Per-file vote counts by collector (see above).
pub(crate) fn collect_space_in_parens_counts(
    votes: &BTreeMap<String, Vec<collect_space_in_parens::Loc>>,
) -> BTreeMap<String, usize> {
    collect_space_in_parens::counts_of(votes)
}

/// Per-file vote counts by collector (see above).
pub(crate) fn collect_space_around_ops_counts(
    votes: &[collect_space_around_ops::OpVote],
) -> BTreeMap<String, usize> {
    collect_space_around_ops::counts_of(votes)
}

/// Per-file vote counts by collector (see above).
pub(crate) fn collect_exception_names_counts(
    found: &[collect_exception_names::Exception],
) -> BTreeMap<String, usize> {
    collect_exception_names::counts_of(found)
}

/// Per-file vote counts by collector (see above).
pub(crate) fn collect_unused_var_names_counts(
    found: &[collect_unused_var_names::Occurrence],
) -> BTreeMap<String, usize> {
    collect_unused_var_names::counts_of(found)
}

/// Winning match across merged per-file counts: explicit `force` wins,
/// otherwise the highest count, ties broken toward the smallest key
/// (mirrors `Enum.sort() |> Enum.max_by(&elem(&1, 1))`, verified natively).
pub(crate) fn majority(counts: &BTreeMap<String, usize>, force: Option<&str>) -> Option<String> {
    if let Some(forced) = force {
        return Some(forced.to_owned());
    }
    let mut best: Option<(&str, usize)> = None;
    for (key, count) in counts {
        if best.is_none_or(|(_, best_count)| *count > best_count) {
            best = Some((key, *count));
        }
    }
    best.map(|(key, _)| key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majority_prefers_highest_count() {
        let counts: BTreeMap<String, usize> =
            [("unix".to_owned(), 9), ("windows".to_owned(), 2)].into();
        assert_eq!(majority(&counts, None), Some("unix".to_owned()));
    }

    #[test]
    fn majority_breaks_ties_toward_smallest_key() {
        let counts: BTreeMap<String, usize> =
            [("windows".to_owned(), 2), ("unix".to_owned(), 2)].into();
        assert_eq!(majority(&counts, None), Some("unix".to_owned()));
    }

    #[test]
    fn force_overrides_counts() {
        let counts: BTreeMap<String, usize> = [("unix".to_owned(), 9)].into();
        assert_eq!(
            majority(&counts, Some("windows")),
            Some("windows".to_owned())
        );
    }

    #[test]
    fn majority_empty_is_none() {
        assert_eq!(majority(&BTreeMap::new(), None), None);
    }

    fn project_files() -> Vec<ProjectFile> {
        vec![
            ProjectFile {
                filename: "a.ex".to_owned(),
                source: "defmodule M do\n\tdef f, do: 1\nend\n".to_owned(),
            },
            ProjectFile {
                filename: "b.ex".to_owned(),
                source: "defmodule N do\n  def g, do: 2\nend\n".to_owned(),
            },
        ]
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "native-verified severity is exactly representable"
    )]
    fn project_issues_become_full_issues() {
        use crate::issue::{Category, IssueTrigger};
        use crate::pipeline::GeneralParams;
        let files = project_files();
        let issues = run_project_issues(
            "Credo.Check.Consistency.TabsOrSpaces",
            &files,
            &BTreeMap::new(),
            &GeneralParams::default(),
        )
        .expect("project pipeline runs");
        assert_eq!(issues.len(), 1);
        let (file, issue) = &issues[0];
        assert_eq!(*file, 0);
        assert_eq!(issue.check, "Credo.Check.Consistency.TabsOrSpaces");
        assert_eq!(issue.category, Category::Consistency);
        assert_eq!(issue.priority, 11);
        assert_eq!(issue.severity, 1.0);
        assert_eq!(issue.filename, "a.ex");
        assert_eq!(issue.line_no, Some(2));
        assert_eq!(issue.column, None);
        assert_eq!(issue.exit_status, 1);
        assert_eq!(issue.trigger, IssueTrigger::Text("\t".to_owned()));
        assert_eq!(issue.scope.as_deref(), Some("M.f"));
        assert_eq!(
            issue.message,
            "File is using tabs while most of the files use spaces for indentation."
        );
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "native-verified severity is exactly representable"
    )]
    fn duplicated_issues_carry_native_severity_and_priority() {
        use crate::issue::IssueTrigger;
        use crate::pipeline::GeneralParams;
        let files = vec![
            ProjectFile {
                filename: "m1.ex".to_owned(),
                source: "defmodule M1 do\n  def f(p1, p2), do: p1 + p2\nend\n".to_owned(),
            },
            ProjectFile {
                filename: "m2.ex".to_owned(),
                source: "defmodule M2 do\n  def f(p1, p2), do: p1 + p2\nend\n".to_owned(),
            },
        ];
        let mut params = BTreeMap::new();
        params.insert("mass_threshold".to_owned(), "5".to_owned());
        let issues = run_project_issues(
            "Credo.Check.Design.DuplicatedCode",
            &files,
            &params,
            &GeneralParams::default(),
        )
        .expect("project pipeline runs");
        assert!(!issues.is_empty());
        for (_, issue) in &issues {
            assert_eq!(issue.severity, 2.0);
            assert_eq!(issue.priority, 22);
            assert_eq!(issue.trigger, IssueTrigger::NoTrigger);
            assert_eq!(issue.line_no, Some(2));
        }
        assert!(issues.iter().any(|(file, _)| *file == 0));
        assert!(issues.iter().any(|(file, _)| *file == 1));
    }

    #[test]
    fn project_pipeline_rejects_unknown_rules() {
        use crate::pipeline::GeneralParams;
        assert!(
            run_project_issues(
                "Credo.Check.Nope",
                &project_files(),
                &BTreeMap::new(),
                &GeneralParams::default(),
            )
            .is_err()
        );
    }
}
