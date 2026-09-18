//! Full multi-check pipeline: selection, per-check file filtering,
//! kernel/project/filename lanes, config-comment checks, suppression,
//! priority filtering, relevant ordering and exit status.
//!
//! Mirrors `Check.Runner` with `SetRelevantIssues`: checks run in config
//! order subject to CLI selection; findings build full issues; suppression
//! and minimum priority filter; survivors sort by check, filename and line;
//! exit statuses combine by bitwise OR. Anything undecidable is an explicit
//! [`RunError`], never silent clean results.

use std::collections::BTreeMap;

use crate::check_meta::{FileMeta, build_issue};
use crate::config_checks;
use crate::config_file::CheckEntry;
use crate::config_file::FileEntry;
use crate::file_select::CheckFileMatcher;
use crate::issue::Issue;
use crate::pipeline::GeneralParams;
use crate::project::ProjectFile;
use crate::selection::Selection;
use crate::suppression::{config_comments, validate_comments};

/// One input file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerFile {
    pub filename: String,
    pub source: String,
}

/// Execution configuration: resolved checks, file entries, CLI selection
/// and minimum priority (`0` default, `-99` for `--strict`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerConfig {
    /// Enabled checks in config order with merged params.
    pub checks: Vec<CheckEntry>,
    /// Config-level `files.included` (empty means every file).
    pub files_included: Vec<FileEntry>,
    /// Config-level `files.excluded`.
    pub files_excluded: Vec<FileEntry>,
    /// CLI `--only`/`--ignore` selection.
    pub selection: Selection,
    /// Minimum priority to report.
    pub min_priority: i32,
    /// General category/exit-status/priority overrides.
    pub general: GeneralParams,
}

/// Explicit pipeline failure modes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RunError {
    /// Check without any implementation.
    UnknownRule(String),
    /// Issue building failed (unknown rule or invalid priority).
    Issue(String),
    /// Uncompilable file pattern.
    Pattern(String),
    /// Comment that would crash upstream registration.
    Comment {
        file: String,
        line_no: usize,
        message: String,
    },
    /// Invalid CLI selection pattern.
    InvalidSelection(String),
    /// Check needing BEAM-validated config data.
    NeedsValidatedConfig(String),
}

/// Complete pipeline output.
#[derive(Debug, Clone, PartialEq)]
pub struct RunReport {
    /// Relevant issues ordered by check, filename and line.
    pub issues: Vec<Issue>,
    /// Bitwise OR of issue exit statuses (`0` when clean).
    pub exit_status: i32,
    /// Explicit errors encountered along the way.
    pub errors: Vec<RunError>,
    /// Invalid-syntax files excluded with a complaint, like upstream.
    pub skipped_invalid: Vec<String>,
}

/// Run every configured check over every file.
#[must_use]
pub fn run_checks(files: &[RunnerFile], config: &RunnerConfig) -> RunReport {
    let mut report = RunReport {
        issues: Vec::new(),
        exit_status: 0,
        errors: Vec::new(),
        skipped_invalid: Vec::new(),
    };
    if let Err(pattern) = config.selection.validate() {
        report.errors.push(RunError::InvalidSelection(pattern));
        return report;
    }
    // Per-file preparation: syntax gate, comment validation, metadata.
    // Files are independent; merge in input order to keep errors and
    // skips positioned exactly like the sequential pass.
    let mut prepared: Vec<PreparedFile<'_>> = Vec::new();
    for (position, outcome) in crate::batch::parallel_map(files, prepare_file) {
        match outcome {
            Ok(Some(ready)) => prepared.push(ready),
            Ok(None) => report
                .skipped_invalid
                .push(files[position].filename.clone()),
            Err(error) => report.errors.push(error),
        }
    }
    // Staged `(file index, issue)` pairs; EX2006 runs after all other
    // checks because it reads every file's collected issues.
    let mut work = Worklist::default();
    for entry in &config.checks {
        if !entry.enabled || !config.selection.should_run(&entry.module) {
            continue;
        }
        run_check_entry(entry, &prepared, config, &mut work, &mut report);
    }
    run_redundant_comments(&mut work, &prepared, config, &mut report);
    finish_report(&mut work, &prepared, &mut report, config.min_priority);
    report
}

