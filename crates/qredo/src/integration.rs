//! Swappable native lint engine with fail-closed fallback.
//!
//! [`select`] serves static `.credo.exs` configs whose enabled checks are all
//! implemented with default parameters. Anything else reports a stable
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

/// Fail-closed gate for one enabled check; `None` means natively servable.
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
    if !crate::supports_per_file(&entry.module) {
        return Some(Fallback {
            reason: format!("project-scope-check:{}", entry.module),
        });
    }
    if !entry.params.is_empty() {
        return Some(Fallback {
            reason: format!("custom-check-params:{}", entry.module),
        });
    }
    None
}

/// Build the native runner over default-param enabled checks.
fn runner_of(
    config: &crate::CredoConfig,
    min_priority: i32,
    selection: crate::Selection,
) -> crate::RunnerConfig {
    crate::RunnerConfig {
        checks: config
            .checks
            .iter()
            .filter(|check| check.enabled)
            .map(|check| crate::CheckEntry {
                module: check.module.clone(),
                enabled: true,
                params: std::collections::BTreeMap::new(),
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
    if let Outcome::Fallback { reason } = select(config_source, config_name) {
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
    match crate::config_file::parse_config(config_source, config_name) {
        Err(unsupported) => Outcome::Fallback {
            reason: format!("unsupported-credo-config:{}", unsupported.0),
        },
        Ok(config) => {
            let mut checks = Vec::new();
            for entry in config.checks.iter().filter(|check| check.enabled) {
                if let Some(fallback) = gate(entry) {
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
        let project = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Consistency.TabsOrSpaces, []}]}}]}\n";
        assert_eq!(
            select(project, "default"),
            Outcome::Fallback {
                reason: "project-scope-check:Credo.Check.Consistency.TabsOrSpaces".to_owned(),
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
