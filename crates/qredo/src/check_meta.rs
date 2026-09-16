//! Per-check issue metadata: base priority, category, exit status.
//!
//! Mirrors `Credo.Check` defaults with `Params` resolution order
//! (`__`-prefixed CLI values, plain params, check defaults):
//! priority resolves to an integer (`nil`/missing yields the check base,
//! numeric strings parse, atoms map by name, anything else errors like
//! upstream), category derives from the module path (verified: no check
//! overrides it), and exit status defaults to the category value.

use std::collections::BTreeMap;

use crate::issue::{Category, Issue, IssueTrigger};
use crate::pipeline::GeneralParams;
use crate::scope::{Scopes, scope_priorities_on_facts};
use crate::{Finding, Trigger};

/// Base priority value of a check: named level or raw `0` default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckBase {
    Higher,
    High,
    Normal,
    Low,
    /// No `base_priority` in `use`: `base_priority()` returns `0`.
    DefaultZero,
}

/// Priority resolution failure naming the offending value.
#[derive(Debug, PartialEq, Eq)]
pub struct PriorityError(pub String);

/// Checks with `HIGHER_PRIORITY` base priority at the pinned commit.
const HIGHER_PRIORITY: &[&str] = &["Credo.Check.Design.DuplicatedCode"];

/// Checks with `HIGH_PRIORITY` base priority at the pinned commit.
const HIGH_PRIORITY: &[&str] = &[
    "Credo.Check.Consistency.ExceptionNames",
    "Credo.Check.Consistency.LineEndings",
    "Credo.Check.Consistency.MultiAliasImportRequireUse",
    "Credo.Check.Consistency.ParameterPatternMatching",
    "Credo.Check.Consistency.SpaceAroundOperators",
    "Credo.Check.Consistency.SpaceInParentheses",
    "Credo.Check.Consistency.TabsOrSpaces",
    "Credo.Check.Consistency.UnusedVariableNames",
    "Credo.Check.Design.MissingCheckInConfig",
    "Credo.Check.Design.TagFIXME",
    "Credo.Check.Readability.FunctionNames",
    "Credo.Check.Readability.LargeNumbers",
    "Credo.Check.Readability.ModuleAttributeNames",
    "Credo.Check.Readability.ModuleNames",
    "Credo.Check.Readability.ParenthesesInCondition",
    "Credo.Check.Readability.PredicateFunctionNames",
    "Credo.Check.Readability.PreferUnquotedAtoms",
    "Credo.Check.Readability.Semicolons",
    "Credo.Check.Readability.SinglePipe",
    "Credo.Check.Readability.VariableNames",
    "Credo.Check.Refactor.FilterCount",
    "Credo.Check.Refactor.LongQuoteBlocks",
    "Credo.Check.Refactor.MapInto",
    "Credo.Check.Refactor.MapJoin",
    "Credo.Check.Refactor.NegatedConditionsInUnless",
    "Credo.Check.Refactor.NegatedConditionsWithElse",
    "Credo.Check.Refactor.RedundantWithClauseResult",
    "Credo.Check.Refactor.UnlessWithElse",
    "Credo.Check.Refactor.UtcNowTruncate",
    "Credo.Check.Refactor.WithClauses",
    "Credo.Check.Warning.ApplicationConfigInModuleAttribute",
    "Credo.Check.Warning.BoolOperationOnSameValues",
    "Credo.Check.Warning.Dbg",
    "Credo.Check.Warning.ExpensiveEmptyEnumCheck",
    "Credo.Check.Warning.ForbiddenFunction",
    "Credo.Check.Warning.ForbiddenModule",
    "Credo.Check.Warning.IExPry",
    "Credo.Check.Warning.IoInspect",
    "Credo.Check.Warning.LazyLogging",
    "Credo.Check.Warning.LeakyEnvironment",
    "Credo.Check.Warning.MissedMetadataKeyInLoggerConfig",
    "Credo.Check.Warning.MixEnv",
    "Credo.Check.Warning.OperationOnSameValues",
    "Credo.Check.Warning.OperationWithConstantResult",
    "Credo.Check.Warning.UnsafeExec",
    "Credo.Check.Warning.UnsafeToAtom",
    "Credo.Check.Warning.UnusedEnumOperation",
    "Credo.Check.Warning.UnusedFileOperation",
    "Credo.Check.Warning.UnusedKeywordOperation",
    "Credo.Check.Warning.UnusedListOperation",
    "Credo.Check.Warning.UnusedMapOperation",
    "Credo.Check.Warning.UnusedOperation",
    "Credo.Check.Warning.UnusedPathOperation",
    "Credo.Check.Warning.UnusedRegexOperation",
    "Credo.Check.Warning.UnusedStringOperation",
    "Credo.Check.Warning.UnusedTupleOperation",
    "Credo.Check.Warning.WrongTestFileExtension",
    "Credo.Check.Warning.WrongTestFilename",
];

