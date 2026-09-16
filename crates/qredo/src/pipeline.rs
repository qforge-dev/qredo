//! First vertical-slice pipeline for `TrailingBlankLine`.
//!
//! Combines the text kernel with filename binding, scope approximation,
//! general parameters, file selection, suppression and pinned-grammar syntax
//! validation. Scope-priority effects remain pending: scope bonus is `0`.
//! Invalid sources yield no issues with an explicit `Invalid` status, never
//! clean credit. The ledger pipeline status therefore stays `unsupported`
//! until native full-issue differential passes.

use std::collections::BTreeMap;

use crate::helpers;
use crate::issue::{BasePriority, Category, Issue, IssueTrigger};
use crate::selection::{ConfigSource, Resolution, Selection, resolve};
use crate::source::SourceSnapshot;
use crate::suppression::suppresses;
use crate::syntax::{SyntaxStatus, validate};

/// General parameters shared by all checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeneralParams {
    pub category: Option<Category>,
    pub exit_status: Option<i32>,
    pub priority: Option<i32>,
    pub files_included: Vec<String>,
    pub files_excluded: Vec<String>,
}

impl GeneralParams {
    /// Parse string params. Keys: `category`, `exit_status`, `priority`,
    /// `files.included`, `files.excluded` (comma-separated globs).
    #[must_use]
    pub fn from_map(params: &BTreeMap<String, String>) -> Self {
        let category = params.get("category").and_then(|v| match v.as_str() {
            "consistency" => Some(Category::Consistency),
            "design" => Some(Category::Design),
            "readability" => Some(Category::Readability),
            "refactor" => Some(Category::Refactor),
            "warning" => Some(Category::Warning),
            _ => None,
        });
        let exit_status = params
            .get("exit_status")
            .and_then(|v| v.parse::<i32>().ok());
        let priority = params.get("priority").and_then(|v| match v.as_str() {
            "higher" => Some(20),
            "high" => Some(10),
            "normal" => Some(1),
            "low" => Some(-10),
            "ignore" => Some(-100),
            _ => v.parse::<i32>().ok(),
        });
        let split_globs = |key: &str| {
            params.get(key).map_or_else(Vec::new, |v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
        };
        Self {
            category,
            exit_status,
            priority,
            files_included: split_globs("files.included"),
            files_excluded: split_globs("files.excluded"),
        }
    }

    #[must_use]
    pub fn file_selected(&self, filename: &str) -> bool {
        if !self.files_included.is_empty()
            && !self.files_included.iter().any(|pattern| {
                crate::file_select::wildcard_match(pattern, filename).unwrap_or(false)
            })
        {
            return false;
        }
        !self
            .files_excluded
            .iter()
            .any(|pattern| crate::file_select::wildcard_match(pattern, filename).unwrap_or(false))
    }
}

/// Full pipeline result with explicit syntax-validation boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineResult {
    pub issues: Vec<Issue>,
    /// Always `true`: the pinned grammar was consulted.
    pub syntax_validated: bool,
    /// Grammar acceptance; `Invalid` yields no issues and no clean credit.
    pub status: SyntaxStatus,
}

const CHECK: &str = "Elixir.Credo.Check.Readability.TrailingBlankLine";

/// Outcome of the selected pipeline: either a normal result or an explicit
/// non-run reason. Neither non-run variant grants clean credit.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectedOutcome {
    Ran(PipelineResult),
    FilteredOut,
    NeedsNativeConfig(String),
    InvalidSelection(String),
}

/// Run the `TrailingBlankLine` pipeline under execution selection and config.
#[must_use]
pub fn run_trailing_blank_line_selected(
    snapshot: &SourceSnapshot,
    general: &GeneralParams,
    selection: &Selection,
    config: &ConfigSource,
) -> SelectedOutcome {
    const RULE: &str = "Credo.Check.Readability.TrailingBlankLine";
    match resolve(RULE, selection, config) {
        Resolution::Run => SelectedOutcome::Ran(run_trailing_blank_line(snapshot, general)),
        Resolution::FilteredOut => SelectedOutcome::FilteredOut,
        Resolution::NeedsNativeConfig(path) => SelectedOutcome::NeedsNativeConfig(path),
        Resolution::InvalidSelection(pattern) => SelectedOutcome::InvalidSelection(pattern),
    }
}

