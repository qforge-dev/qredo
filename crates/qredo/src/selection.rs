//! Execution-level check selection and config fallback.
//!
//! Covers `only`/`ignore` filtering and the executable-config boundary:
//! arbitrary `.credo.exs` files execute Elixir, so a present config file
//! without a statically reviewed shape is an explicit fallback, never silent
//! defaults. Filesystem discovery itself stays caller-side (the engine owns
//! capture); this module resolves already-captured inputs.
//!
//! Name matching mirrors `Credo.Execution`: each pattern is a
//! case-insensitive regex matched against the check name, and `ignore` wins
//! over `only`. Tag filtering uses the [`crate::check_meta::check_tags`]
//! table; unknown tags match nothing, like native atomization.

/// Which checks an execution runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Run only checks matching these patterns (`--only`). Empty: no restriction.
    pub only: Vec<String>,
    /// Skip checks matching these patterns (`--ignore`).
    pub ignore: Vec<String>,
    /// Run only checks carrying any of these tags (`--checks-with-tag`).
    /// Empty: no restriction. Unknown tags match nothing.
    pub checks_with_tag: Vec<String>,
    /// Re-enable disabled config checks matching these patterns
    /// (`--enable-disabled-checks`, case-insensitive regex).
    pub enable_disabled: Vec<String>,
}

impl Selection {
    /// Check that every pattern compiles as a case-insensitive regex.
    ///
    /// # Errors
    /// Returns the first pattern that fails to compile.
    pub fn validate(&self) -> Result<(), String> {
        for pattern in self.only.iter().chain(self.ignore.iter()) {
            matches_pattern(pattern, "").map_err(|_| pattern.clone())?;
        }
        Ok(())
    }

    /// True when `check` (e.g. `Credo.Check.Readability.TrailingBlankLine`)
    /// runs under this selection. `ignore` wins over `only`; a non-empty
    /// tag filter requires the check to carry any listed tag.
    ///
    /// Invalid patterns never match; call [`Selection::validate`] first when
    /// strictness is required ([`resolve`] reports them explicitly).
    #[must_use]
    pub fn should_run(&self, check: &str) -> bool {
        if matches_any(check, &self.ignore) {
            return false;
        }
        if !self.checks_with_tag.is_empty() && !has_any_tag(check, &self.checks_with_tag) {
            return false;
        }
        if self.only.is_empty() {
            return true;
        }
        matches_any(check, &self.only)
    }
}

/// True when the check carries any of the listed tags (union).
/// Unknown tags match nothing, mirroring native `String.to_atom` behavior.
fn has_any_tag(check: &str, tags: &[String]) -> bool {
    let carried = crate::check_meta::check_tags(check);
    tags.iter().any(|tag| carried.contains(&tag.as_str()))
}

/// Case-insensitive regex match of one pattern against `check`.
fn matches_pattern(pattern: &str, check: &str) -> Result<bool, regex::Error> {
    let expression = format!("(?i){pattern}");
    let matched = regex::Regex::new(&expression)?.is_match(check);
    Ok(matched)
}

/// True when any pattern matches; invalid patterns never match.
fn matches_any(check: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| matches_pattern(pattern, check).unwrap_or(false))
}

/// How configuration was provided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// No config file found; Credo defaults apply.
    Default,
    /// Explicit static parameters (already captured, no Elixir involved).
    Explicit,
    /// A `.credo.exs` file is present: it executes Elixir, so native
    /// execution is required.
    ExecutableFile(String),
}

/// Outcome of resolving selection and configuration before any work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Run,
    FilteredOut,
    NeedsNativeConfig(String),
    InvalidSelection(String),
}

