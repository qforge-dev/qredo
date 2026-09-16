//! `# credo:disable` suppression comments.
//!
//! Mirrors `Credo.Check.ConfigCommentFinder` registration with
//! `ConfigComment.ignores_issue?/2` line semantics: the `credo:` marker
//! match is case-insensitive and unanchored (a second `#` counts),
//! instructions keep their case (only exact lowercase names suppress),
//! check filters are bare (all checks), exact atoms, or case-insensitive
//! `/regex/` patterns, and param text is never trimmed (trailing spaces
//! break exact matches upstream too).
//!
//! Intentional deviations from upstream crashes, each with rationale and
//! tests: an invalid regex filter never matches (upstream raises inside
//! the finder) and surfaces via [`validate_comments`]; a malformed
//! `disable-for-lines` count never suppresses and is flagged by the
//! redundant-comment check. Both stay loud; neither invents suppression.

use crate::issue::Issue;

/// A `# credo:` control comment with its own line.
#[derive(Debug, Clone)]
pub struct ConfigComment {
    /// 1-based effective start line (shifted for negative counts).
    pub line_no: usize,
    /// 1-based column of the `#`.
    pub column: usize,
    /// Instruction: a known name or the raw text when unknown/malformed.
    /// Only the four lowercase names ever suppress.
    pub instruction: String,
    /// Check filter.
    pub filter: CheckFilter,
    /// Inclusive range end for `disable-for-lines:N`.
    pub line_no_end: usize,
    /// Lines-form without a valid count: never suppresses, flagged by the
    /// redundant-comment check instead of crashing.
    pub malformed: bool,
}

/// Check filter of a config comment.
#[derive(Debug, Clone)]
pub enum CheckFilter {
    /// No filter text: suppresses every check.
    All,
    /// Exact check atom text, e.g. `Credo.Check.Foo`.
    Exact(String),
    /// Case-insensitive regex source with its compilation.
    Pattern {
        #[allow(dead_code, reason = "surfaced by validate_comments in pipeline wiring")]
        source: String,
        compiled: Option<regex::Regex>,
    },
}

impl CheckFilter {
    /// True when this filter covers `check` (an `Elixir.`-prefixed name).
    #[must_use]
    pub fn matches(&self, check: &str) -> bool {
        match self {
            Self::All => true,
            Self::Exact(atom) => format!("Elixir.{atom}") == check,
            Self::Pattern { compiled, .. } => {
                compiled.as_ref().is_some_and(|re| re.is_match(check))
            }
        }
    }
}

/// Invalid regex filter naming its comment line.
#[derive(Debug, PartialEq, Eq)]
pub struct CommentError {
    /// 1-based line of the offending comment.
    pub line_no: usize,
    /// Human-readable reason.
    pub message: String,
}

impl ConfigComment {
    /// True when this comment suppresses an issue of `check` at `line`,
    /// mirroring `ConfigComment.ignores_issue?/2`.
    #[must_use]
    pub fn ignores(&self, check: &str, line: usize) -> bool {
        if self.malformed || !self.filter.matches(check) {
            return false;
        }
        match self.instruction.as_str() {
            "disable-for-this-file" => true,
            "disable-for-next-line" => line == self.line_no + 1,
            "disable-for-previous-line" => line + 1 == self.line_no,
            "disable-for-lines" => line >= self.line_no && line <= self.line_no_end,
            _ => false,
        }
    }
}

/// Split a comment body into `(instruction, param)` with the upstream
/// finder regex (case-insensitive, unanchored past `#`).
fn split_instruction(body: &str) -> Option<(String, String)> {
    let expression = regex::Regex::new(r"(?i)(?:^|#)\s*credo:([\w\-\:]+)\s*(.*)").ok()?;
    let captures = expression.captures(body)?;
    Some((
        captures.get(1)?.as_str().to_owned(),
        captures.get(2)?.as_str().to_owned(),
    ))
}

/// Check filter from raw param text (never trimmed).
fn parse_filter(param: &str) -> CheckFilter {
    if param.is_empty() {
        CheckFilter::All
    } else if param.len() > 2 && param.starts_with('/') && param.ends_with('/') {
        let source = param[1..param.len() - 1].to_owned();
        let compiled = regex::RegexBuilder::new(&source)
            .case_insensitive(true)
            .build()
            .ok();
        CheckFilter::Pattern { source, compiled }
    } else {
        CheckFilter::Exact(param.to_owned())
    }
}

/// All `# credo:` control comments in source order, including unknown
/// instructions (the redundant-comment check flags unused ones).
#[must_use]
pub fn config_comments(source: &str) -> Vec<ConfigComment> {
    let mut out = Vec::new();
    for (line_no, col, body) in crate::helpers::comments(source) {
        let Some((instruction, param)) = split_instruction(&body) else {
            continue;
        };
        let filter = parse_filter(&param);
        if let Some(count) = instruction.strip_prefix("disable-for-lines:") {
            match count.parse::<i32>() {
                Ok(count) => {
                    let (start, end) = if count >= 0 {
                        (
                            line_no,
                            line_no.saturating_add(usize::try_from(count).unwrap_or(0)),
                        )
                    } else {
                        (
                            line_no.saturating_sub(count.unsigned_abs() as usize),
                            line_no,
                        )
                    };
                    out.push(ConfigComment {
                        line_no: start,
                        column: col,
                        instruction: "disable-for-lines".to_owned(),
                        filter,
                        line_no_end: end,
                        malformed: false,
                    });
                }
                Err(_) => out.push(malformed_comment(line_no, col, instruction, filter)),
            }
            continue;
        }
        if instruction == "disable-for-lines" {
            out.push(malformed_comment(line_no, col, instruction, filter));
            continue;
        }
        out.push(ConfigComment {
            line_no,
            column: col,
            instruction,
            filter,
            line_no_end: line_no,
            malformed: false,
        });
    }
    out
}