/// Lane membership: project checks first, then filename-aware checks,
/// then single-file kernels. The sets mirror their dispatches; the
/// lane-coverage test guards drift.
const PROJECT_LANE: &[&str] = &[
    "Credo.Check.Consistency.LineEndings",
    "Credo.Check.Design.DuplicatedCode",
    "Credo.Check.Consistency.ExceptionNames",
    "Credo.Check.Consistency.MultiAliasImportRequireUse",
    "Credo.Check.Consistency.ParameterPatternMatching",
    "Credo.Check.Consistency.SpaceAroundOperators",
    "Credo.Check.Consistency.SpaceInParentheses",
    "Credo.Check.Consistency.TabsOrSpaces",
    "Credo.Check.Consistency.UnusedVariableNames",
];
const FILENAME_LANE: &[&str] = &[
    "Credo.Check.Design.SkipTestWithoutComment",
    "Credo.Check.Readability.ModuleDoc",
    "Credo.Check.Refactor.ModuleDependencies",
    "Credo.Check.Warning.MixEnv",
    "Credo.Check.Warning.WrongTestFileExtension",
    "Credo.Check.Warning.WrongTestFilename",
];

/// Evaluation lane for one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    Project,
    Filename,
    Kernel,
}
fn lane_for(rule: &str) -> Lane {
    if PROJECT_LANE.contains(&rule) {
        Lane::Project
    } else if FILENAME_LANE.contains(&rule) {
        Lane::Filename
    } else {
        Lane::Kernel
    }
}

/// True when a check evaluates each file independently, so missing cache
/// units may run on a file subset. Project-lane checks aggregate across
/// files, `RedundantConfigComments` needs the complete staged issue set,
/// and the config-validated checks always report errors instead of issues.
#[must_use]
pub fn supports_per_file(rule: &str) -> bool {
    !PROJECT_LANE.contains(&rule)
        && rule != "Credo.Check.Design.RedundantConfigComments"
        && rule != "Credo.Check.Design.MissingCheckInConfig"
        && rule != "Credo.Check.Design.DeprecatedChecksConfig"
}

/// Project-lane checks promoted to full-run native serving after
/// differential proof (real-target campaign + corpus + tie/revert
/// contracts). They still aggregate across files, so subset execution
/// stays fail-closed (`supports_per_file` remains false for all of them)
/// and `DuplicatedCode` stays fully gated (absolute peer paths in
/// messages, unverified scale behavior).
#[must_use]
pub fn promoted_project_check(rule: &str) -> bool {
    matches!(
        rule,
        "Credo.Check.Consistency.LineEndings"
            | "Credo.Check.Consistency.ExceptionNames"
            | "Credo.Check.Consistency.MultiAliasImportRequireUse"
            | "Credo.Check.Consistency.ParameterPatternMatching"
            | "Credo.Check.Consistency.SpaceAroundOperators"
            | "Credo.Check.Consistency.SpaceInParentheses"
            | "Credo.Check.Consistency.TabsOrSpaces"
            | "Credo.Check.Consistency.UnusedVariableNames"
    )
}

/// One file ready for check runs.
struct PreparedFile<'a> {
    index: usize,
    filename: &'a str,
    source: &'a str,
    meta: FileMeta<'a>,
    /// Shared parse facts for every kernel lane evaluation.
    prepared: crate::batch::Prepared<'a>,
    comments: Vec<crate::suppression::ConfigComment>,
}

/// Syntax-gate, validate comments, and cache metadata. `Ok(None)` marks an
/// invalid-syntax file for exclusion with a complaint.
fn prepare_file(index: usize, file: &RunnerFile) -> Result<Option<PreparedFile<'_>>, RunError> {
    // Single parse per file: the syntax gate reuses its tree for all
    // downstream facts instead of parsing a second time in `Prepared`.
    let tree = crate::ts_parser::parse(&file.source);
    let valid = tree
        .as_ref()
        .is_some_and(|tree| !tree.root_node().has_error());
    if !valid {
        return Ok(None);
    }
    if let Err(error) = validate_comments(&file.source) {
        return Err(RunError::Comment {
            file: file.filename.clone(),
            line_no: error.line_no,
            message: error.message,
        });
    }
    // Scopes and bonuses share the prepared facts: one parse and one walk
    // per file across facts, scopes, and bonuses instead of five.
    let prepared = crate::batch::Prepared::with_tree(&file.source, tree);
    let meta = FileMeta::collect_shared(&file.filename, &file.source, prepared.facts());
    Ok(Some(PreparedFile {
        index,
        filename: &file.filename,
        source: &file.source,
        meta,
        prepared,
        comments: config_comments(&file.source),
    }))
}

