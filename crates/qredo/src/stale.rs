//! Incremental `--stale` execution with aggressive disk caching.
//!
//! Steady-state `--stale` runs skip parsing, fact extraction and kernel
//! evaluation for unchanged files entirely:
//!
//! - Kernel/filename-lane issues are reused per file by content hash.
//! - Project-lane (consistency) votes are cached per file as small
//!   `kind -> count` maps; the global majority is recomputed from merged
//!   counts and only fresh files are re-emitted — no AST work for cached
//!   files. A flipped winner fails open to a full run (rare, always
//!   correct).
//! - `RedundantConfigComments` is recomputed globally from the merged issue
//!   set, but only files carrying config comments pay for `FileMeta`.
//!
//! Cache layout and fingerprinting live in [`crate::stale_cache`]. Every
//! mismatch (config, selection, priority, tool version, corrupt payload)
//! fails open to a full run, never to silent divergence.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::integration::{Fallback, Outcome};
use crate::stale_cache::{CachedCommentError, CachedFile, DiskCache};

// Lanes needing the complete staged issue set or whole-project aggregation.
const REDUNDANT: &str = "Credo.Check.Design.RedundantConfigComments";
const VALIDATED: &[&str] = &[
    "Credo.Check.Design.MissingCheckInConfig",
    "Credo.Check.Design.DeprecatedChecksConfig",
];

type ProjectCounts = BTreeMap<String, BTreeMap<String, usize>>;
type ProjectCountsByFile = BTreeMap<String, ProjectCounts>;

/// Run the native pipeline with incremental caching for served configs.
///
/// `root` anchors the cache directory (`~/.cache/qredo/<proj>/`). Behaves
/// like [`crate::integration::execute_selected`] on a cold cache.
///
/// # Errors
/// Returns [`Fallback`] with the stable recorded reason instead of running
/// anything native; the caller runs real Credo in that case.
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors integration::execute_selected plus the cache root"
)]
pub fn execute_stale(
    config_source: &str,
    config_name: &str,
    files: &[crate::RunnerFile],
    min_priority: i32,
    selection: crate::Selection,
    root: &Path,
) -> Result<crate::RunReport, Fallback> {
    execute_stale_with_path(
        config_source,
        config_name,
        files,
        min_priority,
        selection,
        root,
        None,
    )
}

/// [`execute_stale`] with an explicit cache file override for tests.
/// `None` resolves through [`crate::stale_cache::cache_file`].
#[allow(
    clippy::too_many_arguments,
    reason = "test hook mirroring execute_stale plus the cache override"
)]
pub(crate) fn execute_stale_with_path(
    config_source: &str,
    config_name: &str,
    files: &[crate::RunnerFile],
    min_priority: i32,
    selection: crate::Selection,
    root: &Path,
    cache_path: Option<&Path>,
) -> Result<crate::RunReport, Fallback> {
    if let Outcome::Fallback { reason } = crate::integration::select(config_source, config_name) {
        return Err(Fallback { reason });
    }
    let config = crate::parse_config(config_source, config_name).map_err(|bad| Fallback {
        reason: format!("unsupported-credo-config:{}", bad.0),
    })?;
    if let Err(pattern) = selection.validate() {
        return Ok(crate::RunReport {
            issues: Vec::new(),
            exit_status: 0,
            errors: vec![crate::RunError::InvalidSelection(pattern)],
            skipped_invalid: Vec::new(),
            mods_funs: None,
        });
    }
    let runner_config = crate::integration::runner_of(&config, min_priority, selection);
    let fingerprint = crate::stale_cache::fingerprint(
        config_source,
        config_name,
        &config.env_snapshot,
        &runner_config.checks,
        &runner_config.selection,
        min_priority,
    );
    let loaded = match cache_path {
        Some(path) => crate::stale_cache::load_file(path, &fingerprint),
        None => crate::stale_cache::load(root, &fingerprint),
    };
    let saver = |cache: &DiskCache| match cache_path {
        Some(path) => crate::stale_cache::save_file(path, cache),
        None => crate::stale_cache::save(root, cache),
    };
    match loaded {
        None => Ok(full_run_and_save(
            files,
            &runner_config,
            &fingerprint,
            &saver,
        )),
        Some(cache) => Ok(incremental(
            files,
            &runner_config,
            cache,
            &fingerprint,
            &saver,
        )),
    }
}

/// Per-file sub-config for fresh files: per-file lanes plus
/// config-validated error carriers. Project-lane and redundant checks run
/// globally in [`incremental`], never on a subset.
fn fresh_sub_config(full: &crate::RunnerConfig) -> crate::RunnerConfig {
    let mut sub = full.clone();
    sub.checks = full
        .checks
        .iter()
        .filter(|entry| {
            crate::supports_per_file(&entry.module) || VALIDATED.contains(&entry.module.as_str())
        })
        .cloned()
        .collect();
    sub
}

/// Text-only comment validation for a changed file. Unchanged files reuse
/// this exact outcome by content hash from [`CachedFile::comment_error`].
fn validate_comment(file: &crate::RunnerFile) -> Result<(), CachedCommentError> {
    crate::suppression::validate_comments(&file.source).map_err(|error| CachedCommentError {
        line_no: error.line_no,
        message: error.message,
    })
}

/// Full run plus cold-save: issues regrouped per file, votes collected with
/// the same per-check file selection as the pipeline, winners recorded.
fn full_run_and_save(
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    fingerprint: &str,
    saver: &dyn Fn(&DiskCache),
) -> crate::RunReport {
    let mut report = crate::run_checks(files, runner_config);
    let cache = cold_cache(files, &report, runner_config, fingerprint);
    report.mods_funs = Some(cache.mods_funs);
    saver(&cache);
    report
}