/// Checks with `NORMAL_PRIORITY` base priority at the pinned commit.
const NORMAL_PRIORITY: &[&str] = &[
    "Credo.Check.Design.AliasUsage",
    "Credo.Check.Design.DeprecatedChecksConfig",
    "Credo.Check.Design.RedundantConfigComments",
    "Credo.Check.Design.SkipTestWithoutComment",
    "Credo.Check.Readability.ImplTrue",
    "Credo.Check.Readability.UnusedFunctionParameterPattern",
    "Credo.Check.Refactor.ModuleDependencies",
    "Credo.Check.Refactor.PassAsyncInTestCases",
    "Credo.Check.Warning.MapGetUnsafePass",
    "Credo.Check.Warning.SpecWithStruct",
    "Credo.Check.Warning.StructFieldAmount",
];

/// Checks with `LOW_PRIORITY` base priority at the pinned commit.
const LOW_PRIORITY: &[&str] = &[
    "Credo.Check.Readability.AliasAs",
    "Credo.Check.Readability.AliasOrder",
    "Credo.Check.Readability.MaxLineLength",
    "Credo.Check.Readability.MultiAlias",
    "Credo.Check.Readability.OneArityFunctionInPipe",
    "Credo.Check.Readability.ParenthesesOnZeroArityDefs",
    "Credo.Check.Readability.PipeIntoAnonymousFunctions",
    "Credo.Check.Readability.PreferImplicitTry",
    "Credo.Check.Readability.RedundantBlankLines",
    "Credo.Check.Readability.SeparateAliasRequire",
    "Credo.Check.Readability.SpecParameterNames",
    "Credo.Check.Readability.StrictModuleLayout",
    "Credo.Check.Readability.StringSigils",
    "Credo.Check.Readability.TrailingBlankLine",
    "Credo.Check.Readability.TrailingWhiteSpace",
    "Credo.Check.Readability.UnnecessaryAliasExpansion",
    "Credo.Check.Readability.WithCustomTaggedTuple",
    "Credo.Check.Refactor.AppendSingleItem",
    "Credo.Check.Refactor.Apply",
    "Credo.Check.Refactor.CondInsteadOfIfElse",
    "Credo.Check.Refactor.DoubleBooleanNegation",
    "Credo.Check.Refactor.NegatedIsNil",
    "Credo.Check.Refactor.PreferDateTimeShift",
];