/// Run one enabled check over its selected files into the report.
fn run_check_entry(
    entry: &CheckEntry,
    prepared: &[PreparedFile<'_>],
    config: &RunnerConfig,
    work: &mut Worklist,
    report: &mut RunReport,
) {
    let rule = entry.module.as_str();
    if rule == "Credo.Check.Design.RedundantConfigComments" {
        work.redundant.push(entry.params.clone());
        return;
    }
    if crate::check_meta::version_skipped_on_pinned_toolchain(rule) {
        // Native `PrepareChecksToRun` excludes version-gated checks before
        // any file runs; they emit zero issues on this toolchain.
        return;
    }
    if !crate::check_meta::runs_at_min_priority(rule, config.min_priority) {
        // Native check-level pre-exclusion runs before any file.
        return;
    }
    if rule == "Credo.Check.Design.MissingCheckInConfig"
        || rule == "Credo.Check.Design.DeprecatedChecksConfig"
    {
        report
            .errors
            .push(RunError::NeedsValidatedConfig(rule.to_owned()));
        return;
    }
    match lane_for(rule) {
        Lane::Project => {
            run_lane(rule, || {
                run_project_lane(entry, prepared, config, work, report);
            });
        }
        Lane::Filename => {
            run_lane(rule, || {
                run_filename_lane(entry, prepared, config, work, report);
            });
        }
        Lane::Kernel => {
            run_lane(rule, || {
                run_kernel_lane(entry, prepared, config, work, report);
            });
        }
    }
}

/// Run one lane under a hotpath block labeled by check (no-op without
/// the `hotpath` feature) for per-check attribution in profile runs.
fn run_lane(rule: &str, lane: impl FnOnce()) {
    #[cfg(feature = "hotpath")]
    hotpath::measure_block!(hotpath_label(rule), lane());
    #[cfg(not(feature = "hotpath"))]
    {
        let _ = rule;
        lane();
    }
}

/// Leak a per-check label for hotpath blocks. Profiling builds only
/// (plain builds never call this): a few dozen small strings per run,
/// acceptable in a profiler.
#[cfg(feature = "hotpath")]
fn hotpath_label(rule: &str) -> &'static str {
    Box::leak(rule.to_owned().into_boxed_str())
}

/// Staged findings plus deferred config-comment checks.
#[derive(Default)]
struct Worklist {
    staged: Vec<(usize, Issue)>,
    redundant: Vec<BTreeMap<String, String>>,
}

/// Project lane: one run over the check's selected files.
fn run_project_lane(
    entry: &CheckEntry,
    prepared: &[PreparedFile<'_>],
    config: &RunnerConfig,
    work: &mut Worklist,
    report: &mut RunReport,
) {
    let rule = entry.module.as_str();
    let Some(inputs) = lane_inputs(rule, prepared, config, &entry.params, report) else {
        return;
    };
    let LaneInputs {
        index_map,
        files,
        metas,
        trees,
        facts,
    } = inputs;
    if files.is_empty() {
        return;
    }
    let found = match crate::project::run_project_check_with_trees(
        rule,
        &files,
        &trees,
        &facts,
        &entry.params,
    ) {
        Ok(found) => found,
        Err(error) => {
            report.errors.push(RunError::Issue(error.0));
            return;
        }
    };
    let staged = {
        match crate::project::build_project_issues(
            rule,
            &found,
            &entry.params,
            &config.general,
            &metas,
        ) {
            Ok(issues) => issues,
            Err(error) => {
                report.errors.push(RunError::Issue(error.0));
                return;
            }
        }
    };
    for (position, issue) in staged {
        if let Some(original) = index_map.get(position) {
            work.staged.push((*original, issue));
        }
    }
}

/// Selected project-lane inputs: original indexes, owned files for
/// collectors, and prepare-phase trees, facts and metadata aligned
/// with both.
struct LaneInputs<'s> {
    index_map: Vec<usize>,
    files: Vec<ProjectFile>,
    metas: Vec<&'s crate::check_meta::FileMeta<'s>>,
    trees: Vec<Option<&'s tree_sitter::Tree>>,
    facts: Vec<&'s crate::facts::Facts>,
}

/// Selected files with their original indexes and prepare-phase trees
/// and metadata, or `None` when a pattern error stops the check
/// (recorded once). Selection stays sequential so pattern errors keep
/// their ordering.
fn lane_inputs<'s>(
    rule: &str,
    prepared: &'s [PreparedFile<'_>],
    config: &RunnerConfig,
    params: &BTreeMap<String, String>,
    report: &mut RunReport,
) -> Option<LaneInputs<'s>> {
    let mut inputs = LaneInputs {
        index_map: Vec::new(),
        files: Vec::new(),
        metas: Vec::new(),
        trees: Vec::new(),
        facts: Vec::new(),
    };
    let matcher =
        CheckFileMatcher::compile(rule, &config.files_included, &config.files_excluded, params);
    for file in prepared {
        match selected_or_error(&matcher, file.filename, report) {
            Err(()) => return None,
            Ok(false) => continue,
            Ok(true) => {}
        }
        inputs.index_map.push(file.index);
        inputs.trees.push(file.prepared.tree());
        inputs.facts.push(file.prepared.facts());
        inputs.metas.push(&file.meta);
        inputs.files.push(ProjectFile {
            filename: file.filename.to_owned(),
            source: file.source.to_owned(),
        });
    }
    Some(inputs)
}