/// Build a fresh cache payload from a full-run report. Vote collection
/// respects per-check file selection; files failing selection for a check
/// simply do not vote for it.
fn cold_cache(
    files: &[crate::RunnerFile],
    report: &crate::RunReport,
    runner_config: &crate::RunnerConfig,
    fingerprint: &str,
) -> DiskCache {
    let mut cache = DiskCache::empty(fingerprint.to_owned());
    cache.file_order = files.iter().map(|file| file.filename.clone()).collect();
    cache.issues.clone_from(&report.issues);
    cache.exit_status = report.exit_status;
    cache.errors.clone_from(&report.errors);
    cache.skipped_invalid.clone_from(&report.skipped_invalid);
    let (pattern_stopped, pattern_errors) = pattern_stops(files, runner_config);
    cache.pattern_stopped = pattern_stopped;
    cache.pattern_errors = pattern_errors;
    let skipped: BTreeSet<&str> = report.skipped_invalid.iter().map(String::as_str).collect();
    let comment_errors = cached_comment_errors(report);
    // Per-file facts parsed once and shared across the `Facts`-backed
    // collectors (cold runs pay one extra parse pass for votes).
    let prepared: Vec<crate::batch::Prepared<'_>> = files
        .iter()
        .map(|file| crate::batch::Prepared::eager(&file.source))
        .collect();
    let project_matchers: BTreeMap<String, crate::file_select::CheckFileMatcher> =
        project_entries(runner_config)
            .into_iter()
            .map(|entry| (entry.module.clone(), check_matcher(entry, runner_config)))
            .collect();
    for (index, file) in files.iter().enumerate() {
        let (name, entry) = cold_file_entry(
            file,
            &project_matchers,
            &prepared[index],
            &skipped,
            &comment_errors,
        );
        cache.files.insert(name, entry);
    }
    cache.mods_funs = cache.files.values().map(|file| file.mods_funs).sum();
    for entry in project_entries(runner_config) {
        cache.winners.insert(
            entry.module.clone(),
            match cold_winner(&entry.module, &entry.params, &cache) {
                crate::project::StaleWinner::Known(winner) => winner,
                crate::project::StaleWinner::Unsupported => None,
            },
        );
    }
    // Pattern errors recorded by the full run stay in the report; the
    // cache only needs the vote view (errored checks simply have no
    // votes, matching the pipeline which stages nothing for them).
    cache
}

/// Comment failures from a full report, keyed by borrowed filename.
fn cached_comment_errors(report: &crate::RunReport) -> BTreeMap<&str, CachedCommentError> {
    report
        .errors
        .iter()
        .filter_map(|error| match error {
            crate::RunError::Comment {
                file,
                line_no,
                message,
            } => Some((
                file.as_str(),
                CachedCommentError {
                    line_no: *line_no,
                    message: message.clone(),
                },
            )),
            _ => None,
        })
        .collect()
}

/// One cold-cache file unit with its project votes.
fn cold_file_entry(
    file: &crate::RunnerFile,
    project_matchers: &BTreeMap<String, crate::file_select::CheckFileMatcher>,
    prepared: &crate::batch::Prepared<'_>,
    skipped: &BTreeSet<&str>,
    comment_errors: &BTreeMap<&str, CachedCommentError>,
) -> (String, CachedFile) {
    let hash = crate::stale_cache::content_hash(&file.source);
    if let Some(error) = comment_errors.get(file.filename.as_str()) {
        return (
            file.filename.clone(),
            CachedFile {
                hash,
                project_counts: BTreeMap::new(),
                skipped_invalid: false,
                mods_funs: 0,
                comment_error: Some(error.clone()),
            },
        );
    }
    if skipped.contains(file.filename.as_str()) {
        return (
            file.filename.clone(),
            CachedFile {
                hash,
                project_counts: BTreeMap::new(),
                skipped_invalid: true,
                mods_funs: 0,
                comment_error: None,
            },
        );
    }
    let mut project_counts = BTreeMap::new();
    for (module, matcher) in project_matchers {
        let votes = match selected_for(matcher, &file.filename) {
            Ok(true) => collect_one_votes(module, file, prepared),
            Ok(false) | Err(()) => BTreeMap::new(),
        };
        if !votes.is_empty() {
            project_counts.insert(module.clone(), votes);
        }
    }
    (
        file.filename.clone(),
        CachedFile {
            hash,
            project_counts,
            skipped_invalid: false,
            mods_funs: prepared.facts().modules.len() + prepared.facts().defs.len(),
            comment_error: None,
        },
    )
}

/// Merged cold winner for one project check over cached file votes.
fn cold_winner(
    rule: &str,
    params: &BTreeMap<String, String>,
    cache: &DiskCache,
) -> crate::project::StaleWinner {
    let mut merged: BTreeMap<String, usize> = BTreeMap::new();
    let mut names: Vec<&String> = cache.files.keys().collect();
    names.sort();
    for name in names {
        if let Some(counts) = cache
            .files
            .get(name)
            .and_then(|cached| cached.project_counts.get(rule))
        {
            for (kind, count) in counts {
                *merged.entry(kind.clone()).or_insert(0) += count;
            }
        }
    }
    crate::project::stale_winner(rule, &merged, params)
}

/// Collect one file's votes for one project check using its prepared facts.
fn collect_one_votes(
    rule: &str,
    file: &crate::RunnerFile,
    prepared: &crate::batch::Prepared<'_>,
) -> BTreeMap<String, usize> {
    let project_file = crate::project::ProjectFile {
        filename: file.filename.clone(),
        source: file.source.clone(),
    };
    let files = std::slice::from_ref(&project_file);
    let facts = prepared.facts();
    let facts_slice = std::slice::from_ref(&facts);
    let Some((counts, _)) = crate::project::stale_collect(rule, files, facts_slice) else {
        return BTreeMap::new();
    };
    counts
}

/// Project-lane entries the runner would execute (enabled, selected,
/// version- and priority-gated), in config order.
fn project_entries(runner_config: &crate::RunnerConfig) -> Vec<&crate::CheckEntry> {
    runner_config
        .checks
        .iter()
        .filter(|entry| {
            entry.enabled
                && runner_config.selection.should_run_entry(entry)
                && !crate::version_skipped_on_pinned_toolchain(&entry.module)
                && crate::runs_at_min_priority(&entry.module, runner_config.min_priority)
                && !crate::supports_per_file(&entry.module)
                && entry.module.as_str() != REDUNDANT
                && !VALIDATED.contains(&entry.module.as_str())
        })
        .collect()
}

/// Compile one check's file selection once for repeated file matches.
fn check_matcher(
    entry: &crate::CheckEntry,
    runner_config: &crate::RunnerConfig,
) -> crate::file_select::CheckFileMatcher {
    crate::file_select::CheckFileMatcher::compile(
        &entry.module,
        &runner_config.files_included,
        &runner_config.files_excluded,
        &entry.params,
    )
}

/// Per-check file selection mirroring the pipeline. Pattern errors were
/// already recorded by the caller's validation pass.
fn selected_for(
    matcher: &crate::file_select::CheckFileMatcher,
    filename: &str,
) -> Result<bool, ()> {
    matcher.matches(filename).map_err(|_| ())
}