/// Checks with `DEFAULT_ZERO_PRIORITY` base priority at the pinned commit.
const DEFAULT_ZERO_PRIORITY: &[&str] = &[
    "Credo.Check.Consistency.Collector",
    "Credo.Check.Consistency.ExceptionNames.Collector",
    "Credo.Check.Consistency.LineEndings.Collector",
    "Credo.Check.Consistency.MultiAliasImportRequireUse.Collector",
    "Credo.Check.Consistency.ParameterPatternMatching.Collector",
    "Credo.Check.Consistency.SpaceAroundOperators.Collector",
    "Credo.Check.Consistency.SpaceInParentheses.Collector",
    "Credo.Check.Consistency.TabsOrSpaces.Collector",
    "Credo.Check.Consistency.UnusedVariableNames.Collector",
    "Credo.Check.Design.TagTODO",
    "Credo.Check.Readability.BlockPipe",
    "Credo.Check.Readability.CaptureOperator",
    "Credo.Check.Readability.ModuleDoc",
    "Credo.Check.Readability.NestedFunctionCalls",
    "Credo.Check.Readability.OnePipePerLine",
    "Credo.Check.Readability.SingleFunctionToBlockPipe",
    "Credo.Check.Readability.SpaceAfterCommas",
    "Credo.Check.Readability.Specs",
    "Credo.Check.Readability.WithSingleClause",
    "Credo.Check.Refactor.ABCSize",
    "Credo.Check.Refactor.CaseTrivialMatches",
    "Credo.Check.Refactor.CondStatements",
    "Credo.Check.Refactor.CyclomaticComplexity",
    "Credo.Check.Refactor.FilterFilter",
    "Credo.Check.Refactor.FilterReject",
    "Credo.Check.Refactor.FunctionArity",
    "Credo.Check.Refactor.IoPuts",
    "Credo.Check.Refactor.MapMap",
    "Credo.Check.Refactor.MatchInCondition",
    "Credo.Check.Refactor.Nesting",
    "Credo.Check.Refactor.PerceivedComplexity",
    "Credo.Check.Refactor.PipeChainStart",
    "Credo.Check.Refactor.RejectFilter",
    "Credo.Check.Refactor.RejectReject",
    "Credo.Check.Refactor.VariableRebinding",
    "Credo.Check.Warning.RaiseInsideRescue",
];

/// Base priority of a rule, or `None` for unknown rules.
/// Tables sourced from each check's `base_priority` in `use Credo.Check`
/// at the pinned commit (absent means `0`); guarded by ledger coverage.
#[must_use]
pub fn base_priority(rule: &str) -> Option<CheckBase> {
    if HIGHER_PRIORITY.contains(&rule) {
        Some(CheckBase::Higher)
    } else if HIGH_PRIORITY.contains(&rule) {
        Some(CheckBase::High)
    } else if NORMAL_PRIORITY.contains(&rule) {
        Some(CheckBase::Normal)
    } else if LOW_PRIORITY.contains(&rule) {
        Some(CheckBase::Low)
    } else if DEFAULT_ZERO_PRIORITY.contains(&rule) {
        Some(CheckBase::DefaultZero)
    } else {
        None
    }
}

/// Category derived from the module path, or `None` for unknown rules.
/// Verified at the pinned commit: no check overrides its path category.
#[must_use]
pub fn category_for(rule: &str) -> Option<Category> {
    let segment = rule
        .strip_prefix("Credo.Check.")?
        .split('.')
        .next()
        .unwrap_or_default();
    match segment {
        "Consistency" => Some(Category::Consistency),
        "Design" => Some(Category::Design),
        "Readability" => Some(Category::Readability),
        "Refactor" => Some(Category::Refactor),
        "Warning" => Some(Category::Warning),
        _ => None,
    }
}

/// Checks tagged `:formatter` at the pinned commit (12).
const FORMATTER_TAGGED: &[&str] = &[
    "Credo.Check.Consistency.LineEndings",
    "Credo.Check.Consistency.SpaceAroundOperators",
    "Credo.Check.Consistency.SpaceInParentheses",
    "Credo.Check.Consistency.TabsOrSpaces",
    "Credo.Check.Readability.LargeNumbers",
    "Credo.Check.Readability.MaxLineLength",
    "Credo.Check.Readability.ParenthesesInCondition",
    "Credo.Check.Readability.RedundantBlankLines",
    "Credo.Check.Readability.Semicolons",
    "Credo.Check.Readability.SpaceAfterCommas",
    "Credo.Check.Readability.TrailingBlankLine",
    "Credo.Check.Readability.TrailingWhiteSpace",
];