/// Run the `TrailingBlankLine` pipeline on a snapshot.
#[must_use]
pub fn run_trailing_blank_line(
    snapshot: &SourceSnapshot,
    general: &GeneralParams,
) -> PipelineResult {
    let status = validate(snapshot.content());
    if status == SyntaxStatus::Invalid {
        // Normal pipeline filters invalid files; direct kernels still report.
        // Empty here is not clean credit: `status` distinguishes it.
        return PipelineResult {
            issues: Vec::new(),
            syntax_validated: true,
            status,
        };
    }
    if !general.file_selected(snapshot.filename()) {
        return PipelineResult {
            issues: Vec::new(),
            syntax_validated: true,
            status,
        };
    }
    let findings = crate::trailing_blank_line::check(snapshot.content());
    let mut issues = Vec::new();
    for finding in findings {
        let scope = scope_for(snapshot.content(), finding.line);
        let category = general.category.unwrap_or(Category::Readability);
        let exit_status = general
            .exit_status
            .unwrap_or_else(|| category.default_exit_status());
        // A general `priority` overrides the base only; the scope bonus still
        // applies, matching `Credo.Check.format_issue`.
        let base = general
            .priority
            .unwrap_or_else(|| BasePriority::Low.to_integer());
        let priority = base + scope_bonus(snapshot.content(), &scope);
        let issue = Issue {
            check: CHECK.to_owned(),
            category,
            priority,
            severity: 1.0,
            message: finding.message,
            filename: snapshot.filename().to_owned(),
            line_no: Some(finding.line),
            column: finding.column,
            exit_status,
            trigger: IssueTrigger::NoTrigger,
            scope: Some(scope),
        };
        if !suppresses(snapshot.content(), &issue) {
            issues.push(issue);
        }
    }
    PipelineResult {
        issues,
        syntax_validated: true,
        status,
    }
}

/// Approximate enclosing scope: last `defmodule` plus last `def` before `line`.
/// Returns `""` for top-level code, matching Credo's empty scope.
fn scope_for(content: &str, line: usize) -> String {
    let mut module: Option<String> = None;
    let mut fun: Option<String> = None;
    for (line_no, _, name) in helpers::module_names(content) {
        if line_no <= line {
            module = Some(name);
        }
    }
    for (line_no, _, name) in helpers::def_names(content) {
        if line_no <= line {
            fun = Some(name.trim_end_matches(['?', '!']).to_owned());
        }
    }
    match (module, fun) {
        (Some(module), Some(fun)) => format!("{module}.{fun}"),
        (Some(module), None) => module,
        (None, Some(fun)) => fun,
        (None, None) => String::new(),
    }
}

/// Scope bonus mirroring `Credo.Priority`: modules contribute 1 (<5 defs) or
/// 2; functions add a parameter-count bonus (0/1/2/3) on top of their module.
fn scope_bonus(content: &str, scope: &str) -> i32 {
    if scope.is_empty() {
        return 0;
    }
    let def_count = helpers::def_names(content).len();
    let module_bonus = if def_count >= 5 { 2 } else { 1 };
    let last = scope.rsplit('.').next().unwrap_or(scope);
    let is_function = last.starts_with(|c: char| c.is_ascii_lowercase() || c == '_');
    if !is_function {
        return module_bonus;
    }
    module_bonus + param_bonus(content, last)
}