/// Upfront pattern validation over all files for every executed check:
/// a pattern stop yields no issues for that check in a full run, so
/// errored checks are removed from the fresh subset and stripped from
/// reused issues. Cheap glob matching, no AST.
fn pattern_stops(
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
) -> (BTreeSet<String>, Vec<crate::RunError>) {
    let mut stopped = BTreeSet::new();
    let mut errors = Vec::new();
    for entry in runner_config.checks.iter().filter(|entry| {
        entry.enabled
            && runner_config.selection.should_run_entry(entry)
            && !crate::version_skipped_on_pinned_toolchain(&entry.module)
            && crate::runs_at_min_priority(&entry.module, runner_config.min_priority)
            && entry.module.as_str() != REDUNDANT
            && !VALIDATED.contains(&entry.module.as_str())
    }) {
        let matcher = check_matcher(entry, runner_config);
        for file in files {
            match matcher.matches(&file.filename) {
                Ok(_) => {}
                Err(error) => {
                    if stopped.insert(entry.module.clone()) {
                        errors.push(crate::RunError::Pattern(error.0));
                    }
                    break;
                }
            }
        }
    }
    (stopped, errors)
}

/// Validation + partition plan for one incremental run.
struct Partition {
    errors: Vec<crate::RunError>,
    errored_file: BTreeSet<String>,
    comment_errors: BTreeMap<String, CachedCommentError>,
    hashes: Vec<String>,
    dirty: BTreeSet<usize>,
    fresh: Vec<usize>,
    reused: Vec<usize>,
    skipped_set: BTreeSet<String>,
    pattern_stopped: BTreeSet<String>,
    pattern_errors: Vec<crate::RunError>,
    exact_file_order: bool,
}

impl Partition {
    /// Empty classification plan carrying reusable pattern validation.
    fn new(
        file_capacity: usize,
        pattern_stopped: BTreeSet<String>,
        pattern_errors: Vec<crate::RunError>,
        exact_file_order: bool,
    ) -> Self {
        Self {
            errors: pattern_errors.clone(),
            errored_file: BTreeSet::new(),
            comment_errors: BTreeMap::new(),
            hashes: Vec::with_capacity(file_capacity),
            dirty: BTreeSet::new(),
            fresh: Vec::new(),
            reused: Vec::new(),
            skipped_set: BTreeSet::new(),
            pattern_stopped,
            pattern_errors,
            exact_file_order,
        }
    }

    /// Record one exact comment-validation failure in report and cache form.
    fn record_comment_error(&mut self, file: &crate::RunnerFile, error: CachedCommentError) {
        self.errored_file.insert(file.filename.clone());
        self.comment_errors
            .insert(file.filename.clone(), error.clone());
        self.errors.push(crate::RunError::Comment {
            file: file.filename.clone(),
            line_no: error.line_no,
            message: error.message,
        });
    }

    /// Classify one file by exact hash, reusing validation when safe.
    fn classify(&mut self, index: usize, file: &crate::RunnerFile, cache: &DiskCache) {
        let hash = crate::stale_cache::content_hash(&file.source);
        self.hashes.push(hash.clone());
        if let Some(cached) = cache
            .files
            .get(&file.filename)
            .filter(|cached| cached.hash == hash)
        {
            if let Some(error) = &cached.comment_error {
                self.record_comment_error(file, error.clone());
            } else if cached.skipped_invalid {
                self.skipped_set.insert(file.filename.clone());
            } else {
                self.reused.push(index);
            }
            return;
        }
        self.dirty.insert(index);
        match validate_comment(file) {
            Ok(()) => self.fresh.push(index),
            Err(error) => self.record_comment_error(file, error),
        }
    }
}

/// Exact discovery order is required for reusing lazy pattern outcomes and
/// report error ordering.
fn exact_file_order(files: &[crate::RunnerFile], cache: &DiskCache) -> bool {
    cache.file_order.len() == files.len()
        && cache
            .file_order
            .iter()
            .zip(files)
            .all(|(cached, file)| cached == &file.filename)
}

/// Comment validation plus content-hash partitioning. Files failing
/// comment validation are set aside (revalidated cheaply every run);
/// skipped-invalid bits are trusted by hash without re-parsing.
fn plan_partitions(
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    cache: &DiskCache,
) -> Partition {
    let exact_file_order = exact_file_order(files, cache);
    let (pattern_stopped, pattern_errors) = if exact_file_order {
        (cache.pattern_stopped.clone(), cache.pattern_errors.clone())
    } else {
        pattern_stops(files, runner_config)
    };
    let mut plan = Partition::new(
        files.len(),
        pattern_stopped,
        pattern_errors,
        exact_file_order,
    );
    for (index, file) in files.iter().enumerate() {
        plan.classify(index, file, cache);
    }
    plan
}

/// Move out the exact final report stored by a hash-clean cache hit.
fn cached_report(cache: DiskCache) -> crate::RunReport {
    crate::RunReport {
        issues: cache.issues,
        exit_status: cache.exit_status,
        errors: cache.errors,
        skipped_invalid: cache.skipped_invalid,
        mods_funs: Some(cache.mods_funs),
    }
}

/// Incremental run against a fingerprint-matched cache.
fn incremental(
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    cache: DiskCache,
    fingerprint: &str,
    saver: &dyn Fn(&DiskCache),
) -> crate::RunReport {
    let mut plan = plan_partitions(files, runner_config, &cache);
    if plan.exact_file_order && plan.dirty.is_empty() {
        return cached_report(cache);
    }
    let fresh_run = run_fresh_subset(files, runner_config, &mut plan);
    let mut issues = merge_reused_issues(files, &cache, &plan, fresh_run.issues);

    // Project phase per check; a flipped majority fails open to a full
    // run rather than guessing.
    let Some(phase) = project_phase(
        files,
        runner_config,
        &cache,
        &plan,
        &fresh_run.fresh_valid,
        &fresh_run.prepared,
    ) else {
        return full_run_and_save(files, runner_config, fingerprint, saver);
    };
    // Pattern-errored project checks contribute nothing: strip them.
    issues.retain(|issue| !phase.drop_checks.contains(&issue.check));
    issues.extend(phase.fresh_issues);

    // Global redundant pass over the merged pre-redundant set.
    issues.extend(redundant_globally(
        files,
        runner_config,
        &issues,
        &plan.errored_file,
    ));

    sort_report(&mut issues);
    let exit_status = issues
        .iter()
        .fold(0, |status, issue| status | issue.exit_status);

    let mods_funs = persist_cache(
        files,
        runner_config,
        &cache,
        fingerprint,
        saver,
        &plan,
        phase.new_winners,
        &phase.drop_checks,
        &fresh_run.fresh_valid,
        &fresh_run.prepared,
        &issues,
        exit_status,
        &fresh_run.skipped_invalid,
    );

    crate::RunReport {
        issues,
        exit_status,
        errors: plan.errors,
        skipped_invalid: fresh_run.skipped_invalid,
        mods_funs: Some(mods_funs),
    }
}

