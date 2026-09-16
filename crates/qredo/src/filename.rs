//! Filename-aware evaluation for checks the single-source kernel cannot decide.
//!
//! Some checks read the filename (test-file extensions, path exclusions) or
//! only run on selected paths. These runners take the file identity plus the
//! source and return project issues for a single file; anything undecidable
//! stays an explicit empty with documented reason, never clean credit by
//! confusion. Path-list params arrive in the corpus encoding (strings and
//! `{"regex": ...}` as compact JSON).

use std::collections::BTreeMap;

use crate::batch::Prepared;
use crate::pipeline::GeneralParams;
use crate::project::{ProjectFile, ProjectIssue};

/// A check without a filename-aware implementation.
#[derive(Debug, PartialEq, Eq)]
pub struct UnsupportedRule(pub String);

/// Run a filename-aware check on one file.
///
/// # Errors
/// Returns `UnsupportedRule` for checks without an implementation.
pub fn run_filename_check(
    rule: &str,
    file: &ProjectFile,
    params: &BTreeMap<String, String>,
) -> Result<Vec<ProjectIssue>, UnsupportedRule> {
    run_filename_check_prepared(rule, &file.filename, &Prepared::lazy(&file.source), params)
}

/// Run a filename-aware check over shared prepare-phase facts.
///
/// The pipeline passes its per-file `Prepared` (one parse per file);
/// single-file callers use [`run_filename_check`], which parses lazily
/// with identical results (see `prepared_lane_matches_owned_lane`).
///
/// # Errors
/// Returns `UnsupportedRule` for checks without an implementation.
pub(crate) fn run_filename_check_prepared(
    rule: &str,
    filename: &str,
    prepared: &Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Result<Vec<ProjectIssue>, UnsupportedRule> {
    match rule {
        "Credo.Check.Design.SkipTestWithoutComment" => Ok(skip_test(prepared)),
        "Credo.Check.Readability.ModuleDoc" => Ok(module_doc(filename, prepared, params)),
        "Credo.Check.Refactor.ModuleDependencies" => Ok(module_deps(filename, prepared, params)),
        "Credo.Check.Warning.MixEnv" => Ok(mix_env(filename, prepared, params)),
        "Credo.Check.Warning.WrongTestFileExtension" => Ok(wrong_ext(filename, params)),
        "Credo.Check.Warning.WrongTestFilename" => Ok(wrong_name(filename, prepared)),
        _ => Err(UnsupportedRule(rule.to_owned())),
    }
}

/// A path matcher: substring on the directory or a regex.
enum PathMatcher {
    Prefix(String),
    Pattern(String),
}

/// Parse a path list param (compact JSON array of strings/`{"regex": ...}`).
fn parse_path_list(raw: Option<&String>) -> Vec<PathMatcher> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            serde_json::Value::String(path) => {
                Some(PathMatcher::Prefix(path.trim_start_matches(':').to_owned()))
            }
            serde_json::Value::Object(_) => item
                .get("regex")
                .and_then(serde_json::Value::as_str)
                .map(|pattern| PathMatcher::Pattern(pattern.to_owned())),
            _ => None,
        })
        .collect()
}

/// Directory part of a filename (`""` when there is none).
fn dirname(filename: &str) -> &str {
    filename.rfind('/').map_or("", |at| &filename[..at])
}

/// Extension including the dot (`".exs"`, `""` when there is none).
fn extname(filename: &str) -> &str {
    let base = filename.rsplit('/').next().unwrap_or(filename);
    base.rfind('.').map_or("", |at| &base[at..])
}

/// True when the file's directory matches any exclusion.
fn ignore_path(filename: &str, excluded: &[PathMatcher]) -> bool {
    let directory = dirname(filename);
    excluded.iter().any(|matcher| match matcher {
        PathMatcher::Prefix(prefix) => directory.starts_with(prefix.as_str()),
        PathMatcher::Pattern(pattern) => {
            regex::Regex::new(pattern).is_ok_and(|regex| regex.is_match(directory))
        }
    })
}

/// `EX2003`: upstream `run/2` ignores the filename (execution filters by
/// `files`); the recorded filename is context for the gate.
fn skip_test(prepared: &Prepared<'_>) -> Vec<ProjectIssue> {
    find_to_project(crate::design::skip_test::check_prepared(prepared))
}