/// Filename lane: per-file filename-aware evaluation.
fn run_filename_lane(
    entry: &CheckEntry,
    prepared: &[PreparedFile<'_>],
    config: &RunnerConfig,
    work: &mut Worklist,
    report: &mut RunReport,
) {
    let rule = entry.module.as_str();
    let Some(selected) = selected_files(rule, prepared, config, &entry.params, report) else {
        return;
    };
    let task = |_position: usize, file: &&PreparedFile<'_>| {
        eval_filename_file(rule, file, &entry.params, &config.general)
    };
    for (_, output) in crate::batch::parallel_map(&selected, task) {
        drain_staged(work, report, output);
    }
}

/// One file's staged results for a filename-aware check, in finding order.
fn eval_filename_file(
    rule: &str,
    file: &PreparedFile<'_>,
    params: &BTreeMap<String, String>,
    general: &GeneralParams,
) -> (usize, Vec<Staged>) {
    let mut items = Vec::new();
    // Filename checks share the prepare-phase parse like the kernel lane
    // instead of reparsing each file per check.
    match crate::filename::run_filename_check_prepared(rule, file.filename, &file.prepared, params)
    {
        Ok(found) => {
            for project_issue in found {
                let finding = crate::project::project_finding(&project_issue);
                match build_issue(rule, finding, params, general, &file.meta) {
                    Ok(issue) => items.push(Staged::Issue(issue)),
                    Err(error) => items.push(Staged::Error(RunError::Issue(error.0))),
                }
            }
        }
        Err(error) => items.push(Staged::Error(RunError::UnknownRule(error.0))),
    }
    (file.index, items)
}

/// Kernel lane: per-file single-source evaluation over shared facts.
fn run_kernel_lane(
    entry: &CheckEntry,
    prepared: &[PreparedFile<'_>],
    config: &RunnerConfig,
    work: &mut Worklist,
    report: &mut RunReport,
) {
    let rule = entry.module.as_str();
    if !crate::batch::knows_rule(rule) {
        report.errors.push(RunError::UnknownRule(rule.to_owned()));
        return;
    }
    // Selection stays sequential: pattern errors keep their record-once
    // ordering and stop-the-check semantics.
    let Some(selected) = selected_files(rule, prepared, config, &entry.params, report) else {
        return;
    };
    let task = |_position: usize, file: &&PreparedFile<'_>| {
        eval_kernel_file(rule, file, &entry.params, &config.general)
    };
    for (_, output) in crate::batch::parallel_map(&selected, task) {
        drain_staged(work, report, output);
    }
}

/// Merge one file's staged results in finding order.
fn drain_staged(work: &mut Worklist, report: &mut RunReport, output: (usize, Vec<Staged>)) {
    let (file_index, items) = output;
    for item in items {
        match item {
            Staged::Issue(issue) => work.staged.push((file_index, issue)),
            Staged::Error(error) => report.errors.push(error),
        }
    }
}

/// One staged file result in finding order: issue errors stay inline
/// with issues so parallel merges match the sequential lane exactly.
enum Staged {
    Issue(Issue),
    Error(RunError),
}

/// One file's staged results for a kernel rule, in finding order.
fn eval_kernel_file(
    rule: &str,
    file: &PreparedFile<'_>,
    params: &BTreeMap<String, String>,
    general: &GeneralParams,
) -> (usize, Vec<Staged>) {
    let mut items = Vec::new();
    for finding in once_rule_named(rule, file, params) {
        match build_issue(rule, finding, params, general, &file.meta) {
            Ok(issue) => items.push(Staged::Issue(issue)),
            Err(error) => items.push(Staged::Error(RunError::Issue(error.0))),
        }
    }
    (file.index, items)
}

/// Files a check runs on, in input order; `None` when a pattern error
/// stops the check. Selection stays sequential so pattern errors keep
/// their record-once ordering.
fn selected_files<'s, 'f>(
    rule: &str,
    prepared: &'s [PreparedFile<'f>],
    config: &RunnerConfig,
    params: &BTreeMap<String, String>,
    report: &mut RunReport,
) -> Option<Vec<&'s PreparedFile<'f>>> {
    let mut selected = Vec::new();
    let matcher =
        CheckFileMatcher::compile(rule, &config.files_included, &config.files_excluded, params);
    for file in prepared {
        match selected_or_error(&matcher, file.filename, report) {
            Err(()) => return None,
            Ok(false) => {}
            Ok(true) => selected.push(file),
        }
    }
    Some(selected)
}