/// Reused kernel/filename issues (pattern-stopped checks stripped) plus
/// fresh subset issues.
fn merge_reused_issues(
    files: &[crate::RunnerFile],
    cache: &DiskCache,
    plan: &Partition,
    fresh: Vec<crate::Issue>,
) -> Vec<crate::Issue> {
    let reused_names: BTreeSet<&str> = plan
        .reused
        .iter()
        .map(|index| files[*index].filename.as_str())
        .collect();
    let mut issues: Vec<crate::Issue> = cache
        .issues
        .iter()
        .filter(|issue| reused_names.contains(issue.filename.as_str()))
        .filter(|issue| !plan.pattern_stopped.contains(&issue.check))
        .filter(|issue| issue.check != REDUNDANT)
        .cloned()
        .collect();
    issues.extend(fresh);
    issues
}

/// Fresh-subset run outputs: per-file-lane issues plus shared facts.
struct FreshRun<'a> {
    issues: Vec<crate::Issue>,
    fresh_valid: Vec<usize>,
    prepared: Vec<crate::batch::Prepared<'a>>,
    skipped_invalid: Vec<String>,
}

/// Fresh per-file lanes on the subset, plus one shared parse per fresh
/// valid file for the `Facts`-backed project collectors.
fn run_fresh_subset<'a>(
    files: &'a [crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    plan: &mut Partition,
) -> FreshRun<'a> {
    let mut sub = fresh_sub_config(runner_config);
    sub.checks
        .retain(|entry| !plan.pattern_stopped.contains(&entry.module));
    let fresh_files: Vec<crate::RunnerFile> = plan
        .fresh
        .iter()
        .map(|index| files[*index].clone())
        .collect();
    let sub_report = crate::run_checks(&fresh_files, &sub);
    plan.errors.extend(sub_report.errors);
    for name in sub_report.skipped_invalid {
        plan.skipped_set.insert(name);
    }
    let skipped_invalid: Vec<String> = files
        .iter()
        .map(|file| file.filename.clone())
        .filter(|name| plan.skipped_set.contains(name))
        .collect();
    let fresh_valid: Vec<usize> = plan
        .fresh
        .iter()
        .filter(|index| !plan.skipped_set.contains(&files[**index].filename))
        .copied()
        .collect();
    let prepared: Vec<crate::batch::Prepared<'_>> = fresh_valid
        .iter()
        .map(|index| crate::batch::Prepared::eager(&files[*index].source))
        .collect();
    FreshRun {
        issues: sub_report.issues,
        fresh_valid,
        prepared,
        skipped_invalid,
    }
}

/// Inputs used to rebuild per-file cache entries after an incremental run.
struct PersistEntries<'a> {
    cache: &'a DiskCache,
    plan: &'a Partition,
    drop_checks: &'a BTreeSet<String>,
    fresh_counts: &'a ProjectCountsByFile,
    fresh_mods_funs: &'a BTreeMap<String, usize>,
}

impl PersistEntries<'_> {
    /// One hash-bound file payload, reusing or replacing project votes.
    fn file(&self, index: usize, file: &crate::RunnerFile) -> CachedFile {
        let hash = self.plan.hashes[index].clone();
        if let Some(error) = self.plan.comment_errors.get(&file.filename) {
            return CachedFile {
                hash,
                project_counts: BTreeMap::new(),
                skipped_invalid: false,
                mods_funs: 0,
                comment_error: Some(error.clone()),
            };
        }
        if self.plan.skipped_set.contains(&file.filename) {
            return CachedFile {
                hash,
                project_counts: BTreeMap::new(),
                skipped_invalid: true,
                mods_funs: 0,
                comment_error: None,
            };
        }
        CachedFile {
            hash,
            project_counts: self.project_counts(index, file),
            skipped_invalid: false,
            mods_funs: self.mods_funs(index, file),
            comment_error: None,
        }
    }

    /// Reused scope count for clean files, freshly collected count otherwise.
    fn mods_funs(&self, index: usize, file: &crate::RunnerFile) -> usize {
        if self.plan.reused.contains(&index) {
            return self
                .cache
                .files
                .get(&file.filename)
                .map_or(0, |cached| cached.mods_funs);
        }
        self.fresh_mods_funs
            .get(&file.filename)
            .copied()
            .unwrap_or(0)
    }

    /// Reused votes for clean files, fresh votes for changed files.
    fn project_counts(
        &self,
        index: usize,
        file: &crate::RunnerFile,
    ) -> BTreeMap<String, BTreeMap<String, usize>> {
        let mut counts = if self.plan.reused.contains(&index) {
            self.cache
                .files
                .get(&file.filename)
                .map(|cached| cached.project_counts.clone())
                .unwrap_or_default()
        } else {
            self.fresh_counts
                .get(&file.filename)
                .cloned()
                .unwrap_or_default()
        };
        for stopped in self.drop_checks {
            counts.remove(stopped);
        }
        counts
    }
}

