//! Swappable native lint engine with fail-closed fallback.
//!
//! [`select`] serves static `.credo.exs` configs whose enabled checks and
//! parameter schemas are implemented. Anything else reports a stable
//! machine-readable reason instead of running. Issue presence and locations
//! must match on served configs; message text and severities are
//! non-contractual. Only differential failures on real targets authorize
//! expanding the served set.

/// How a caller should satisfy lint for one static config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Run the native pipeline over these enabled checks in config order.
    Serve {
        /// Enabled check modules in config order.
        checks: Vec<String>,
    },
    /// Run real Credo and record this reason; never silent divergence.
    Fallback {
        /// Stable machine-readable reason (`unsupported-credo-config:*`,
        /// `unsupported-check:*`, `needs-validated-config:*`,
        /// `project-scope-check:*`, `custom-check-params:*`),
        /// plus execution-time `native-pipeline-errors`).
        reason: String,
    },
}

/// Fail-closed fallback carrying the stable recorded reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    /// `unsupported-credo-config:*`, `unsupported-check:*`,
    /// `needs-validated-config:*`, `project-scope-check:*` or
    /// `custom-check-params:*`; callers record this and run real Credo.
    pub reason: String,
}

/// Fail-closed gate for one enabled check in a full run; `None` means
/// natively servable. Promoted project-lane checks serve here (differential
/// proof on file); subset runs use [`subset_gate`] and stay fail-closed.
fn gate(entry: &crate::CheckEntry) -> Option<Fallback> {
    if crate::check_kernel(&entry.module, "defmodule Probe do end").is_err() {
        return Some(Fallback {
            reason: format!("unsupported-check:{}", entry.module),
        });
    }
    if matches!(
        entry.module.as_str(),
        "Credo.Check.Design.MissingCheckInConfig" | "Credo.Check.Design.DeprecatedChecksConfig"
    ) {
        return Some(Fallback {
            reason: format!("needs-validated-config:{}", entry.module),
        });
    }
    if !crate::supports_per_file(&entry.module) && !crate::promoted_project_check(&entry.module) {
        return Some(Fallback {
            reason: format!("project-scope-check:{}", entry.module),
        });
    }
    if !params_supported(entry) {
        return Some(Fallback {
            reason: format!("custom-check-params:{}", entry.module),
        });
    }
    None
}

/// True when every configured parameter is consumed with the same scalar
/// shape as the native implementation. Anything not explicitly promoted
/// remains on the real-Credo path rather than silently running defaults.
fn params_supported(entry: &crate::CheckEntry) -> bool {
    entry
        .params
        .iter()
        .all(|(name, value)| match name.as_str() {
            "priority" => crate::resolve_priority(&entry.module, None, &entry.params).is_ok(),
            "exit_status" => valid_exit_status(value),
            "category" => valid_category(value),
            // The static parser has already required lists of plain strings and
            // flattened them to the comma-separated form consumed by file_select.
            "files.included" | "files.excluded" => true,
            _ => supported_check_param(&entry.module, name, value),
        })
}

/// Per-check schemas for the first real-project parameter slice.
fn supported_check_param(module: &str, name: &str, value: &str) -> bool {
    let nonnegative_integer = || value.parse::<usize>().is_ok();
    match (module, name) {
        (
            "Credo.Check.Design.AliasUsage",
            "if_nested_deeper_than" | "if_called_more_often_than",
        )
        | ("Credo.Check.Refactor.CyclomaticComplexity", "max_complexity")
        | ("Credo.Check.Refactor.FunctionArity", "max_arity")
        | ("Credo.Check.Refactor.Nesting", "max_nesting") => nonnegative_integer(),
        _ => false,
    }
}

/// Configured categories qredo can represent without changing issue shape.
fn valid_category(value: &str) -> bool {
    matches!(
        value,
        "consistency" | "design" | "readability" | "refactor" | "warning"
    )
}

/// Native accepts integer statuses or category atoms. Unknown atoms map to
/// zero upstream, but are rejected here so a typo cannot silently change CI.
fn valid_exit_status(value: &str) -> bool {
    value.parse::<i32>().is_ok() || valid_category(value)
}