/// One rule's findings over prepared facts (batch sharing without the
/// 120-rule fan-out).
fn once_rule(
    rule: &str,
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<crate::Finding> {
    crate::batch::run_one(rule, prepared, params).unwrap_or_default()
}

/// One rule's findings with the checked filename available for messages
/// that name it (currently only `DuplicatedCode` single-file wording).
fn once_rule_named(
    rule: &str,
    file: &PreparedFile<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<crate::Finding> {
    if rule == "Credo.Check.Design.DuplicatedCode" {
        return crate::design::duplicated::check_prepared_with_filename(
            &file.prepared,
            params,
            file.filename,
        );
    }
    once_rule(rule, &file.prepared, params)
}

/// Per-check file selection; pattern errors are recorded once and stop
/// the check (`Err(())` signals the recorded stop).
fn selected_or_error(
    matcher: &CheckFileMatcher,
    filename: &str,
    report: &mut RunReport,
) -> Result<bool, ()> {
    matcher.matches(filename).map_err(|error| {
        report.errors.push(RunError::Pattern(error.0));
    })
}

/// Redundant-comment pass over every file's collected issues.
fn run_redundant_comments(
    work: &mut Worklist,
    prepared: &[PreparedFile<'_>],
    config: &RunnerConfig,
    report: &mut RunReport,
) {
    const RULE: &str = "Credo.Check.Design.RedundantConfigComments";
    for params in &work.redundant {
        for file in prepared {
            let file_issues: Vec<(String, usize)> = work
                .staged
                .iter()
                .filter(|(index, _)| *index == file.index)
                .filter_map(|(_, issue)| issue.line_no.map(|line| (issue.check.clone(), line)))
                .collect();
            let registered = !file.comments.is_empty();
            for finding in config_checks::redundant_comments(file.source, registered, &file_issues)
            {
                let kernel = crate::Finding {
                    line: finding.line,
                    column: finding.column,
                    message: finding.message.clone(),
                    trigger: crate::Trigger::Text(finding.trigger.clone()),
                    severity: None,
                };
                match build_issue(RULE, kernel, params, &config.general, &file.meta) {
                    Ok(issue) => work.staged.push((file.index, issue)),
                    Err(error) => report.errors.push(RunError::Issue(error.0)),
                }
            }
        }
    }
}

/// Priority filter, suppression, ordering and exit-status combination.
fn finish_report(
    work: &mut Worklist,
    prepared: &[PreparedFile<'_>],
    report: &mut RunReport,
    min_priority: i32,
) {
    for (index, issue) in work.staged.drain(..) {
        if issue.priority < min_priority {
            continue;
        }
        let suppressed = prepared
            .iter()
            .find(|file| file.index == index)
            .is_some_and(|file| {
                file.comments
                    .iter()
                    .any(|comment| comment.ignores(&issue.check, issue.line_no.unwrap_or(0)))
            });
        if !suppressed {
            if issue.exit_status != 0 {
                report.exit_status |= issue.exit_status;
            }
            report.issues.push(issue);
        }
    }
    report.issues.sort_by(|left, right| {
        (
            left.check.clone(),
            left.filename.clone(),
            left.line_no,
            left.column,
        )
            .cmp(&(
                right.check.clone(),
                right.filename.clone(),
                right.line_no,
                right.column,
            ))
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::Category;

    fn lib_file() -> RunnerFile {
        RunnerFile {
            filename: "lib/a.ex".to_owned(),
            source: "defmodule A do\n  IO.inspect(x)\nend\n".to_owned(),
        }
    }

    fn io_check() -> CheckEntry {
        CheckEntry {
            module: "Credo.Check.Warning.IoInspect".to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        }
    }

    fn default_config(checks: Vec<CheckEntry>) -> RunnerConfig {
        RunnerConfig {
            checks,
            files_included: Vec::new(),
            files_excluded: Vec::new(),
            selection: Selection::default(),
            min_priority: 0,
            general: crate::pipeline::GeneralParams::default(),
        }
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "default severity is exactly representable"
    )]
    fn kernel_lane_builds_sorted_issue() {
        let report = run_checks(&[lib_file()], &default_config(vec![io_check()]));
        assert!(report.errors.is_empty());
        assert_eq!(report.issues.len(), 1);
        let issue = &report.issues[0];
        assert_eq!(issue.check, "Credo.Check.Warning.IoInspect");
        assert_eq!(issue.category, Category::Warning);
        assert_eq!(issue.priority, 11);
        assert_eq!(issue.severity, 1.0);
        assert_eq!(issue.filename, "lib/a.ex");
        assert_eq!(issue.line_no, Some(2));
        assert_eq!(issue.column, Some(3));
        assert_eq!(issue.exit_status, 16);
        assert_eq!(issue.scope.as_deref(), Some("A"));
        assert_eq!(report.exit_status, 16);
    }

    #[test]
    fn kernel_lane_is_deterministic_across_runs() {
        // Parallel evaluation must stage byte-identical output on every
        // run: same issues in the same order with the same errors.
        let files = vec![
            RunnerFile {
                filename: "lib/b.ex".to_owned(),
                source: "defmodule B do\n  IO.inspect(x)\n  IO.inspect(y)\nend\n".to_owned(),
            },
            RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: "defmodule A do\n  IO.inspect(z)\nend\n".to_owned(),
            },
        ];
        let config = default_config(vec![io_check()]);
        let first = run_checks(&files, &config);
        assert_eq!(first.issues.len(), 3);
        for _ in 0..4 {
            let report = run_checks(&files, &config);
            assert_eq!(report.issues, first.issues);
            assert_eq!(report.errors, first.errors);
            assert_eq!(report.exit_status, first.exit_status);
        }
    }

    #[test]
    fn filename_lane_is_deterministic_across_runs() {
        let files = vec![
            RunnerFile {
                filename: "test/other.exs".to_owned(),
                source: "defmodule M do\n  use ExUnit.Case\nend\n".to_owned(),
            },
            RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: "defmodule A do\n  def f, do: 1\nend\n".to_owned(),
            },
        ];
        let check = CheckEntry {
            module: "Credo.Check.Warning.WrongTestFilename".to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        };
        let config = default_config(vec![check]);
        let first = run_checks(&files, &config);
        assert_eq!(first.issues.len(), 1);
        for _ in 0..4 {
            let report = run_checks(&files, &config);
            assert_eq!(report.issues, first.issues);
            assert_eq!(report.errors, first.errors);
        }
    }

    #[test]
    fn file_errors_and_skips_stay_in_file_order() {
        // Parallel preparation must keep per-file errors and skips in
        // input order across repeated runs.
        let files = vec![
            RunnerFile {
                filename: "lib/bad1.ex".to_owned(),
                source: "# credo:disable-for-next-line /[/\nIO.inspect(x)\n".to_owned(),
            },
            RunnerFile {
                filename: "lib/broken.ex".to_owned(),
                source: "def foo( do\n".to_owned(),
            },
            RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: "defmodule A do\n  IO.inspect(x)\nend\n".to_owned(),
            },
            RunnerFile {
                filename: "lib/bad2.ex".to_owned(),
                source: "# credo:disable-for-next-line /[/\nIO.inspect(y)\n".to_owned(),
            },
        ];
        let config = default_config(vec![io_check()]);
        let first = run_checks(&files, &config);
        assert_eq!(first.issues.len(), 1);
        assert_eq!(first.skipped_invalid, vec!["lib/broken.ex".to_owned()]);
        assert_eq!(first.errors.len(), 2);
        for _ in 0..4 {
            let report = run_checks(&files, &config);
            assert_eq!(report.issues, first.issues);
            assert_eq!(report.errors, first.errors);
            assert_eq!(report.skipped_invalid, first.skipped_invalid);
        }
    }

    #[test]
    fn only_miss_skips_check_silently() {
        let config = RunnerConfig {
            selection: Selection {
                only: vec!["Credo.Check.Warning.Dbg".to_owned()],
                ignore: Vec::new(),
                checks_with_tag: Vec::new(),
                enable_disabled: Vec::new(),
            },
            ..default_config(vec![io_check()])
        };
        let report = run_checks(&[lib_file()], &config);
        assert!(report.errors.is_empty());
        assert!(report.issues.is_empty());
        assert_eq!(report.exit_status, 0);
    }

    #[test]
    fn min_priority_filters() {
        let config = RunnerConfig {
            min_priority: 99,
            ..default_config(vec![io_check()])
        };
        let report = run_checks(&[lib_file()], &config);
        assert!(report.errors.is_empty());
        assert!(report.issues.is_empty());
    }

    #[test]
    fn suppression_removes_issue() {
        let file = RunnerFile {
            filename: "lib/a.ex".to_owned(),
            source: "defmodule A do\n  # credo:disable-for-next-line\n  IO.inspect(x)\nend\n"
                .to_owned(),
        };
        let report = run_checks(&[file], &default_config(vec![io_check()]));
        assert!(report.errors.is_empty());
        assert!(report.issues.is_empty());
    }

    #[test]
    fn project_lane_routes_consistency() {
        let files = vec![
            RunnerFile {
                filename: "a.ex".to_owned(),
                source: "defmodule M do\n\tdef f, do: 1\nend\n".to_owned(),
            },
            RunnerFile {
                filename: "b.ex".to_owned(),
                source: "defmodule N do\n  def g, do: 2\nend\n".to_owned(),
            },
        ];
        let check = CheckEntry {
            module: "Credo.Check.Consistency.TabsOrSpaces".to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        };
        let report = run_checks(&files, &default_config(vec![check]));
        assert!(report.errors.is_empty());
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].priority, 11);
        assert_eq!(report.issues[0].scope.as_deref(), Some("M.f"));
        assert_eq!(report.exit_status, 1);
    }

    #[test]
    fn filename_lane_reports() {
        let file = RunnerFile {
            filename: "test/other.exs".to_owned(),
            source: "defmodule M do\n  use ExUnit.Case\nend\n".to_owned(),
        };
        let check = CheckEntry {
            module: "Credo.Check.Warning.WrongTestFilename".to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        };
        let report = run_checks(&[file], &default_config(vec![check]));
        assert!(report.errors.is_empty());
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].filename, "test/other.exs");
    }

    #[test]
    fn unknown_rule_is_explicit_error() {
        let check = CheckEntry {
            module: "Credo.Check.Nope".to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        };
        let report = run_checks(&[lib_file()], &default_config(vec![check]));
        assert_eq!(
            report.errors,
            vec![RunError::UnknownRule("Credo.Check.Nope".to_owned())]
        );
        assert!(report.issues.is_empty());
    }

    #[test]
    fn named_kernel_lane_reports_checked_filename() {
        // The filename-aware lane names the checked file; the anonymous
        // `once_rule` path keeps the generic rendering.
        let block = "  x = 1_000_000_000_000_000_000_000_000\n  y = 2_000_000_000_000_000_000_000_000\n  z = x + y + 1_000_000_000_000_000_000_000\n  w = z * 2_000_000_000_000_000_000_000_000\n";
        let source = format!("def a do\n{block}end\ndef b do\n{block}end\n");
        let file = RunnerFile {
            filename: "lib/a.ex".to_owned(),
            source,
        };
        let prepared = crate::batch::Prepared::lazy(&file.source);
        let meta = FileMeta::collect_shared(&file.filename, &file.source, prepared.facts());
        let prepared_file = PreparedFile {
            index: 0,
            filename: &file.filename,
            source: &file.source,
            meta,
            prepared,
            comments: config_comments(&file.source),
        };
        let params = BTreeMap::new();
        let named = once_rule_named("Credo.Check.Design.DuplicatedCode", &prepared_file, &params);
        assert_eq!(named.len(), 1);
        assert!(
            named[0]
                .message
                .starts_with("Duplicate code found in lib/a.ex (mass: "),
            "unexpected message: {}",
            named[0].message
        );
        let anonymous = once_rule(
            "Credo.Check.Design.DuplicatedCode",
            &prepared_file.prepared,
            &params,
        );
        assert_eq!(anonymous.len(), 1);
        assert!(
            anonymous[0]
                .message
                .starts_with("Duplicate code found in file (mass: "),
            "unexpected message: {}",
            anonymous[0].message
        );
    }

    #[test]
    fn version_gated_kernels_still_fire_directly() {
        // Native `run/2` bypasses the version gate (it lives in
        // `PrepareChecksToRun`); direct kernel calls keep answering.
        assert_eq!(
            crate::check_kernel("Credo.Check.Warning.LazyLogging", lazy_src())
                .expect("kernel")
                .len(),
            1
        );
        assert_eq!(
            crate::check_kernel(
                "Credo.Check.Readability.PreferUnquotedAtoms",
                unquoted_src()
            )
            .expect("kernel")
            .len(),
            1
        );
        assert_eq!(
            crate::check_kernel("Credo.Check.Refactor.MapInto", map_src())
                .expect("kernel")
                .len(),
            1
        );
    }

    fn lazy_src() -> &'static str {
        "Logger.debug(\"hi #{x}\")\n"
    }

    fn unquoted_src() -> &'static str {
        "x = :\"foo\"\n"
    }

    fn map_src() -> &'static str {
        "x = Enum.map(a, f) |> Enum.into(b)\n"
    }

    #[test]
    fn version_gated_checks_emit_nothing_on_pinned_toolchain() {
        // Execution skips what native `PrepareChecksToRun` skips on
        // Elixir 1.20.2: zero issues from firing kernels.
        let files = vec![
            RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: lazy_src().to_owned(),
            },
            RunnerFile {
                filename: "lib/b.ex".to_owned(),
                source: unquoted_src().to_owned(),
            },
            RunnerFile {
                filename: "lib/c.ex".to_owned(),
                source: map_src().to_owned(),
            },
        ];
        let gated = vec![
            "Credo.Check.Warning.LazyLogging",
            "Credo.Check.Readability.PreferUnquotedAtoms",
            "Credo.Check.Refactor.MapInto",
        ]
        .into_iter()
        .map(|module| CheckEntry {
            module: module.to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        })
        .collect();
        let report = run_checks(&files, &default_config(gated));
        assert!(report.errors.is_empty());
        assert!(report.issues.is_empty());
        assert_eq!(report.exit_status, 0);
    }

    #[test]
    fn invalid_selection_is_explicit_error() {
        let config = RunnerConfig {
            selection: Selection {
                only: vec!["([".to_owned()],
                ignore: Vec::new(),
                checks_with_tag: Vec::new(),
                enable_disabled: Vec::new(),
            },
            ..default_config(vec![io_check()])
        };
        let report = run_checks(&[lib_file()], &config);
        assert_eq!(
            report.errors,
            vec![RunError::InvalidSelection("([".to_owned())]
        );
        assert!(report.issues.is_empty());
    }

    #[test]
    fn comment_error_skips_file_loudly() {
        let bad = RunnerFile {
            filename: "lib/bad.ex".to_owned(),
            source: "# credo:disable-for-next-line /[/\nIO.inspect(x)\n".to_owned(),
        };
        let report = run_checks(&[lib_file(), bad], &default_config(vec![io_check()]));
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].filename, "lib/a.ex");
        assert_eq!(report.errors.len(), 1);
        assert!(matches!(report.errors[0], RunError::Comment { .. }));
    }

    #[test]
    fn config_checks_need_validated_config() {
        for module in [
            "Credo.Check.Design.MissingCheckInConfig",
            "Credo.Check.Design.DeprecatedChecksConfig",
        ] {
            let check = CheckEntry {
                module: module.to_owned(),
                enabled: true,
                params: BTreeMap::new(),
            };
            let report = run_checks(&[lib_file()], &default_config(vec![check]));
            assert_eq!(
                report.errors,
                vec![RunError::NeedsValidatedConfig(module.to_owned())]
            );
            assert!(report.issues.is_empty());
        }
    }

    #[test]
    fn exit_status_ors_issues() {
        let todo = CheckEntry {
            module: "Credo.Check.Design.TagTODO".to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        };
        let file = RunnerFile {
            filename: "lib/a.ex".to_owned(),
            source: "defmodule A do\n  # TODO: x\n  IO.inspect(y)\nend\n".to_owned(),
        };
        let report = run_checks(&[file], &default_config(vec![io_check(), todo]));
        assert!(report.errors.is_empty());
        assert_eq!(report.issues.len(), 2);
        assert_eq!(report.exit_status, 16 | 2);
        // Relevant order sorts by check name, not config order.
        assert_eq!(
            report
                .issues
                .iter()
                .map(|issue| issue.check.clone())
                .collect::<Vec<_>>(),
            vec![
                "Credo.Check.Design.TagTODO".to_owned(),
                "Credo.Check.Warning.IoInspect".to_owned()
            ]
        );
    }

    #[test]
    fn per_file_scope_covers_all_dispatches() {
        for rule in PROJECT_LANE {
            assert!(!super::supports_per_file(rule), "{rule}");
        }
        for rule in FILENAME_LANE {
            assert!(super::supports_per_file(rule), "{rule}");
        }
        assert!(!super::supports_per_file(
            "Credo.Check.Design.RedundantConfigComments"
        ));
        for rule in [
            "Credo.Check.Design.MissingCheckInConfig",
            "Credo.Check.Design.DeprecatedChecksConfig",
        ] {
            assert!(!super::supports_per_file(rule), "{rule}");
        }
        assert!(super::supports_per_file("Credo.Check.Warning.IoInspect"));
        assert!(super::supports_per_file(
            "Credo.Check.Readability.TrailingBlankLine"
        ));
    }

    #[test]
    fn lanes_cover_dispatches_without_drift() {
        use crate::project::ProjectFile;
        let empty_files: Vec<ProjectFile> = Vec::new();
        let empty_params = BTreeMap::new();
        let empty_file = ProjectFile {
            filename: String::new(),
            source: String::new(),
        };
        for rule in PROJECT_LANE {
            assert!(
                crate::project::run_project_check(rule, &empty_files, &empty_params).is_ok(),
                "{rule}"
            );
        }
        for rule in FILENAME_LANE {
            assert!(
                crate::run_filename_check(rule, &empty_file, &empty_params).is_ok(),
                "{rule}"
            );
        }
        let text = include_str!("../compatibility/rules.json");
        let json: serde_json::Value = serde_json::from_str(text).expect("valid JSON");
        let rules = json["rules"].as_array().expect("rules array");
        assert_eq!(rules.len(), 120);
        for rule in rules {
            let id = rule["id"].as_str().expect("id");
            if PROJECT_LANE.contains(&id) || FILENAME_LANE.contains(&id) {
                continue;
            }
            assert!(crate::batch::knows_rule(id), "{id}");
        }
    }
}