/// Persist one incremental run: reused entries keep vote counts, fresh
/// entries are built from this run, and every hash-bound validation outcome
/// plus the sorted report is recorded for a direct clean-hit return.
#[allow(
    clippy::too_many_arguments,
    reason = "one cache-save spine; inputs are the plan plus merge outputs"
)]
fn persist_cache(
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    cache: &DiskCache,
    fingerprint: &str,
    saver: &dyn Fn(&DiskCache),
    plan: &Partition,
    new_winners: BTreeMap<String, Option<String>>,
    drop_checks: &BTreeSet<String>,
    fresh_valid: &[usize],
    fresh_prepared: &[crate::batch::Prepared<'_>],
    issues: &[crate::Issue],
    exit_status: i32,
    skipped_invalid: &[String],
) -> usize {
    let mut next = DiskCache::empty(fingerprint.to_owned());
    next.file_order
        .extend(files.iter().map(|file| file.filename.clone()));
    next.issues.extend_from_slice(issues);
    next.exit_status = exit_status;
    next.errors.clone_from(&plan.errors);
    next.skipped_invalid.extend_from_slice(skipped_invalid);
    next.pattern_stopped.clone_from(&plan.pattern_stopped);
    next.pattern_errors.clone_from(&plan.pattern_errors);
    next.winners = new_winners;
    let fresh_counts = fresh_vote_counts(files, runner_config, fresh_valid, fresh_prepared);
    let fresh_mods_funs: BTreeMap<String, usize> = fresh_valid
        .iter()
        .zip(fresh_prepared)
        .map(|(index, prepared)| {
            (
                files[*index].filename.clone(),
                prepared.facts().modules.len() + prepared.facts().defs.len(),
            )
        })
        .collect();
    let entries = PersistEntries {
        cache,
        plan,
        drop_checks,
        fresh_counts: &fresh_counts,
        fresh_mods_funs: &fresh_mods_funs,
    };
    for (index, file) in files.iter().enumerate() {
        next.files
            .insert(file.filename.clone(), entries.file(index, file));
    }
    next.mods_funs = next.files.values().map(|file| file.mods_funs).sum();
    let mods_funs = next.mods_funs;
    saver(&next);
    mods_funs
}

/// Merged vote counts for one project check: cached votes for reused
/// files plus freshly collected votes. Returns the merged counts with the
/// fresh files and details needed for emission. `None` fails open to a
/// full run (missing votes or unsupported check).
#[allow(
    clippy::too_many_arguments,
    reason = "one merge spine per project check"
)]
fn merged_project_counts(
    entry: &crate::CheckEntry,
    files: &[crate::RunnerFile],
    cache: &DiskCache,
    voting_reused: &[usize],
    voting_fresh: &[usize],
    fresh_valid: &[usize],
    fresh_prepared: &[crate::batch::Prepared<'_>],
) -> Option<(
    BTreeMap<String, usize>,
    Vec<crate::project::ProjectFile>,
    crate::project::StaleDetails,
)> {
    let mut merged: BTreeMap<String, usize> = BTreeMap::new();
    for index in voting_reused {
        // Missing entries count as empty votes: under a matching
        // fingerprint the file voted (or was never consulted for this
        // check), and empty votes contribute nothing to the majority.
        if let Some(counts) = cache
            .files
            .get(&files[*index].filename)
            .and_then(|cached| cached.project_counts.get(&entry.module))
        {
            for (kind, count) in counts {
                *merged.entry(kind.clone()).or_insert(0) += count;
            }
        }
    }
    // Fresh details via one shared parse per file.
    let fresh_project: Vec<crate::project::ProjectFile> = voting_fresh
        .iter()
        .map(|index| crate::project::ProjectFile {
            filename: files[*index].filename.clone(),
            source: files[*index].source.clone(),
        })
        .collect();
    let mut facts_refs: Vec<&crate::facts::Facts> = Vec::new();
    for global in voting_fresh {
        let slot = fresh_valid
            .iter()
            .position(|index: &usize| index == global)?;
        facts_refs.push(fresh_prepared[slot].facts());
    }
    let (fresh_counts, details) =
        crate::project::stale_collect(&entry.module, &fresh_project, &facts_refs)?;
    for (kind, count) in &fresh_counts {
        *merged.entry(kind.clone()).or_insert(0) += count;
    }
    Some((merged, fresh_project, details))
}

/// Voting global indices among reused and fresh files for one project
/// check, in input order.
fn voting_files(
    entry: &crate::CheckEntry,
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    reused: &[usize],
    fresh_valid: &[usize],
) -> (Vec<usize>, Vec<usize>) {
    let matcher = check_matcher(entry, runner_config);
    let mut voting_reused = Vec::new();
    for index in reused {
        if selected_for(&matcher, &files[*index].filename).unwrap_or(false) {
            voting_reused.push(*index);
        }
    }
    let mut voting_fresh = Vec::new();
    for index in fresh_valid {
        if selected_for(&matcher, &files[*index].filename).unwrap_or(false) {
            voting_fresh.push(*index);
        }
    }
    (voting_reused, voting_fresh)
}

/// Project-phase outputs for one incremental run.
struct ProjectPhase {
    new_winners: BTreeMap<String, Option<String>>,
    fresh_issues: Vec<crate::Issue>,
    drop_checks: BTreeSet<String>,
}

/// Project phase over every project-lane check: merged majorities plus
/// fresh-file issues. `None` fails open to a full run (flipped majority
/// or missing votes).
#[allow(
    clippy::too_many_arguments,
    reason = "one phase spine threading plan outputs through checks"
)]
fn project_phase(
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    cache: &DiskCache,
    plan: &Partition,
    fresh_valid: &[usize],
    fresh_prepared: &[crate::batch::Prepared<'_>],
) -> Option<ProjectPhase> {
    let mut new_winners: BTreeMap<String, Option<String>> = cache.winners.clone();
    let mut fresh_issues: Vec<crate::Issue> = Vec::new();
    let mut drop_checks: BTreeSet<String> = plan.pattern_stopped.clone();
    for entry in project_entries(runner_config) {
        if plan.pattern_stopped.contains(&entry.module) {
            new_winners.insert(entry.module.clone(), None);
            drop_checks.insert(entry.module.clone());
            continue;
        }
        match project_increment(
            entry,
            files,
            runner_config,
            cache,
            &plan.reused,
            fresh_valid,
            fresh_prepared,
        ) {
            ProjectIncrement::Same {
                winner,
                fresh_issues: fresh,
            } => {
                new_winners.insert(entry.module.clone(), winner);
                fresh_issues.extend(fresh);
            }
            ProjectIncrement::Flipped | ProjectIncrement::Unsupported => {
                return None;
            }
        }
    }
    Some(ProjectPhase {
        new_winners,
        fresh_issues,
        drop_checks,
    })
}

/// Winner stability gate: a flipped cached majority must fail open to a
/// full run rather than reuse issues blamed under the old winner.
fn winner_stable(cache: &DiskCache, module: &str, new_winner: Option<&String>) -> bool {
    cache
        .winners
        .get(module)
        .is_some_and(|old| old.as_ref() == new_winner)
}

/// Outcome of one project check's incremental step.
enum ProjectIncrement {
    /// Winner unchanged: fresh-file issues (built, filtered) to merge.
    Same {
        winner: Option<String>,
        fresh_issues: Vec<crate::Issue>,
    },
    /// Majority flipped, or no incremental support: fail open to full run.
    Flipped,
    /// Missing cached votes: fail open to full run.
    Unsupported,
}

/// Incremental step for one project check: merge cached and fresh counts,
/// compare winners, emit fresh-file issues under a stable winner.
#[allow(
    clippy::too_many_arguments,
    reason = "one merge spine; phases split below"
)]
fn project_increment(
    entry: &crate::CheckEntry,
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    cache: &DiskCache,
    reused: &[usize],
    fresh_valid: &[usize],
    fresh_prepared: &[crate::batch::Prepared<'_>],
) -> ProjectIncrement {
    // Voting files among all valid files (selection was validated upfront,
    // so no new pattern errors arise here).
    let (voting_reused, voting_fresh) =
        voting_files(entry, files, runner_config, reused, fresh_valid);
    // Merged counts over cached votes plus fresh details.
    let Some(merged) = merged_project_counts(
        entry,
        files,
        cache,
        &voting_reused,
        &voting_fresh,
        fresh_valid,
        fresh_prepared,
    ) else {
        return ProjectIncrement::Unsupported;
    };
    let (merged, fresh_project, details) = merged;
    let crate::project::StaleWinner::Known(new_winner) =
        crate::project::stale_winner(&entry.module, &merged, &entry.params)
    else {
        return ProjectIncrement::Unsupported;
    };
    if !winner_stable(cache, &entry.module, new_winner.as_ref()) {
        return ProjectIncrement::Flipped;
    }
    let Some(winner) = new_winner.clone() else {
        return ProjectIncrement::Same {
            winner: None,
            fresh_issues: Vec::new(),
        };
    };
    let Some(found) = crate::project::stale_emit(
        &entry.module,
        &fresh_project,
        &details,
        &winner,
        &entry.params,
    ) else {
        return ProjectIncrement::Unsupported;
    };
    let fresh_issues = build_fresh_project_issues(
        entry,
        files,
        runner_config,
        &voting_fresh,
        fresh_valid,
        fresh_prepared,
        &found,
    );
    ProjectIncrement::Same {
        winner: new_winner,
        fresh_issues,
    }
}

/// Build full post-filter issues for fresh voting files. Subset-relative
/// `found` indices align with `voting_fresh` order; filenames carry the
/// global identity downstream.
#[allow(
    clippy::too_many_arguments,
    reason = "one build spine for fresh project issues"
)]
fn build_fresh_project_issues(
    entry: &crate::CheckEntry,
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    voting_fresh: &[usize],
    fresh_valid: &[usize],
    fresh_prepared: &[crate::batch::Prepared<'_>],
    found: &[crate::project::ProjectIssue],
) -> Vec<crate::Issue> {
    let metas: Vec<crate::FileMeta<'_>> = voting_fresh
        .iter()
        .map(|global| {
            let slot = fresh_valid
                .iter()
                .position(|index| *index == *global)
                .unwrap_or(0);
            crate::FileMeta::collect_shared(
                &files[*global].filename,
                &files[*global].source,
                fresh_prepared[slot].facts(),
            )
        })
        .collect();
    let meta_refs: Vec<&crate::FileMeta<'_>> = metas.iter().collect();
    // Mirrors the pipeline: a build failure stages nothing for the check.
    let Ok(built) = crate::project::build_project_issues(
        &entry.module,
        found,
        &entry.params,
        &runner_config.general,
        &meta_refs,
    ) else {
        return Vec::new();
    };
    // Comments per fresh voting file for suppression.
    let comments: Vec<Vec<crate::suppression::ConfigComment>> = voting_fresh
        .iter()
        .map(|global| crate::suppression::config_comments(&files[*global].source))
        .collect();
    let mut fresh_issues = Vec::new();
    for (position, issue) in built {
        if issue.priority < runner_config.min_priority {
            continue;
        }
        let suppressed = comments.get(position).is_some_and(|list| {
            list.iter()
                .any(|comment| comment.ignores(&issue.check, issue.line_no.unwrap_or(0)))
        });
        if !suppressed {
            fresh_issues.push(issue);
        }
    }
    fresh_issues
}

/// Per-file vote counts for fresh files across every project check, keyed
/// by filename then check. Powers the cache save without re-parsing.
fn fresh_vote_counts(
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    fresh_valid: &[usize],
    fresh_prepared: &[crate::batch::Prepared<'_>],
) -> ProjectCountsByFile {
    let mut out = ProjectCountsByFile::new();
    for entry in project_entries(runner_config) {
        let matcher = check_matcher(entry, runner_config);
        let mut voting: Vec<usize> = Vec::new();
        let mut slots: Vec<usize> = Vec::new();
        for (slot, global) in fresh_valid.iter().enumerate() {
            if selected_for(&matcher, &files[*global].filename).unwrap_or(false) {
                voting.push(*global);
                slots.push(slot);
            }
        }
        if voting.is_empty() {
            continue;
        }
        let project_files: Vec<crate::project::ProjectFile> = voting
            .iter()
            .map(|global| crate::project::ProjectFile {
                filename: files[*global].filename.clone(),
                source: files[*global].source.clone(),
            })
            .collect();
        let refs: Vec<&crate::facts::Facts> = slots
            .iter()
            .map(|slot| fresh_prepared[*slot].facts())
            .collect();
        let Some((_, details)) =
            crate::project::stale_collect(&entry.module, &project_files, &refs)
        else {
            continue;
        };
        for (position, global) in voting.iter().enumerate() {
            let counts = per_file_counts(&details, position);
            if !counts.is_empty() {
                out.entry(files[*global].filename.clone())
                    .or_default()
                    .insert(entry.module.clone(), counts);
            }
        }
    }
    out
}

/// One file's counts out of fresh details (position is subset-relative).
/// Delegates to each collector's own merge accounting so the disk view can
/// never drift from the pipeline view.
fn per_file_counts(
    details: &crate::project::StaleDetails,
    position: usize,
) -> BTreeMap<String, usize> {
    use crate::project::StaleDetails as Details;
    match details {
        Details::Counts(per_file) => per_file.get(position).cloned().unwrap_or_default(),
        Details::ParamPattern(per_file) => per_file
            .get(position)
            .map(|found| crate::project::collect_param_pattern_counts(found))
            .unwrap_or_default(),
        Details::SpaceInParens(per_file) => per_file
            .get(position)
            .map(crate::project::collect_space_in_parens_counts)
            .unwrap_or_default(),
        Details::SpaceAroundOps(per_file) => per_file
            .get(position)
            .map(|votes| crate::project::collect_space_around_ops_counts(votes))
            .unwrap_or_default(),
        Details::ExceptionNames(per_file) => per_file
            .get(position)
            .map(|found| crate::project::collect_exception_names_counts(found))
            .unwrap_or_default(),
        Details::MultiAlias(per_file) => per_file
            .get(position)
            .map(|(stats, _)| stats.clone())
            .unwrap_or_default(),
        Details::UnusedVarNames(per_file) => per_file
            .get(position)
            .map(|found| crate::project::collect_unused_var_names_counts(found))
            .unwrap_or_default(),
    }
}

/// Global redundant-comment pass over the merged issue set. Text scans run
/// over every valid file; only files with findings pay for `FileMeta`.
fn redundant_globally(
    files: &[crate::RunnerFile],
    runner_config: &crate::RunnerConfig,
    issues: &[crate::Issue],
    errored_file: &BTreeSet<String>,
) -> Vec<crate::Issue> {
    let redundant: Vec<&crate::CheckEntry> = runner_config
        .checks
        .iter()
        .filter(|entry| {
            entry.module.as_str() == REDUNDANT
                && entry.enabled
                && runner_config.selection.should_run_entry(entry)
        })
        .collect();
    if redundant.is_empty() {
        return Vec::new();
    }
    let mut staged: BTreeMap<&str, Vec<(String, usize)>> = BTreeMap::new();
    for issue in issues {
        if let Some(line) = issue.line_no {
            staged
                .entry(issue.filename.as_str())
                .or_default()
                .push((issue.check.clone(), line));
        }
    }
    let mut out = Vec::new();
    for file in files {
        if errored_file.contains(file.filename.as_str()) {
            continue;
        }
        out.extend(redundant_for_file(file, runner_config, &staged, &redundant));
    }
    out
}

/// Redundant-comment issues for one file. Text scans are cheap; only
/// finding-bearing files pay for a parse via `FileMeta`.
fn redundant_for_file(
    file: &crate::RunnerFile,
    runner_config: &crate::RunnerConfig,
    staged: &BTreeMap<&str, Vec<(String, usize)>>,
    redundant: &[&crate::CheckEntry],
) -> Vec<crate::Issue> {
    let comments = crate::suppression::config_comments(&file.source);
    if comments.is_empty() {
        return Vec::new();
    }
    let file_issues = staged
        .get(file.filename.as_str())
        .cloned()
        .unwrap_or_default();
    let findings = crate::config_checks::redundant_comments(&file.source, true, &file_issues);
    if findings.is_empty() {
        return Vec::new();
    }
    // Only finding-bearing files pay for a parse.
    let prepared = crate::batch::Prepared::eager(&file.source);
    let meta = crate::FileMeta::collect_shared(&file.filename, &file.source, prepared.facts());
    let mut out = Vec::new();
    for entry in redundant {
        for finding in &findings {
            let kernel = crate::Finding {
                line: finding.line,
                column: finding.column,
                message: finding.message.clone(),
                trigger: crate::Trigger::Text(finding.trigger.clone()),
                severity: None,
            };
            if let Ok(issue) = crate::build_issue(
                REDUNDANT,
                kernel,
                &entry.params,
                &runner_config.general,
                &meta,
            ) {
                if issue.priority < runner_config.min_priority {
                    continue;
                }
                if comments
                    .iter()
                    .any(|comment| comment.ignores(&issue.check, issue.line_no.unwrap_or(0)))
                {
                    continue;
                }
                out.push(issue);
            }
        }
    }
    out
}

/// Relevant ordering mirroring the pipeline: check, filename, line, column.
fn sort_report(issues: &mut [crate::Issue]) {
    issues.sort_by(|left, right| {
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
    use std::path::PathBuf;

    const TWO_CHECKS: &str = "%{\n  configs: [\n    %{\n      name: \"default\",\n      files: %{included: [\"lib/\"]},\n      checks: %{enabled: [{Credo.Check.Readability.TrailingBlankLine, []}, {Credo.Check.Design.TagTODO, []}]}\n    }\n  ]\n}\n";

    const TABS_CHECK: &str = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Consistency.TabsOrSpaces, []}]}}]}\n";

    fn cache_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "qredo-stale-test-{}-{name}.json",
            std::process::id()
        ))
    }

    fn fresh_path(name: &str) -> PathBuf {
        let path = cache_path(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    fn files() -> Vec<crate::RunnerFile> {
        vec![
            crate::RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: "defmodule Example do\nend".to_owned(),
            },
            crate::RunnerFile {
                filename: "lib/b.ex".to_owned(),
                source: "# TODO: polish\ndefmodule B do\nend\n".to_owned(),
            },
        ]
    }

    fn tabs_files() -> Vec<crate::RunnerFile> {
        vec![
            crate::RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: "defmodule A do\n  def a, do: 1\nend\n".to_owned(),
            },
            crate::RunnerFile {
                filename: "lib/b.ex".to_owned(),
                source: "defmodule B do\n  def b, do: 2\nend\n".to_owned(),
            },
            crate::RunnerFile {
                filename: "lib/c.ex".to_owned(),
                source: "defmodule C do\n\tdef c, do: 3\nend\n".to_owned(),
            },
        ]
    }

    fn stale(
        config: &str,
        files: &[crate::RunnerFile],
        min_priority: i32,
        path: &Path,
    ) -> crate::RunReport {
        execute_stale_with_path(
            config,
            "default",
            files,
            min_priority,
            crate::Selection::default(),
            Path::new("/test-root"),
            Some(path),
        )
        .expect("served config runs")
    }

    fn fresh(config: &str, files: &[crate::RunnerFile], min_priority: i32) -> crate::RunReport {
        crate::integration::execute(config, "default", files, min_priority).expect("served")
    }

    fn runner_and_fingerprint(
        config_source: &str,
        min_priority: i32,
    ) -> (crate::RunnerConfig, String) {
        let config = crate::parse_config(config_source, "default").expect("config parses");
        let runner =
            crate::integration::runner_of(&config, min_priority, crate::Selection::default());
        let fingerprint = crate::stale_cache::fingerprint(
            config_source,
            "default",
            &config.env_snapshot,
            &runner.checks,
            &runner.selection,
            min_priority,
        );
        (runner, fingerprint)
    }

    #[test]
    fn stale_cold_run_matches_fresh() {
        let path = fresh_path("cold");
        let files = files();
        assert_eq!(
            stale(TWO_CHECKS, &files, -99, &path),
            fresh(TWO_CHECKS, &files, -99)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_hit_reuses_unchanged_files() {
        let path = fresh_path("hit");
        let files = files();
        let first = stale(TWO_CHECKS, &files, -99, &path);
        let second = stale(TWO_CHECKS, &files, -99, &path);
        assert_eq!(second, first);
        assert_eq!(second, fresh(TWO_CHECKS, &files, -99));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_respects_configured_check_tags() {
        let path = fresh_path("configured-tags");
        let config = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Readability.TrailingWhiteSpace, [tags: [:custom]]}]}}]}\n";
        let files = vec![crate::RunnerFile {
            filename: "lib/a.ex".to_owned(),
            source: "x = 1 \n".to_owned(),
        }];
        let selection = crate::Selection {
            checks_with_tag: vec!["custom".to_owned()],
            ..crate::Selection::default()
        };
        let report = execute_stale_with_path(
            config,
            "default",
            &files,
            -99,
            selection.clone(),
            Path::new("/test-root"),
            Some(&path),
        )
        .expect("configured tags serve");
        let fresh = crate::integration::execute_selected(config, "default", &files, -99, selection)
            .expect("fresh configured tags serve");
        assert_eq!(report, fresh);
        assert_eq!(report.issues.len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stale_hit_reuses_comment_validation_error() {
        let path = fresh_path("comment-error-hit");
        let files = vec![crate::RunnerFile {
            filename: "lib/a.ex".to_owned(),
            source: "# credo:disable-for-next-line /[/\ndefmodule A do\nend\n".to_owned(),
        }];
        let first = stale(TWO_CHECKS, &files, -99, &path);
        assert_eq!(first, fresh(TWO_CHECKS, &files, -99));
        let cache: crate::stale_cache::DiskCache =
            serde_json::from_slice(&std::fs::read(&path).expect("cache readable"))
                .expect("cache parses");
        assert!(
            cache.files["lib/a.ex"].comment_error.is_some(),
            "comment error must be hash-cached"
        );
        assert_eq!(stale(TWO_CHECKS, &files, -99, &path), first);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_changed_file_recomputes() {
        let path = fresh_path("changed");
        let files = files();
        let _ = stale(TWO_CHECKS, &files, -99, &path);
        let mut changed = files.clone();
        changed[0].source = "defmodule Example do\n  IO.inspect(x)\nend\n".to_owned();
        // Different config without IoInspect served here would fall back;
        // stay on the same served config and change TODO presence instead.
        changed[1].source = "defmodule B do\nend\n".to_owned();
        assert_eq!(
            stale(TWO_CHECKS, &changed, -99, &path),
            fresh(TWO_CHECKS, &changed, -99)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_project_winner_stable_matches_fresh() {
        let path = fresh_path("tabs-stable");
        let files = tabs_files();
        let _ = stale(TABS_CHECK, &files, -99, &path);
        // Edit the minority file without flipping the spaces majority.
        let mut changed = files.clone();
        changed[2].source = "defmodule C do\n\tdef c, do: 4\nend\n".to_owned();
        assert_eq!(
            stale(TABS_CHECK, &changed, -99, &path),
            fresh(TABS_CHECK, &changed, -99)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_project_winner_flip_matches_fresh() {
        let path = fresh_path("tabs-flip");
        let files = tabs_files();
        let _ = stale(TABS_CHECK, &files, -99, &path);
        // Flip the majority to tabs: two tabbed files outvote one spaces file.
        let mut flipped = files.clone();
        flipped[0].source = "defmodule A do\n\tdef a, do: 1\nend\n".to_owned();
        flipped[1].source = "defmodule B do\n\tdef b, do: 2\nend\n".to_owned();
        assert_eq!(
            stale(TABS_CHECK, &flipped, -99, &path),
            fresh(TABS_CHECK, &flipped, -99)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_priority_change_invalidates() {
        let path = fresh_path("priority");
        let files = files();
        let _ = stale(TWO_CHECKS, &files, -99, &path);
        assert_eq!(
            stale(TWO_CHECKS, &files, 0, &path),
            fresh(TWO_CHECKS, &files, 0)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_deleted_file_drops_issues() {
        let path = fresh_path("deleted");
        let files = files();
        let _ = stale(TWO_CHECKS, &files, -99, &path);
        let kept = files[..1].to_vec();
        assert_eq!(
            stale(TWO_CHECKS, &kept, -99, &path),
            fresh(TWO_CHECKS, &kept, -99)
        );
        let _ = std::fs::remove_file(&path);
    }

    fn tabs_entry() -> crate::CheckEntry {
        crate::CheckEntry {
            module: "Credo.Check.Consistency.TabsOrSpaces".to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        }
    }

    fn tabs_config(entry: &crate::CheckEntry) -> crate::RunnerConfig {
        crate::RunnerConfig {
            checks: vec![entry.clone()],
            files_included: Vec::new(),
            files_excluded: Vec::new(),
            selection: crate::Selection::default(),
            min_priority: -99,
            general: crate::GeneralParams::default(),
        }
    }

    fn empty_votes_cache(
        entry: &crate::CheckEntry,
        filename: &str,
        source: &str,
        winner: Option<String>,
    ) -> crate::stale_cache::DiskCache {
        let mut cached_files = BTreeMap::new();
        cached_files.insert(
            filename.to_owned(),
            crate::stale_cache::CachedFile {
                hash: crate::stale_cache::content_hash(source),
                project_counts: BTreeMap::new(),
                skipped_invalid: false,
                mods_funs: 0,
                comment_error: None,
            },
        );
        let mut winners = BTreeMap::new();
        winners.insert(entry.module.clone(), winner);
        let mut cache = crate::stale_cache::DiskCache::empty("test".to_owned());
        cache.winners = winners;
        cache.files = cached_files;
        cache
    }

    #[test]
    fn missing_votes_count_as_empty() {
        // `lib/a.ex` has no indented lines: it votes empty for
        // `TabsOrSpaces` and stores no counts entry. That must merge as
        // empty — not fail open to a full run.
        let files = vec![
            crate::RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: "x = 1\n".to_owned(),
            },
            crate::RunnerFile {
                filename: "lib/b.ex".to_owned(),
                source: "defmodule B do\n  def b, do: 2\nend\n".to_owned(),
            },
        ];
        let entry = tabs_entry();
        let runner_config = tabs_config(&entry);
        let cache = empty_votes_cache(
            &entry,
            "lib/a.ex",
            &files[0].source,
            Some("spaces".to_owned()),
        );
        let prepared_b = crate::batch::Prepared::eager(&files[1].source);
        let fresh_prepared = vec![prepared_b];
        match project_increment(
            &entry,
            &files,
            &runner_config,
            &cache,
            &[0],
            &[1],
            &fresh_prepared,
        ) {
            ProjectIncrement::Same {
                winner,
                fresh_issues,
            } => {
                assert_eq!(winner, Some("spaces".to_owned()));
                assert!(fresh_issues.is_empty());
            }
            ProjectIncrement::Flipped | ProjectIncrement::Unsupported => {
                panic!("missing votes must count as empty, not fail open");
            }
        }
    }

    #[test]
    fn clean_hit_does_not_save_unchanged_cache() {
        let files = files();
        let (runner, fingerprint) = runner_and_fingerprint(TWO_CHECKS, -99);
        let expected = crate::run_checks(&files, &runner);
        let cache = cold_cache(&files, &expected, &runner, &fingerprint);
        let issue_allocation = cache.issues.as_ptr();
        let saves = std::cell::Cell::new(0_usize);
        let actual = incremental(&files, &runner, cache, &fingerprint, &|_| {
            saves.set(saves.get() + 1);
        });
        assert_eq!(actual, expected);
        assert_eq!(
            actual.issues.as_ptr(),
            issue_allocation,
            "clean hit must move, not clone, cached issues"
        );
        assert_eq!(saves.get(), 0, "clean hit must not rewrite its cache");
    }

    #[test]
    fn stale_fallback_matches_fresh() {
        let path = fresh_path("fallback");
        let unknown = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Custom.NotARealCheck, []}]}}]}\n";
        assert_eq!(
            execute_stale_with_path(
                unknown,
                "default",
                &files(),
                -99,
                crate::Selection::default(),
                Path::new("/test-root"),
                Some(&path),
            )
            .expect_err("falls back"),
            crate::integration::execute(unknown, "default", &files(), -99).expect_err("falls back")
        );
        let _ = std::fs::remove_file(&path);
    }
}