/// `EX3009`: `.exs` files are exempt, otherwise the full kernel.
fn module_doc(
    filename: &str,
    prepared: &Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    if extname(filename) == ".exs" {
        return Vec::new();
    }
    find_to_project(crate::readability::module_doc::check_prepared(
        prepared, params,
    ))
}

/// `EX4017`: excluded paths (default `[/test/, test]`) are exempt.
fn module_deps(
    filename: &str,
    prepared: &Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    let excluded = match params.get("excluded_paths") {
        Some(raw) => parse_path_list(Some(raw)),
        None => vec![
            PathMatcher::Pattern("/test/".to_owned()),
            PathMatcher::Prefix("test".to_owned()),
        ],
    };
    if ignore_path(filename, &excluded) {
        return Vec::new();
    }
    find_to_project(crate::refactor::module_deps::check_prepared(
        prepared, params,
    ))
}

/// `EX5010`: excluded paths and `.exs` files are exempt.
fn mix_env(
    filename: &str,
    prepared: &Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    if ignore_path(filename, &parse_path_list(params.get("excluded_paths"))) {
        return Vec::new();
    }
    if extname(filename) == ".exs" {
        return Vec::new();
    }
    find_to_project(crate::warning::mix_env::check_prepared(prepared))
}

/// `EX5025`: files selected by `files.included` get the one-line issue.
fn wrong_ext(filename: &str, params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let general = GeneralParams::from_map(params);
    let (included, excluded) =
        if general.files_included.is_empty() && general.files_excluded.is_empty() {
            (
                vec![
                    "test/**/*_test.ex".to_owned(),
                    "apps/**/test/**/*_test.ex".to_owned(),
                ],
                Vec::new(),
            )
        } else {
            (
                general.files_included.clone(),
                general.files_excluded.clone(),
            )
        };
    let selected = (included.is_empty()
        || included
            .iter()
            .any(|pattern| crate::file_select::wildcard_match(pattern, filename).unwrap_or(false)))
        && !excluded
            .iter()
            .any(|pattern| crate::file_select::wildcard_match(pattern, filename).unwrap_or(false));
    if !selected {
        return Vec::new();
    }
    vec![ProjectIssue {
        file: 0,
        line: Some(1),
        column: None,
        trigger: "no_trigger".to_owned(),
        message: "Test files should end with `_test.exs`.".to_owned(),
        severity: None,
    }]
}

/// `EX5030`: `*_test.exs` files are exempt, otherwise scan `use XCase`.
fn wrong_name(filename: &str, prepared: &Prepared<'_>) -> Vec<ProjectIssue> {
    if filename.ends_with("_test.exs") {
        return Vec::new();
    }
    find_to_project(crate::warning::wrong_name::check_prepared(prepared))
}