/// Fail-closed gate for one enabled check in a subset run: project-lane
/// checks aggregate across the full file set, so even promoted ones refuse
/// here (a lone minority file is clean natively but blamed in-project).
fn subset_gate(entry: &crate::CheckEntry) -> Option<Fallback> {
    if !crate::supports_per_file(&entry.module) {
        return Some(Fallback {
            reason: format!("project-scope-check:{}", entry.module),
        });
    }
    gate(entry)
}

/// Build the native runner over enabled checks and their configured params.
/// Disabled checks matching `selection.enable_disabled` (case-insensitive
/// regex, mirroring native) rejoin the run. Exposed for `--stale`, which
/// builds filtered sub-configs over the same resolution.
pub(crate) fn runner_of(
    config: &crate::CredoConfig,
    min_priority: i32,
    selection: crate::Selection,
) -> crate::RunnerConfig {
    crate::RunnerConfig {
        checks: enabled_checks(config, &selection)
            .into_iter()
            .map(|mut entry| {
                entry.enabled = true;
                entry
            })
            .collect(),
        files_included: config
            .files_included
            .clone()
            .into_iter()
            .map(crate::FileEntry::Glob)
            .collect(),
        files_excluded: config.files_excluded.clone(),
        selection,
        min_priority,
        general: crate::GeneralParams::default(),
    }
}

/// Run the native pipeline for served configs; otherwise return the fallback.
///
/// `min_priority` mirrors the CLI (`0` default, `-99` for `--strict`).
/// Only issue presence and locations are contractual; message text and
/// severities are not.
///
/// # Errors
/// Returns [`Fallback`] with the stable recorded reason instead of running
/// anything native; the caller runs real Credo in that case.
pub fn execute(
    config_source: &str,
    config_name: &str,
    files: &[crate::RunnerFile],
    min_priority: i32,
) -> Result<crate::RunReport, Fallback> {
    execute_selected(
        config_source,
        config_name,
        files,
        min_priority,
        crate::Selection::default(),
    )
}

/// Enabled check modules after `--enable-disabled-checks` rejoining,
/// before CLI selection and execution gates. Shared by the runner and by
/// CLI check-count reporting so both agree.
#[must_use]
pub fn enabled_modules(config: &crate::CredoConfig, selection: &crate::Selection) -> Vec<String> {
    enabled_checks(config, selection)
        .into_iter()
        .map(|check| check.module)
        .collect()
}

/// Enabled check entries with their configured parameters preserved.
fn enabled_checks(
    config: &crate::CredoConfig,
    selection: &crate::Selection,
) -> Vec<crate::CheckEntry> {
    config
        .checks
        .iter()
        .filter(|check| check.enabled || reenabled(&check.module, &selection.enable_disabled))
        .cloned()
        .collect()
}
/// True when a disabled check rejoins via `--enable-disabled-checks`:
/// any pattern case-insensitive-regex-matches the check name.
fn reenabled(module: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .is_ok_and(|expression| expression.is_match(module))
    })
}

/// Run the native pipeline for served configs with CLI check selection.
///
/// `selection.only`/`selection.ignore` narrow the enabled checks after the
/// serve gate (an invalid selection is recorded in the report errors).
/// Otherwise identical to [`execute`].
///
/// # Errors
/// Same fallback contract as [`execute`].
pub fn execute_selected(
    config_source: &str,
    config_name: &str,
    files: &[crate::RunnerFile],
    min_priority: i32,
    selection: crate::Selection,
) -> Result<crate::RunReport, Fallback> {
    if let Outcome::Fallback { reason } = select(config_source, config_name) {
        return Err(Fallback { reason });
    }
    let config =
        crate::config_file::parse_config(config_source, config_name).map_err(|bad| Fallback {
            reason: format!("unsupported-credo-config:{}", bad.0),
        })?;
    Ok(crate::run_checks(
        files,
        &runner_of(&config, min_priority, selection),
    ))
}