/// Checks tagged `:controversial` at the pinned commit (22).
const CONTROVERSIAL_TAGGED: &[&str] = &[
    "Credo.Check.Consistency.MultiAliasImportRequireUse",
    "Credo.Check.Design.DuplicatedCode",
    "Credo.Check.Readability.BlockPipe",
    "Credo.Check.Readability.MultiAlias",
    "Credo.Check.Readability.NestedFunctionCalls",
    "Credo.Check.Readability.SingleFunctionToBlockPipe",
    "Credo.Check.Readability.SinglePipe",
    "Credo.Check.Readability.Specs",
    "Credo.Check.Readability.StrictModuleLayout",
    "Credo.Check.Refactor.ABCSize",
    "Credo.Check.Refactor.AppendSingleItem",
    "Credo.Check.Refactor.DoubleBooleanNegation",
    "Credo.Check.Refactor.FilterReject",
    "Credo.Check.Refactor.IoPuts",
    "Credo.Check.Refactor.ModuleDependencies",
    "Credo.Check.Refactor.NegatedIsNil",
    "Credo.Check.Refactor.PipeChainStart",
    "Credo.Check.Refactor.RejectFilter",
    "Credo.Check.Refactor.VariableRebinding",
    "Credo.Check.Warning.ApplicationConfigInModuleAttribute",
    "Credo.Check.Warning.LeakyEnvironment",
    "Credo.Check.Warning.MapGetUnsafePass",
];

/// Checks tagged `:experimental` at the pinned commit (1).
const EXPERIMENTAL_TAGGED: &[&str] = &["Credo.Check.Readability.AliasAs"];

/// Tags carried by a check at the pinned commit (`tags:` in
/// `use Credo.Check`; unlisted checks carry none). Mirrors the native
/// `--checks-with-tag` matching over atoms.
#[must_use]
pub fn check_tags(rule: &str) -> &'static [&'static str] {
    if FORMATTER_TAGGED.contains(&rule) {
        &["formatter"]
    } else if CONTROVERSIAL_TAGGED.contains(&rule) {
        &["controversial"]
    } else if EXPERIMENTAL_TAGGED.contains(&rule) {
        &["experimental"]
    } else {
        &[]
    }
}

/// Checks the native pipeline skips on the pinned toolchain
/// (Elixir 1.20.2) via `elixir_version` requirements
/// (`prepare_checks_to_run.ex:72-88` evaluates
/// `Version.match?(System.version(), check.elixir_version())` and emits
/// zero issues for non-matching checks). Verified on the pinned checkout:
/// `Version.match?("1.20.2", "< 1.7.0")`, `"< 1.7.0-dev"` and `"< 1.8.0"`
/// are false; `">= 1.14.0-dev"`, `">= 1.17.0"` and the default
/// `">= 0.0.1"` are true. Applies at execution level only (native `run/2`
/// bypasses the gate, like [`crate::check_kernel`]).
#[must_use]
pub fn version_skipped_on_pinned_toolchain(rule: &str) -> bool {
    matches!(
        rule,
        "Credo.Check.Warning.LazyLogging"
            | "Credo.Check.Readability.PreferUnquotedAtoms"
            | "Credo.Check.Refactor.MapInto"
    )
}

/// True when a check runs at all under `min_priority`: native
/// `PrepareChecksToRun` excludes checks whose base priority is below
/// `min_priority - 9`, before any file runs. Issue-level `>=` filtering
/// still applies to what runs.
#[must_use]
pub fn runs_at_min_priority(rule: &str, min_priority: i32) -> bool {
    let base = match base_priority(rule) {
        None => return true,
        Some(CheckBase::Higher) => 20,
        Some(CheckBase::High) => 10,
        Some(CheckBase::Normal) => 1,
        Some(CheckBase::Low) => -10,
        Some(CheckBase::DefaultZero) => 0,
    };
    base >= min_priority - 9
}

/// Resolve an issue priority: general override, `priority` param, or base.
/// Mirrors `Params.priority/2` with `Priority.to_integer/1` (`nil` → 0).
///
/// # Errors
/// Returns [`PriorityError`] for unparsable values, like upstream raises.
pub fn resolve_priority(
    rule: &str,
    general: Option<i32>,
    params: &BTreeMap<String, String>,
) -> Result<i32, PriorityError> {
    if let Some(priority) = general {
        return Ok(priority);
    }
    if let Some(raw) = params.get("priority") {
        return to_integer(raw);
    }
    Ok(match base_priority(rule) {
        None => return Err(PriorityError(format!("unknown check `{rule}`"))),
        Some(CheckBase::Higher) => 20,
        Some(CheckBase::High) => 10,
        Some(CheckBase::Normal) => 1,
        Some(CheckBase::Low) => -10,
        Some(CheckBase::DefaultZero) => 0,
    })
}