/// Findings (`file` always 0 here) into single-file project issues.
fn find_to_project(findings: Vec<crate::Finding>) -> Vec<ProjectIssue> {
    findings
        .into_iter()
        .map(|finding| ProjectIssue {
            file: 0,
            line: Some(finding.line),
            column: finding.column,
            trigger: match finding.trigger {
                crate::Trigger::NoTrigger => "no_trigger".to_owned(),
                crate::Trigger::Text(text) => text,
            },
            message: finding.message,
            severity: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectFile;

    fn project_file(filename: &str, source: &str) -> ProjectFile {
        ProjectFile {
            filename: filename.to_owned(),
            source: source.to_owned(),
        }
    }

    fn params() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn skip_test_reports_without_comment() {
        let file = project_file("foo_test.exs", "@tag :skip\ntest \"x\", do: 1\n");
        assert_eq!(
            run_filename_check(
                "Credo.Check.Design.SkipTestWithoutComment",
                &file,
                &params()
            )
            .unwrap()
            .len(),
            1
        );
    }

    #[test]
    fn module_doc_exempts_exs() {
        let file = project_file("a.exs", "defmodule M do\nend\n");
        assert!(
            run_filename_check("Credo.Check.Readability.ModuleDoc", &file, &params())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn module_deps_exempts_test_paths() {
        let file = project_file("test/a.ex", "defmodule M do\n  alias A\nend\n");
        assert!(
            run_filename_check("Credo.Check.Refactor.ModuleDependencies", &file, &params())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn mix_env_exempts_exs_and_excluded() {
        let exs = project_file("a.exs", "defmodule M do\n  def f, do: Mix.env()\nend\n");
        assert!(
            run_filename_check("Credo.Check.Warning.MixEnv", &exs, &params())
                .unwrap()
                .is_empty()
        );
        let mut excluded = BTreeMap::new();
        excluded.insert("excluded_paths".to_owned(), "[\"foo\"]".to_owned());
        let other = project_file("foo/a.ex", "defmodule M do\n  def f, do: Mix.env()\nend\n");
        assert!(
            run_filename_check("Credo.Check.Warning.MixEnv", &other, &excluded)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn wrong_ext_reports_selected() {
        let file = project_file("test/a_test.ex", "x = 1\n");
        let issues = run_filename_check(
            "Credo.Check.Warning.WrongTestFileExtension",
            &file,
            &params(),
        )
        .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, Some(1));
    }

    #[test]
    fn wrong_ext_matches_native_issue_shape() {
        // EX5025.native-shape: native `run/2` always reports line 1,
        // no trigger, "Test files should end with `_test.exs`."; the
        // filename runner only runs it on `files.included` selection.
        let selected = project_file("test/a_test.ex", "x = 1\n");
        let issues = run_filename_check(
            "Credo.Check.Warning.WrongTestFileExtension",
            &selected,
            &params(),
        )
        .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, Some(1));
        assert_eq!(issues[0].column, None);
        assert_eq!(issues[0].trigger, "no_trigger");
        assert_eq!(issues[0].message, "Test files should end with `_test.exs`.");
        let unselected = project_file("lib/a.ex", "x = 1\n");
        assert!(
            run_filename_check(
                "Credo.Check.Warning.WrongTestFileExtension",
                &unselected,
                &params(),
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn wrong_name_exempts_test_files() {
        let file = project_file(
            "test/a_test.exs",
            "defmodule M do\n  use ExUnit.Case\nend\n",
        );
        assert!(
            run_filename_check("Credo.Check.Warning.WrongTestFilename", &file, &params())
                .unwrap()
                .is_empty()
        );
        let other = project_file("lib/a.ex", "defmodule M do\n  use ExUnit.Case\nend\n");
        assert_eq!(
            run_filename_check("Credo.Check.Warning.WrongTestFilename", &other, &params())
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn unknown_rule_errors() {
        let file = project_file("a.ex", "x = 1\n");
        assert!(run_filename_check("Credo.Check.Nope", &file, &params()).is_err());
    }

    #[test]
    fn prepared_lane_matches_owned_lane() {
        // The pipeline shares its prepare-phase parse; single-file calls
        // parse lazily. Both must report identical issues.
        let cases = [
            (
                "lib/a.ex",
                "defmodule A do\n  @moduledoc \"Hi\"\n  def f, do: Mix.env()\nend\n",
            ),
            (
                "test/a_test.exs",
                "defmodule ATest do\n  use ExUnit.Case\nend\n",
            ),
            ("test/plain.exs", "x = 1\n"),
            ("lib/broken.ex", "def foo( do\n"),
        ];
        let rules = [
            "Credo.Check.Design.SkipTestWithoutComment",
            "Credo.Check.Readability.ModuleDoc",
            "Credo.Check.Refactor.ModuleDependencies",
            "Credo.Check.Warning.MixEnv",
            "Credo.Check.Warning.WrongTestFileExtension",
            "Credo.Check.Warning.WrongTestFilename",
        ];
        for (filename, source) in cases {
            let file = project_file(filename, source);
            let tree = crate::ts_parser::parse(source);
            let prepared = crate::batch::Prepared::with_tree(source, tree);
            for rule in rules {
                let owned =
                    run_filename_check(rule, &file, &params()).expect("known rule dispatches");
                let shared = run_filename_check_prepared(rule, filename, &prepared, &params())
                    .expect("known rule dispatches");
                assert_eq!(owned, shared, "{rule} on {filename}");
            }
        }
    }
}
