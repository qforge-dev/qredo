//! Native Elixir linting with Credo-compatible behavior.
//!
//! The main entry points are [`check_kernel`] for single-rule evaluation
//! and [`integration::execute`] for full-pipeline runs over a project
//! config (what the `qredo` binary uses). See `ARCHITECTURE.md` for the
//! pipeline and facts design.

use std::collections::BTreeMap;

pub use batch::{FileOutcome, RuleOutcome, check_all_kernels, check_sources_parallel};
use batch::{Prepared, run_one};
pub use check_meta::{
    CheckBase, FileMeta, IssueError, PriorityError, backfill_column, base_priority, build_issue,
    category_for, check_tags, resolve_category, resolve_exit_status, resolve_priority,
    runs_at_min_priority, severity, version_skipped_on_pinned_toolchain,
};
pub use config_file::{CheckEntry, CredoConfig, FileEntry, UnsupportedConfig, parse_config};
pub use file_select::{PatternError, check_runs_on_entries, check_runs_on_file, wildcard_match};
mod batch;
pub mod check_docs;
mod check_meta;
pub mod cmd_diff;
mod config_data;
mod config_file;
mod consistency;
mod design;
mod facts;
mod file_select;
mod filename;
pub mod format_default;
pub mod format_machine;
mod helpers;
pub mod integration;
mod issue;
mod pipeline;
mod project;
mod readability;
mod refactor;
mod runner;
mod scope;
mod selection;
mod source;
mod suppression;
mod syntax;
mod trailing_blank_line;
mod ts_parser;
mod warning;

pub use filename::run_filename_check;
pub use issue::{BasePriority, Category, Issue, IssueTrigger};
pub use scope::{Scopes, scope_for};
pub mod config_checks;
pub use pipeline::{
    GeneralParams, PipelineResult, SelectedOutcome, run_trailing_blank_line,
    run_trailing_blank_line_selected,
};
pub use project::{ProjectFile, ProjectIssue, run_project_check};
pub use runner::{
    RunError, RunReport, RunnerConfig, RunnerFile, promoted_project_check, run_checks,
    supports_per_file,
};
pub use selection::{ConfigSource, Resolution, Selection, resolve};
pub use source::SourceSnapshot;
pub use syntax::{SyntaxStatus, is_valid, validate};

/// A rule-local finding, before Credo scope, priority and filtering processing.
///
/// Kernels emit findings in ascending `(line, column)` order. Native `run/2`
/// may accumulate in reverse; Credo sorts by `{filename, line_no, column}`
/// before presentation (`cli/output/formatter/oneline.ex`,
/// `cli/task/set_relevant_issues.ex`).
#[derive(Debug, PartialEq, Clone)]
pub struct Finding {
    pub line: usize,
    pub column: Option<usize>,
    pub message: String,
    pub trigger: Trigger,
    /// `Severity.compute(actual, max)` for the checks that pass it
    /// explicitly (complexity/size/arity/nesting/duplication); `None`
    /// means the upstream default of `1`.
    pub severity: Option<f64>,
}

impl Finding {
    #[must_use]
    pub fn no_trigger(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            column: None,
            message: message.into(),
            trigger: Trigger::NoTrigger,
            severity: None,
        }
    }

    #[must_use]
    pub fn with_trigger(
        line: usize,
        column: Option<usize>,
        message: impl Into<String>,
        trigger: impl Into<String>,
    ) -> Self {
        Self {
            line,
            column,
            message: message.into(),
            trigger: Trigger::Text(trigger.into()),
            severity: None,
        }
    }

    /// Attach an explicit `Severity.compute` value, mirroring checks that
    /// pass `severity:` to `format_issue`.
    #[must_use]
    pub fn with_severity(mut self, severity: f64) -> Self {
        self.severity = Some(severity);
        self
    }
}

/// Preserve Credo's explicit no-trigger sentinel separately from missing data.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Trigger {
    NoTrigger,
    Text(String),
}

/// The requested rule has no implemented kernel.
#[derive(Debug, PartialEq, Eq)]
pub struct UnsupportedRule(pub String);

/// Evaluate only a rule's source logic with default parameters.
///
/// # Errors
/// Returns `UnsupportedRule` for unknown rule IDs.
pub fn check_kernel(rule: &str, source: &str) -> Result<Vec<Finding>, UnsupportedRule> {
    check_kernel_with_params(rule, source, &BTreeMap::new())
}

/// Evaluate a rule's source logic with explicit string parameters.
///
/// Parameter keys/values use their Elixir literal rendering (for example
/// `"true"`, `"120"`, `"unix"`). Missing keys use Credo defaults.
///
/// # Errors
/// Returns `UnsupportedRule` for unknown rule IDs.
pub fn check_kernel_with_params(
    rule: &str,
    source: &str,
    params: &BTreeMap<String, String>,
) -> Result<Vec<Finding>, UnsupportedRule> {
    run_one(rule, &Prepared::lazy(source), params)
}