/// `Priority.to_integer/1`: numbers pass through, names map, `nil` is 0.
/// Leading whitespace parses like upstream; trailing junk errors.
fn to_integer(raw: &str) -> Result<i32, PriorityError> {
    if let Ok(value) = raw.trim_start().parse::<i32>() {
        return Ok(value);
    }
    match raw {
        "higher" => Ok(20),
        "high" => Ok(10),
        "normal" => Ok(1),
        "low" => Ok(-10),
        "ignore" => Ok(-100),
        _ => Err(PriorityError(format!("invalid priority `{raw}`"))),
    }
}

/// Resolve an issue exit status: general override, `exit_status` param
/// (integer or category atom via the category map, unknown atoms yield 0),
/// or the check default, which is always its category value (verified: no
/// check overrides `exit_status` in `use`).
#[must_use]
pub fn resolve_exit_status(
    rule: &str,
    general: Option<i32>,
    params: &BTreeMap<String, String>,
) -> i32 {
    if let Some(status) = general {
        return status;
    }
    if let Some(raw) = params.get("exit_status") {
        if let Ok(status) = raw.parse::<i32>() {
            return status;
        }
        // Atom form goes through the category map; unknown atoms yield 0.
        return match raw.as_str() {
            "consistency" => 1,
            "design" => 2,
            "readability" => 4,
            "refactor" => 8,
            "warning" => 16,
            _ => 0,
        };
    }
    category_for(rule).map_or(0, Category::default_exit_status)
}

/// Resolve an issue category: general override, `category` param, or the
/// check default from its module path. Unknown category names fall back to
/// the check default (documented deviation: upstream keeps open atoms
/// while our `Category` is closed).
#[must_use]
pub fn resolve_category(
    rule: &str,
    general: Option<Category>,
    params: &BTreeMap<String, String>,
) -> Option<Category> {
    if let Some(category) = general {
        return Some(category);
    }
    if let Some(raw) = params.get("category") {
        let parsed = match raw.as_str() {
            "consistency" => Some(Category::Consistency),
            "design" => Some(Category::Design),
            "readability" => Some(Category::Readability),
            "refactor" => Some(Category::Refactor),
            "warning" => Some(Category::Warning),
            _ => None,
        };
        if parsed.is_some() {
            return parsed;
        }
    }
    category_for(rule)
}

/// Full-issue construction failure: unknown rule or invalid priority.
#[derive(Debug, PartialEq, Eq)]
pub struct IssueError(pub String);

/// Per-file cached metadata for issue building: source lines, scopes and
/// scope bonuses, each computed once (upstream caches the same per file).
pub struct FileMeta<'a> {
    /// Project-relative filename for built issues.
    pub filename: &'a str,
    /// Original source text.
    pub source: &'a str,
    /// Raw lines for trigger-column backfill.
    pub lines: Vec<&'a str>,
    /// Scope names per line.
    pub scopes: Scopes,
    /// Scope priority bonuses by scope name.
    pub bonuses: std::collections::BTreeMap<String, i32>,
}

impl<'a> FileMeta<'a> {
    /// Collect all per-file metadata in one pass.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    #[must_use]
    pub fn collect(filename: &'a str, source: &'a str) -> Self {
        match crate::ts_parser::parse(source) {
            Some(tree) => {
                let facts = crate::facts::extract(&tree, source);
                Self::collect_shared(filename, source, &facts)
            }
            None => Self {
                filename,
                source,
                lines: source.split('\n').collect(),
                scopes: Scopes::collect(""),
                bonuses: std::collections::BTreeMap::new(),
            },
        }
    }