/// Resolve whether `check` runs given captured inputs.
#[must_use]
pub fn resolve(check: &str, selection: &Selection, config: &ConfigSource) -> Resolution {
    if let ConfigSource::ExecutableFile(path) = config {
        return Resolution::NeedsNativeConfig(path.clone());
    }
    for pattern in selection.only.iter().chain(selection.ignore.iter()) {
        if matches_pattern(pattern, "").is_err() {
            return Resolution::InvalidSelection(pattern.clone());
        }
    }
    if selection.should_run(check) {
        Resolution::Run
    } else {
        Resolution::FilteredOut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECK: &str = "Credo.Check.Readability.TrailingBlankLine";

    #[test]
    fn default_selection_runs() {
        assert!(Selection::default().should_run(CHECK));
    }

    #[test]
    fn only_matching_runs() {
        let selection = Selection {
            only: vec![CHECK.to_owned()],
            ignore: Vec::new(),
            ..Selection::default()
        };
        assert!(selection.should_run(CHECK));
    }

    #[test]
    fn only_missing_filters_out() {
        let selection = Selection {
            only: vec!["Credo.Check.Warning.IoInspect".to_owned()],
            ignore: Vec::new(),
            ..Selection::default()
        };
        assert!(!selection.should_run(CHECK));
    }

    #[test]
    fn ignore_wins_over_only() {
        let selection = Selection {
            only: vec![CHECK.to_owned()],
            ignore: vec![CHECK.to_owned()],
            ..Selection::default()
        };
        assert!(!selection.should_run(CHECK));
    }

    #[test]
    fn resolve_runs_by_default() {
        assert_eq!(
            resolve(CHECK, &Selection::default(), &ConfigSource::Default),
            Resolution::Run
        );
    }

    #[test]
    fn resolve_filtered_out_on_only_miss() {
        let selection = Selection {
            only: vec!["Credo.Check.Warning.IoInspect".to_owned()],
            ignore: Vec::new(),
            ..Selection::default()
        };
        assert_eq!(
            resolve(CHECK, &selection, &ConfigSource::Default),
            Resolution::FilteredOut
        );
    }

    #[test]
    fn resolve_falls_back_on_executable_config() {
        let config = ConfigSource::ExecutableFile(".credo.exs".to_owned());
        assert_eq!(
            resolve(CHECK, &Selection::default(), &config),
            Resolution::NeedsNativeConfig(".credo.exs".to_owned())
        );
    }

    #[test]
    fn executable_config_wins_over_selection() {
        let selection = Selection {
            only: vec!["Credo.Check.Warning.IoInspect".to_owned()],
            ignore: Vec::new(),
            ..Selection::default()
        };
        let config = ConfigSource::ExecutableFile("config/.credo.exs".to_owned());
        assert_eq!(
            resolve(CHECK, &selection, &config),
            Resolution::NeedsNativeConfig("config/.credo.exs".to_owned())
        );
    }

    #[test]
    fn pattern_matches_case_insensitive_substring() {
        let selection = Selection {
            only: vec!["trailingblankline".to_owned()],
            ignore: Vec::new(),
            ..Selection::default()
        };
        assert!(selection.should_run(CHECK));
    }

    #[test]
    fn invalid_pattern_is_reported() {
        let selection = Selection {
            only: vec!["([".to_owned()],
            ignore: Vec::new(),
            ..Selection::default()
        };
        assert_eq!(
            resolve(CHECK, &selection, &ConfigSource::Default),
            Resolution::InvalidSelection("([".to_owned())
        );
        assert_eq!(selection.validate(), Err("([".to_owned()));
    }

    #[test]
    fn valid_selection_validates() {
        assert_eq!(Selection::default().validate(), Ok(()));
    }

    #[test]
    fn tag_filter_keeps_tagged_checks() {
        let selection = Selection {
            checks_with_tag: vec!["formatter".to_owned()],
            ..Selection::default()
        };
        assert!(selection.should_run("Credo.Check.Readability.TrailingWhiteSpace"));
        assert!(!selection.should_run("Credo.Check.Warning.IoInspect"));
    }

    #[test]
    fn tag_filter_is_union_and_unknown_matches_nothing() {
        let selection = Selection {
            checks_with_tag: vec!["formatter".to_owned(), "controversial".to_owned()],
            ..Selection::default()
        };
        assert!(selection.should_run("Credo.Check.Readability.TrailingWhiteSpace"));
        assert!(selection.should_run("Credo.Check.Refactor.DoubleBooleanNegation"));
        let unknown = Selection {
            checks_with_tag: vec!["no-such-tag".to_owned()],
            ..Selection::default()
        };
        assert!(!unknown.should_run("Credo.Check.Readability.TrailingWhiteSpace"));
    }

    #[test]
    fn ignore_wins_over_tag_filter() {
        let selection = Selection {
            ignore: vec!["TrailingWhiteSpace".to_owned()],
            checks_with_tag: vec!["formatter".to_owned()],
            ..Selection::default()
        };
        assert!(!selection.should_run("Credo.Check.Readability.TrailingWhiteSpace"));
    }
}