/// Parameter-count bonus for the last definition named `fun`.
fn param_bonus(content: &str, fun: &str) -> i32 {
    let masked = helpers::mask_strings_comments(content);
    let mut arity = 0_usize;
    for line in masked.split('\n') {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("def ") || trimmed.starts_with("defp ")) {
            continue;
        }
        if !line.contains(fun) {
            continue;
        }
        if let Some(open) = line.find('(') {
            if let Some(close) = line.find(')') {
                let inside = line[open + 1..close].trim();
                arity = if inside.is_empty() {
                    0
                } else {
                    inside.matches(',').count() + 1
                };
            }
        } else {
            arity = 0;
        }
    }
    match arity {
        0 => 0,
        1 | 2 => 1,
        3 | 4 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(content: &str) -> SourceSnapshot {
        SourceSnapshot::parse(content, "lib/a.ex")
    }

    #[test]
    fn glob_double_star_spans_zero_directories() {
        use crate::file_select::wildcard_match;
        let run = |pattern: &str, path: &str| wildcard_match(pattern, path).expect("valid");
        assert!(run("test/**/*_test.ex", "test/a_test.ex"));
        assert!(run("test/**/*_test.ex", "test/foo/a_test.ex"));
        assert!(!run("test/**/*_test.ex", "test/a_test.exs"));
        assert!(!run("test/**/*_test.ex", "other/a_test.ex"));
    }

    #[test]
    fn glob_star_stays_within_segments() {
        use crate::file_select::wildcard_match;
        let run = |pattern: &str, path: &str| wildcard_match(pattern, path).expect("valid");
        assert!(run("test/*.ex", "test/a.ex"));
        assert!(!run("test/*.ex", "test/foo/a.ex"));
        assert!(run("lib/my_module.ex", "lib/my_module.ex"));
    }

    #[test]
    fn clean_file_has_no_issues() {
        let result = run_trailing_blank_line(
            &snapshot("defmodule M do\nend\n"),
            &GeneralParams::default(),
        );
        assert!(result.issues.is_empty());
        assert!(result.syntax_validated);
        assert_eq!(result.status, SyntaxStatus::Valid);
    }

    #[test]
    fn violation_carries_full_issue_shape() {
        let result =
            run_trailing_blank_line(&snapshot("defmodule M do\nend"), &GeneralParams::default());
        assert_eq!(result.issues.len(), 1);
        let issue = &result.issues[0];
        assert_eq!(issue.check, CHECK);
        assert_eq!(issue.category, Category::Readability);
        // Base low (-10) plus module scope bonus (+1).
        assert_eq!(issue.priority, -9);
        assert_eq!(issue.exit_status, 4);
        assert_eq!(issue.filename, "lib/a.ex");
        assert_eq!(issue.line_no, Some(2));
        assert_eq!(issue.column, None);
        assert_eq!(issue.trigger, IssueTrigger::NoTrigger);
        assert_eq!(issue.scope.as_deref(), Some("M"));
    }

    #[test]
    fn function_scope_carries_param_bonus() {
        let src = "defmodule M do\n  def foo(a, b, c, d, e), do: a\nend";
        let result = run_trailing_blank_line(&snapshot(src), &GeneralParams::default());
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].scope.as_deref(), Some("M.foo"));
        // Base low (-10) plus module (+1) plus five params (+3).
        assert_eq!(result.issues[0].priority, -6);
    }

    #[test]
    fn suppression_removes_issue() {
        let content = "# credo:disable-for-this-file\ndefmodule M do\nend";
        let result = run_trailing_blank_line(&snapshot(content), &GeneralParams::default());
        assert!(result.issues.is_empty());
    }

    #[test]
    fn excluded_file_is_not_selected() {
        let mut map = BTreeMap::new();
        map.insert("files.excluded".to_owned(), "lib/a.ex".to_owned());
        let general = GeneralParams::from_map(&map);
        let result = run_trailing_blank_line(&snapshot("defmodule M do\nend"), &general);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn general_overrides_apply() {
        let mut map = BTreeMap::new();
        map.insert("category".to_owned(), "warning".to_owned());
        map.insert("priority".to_owned(), "high".to_owned());
        map.insert("exit_status".to_owned(), "16".to_owned());
        let general = GeneralParams::from_map(&map);
        let result = run_trailing_blank_line(&snapshot("defmodule M do\nend"), &general);
        assert_eq!(result.issues[0].category, Category::Warning);
        // Base high (10) plus module scope bonus (+1), matching native.
        assert_eq!(result.issues[0].priority, 11);
        assert_eq!(result.issues[0].exit_status, 16);
    }

    #[test]
    fn numeric_priority_keeps_scope_bonus() {
        let mut map = BTreeMap::new();
        map.insert("priority".to_owned(), "42".to_owned());
        let general = GeneralParams::from_map(&map);
        let result = run_trailing_blank_line(&snapshot("defmodule M do\nend"), &general);
        assert_eq!(result.issues[0].priority, 43);
    }

    #[test]
    fn selected_only_match_runs() {
        use crate::selection::{ConfigSource, Selection};
        let selection = Selection {
            only: vec!["Credo.Check.Readability.TrailingBlankLine".to_owned()],
            ignore: Vec::new(),
        };
        let outcome = run_trailing_blank_line_selected(
            &snapshot("defmodule M do\nend"),
            &GeneralParams::default(),
            &selection,
            &ConfigSource::Default,
        );
        assert!(matches!(outcome, SelectedOutcome::Ran(_)));
    }

    #[test]
    fn selected_only_miss_filters_out() {
        use crate::selection::{ConfigSource, Selection};
        let selection = Selection {
            only: vec!["Credo.Check.Warning.IoInspect".to_owned()],
            ignore: Vec::new(),
        };
        let outcome = run_trailing_blank_line_selected(
            &snapshot("defmodule M do\nend"),
            &GeneralParams::default(),
            &selection,
            &ConfigSource::Default,
        );
        assert_eq!(outcome, SelectedOutcome::FilteredOut);
    }

    #[test]
    fn selected_executable_config_needs_native() {
        let outcome = run_trailing_blank_line_selected(
            &snapshot("defmodule M do\nend"),
            &GeneralParams::default(),
            &Selection::default(),
            &ConfigSource::ExecutableFile(".credo.exs".to_owned()),
        );
        assert_eq!(
            outcome,
            SelectedOutcome::NeedsNativeConfig(".credo.exs".to_owned())
        );
    }

    #[test]
    fn selected_invalid_pattern_is_explicit() {
        let selection = Selection {
            only: vec!["([".to_owned()],
            ignore: Vec::new(),
        };
        let outcome = run_trailing_blank_line_selected(
            &snapshot("defmodule M do\nend"),
            &GeneralParams::default(),
            &selection,
            &ConfigSource::Default,
        );
        assert_eq!(outcome, SelectedOutcome::InvalidSelection("([".to_owned()));
    }

    #[test]
    fn invalid_source_yields_no_issues_without_clean_credit() {
        let result = run_trailing_blank_line(&snapshot("def foo( do\n"), &GeneralParams::default());
        assert!(result.issues.is_empty());
        assert!(result.syntax_validated);
        assert_eq!(result.status, SyntaxStatus::Invalid);
    }

    #[test]
    fn bare_cr_is_invalid_without_clean_credit() {
        // Direct kernel still reports (bypasses pipeline); pipeline filters.
        assert_eq!(crate::trailing_blank_line::check("x = 1\r").len(), 1);
        let result = run_trailing_blank_line(&snapshot("x = 1\r"), &GeneralParams::default());
        assert!(result.issues.is_empty());
        assert_eq!(result.status, SyntaxStatus::Invalid);
    }
}