    /// Collect metadata over shared single-walk facts: the pipeline shares
    /// one parse and one walk per file across facts, scopes, and bonuses.
    /// Empty facts report empty scopes and bonuses, like an unavailable
    /// grammar.
    #[must_use]
    pub fn collect_shared(filename: &'a str, source: &'a str, facts: &crate::facts::Facts) -> Self {
        Self {
            filename,
            source,
            lines: source.split('\n').collect(),
            scopes: Scopes::collect_on_facts(facts),
            bonuses: scope_priorities_on_facts(facts),
        }
    }
}

/// Build a full [`Issue`] from a kernel finding, mirroring `put_issue` and
/// `format_issue`: resolved category, base priority plus scope bonus,
/// severity default, exit status, trigger-searched column backfill and
/// scope name.
///
/// # Errors
/// Returns [`IssueError`] for unknown rules or invalid priority params.
pub fn build_issue(
    rule: &str,
    finding: Finding,
    params: &BTreeMap<String, String>,
    general: &GeneralParams,
    meta: &FileMeta<'_>,
) -> Result<Issue, IssueError> {
    let category = resolve_category(rule, general.category, params)
        .ok_or_else(|| IssueError(format!("unknown check `{rule}`")))?;
    let base =
        resolve_priority(rule, general.priority, params).map_err(|error| IssueError(error.0))?;
    let scope = meta.scopes.at(finding.line).to_owned();
    let bonus = meta.bonuses.get(&scope).copied().unwrap_or(0);
    let column = finding.column.or_else(|| match &finding.trigger {
        Trigger::Text(trigger) => meta
            .lines
            .get(finding.line.wrapping_sub(1))
            .and_then(|line| backfill_column(line, trigger)),
        Trigger::NoTrigger => None,
    });
    Ok(Issue {
        check: rule.to_owned(),
        category,
        priority: base + bonus,
        severity: finding.severity.unwrap_or(1.0),
        message: finding.message,
        filename: meta.filename.to_owned(),
        line_no: Some(finding.line),
        column,
        exit_status: resolve_exit_status(rule, general.exit_status, params),
        trigger: match finding.trigger {
            Trigger::Text(text) => IssueTrigger::Text(text),
            Trigger::NoTrigger => IssueTrigger::NoTrigger,
        },
        scope: Some(scope),
    })
}
/// `Severity.compute/2` for integer counts (complexities, arities,
/// depths); exact below 2^53 like upstream integers.
#[must_use]
pub fn severity_count(actual: usize, max: usize) -> f64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "complexity counts never approach 2^53"
    )]
    severity(actual as f64, max as f64)
}

/// `Severity.compute/2`: ratio as float, `65536.0` for a zero maximum.
#[must_use]
pub fn severity(actual: f64, max: f64) -> f64 {
    if max == 0.0 { 65536.0 } else { actual / max }
}