/// Run the native pipeline over an explicit file subset for served configs.
///
/// The subset may be exactly the missing cache units: every served check
/// evaluates files independently. Returns per-filename issue arrays for
/// precisely the requested files, or the stable fallback without running
/// anything native.
///
/// # Errors
/// Returns [`Fallback`] with the stable recorded reason, including
/// `native-pipeline-errors` when a file stops the pipeline the way only
/// real Credo may then adjudicate.
pub fn execute_files(
    config_source: &str,
    config_name: &str,
    files: &[(String, String)],
    min_priority: i32,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, Fallback> {
    if let Outcome::Fallback { reason } = select_subset(config_source, config_name) {
        return Err(Fallback { reason });
    }
    let config =
        crate::config_file::parse_config(config_source, config_name).map_err(|bad| Fallback {
            reason: format!("unsupported-credo-config:{}", bad.0),
        })?;
    let runner_files: Vec<crate::RunnerFile> = files
        .iter()
        .map(|(filename, source)| crate::RunnerFile {
            filename: filename.clone(),
            source: source.clone(),
        })
        .collect();
    let report = crate::run_checks(
        &runner_files,
        &runner_of(&config, min_priority, crate::Selection::default()),
    );
    if !report.errors.is_empty() {
        return Err(Fallback {
            reason: "native-pipeline-errors".to_owned(),
        });
    }
    let mut by_file = std::collections::BTreeMap::new();
    for (filename, _) in files {
        let issues: Vec<&crate::Issue> = report
            .issues
            .iter()
            .filter(|issue| &issue.filename == filename)
            .collect();
        by_file.insert(
            filename.clone(),
            serde_json::to_value(&issues).map_err(|_| Fallback {
                reason: "native-pipeline-errors".to_owned(),
            })?,
        );
    }
    Ok(by_file)
}

/// Choose the native engine or fail-closed real-Credo fallback.
#[must_use]
pub fn select(config_source: &str, config_name: &str) -> Outcome {
    select_with(config_source, config_name, gate)
}

/// Subset-run variant: project-lane checks refuse even when promoted.
fn select_subset(config_source: &str, config_name: &str) -> Outcome {
    select_with(config_source, config_name, subset_gate)
}

/// Shared selection over one gate function.
fn select_with(
    config_source: &str,
    config_name: &str,
    gate_one: fn(&crate::CheckEntry) -> Option<Fallback>,
) -> Outcome {
    match crate::config_file::parse_config(config_source, config_name) {
        Err(unsupported) => Outcome::Fallback {
            reason: format!("unsupported-credo-config:{}", unsupported.0),
        },
        Ok(config) => {
            let mut checks = Vec::new();
            for entry in config.checks.iter().filter(|check| check.enabled) {
                if let Some(fallback) = gate_one(entry) {
                    return Outcome::Fallback {
                        reason: fallback.reason,
                    };
                }
                checks.push(entry.module.clone());
            }
            Outcome::Serve { checks }
        }
    }
}

#[cfg(test)]
mod execute_tests {
    use super::*;
    use crate::RunnerFile;

    const TWO_CHECKS: &str = "%{\n  configs: [\n    %{\n      name: \"default\",\n      files: %{included: [\"lib/\"]},\n      checks: %{enabled: [{Credo.Check.Readability.TrailingBlankLine, []}, {Credo.Check.Design.TagTODO, []}]}\n    }\n  ]\n}\n";

    fn files() -> Vec<RunnerFile> {
        vec![
            RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: "defmodule Example do\nend".to_owned(),
            },
            RunnerFile {
                filename: "lib/b.ex".to_owned(),
                source: "# TODO: polish\ndefmodule B do\nend\n".to_owned(),
            },
        ]
    }

    #[test]
    fn served_configs_report_native_issues_with_locations() {
        let report = execute(TWO_CHECKS, "default", &files(), -99).expect("served config runs");
        assert!(
            report.errors.is_empty(),
            "no pipeline errors: {:?}",
            report.errors
        );
        let mut locations: Vec<(String, usize)> = report
            .issues
            .iter()
            .map(|issue| (issue.filename.clone(), issue.line_no.unwrap_or(0)))
            .collect();
        locations.sort();
        assert_eq!(
            locations,
            vec![("lib/a.ex".to_owned(), 2), ("lib/b.ex".to_owned(), 1),]
        );
    }

    const ONE_CHECK: &str = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}]}}]}\n";

    const TABS_CHECK: &str = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Consistency.TabsOrSpaces, []}]}}]}\n";

    fn tabs_project() -> Vec<RunnerFile> {
        vec![
            RunnerFile {
                filename: "lib/a.ex".to_owned(),
                source: "defmodule A do\n  def a, do: 1\nend\n".to_owned(),
            },
            RunnerFile {
                filename: "lib/b.ex".to_owned(),
                source: "defmodule B do\n  def b, do: 2\nend\n".to_owned(),
            },
            RunnerFile {
                filename: "lib/c.ex".to_owned(),
                source: "defmodule C do\n\tdef c, do: 3\nend\n".to_owned(),
            },
        ]
    }

    #[test]
    fn promoted_project_checks_serve_full_runs() {
        let report = execute(TABS_CHECK, "default", &tabs_project(), -99).expect("served");
        assert!(report.errors.is_empty());
        let files: Vec<&str> = report
            .issues
            .iter()
            .map(|issue| issue.filename.as_str())
            .collect();
        assert_eq!(files, vec!["lib/c.ex"]);
    }

    #[test]
    fn project_checks_still_refuse_subset_runs() {
        // A lone minority file is clean natively but blamed in-project;
        // subset execution stays fail-closed instead of guessing.
        let subset = vec![("lib/c.ex".to_owned(), tabs_project()[2].source.clone())];
        assert_eq!(
            execute_files(TABS_CHECK, "default", &subset, -99).expect_err("subset refuses"),
            Fallback {
                reason: "project-scope-check:Credo.Check.Consistency.TabsOrSpaces".to_owned(),
            }
        );
    }

    #[test]
    fn duplicated_code_stays_gated() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Design.DuplicatedCode, []}]}}]}\n";
        assert_eq!(
            execute(source, "default", &tabs_project(), -99).expect_err("gated"),
            Fallback {
                reason: "project-scope-check:Credo.Check.Design.DuplicatedCode".to_owned(),
            }
        );
    }

    #[test]
    fn project_majority_revert_returns_original_issues() {
        let report = execute(TABS_CHECK, "default", &tabs_project(), -99).expect("served");
        let mut flipped = tabs_project();
        flipped[2].source = "defmodule C do\n  def c, do: 3\nend\n".to_owned();
        let clean = execute(TABS_CHECK, "default", &flipped, -99).expect("served");
        assert!(clean.issues.is_empty());
        let reverted = execute(TABS_CHECK, "default", &tabs_project(), -99).expect("served");
        assert_eq!(reverted.issues, report.issues);
    }

    const DISABLED_CHECK: &str = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}], disabled: [{Credo.Check.Warning.Dbg, []}]}}]}\n";

    #[test]
    fn enable_disabled_checks_rejoins_the_run() {
        let files = vec![RunnerFile {
            filename: "lib/a.ex".to_owned(),
            source: "defmodule A do\n  def a(x) do\n    IO.inspect(x)\n    dbg(x)\n  end\nend\n"
                .to_owned(),
        }];
        let base = execute(DISABLED_CHECK, "default", &files, -99).expect("served");
        assert_eq!(base.issues.len(), 1);
        let selection = crate::Selection {
            enable_disabled: vec!["Dbg".to_owned()],
            ..crate::Selection::default()
        };
        let report =
            execute_selected(DISABLED_CHECK, "default", &files, -99, selection).expect("served");
        assert_eq!(report.issues.len(), 2);
        let nomatch = crate::Selection {
            enable_disabled: vec!["NoSuchCheck".to_owned()],
            ..crate::Selection::default()
        };
        let stayed =
            execute_selected(DISABLED_CHECK, "default", &files, -99, nomatch).expect("served");
        assert_eq!(stayed.issues.len(), 1);
    }

    #[test]
    fn subset_execution_returns_only_requested_files() {
        let both = [
            ("lib/a.ex".to_owned(), "defmodule A do\nend\n".to_owned()),
            (
                "lib/b.ex".to_owned(),
                "defmodule B do\n  def b, do: IO.inspect(1)\nend\n".to_owned(),
            ),
        ];
        let only_b = execute_files(ONE_CHECK, "default", &both[1..], -99).expect("subset runs");
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b["lib/b.ex"].as_array().unwrap().len(), 1);
        let clean = execute_files(ONE_CHECK, "default", &both[..1], -99).expect("clean runs");
        assert!(clean["lib/a.ex"].as_array().unwrap().is_empty());
    }

    #[test]
    fn subset_execution_falls_back_closed() {
        let both = vec![("lib/a.ex".to_owned(), "defmodule A do\nend\n".to_owned())];
        let project = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Consistency.TabsOrSpaces, []}]}}]}\n";
        assert!(matches!(
            execute_files(project, "default", &both, -99),
            Err(Fallback { reason }) if reason.starts_with("project-scope-check:")
        ));
        let comment_error = vec![(
            "lib/bad.ex".to_owned(),
            "# credo:disable-for-next-line /[/\nIO.inspect(x)\n".to_owned(),
        )];
        assert_eq!(
            execute_files(TWO_CHECKS, "default", &comment_error, -99)
                .expect_err("comment errors fall back"),
            Fallback {
                reason: "native-pipeline-errors".to_owned(),
            }
        );
    }

    #[test]
    fn fallback_configs_never_run_the_native_pipeline() {
        let unknown = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Custom.NotARealCheck, []}]}}]}\n";
        assert_eq!(
            execute(unknown, "default", &files(), -99).expect_err("unknown check falls back"),
            Fallback {
                reason: "unsupported-check:Credo.Check.Custom.NotARealCheck".to_owned(),
            }
        );
        let params = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Design.TagTODO, [include_doc: false]}]}}]}\n";
        assert!(matches!(
            execute(params, "default", &files(), -99),
            Err(Fallback { reason }) if reason.starts_with("custom-check-params:")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "%{\n  configs: [\n    %{\n      name: \"default\",\n      files: %{included: [\"lib/\", \"test/\"]},\n      checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}]}\n    }\n  ]\n}\n";

    #[test]
    fn runner_of_preserves_check_params() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, [exit_status: 2]}]}}]}\n";
        let config = crate::parse_config(source, "default").expect("parses");
        let runner = super::runner_of(&config, 0, crate::Selection::default());
        assert_eq!(runner.checks.len(), 1);
        assert_eq!(
            runner.checks[0]
                .params
                .get("exit_status")
                .map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn labqoat_configured_params_serve() {
        let source = "%{\n  configs: [\n    %{\n      name: \"default\",\n      checks: %{enabled: [\n        {Credo.Check.Design.AliasUsage, [priority: :low, if_nested_deeper_than: 2, if_called_more_often_than: 0]},\n        {Credo.Check.Design.TagTODO, [exit_status: 2]},\n        {Credo.Check.Refactor.CyclomaticComplexity, [max_complexity: 8]},\n        {Credo.Check.Refactor.FunctionArity, [max_arity: 8]},\n        {Credo.Check.Refactor.Nesting, [max_nesting: 3]}\n      ]}\n    }\n  ]\n}\n";
        assert!(
            matches!(select(source, "default"), Outcome::Serve { .. }),
            "labqoat params must serve"
        );
    }

    #[test]
    fn promoted_numeric_params_accept_zero_boundary() {
        for (module, param) in [
            ("Credo.Check.Design.AliasUsage", "if_nested_deeper_than"),
            ("Credo.Check.Design.AliasUsage", "if_called_more_often_than"),
            (
                "Credo.Check.Refactor.CyclomaticComplexity",
                "max_complexity",
            ),
            ("Credo.Check.Refactor.FunctionArity", "max_arity"),
            ("Credo.Check.Refactor.Nesting", "max_nesting"),
        ] {
            let source = format!(
                "%{{configs: [%{{name: \"default\", checks: %{{enabled: [{{{module}, [{param}: 0]}}]}}}}]}}\n"
            );
            assert!(
                matches!(select(&source, "default"), Outcome::Serve { .. }),
                "{module}.{param} must accept zero"
            );
        }
    }

    #[test]
    fn common_builtin_params_serve_when_valid() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, [priority: :ignore, exit_status: 0, category: :warning, files: %{included: [\"lib/\"], excluded: [\"lib/generated/\"]}]}]}}]}\n";
        assert!(matches!(select(source, "default"), Outcome::Serve { .. }));
    }

    #[test]
    fn invalid_promoted_param_values_fall_back_closed() {
        for (module, param, value) in [
            ("Credo.Check.Design.AliasUsage", "priority", ":urgent"),
            (
                "Credo.Check.Design.AliasUsage",
                "if_nested_deeper_than",
                "-1",
            ),
            (
                "Credo.Check.Design.AliasUsage",
                "if_called_more_often_than",
                "false",
            ),
            ("Credo.Check.Design.TagTODO", "exit_status", ":urgent"),
            (
                "Credo.Check.Refactor.CyclomaticComplexity",
                "max_complexity",
                "-1",
            ),
            ("Credo.Check.Refactor.FunctionArity", "max_arity", "false"),
            ("Credo.Check.Refactor.Nesting", "max_nesting", ":deep"),
            ("Credo.Check.Design.AliasUsage", "max_complexity", "8"),
            ("Credo.Check.Warning.IoInspect", "category", ":unknown"),
        ] {
            let source = format!(
                "%{{configs: [%{{name: \"default\", checks: %{{enabled: [{{{module}, [{param}: {value}]}}]}}}}]}}\n"
            );
            assert_eq!(
                select(&source, "default"),
                Outcome::Fallback {
                    reason: format!("custom-check-params:{module}"),
                },
                "{module}.{param}={value} must fail closed"
            );
        }
    }

    #[test]
    fn static_default_params_serve() {
        assert_eq!(
            select(MINIMAL, "default"),
            Outcome::Serve {
                checks: vec!["Credo.Check.Warning.IoInspect".to_owned()],
            }
        );
    }

    #[test]
    fn unknown_enabled_checks_fall_back() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Custom.NotARealCheck, []}]}}]}\n";
        assert_eq!(
            select(source, "default"),
            Outcome::Fallback {
                reason: "unsupported-check:Credo.Check.Custom.NotARealCheck".to_owned(),
            }
        );
    }

    #[test]
    fn custom_params_fall_back_closed() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, [include_iso: true]}]}}]}\n";
        assert_eq!(
            select(source, "default"),
            Outcome::Fallback {
                reason: "custom-check-params:Credo.Check.Warning.IoInspect".to_owned(),
            }
        );
    }

    #[test]
    fn project_and_config_validated_checks_fall_back() {
        // TabsOrSpaces is promoted (serves); DuplicatedCode stays gated.
        let promoted = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Consistency.TabsOrSpaces, []}]}}]}\n";
        assert_eq!(
            select(promoted, "default"),
            Outcome::Serve {
                checks: vec!["Credo.Check.Consistency.TabsOrSpaces".to_owned()],
            }
        );
        let project = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Design.DuplicatedCode, []}]}}]}\n";
        assert_eq!(
            select(project, "default"),
            Outcome::Fallback {
                reason: "project-scope-check:Credo.Check.Design.DuplicatedCode".to_owned(),
            }
        );
        let validated = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Design.MissingCheckInConfig, []}]}}]}\n";
        assert_eq!(
            select(validated, "default"),
            Outcome::Fallback {
                reason: "needs-validated-config:Credo.Check.Design.MissingCheckInConfig".to_owned(),
            }
        );
    }

    #[test]
    fn executable_configs_fall_back_closed() {
        let source = "%{configs: [%{name: \"default\", checks: Mix.env()}]}\n";
        assert!(
            matches!(select(source, "default"), Outcome::Fallback { reason } if reason.starts_with("unsupported-credo-config:"))
        );
    }

    #[test]
    fn disabled_unsupported_checks_do_not_force_fallback() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}], disabled: [{Credo.Check.Custom.NotARealCheck, []}]}}]}\n";
        assert_eq!(
            select(source, "default"),
            Outcome::Serve {
                checks: vec!["Credo.Check.Warning.IoInspect".to_owned()],
            }
        );
    }
}