/// A lines-form comment without a valid count: registered but inert.
fn malformed_comment(
    line_no: usize,
    col: usize,
    instruction: String,
    filter: CheckFilter,
) -> ConfigComment {
    ConfigComment {
        line_no,
        column: col,
        instruction,
        filter,
        line_no_end: line_no,
        malformed: true,
    }
}

/// Reject files whose comments would crash upstream registration:
/// invalid regex filters. Malformed counts stay visible to the
/// redundant-comment check instead. Returns the first offender.
///
/// Consumed by pipeline wiring (stage 7); unit-tested here.
///
/// # Errors
/// Returns [`CommentError`] for the first invalid regex filter.
#[allow(dead_code, reason = "consumed by pipeline wiring in stage 7")]
pub fn validate_comments(source: &str) -> Result<(), CommentError> {
    for comment in config_comments(source) {
        if let CheckFilter::Pattern { source, compiled } = &comment.filter
            && compiled.is_none()
        {
            return Err(CommentError {
                line_no: comment.line_no,
                message: format!("invalid regex filter `/{source}/`"),
            });
        }
    }
    Ok(())
}

/// True when any comment suppresses `issue`.
#[must_use]
pub fn suppresses(source: &str, issue: &Issue) -> bool {
    let Some(line) = issue.line_no else {
        return false;
    };
    config_comments(source)
        .iter()
        .any(|comment| comment.ignores(&issue.check, line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::{BasePriority, Category, IssueTrigger};

    fn trailing_issue(line: usize) -> Issue {
        Issue {
            check: "Elixir.Credo.Check.Readability.TrailingBlankLine".to_owned(),
            category: Category::Readability,
            priority: BasePriority::Low.to_integer(),
            severity: 1.0,
            message: "There should be a final \\n at the end of each file.".to_owned(),
            filename: "lib/a.ex".to_owned(),
            line_no: Some(line),
            column: None,
            exit_status: 4,
            trigger: IssueTrigger::NoTrigger,
            scope: None,
        }
    }

    #[test]
    fn bare_file_disable_suppresses() {
        let src = "# credo:disable-for-this-file\ndefmodule M do\nend";
        assert!(suppresses(src, &trailing_issue(3)));
    }

    #[test]
    fn exact_check_disable_suppresses() {
        let src = "# credo:disable-for-this-file Credo.Check.Readability.TrailingBlankLine\ndefmodule M do\nend";
        assert!(suppresses(src, &trailing_issue(2)));
    }

    #[test]
    fn other_check_does_not_suppress() {
        let src =
            "# credo:disable-for-this-file Credo.Check.Warning.IoInspect\ndefmodule M do\nend";
        assert!(!suppresses(src, &trailing_issue(2)));
    }

    #[test]
    fn regex_filter_suppresses_case_insensitively() {
        let src = "# credo:disable-for-this-file /trailingblankline/\ndefmodule M do\nend";
        assert!(suppresses(src, &trailing_issue(2)));
    }

    #[test]
    fn trailing_spaces_break_exact_matches_upstream() {
        // Verified natively: the param keeps trailing spaces, so the atom
        // never equals the check.
        let src = "# credo:disable-for-this-file Credo.Check.Readability.TrailingBlankLine   \ndefmodule M do\nend";
        assert!(!suppresses(src, &trailing_issue(2)));
    }

    #[test]
    fn uppercase_marker_registers() {
        let src = "# CREDO:disable-for-this-file\ndefmodule M do\nend";
        assert!(suppresses(src, &trailing_issue(2)));
    }

    #[test]
    fn mid_comment_marker_registers() {
        let src = "# foo # credo:disable-for-this-file\ndefmodule M do\nend";
        assert!(suppresses(src, &trailing_issue(2)));
    }

    #[test]
    fn unknown_instruction_never_suppresses_but_registers() {
        let src = "# credo:bogus-instruction\ndefmodule M do\nend";
        assert!(!suppresses(src, &trailing_issue(2)));
        assert_eq!(config_comments(src).len(), 1);
    }

    #[test]
    fn invalid_regex_never_suppresses_and_validates_loud() {
        let src = "# credo:disable-for-next-line /[/\ndefmodule M do\nend";
        assert!(!suppresses(src, &trailing_issue(2)));
        let error = validate_comments(src).expect_err("invalid regex errors");
        assert_eq!(error.line_no, 1);
    }

    #[test]
    fn bad_line_count_is_inert_but_registered() {
        let src = "# credo:disable-for-lines:x\ndefmodule M do\nend";
        assert!(!suppresses(src, &trailing_issue(2)));
        let comments = config_comments(src);
        assert_eq!(comments.len(), 1);
        assert!(comments[0].malformed);
        assert!(validate_comments(src).is_ok());
    }

    #[test]
    fn next_line_suppresses_only_next_line() {
        let src = "x = 1\n# credo:disable-for-next-line\ndefmodule M do\nend";
        assert!(suppresses(src, &trailing_issue(3)));
        assert!(!suppresses(src, &trailing_issue(1)));
    }

    #[test]
    fn lines_range_with_negative_count() {
        let src = "x = 1\ny = 2\n# credo:disable-for-lines:-2\nz = 3\n";
        assert!(suppresses(src, &trailing_issue(1)));
        assert!(suppresses(src, &trailing_issue(2)));
        assert!(suppresses(src, &trailing_issue(3)));
        assert!(!suppresses(src, &trailing_issue(4)));
    }
}