/// Trigger-searched 1-based BYTE column mirroring `SourceFile.column/3`.
///
/// Used when a finding carries a trigger but no column. Byte-based like
/// upstream (multibyte lines differ from char columns), `None` on no match.
#[must_use]
pub fn backfill_column(line: &str, trigger: &str) -> Option<usize> {
    let pattern = format!(
        r"(\s|\b|\(|\)|,)({})(\s|\b|\(|\)|,)",
        regex::escape(trigger)
    );
    let expression = regex::Regex::new(&pattern).ok()?;
    let captures = expression.captures(line)?;
    Some(captures.get(2)?.start() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_priorities_match_upstream_table() {
        assert_eq!(
            base_priority("Credo.Check.Warning.IoInspect"),
            Some(CheckBase::High)
        );
        assert_eq!(
            base_priority("Credo.Check.Readability.TrailingBlankLine"),
            Some(CheckBase::Low)
        );
        assert_eq!(
            base_priority("Credo.Check.Refactor.CyclomaticComplexity"),
            Some(CheckBase::DefaultZero)
        );
        assert_eq!(base_priority("Credo.Check.Nope"), None);
    }

    #[test]
    fn categories_derive_from_module_path() {
        assert_eq!(
            category_for("Credo.Check.Consistency.TabsOrSpaces"),
            Some(Category::Consistency)
        );
        assert_eq!(
            category_for("Credo.Check.Warning.WrongTestFilename"),
            Some(Category::Warning)
        );
        assert_eq!(category_for("Credo.Check.Nope"), None);
    }

    #[test]
    fn tags_match_upstream_table() {
        assert_eq!(
            check_tags("Credo.Check.Readability.TrailingWhiteSpace"),
            &["formatter"]
        );
        assert_eq!(
            check_tags("Credo.Check.Refactor.DoubleBooleanNegation"),
            &["controversial"]
        );
        assert_eq!(
            check_tags("Credo.Check.Readability.AliasAs"),
            &["experimental"]
        );
        assert_eq!(check_tags("Credo.Check.Warning.IoInspect"), &[] as &[&str]);
        assert_eq!(check_tags("Credo.Check.Nope"), &[] as &[&str]);
    }

    #[test]
    fn version_gate_matches_pinned_toolchain() {
        // Native skips these on Elixir 1.20.2 (`< 1.7.0`, `< 1.7.0-dev`,
        // `< 1.8.0`); running gates and the default stay enabled.
        assert!(version_skipped_on_pinned_toolchain(
            "Credo.Check.Warning.LazyLogging"
        ));
        assert!(version_skipped_on_pinned_toolchain(
            "Credo.Check.Readability.PreferUnquotedAtoms"
        ));
        assert!(version_skipped_on_pinned_toolchain(
            "Credo.Check.Refactor.MapInto"
        ));
        assert!(!version_skipped_on_pinned_toolchain(
            "Credo.Check.Warning.Dbg"
        ));
        assert!(!version_skipped_on_pinned_toolchain(
            "Credo.Check.Refactor.PreferDateTimeShift"
        ));
        assert!(!version_skipped_on_pinned_toolchain(
            "Credo.Check.Warning.IoInspect"
        ));
    }

    #[test]
    fn priority_pre_exclusion_matches_native_threshold() {
        // Checks run iff base >= min_priority - 9.
        let low = "Credo.Check.Readability.TrailingWhiteSpace";
        assert!(!runs_at_min_priority(low, 0));
        assert!(runs_at_min_priority(low, -99));
        assert!(runs_at_min_priority(low, -1));
        let normal = "Credo.Check.Design.TagTODO";
        assert!(runs_at_min_priority(normal, 0));
        assert!(!runs_at_min_priority(normal, 10));
        assert!(runs_at_min_priority("Credo.Check.Nope", 999));
    }

    #[test]
    fn general_priority_overrides_base() {
        assert_eq!(
            resolve_priority("Credo.Check.Warning.IoInspect", Some(42), &BTreeMap::new()),
            Ok(42)
        );
    }

    #[test]
    fn priority_param_beats_base() {
        let mut params = BTreeMap::new();
        params.insert("priority".to_owned(), "low".to_owned());
        assert_eq!(
            resolve_priority("Credo.Check.Warning.IoInspect", None, &params),
            Ok(-10)
        );
        params.insert("priority".to_owned(), "3".to_owned());
        assert_eq!(
            resolve_priority("Credo.Check.Warning.IoInspect", None, &params),
            Ok(3)
        );
    }

    #[test]
    fn missing_priority_falls_back_to_base() {
        assert_eq!(
            resolve_priority("Credo.Check.Warning.IoInspect", None, &BTreeMap::new()),
            Ok(10)
        );
        assert_eq!(
            resolve_priority(
                "Credo.Check.Refactor.CyclomaticComplexity",
                None,
                &BTreeMap::new()
            ),
            Ok(0)
        );
    }

    #[test]
    fn invalid_priority_is_explicit() {
        let mut params = BTreeMap::new();
        params.insert("priority".to_owned(), "bogus".to_owned());
        assert!(resolve_priority("Credo.Check.Warning.IoInspect", None, &params).is_err());
    }

    #[test]
    fn exit_status_defaults_to_category() {
        assert_eq!(
            resolve_exit_status("Credo.Check.Warning.IoInspect", None, &BTreeMap::new()),
            16
        );
        let mut params = BTreeMap::new();
        params.insert("exit_status".to_owned(), "2".to_owned());
        assert_eq!(
            resolve_exit_status("Credo.Check.Design.TagTODO", None, &params),
            2
        );
    }

    #[test]
    fn unknown_category_falls_back_to_check_default() {
        assert_eq!(
            resolve_category("Credo.Check.Warning.IoInspect", None, &BTreeMap::new()),
            Some(Category::Warning)
        );
        let mut params = BTreeMap::new();
        params.insert("category".to_owned(), "readability".to_owned());
        assert_eq!(
            resolve_category("Credo.Check.Warning.IoInspect", None, &params),
            Some(Category::Readability)
        );
        params.insert("category".to_owned(), "bogus".to_owned());
        assert_eq!(
            resolve_category("Credo.Check.Warning.IoInspect", None, &params),
            Some(Category::Warning)
        );
    }

    #[test]
    fn table_covers_ledger_exactly() {
        let text = include_str!("../compatibility/rules.json");
        let json: serde_json::Value = serde_json::from_str(text).expect("valid JSON");
        let rules = json["rules"].as_array().expect("rules array");
        assert_eq!(rules.len(), 120);
        for rule in rules {
            let id = rule["id"].as_str().expect("id");
            assert!(base_priority(id).is_some(), "{id}");
            assert!(category_for(id).is_some(), "{id}");
        }
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "exact IEEE equality with the upstream formula is the contract"
    )]
    fn severity_matches_upstream_arithmetic() {
        assert_eq!(severity(10.0, 9.0), 10.0 / 9.0);
        assert_eq!(severity(3.0, 2.0), 1.5);
        assert_eq!(severity(5.0, 0.0), 65536.0);
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "default severity is exactly representable"
    )]
    fn build_issue_produces_full_shape() {
        use crate::{Finding, Trigger};
        let source = "defmodule M do\n  IO.inspect(x)\nend\n";
        let meta = FileMeta::collect("lib/a.ex", source);
        let finding = Finding {
            line: 2,
            column: None,
            message: "Do not use `IO.inspect`.".to_owned(),
            trigger: Trigger::Text("IO.inspect".to_owned()),
            severity: None,
        };
        let issue = build_issue(
            "Credo.Check.Warning.IoInspect",
            finding,
            &BTreeMap::new(),
            &GeneralParams::default(),
            &meta,
        )
        .expect("builds");
        assert_eq!(issue.check, "Credo.Check.Warning.IoInspect");
        assert_eq!(issue.category, Category::Warning);
        assert_eq!(issue.priority, 11);
        assert_eq!(issue.severity, 1.0);
        assert_eq!(issue.filename, "lib/a.ex");
        assert_eq!(issue.line_no, Some(2));
        assert_eq!(issue.column, Some(3));
        assert_eq!(issue.exit_status, 16);
        assert_eq!(issue.scope.as_deref(), Some("M"));
    }

    #[test]
    fn build_issue_rejects_unknown_rules() {
        use crate::Finding;
        let meta = FileMeta::collect("a.ex", "x = 1\n");
        let finding = Finding::no_trigger(1, "x");
        assert!(
            build_issue(
                "Credo.Check.Nope",
                finding,
                &BTreeMap::new(),
                &GeneralParams::default(),
                &meta
            )
            .is_err()
        );
    }

    #[test]
    fn backfill_column_matches_native_byte_columns() {
        assert_eq!(backfill_column("  def foo(x), do: x", "foo"), Some(7));
        assert_eq!(backfill_column("  # héllo foo", "foo"), Some(12));
        assert_eq!(backfill_column("  def foo(x), do: x", "missing"), None);
        assert_eq!(backfill_column("x = foobar", "foo"), None);
        // Leading tab trigger: nothing precedes it for group one, and
        // `\b` needs a word character first (verified native `nil`).
        assert_eq!(backfill_column("\tdef f, do: 1", "\t"), None);
        assert_eq!(backfill_column("\tdef f, do: 1", "def"), Some(2));
    }
}
