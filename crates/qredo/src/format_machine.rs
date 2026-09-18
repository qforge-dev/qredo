//! Byte-exact machine formatters: `oneline`, `flycheck`, `json`, `sarif`.
//!
//! These renderers reproduce the piped stdout bytes of the pinned Credo
//! (`work/ref/credo`, `ea1ccb9`) for the four machine formats. Native
//! stdout is never colorized when piped, so the renderers emit plain text.
//! See `work/p1-formats-probe.md` for the original capture and the notes
//! below for every derivation re-probed for this module.
//!
//! ## Ordering (re-probed, corrects the probe doc on one point)
//!
//! * `oneline` sorts by `(filename, line_no, column)` inside the formatter
//!   (`formatter/oneline.ex`).
//! * `flycheck` and `json` do **not** sort: they print
//!   `Execution.get_issues/1` order, which groups by filename (Elixir
//!   flatmaps enumerate ascending) with the `SetRelevantIssues` order
//!   `(check id, filename, line_no)` inside each file. The probe doc calls
//!   this "file order", which coincides with line order only when check-id
//!   order happens to match line order. A swapped fixture (`dbg` on line 3,
//!   `IO.inspect` on line 4) prints `IoInspect` before `Dbg` in
//!   flycheck/json but line order in oneline.
//! * `sarif` maps the same input order, then `sum_rules_and_results`
//!   prepends each result, so `results` are the exact reverse while `rules`
//!   keep first-seen order (`uniq_by` over the input).
//!
//! ## Field derivations
//!
//! * Severity tag: first category letter uppercased, except `refactor` →
//!   `F` (`@category_tag_map` in `cli/output.ex`).
//! * Arrow: priority-based, not category-based (`Priority.to_atom/1` then
//!   `priority_arrow/1`): `> 19 → ↑`, `10..=19 → ↗`, `0..=9 → →`,
//!   `-10..=-1 → ↘`, `< -10 → ↓`.
//! * `column_end` (json) / `endColumn` (sarif): `column +
//!   String.length(trigger)` when both are present, else absent/null. Length
//!   is Elixir `String.length/1` (grapheme clusters); the renderer uses
//!   Unicode scalar count, which agrees on ASCII (residual below).
//! * SARIF `rank`: `priority + 11`, clamped to `1` below `-10` and `100`
//!   above `89`. SARIF `level`: `priority >= 10 → "error"`,
//!   `0..=9 → key omitted`, `priority < 0 → "note"`.
//! * SARIF `text` variants (message and rule descriptions) replace every
//!   backtick with a single quote; `markdown` keeps backticks.
//! * SARIF `uri`: `Path.relative_to(filename, invocation path)`; outside
//!   files keep their absolute path. `ROOTPATH` is `to_file_uri` of the
//!   invocation path (the Mix cwd, or the leading positional directory).
//! * Rule `id` is the check's `id/0` from the pinned 120-check table
//!   (`EX3009`, `EX5006`, `EX5026`, ...); explicit context docs override it
//!   and unknown checks fall back to the full module name, and then the
//!   rule `name` key is omitted (`remove_redundant_name`).
//!
//! ## Gaps and residuals
//!
//! * Our [`Issue`] has no `column_end` field; it is derived as above. For
//!   the fixture (`Smells`/`IO.inspect`/`dbg`) the derivation is byte-exact.
//! * Our [`IssueTrigger::NoTrigger`] maps to JSON `null` and an empty SARIF
//!   snippet with no `endColumn`. Real Credo **crashes** (`String.Chars`
//!   for tuples) if the raw `no_trigger` sentinel reaches these
//!   formatters, so no native output exists to diverge from.
//! * `String.length/1` counts grapheme clusters; exotic multi-scalar
//!   triggers could shift `column_end` by a cluster boundary.
//! * Filenames sort ascending; Elixir maps with more than 32 files use hash
//!   order (upstream itself is unstable there), while the renderer stays
//!   ascending.
//! * Rule explanations and help URIs come from [`MachineContext::rule_docs`]
//!   (unknown checks fall back to the hexdocs URL pattern with an empty
//!   explanation); rule *ids* additionally consult the pinned table, so the
//!   wiring layer only needs to supply prose. The wiring layer must supply
//!   the pinned texts.
//! * `nil` line/column sort after numbers (Elixir term order), replicated
//!   by [`none_last`] comparison.
//!
//! Do not confuse [`render_json`] with the existing JSONL `--format json`
//! CLI output: that shape (one object per line plus a summary) is consumed
//! by `scripts/differential` and must stay untouched. [`render_json`] is the
//! native-shape `{"issues": [...]}` renderer for future wiring.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::issue::{Category, Issue, IssueTrigger};
use crate::runner::RunReport;

/// Pinned Credo version reported in the SARIF driver block.
pub const PINNED_CREDO_VERSION: &str = "1.8.0-dev";

/// SARIF `$schema` URI emitted by the pinned Credo.
pub const SARIF_SCHEMA_URI: &str =
    "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0-rtm.5.json";

/// SARIF `version` string emitted by the pinned Credo.
pub const SARIF_VERSION: &str = "2.1.0";

/// Check documentation backing one SARIF rule entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDoc {
    /// Check `id/0`, e.g. `EX3009`.
    pub id: String,
    /// `explanation/0` markdown source; the `text` variant derives from it.
    pub explanation: String,
    /// `docs_uri/0`, e.g. the hexdocs check page.
    pub help_uri: String,
}

impl RuleDoc {
    /// Build one rule document from its parts.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        explanation: impl Into<String>,
        help_uri: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            explanation: explanation.into(),
            help_uri: help_uri.into(),
        }
    }

    /// Credo default for a check without pinned docs: the rule id falls back
    /// to the full check module name (so the rule `name` key is omitted) and
    /// the help URI follows the default hexdocs pattern. The explanation
    /// stays empty rather than fabricating check prose.
    #[must_use]
    pub fn fallback(check: &str) -> Self {
        Self {
            id: check.to_owned(),
            explanation: String::new(),
            help_uri: format!("https://hexdocs.pm/credo/{check}.html"),
        }
    }
}

/// Invocation context for the machine formatters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineContext {
    /// Invocation directory (or the leading positional directory): feeds the
    /// SARIF `ROOTPATH` URI and artifact relativization.
    pub root_dir: PathBuf,
    /// Credo version string for the SARIF driver block.
    pub credo_version: String,
    /// Check module name to rule documentation for the SARIF rules section.
    pub rule_docs: BTreeMap<String, RuleDoc>,
}

impl MachineContext {
    /// Context rooted at `root_dir` with the pinned Credo version and no
    /// rule docs (unknown checks use [`RuleDoc::fallback`]).
    #[must_use]
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
            credo_version: PINNED_CREDO_VERSION.to_owned(),
            rule_docs: BTreeMap::new(),
        }
    }

    /// Register one check's rule documentation.
    #[must_use]
    pub fn with_rule_doc(mut self, check: impl Into<String>, doc: RuleDoc) -> Self {
        self.rule_docs.insert(check.into(), doc);
        self
    }

    /// Override the SARIF driver version (default [`PINNED_CREDO_VERSION`]).
    #[must_use]
    pub fn with_credo_version(mut self, version: impl Into<String>) -> Self {
        self.credo_version = version.into();
        self
    }

    /// Rule document for a check: pinned docs or the Credo default.
    fn rule_doc(&self, check: &str) -> RuleDoc {
        self.rule_docs
            .get(check)
            .cloned()
            .unwrap_or_else(|| RuleDoc::fallback(check))
    }

    /// Rule id for a check: explicit docs first (a check declares its own
    /// id), then the pinned upstream table, else the full module name
    /// (Credo's default `id/0`).
    fn rule_id(&self, check: &str) -> String {
        if let Some(doc) = self.rule_docs.get(check) {
            return doc.id.clone();
        }
        pinned_check_id(check).unwrap_or(check).to_owned()
    }
}

/// `oneline` output: `[<SEV>] <arrow> <path>:<line>:<col> <message>` per
/// issue, sorted by file, line and column.
#[must_use]
pub fn render_oneline(report: &RunReport, _ctx: &MachineContext) -> String {
    let mut ordered: Vec<&Issue> = report.issues.iter().collect();
    ordered.sort_by(|left, right| {
        left.filename
            .cmp(&right.filename)
            .then_with(|| none_last(left.line_no, right.line_no))
            .then_with(|| none_last(left.column, right.column))
    });
    let mut out = String::new();
    for issue in ordered {
        // `write!` to a `String` cannot fail; the result is ignored.
        let _ = writeln!(
            out,
            "[{}] {} {}{} {}",
            severity_tag(issue.category),
            priority_arrow(issue.priority),
            issue.filename,
            pos_suffix(issue.line_no, issue.column),
            issue.message
        );
    }
    out
}

/// `flycheck` output: `<path>:<line>:<col>: <SEV>: <message>` per issue in
/// `Execution.get_issues/1` order (filename ascending, check id, line).
#[must_use]
pub fn render_flycheck(report: &RunReport, ctx: &MachineContext) -> String {
    let mut out = String::new();
    for issue in machine_order(&report.issues, ctx) {
        // `write!` to a `String` cannot fail; the result is ignored.
        let _ = writeln!(
            out,
            "{}{}: {}: {}",
            issue.filename,
            pos_suffix(issue.line_no, issue.column),
            severity_tag(issue.category),
            issue.message
        );
    }
    out
}

/// Native-shape `json` output: a single 2-space pretty object with exactly
/// the `issues` key. This is **not** the JSONL `--format json` CLI output.
#[must_use]
pub fn render_json(report: &RunReport, ctx: &MachineContext) -> String {
    let issues = machine_order(&report.issues, ctx)
        .into_iter()
        .map(JsonIssue::from_native)
        .collect();
    let document = JsonDocument { issues };
    let mut out = String::new();
    out.push_str(&serde_json::to_string_pretty(&document).unwrap_or_default());
    out.push('\n');
    out
}

/// `sarif` output: the full 2.1.0 document with reversed results and
/// first-seen rules.
#[must_use]
pub fn render_sarif(report: &RunReport, ctx: &MachineContext) -> String {
    let ordered = machine_order(&report.issues, ctx);
    let mut seen: Vec<String> = Vec::new();
    let mut rules: Vec<SarifRule> = Vec::new();
    for issue in &ordered {
        let rule_id = ctx.rule_id(&issue.check);
        if !seen.contains(&rule_id) {
            seen.push(rule_id.clone());
            let doc = ctx.rule_doc(&issue.check);
            rules.push(SarifRule::new(
                &issue.check,
                issue.category.as_str(),
                &rule_id,
                &doc,
            ));
        }
    }
    let results: Vec<SarifResult> = ordered
        .iter()
        .rev()
        .map(|&issue| SarifResult::new(issue, &ctx.root_dir, &ctx.rule_id(&issue.check)))
        .collect();
    let document = SarifDocument {
        schema: SARIF_SCHEMA_URI,
        version: SARIF_VERSION,
        runs: vec![SarifRun {
            column_kind: "utf16CodeUnits",
            uri_bases: SarifUriBases {
                rootpath: SarifUriBase {
                    uri: file_uri(&ctx.root_dir),
                },
            },
            results,
            tool: SarifTool {
                driver: SarifDriver {
                    information_uri: "http://credo-ci.org/".to_owned(),
                    name: "Credo".to_owned(),
                    rules,
                    version: ctx.credo_version.clone(),
                },
            },
        }],
    };
    let mut out = String::new();
    out.push_str(&serde_json::to_string_pretty(&document).unwrap_or_default());
    out.push('\n');
    out
}

/// Elixir term order for optional positions: numbers sort before `nil`.
fn none_last(left: Option<usize>, right: Option<usize>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => left.cmp(&right),
    }
}

/// `Execution.get_issues/1` order: filenames ascending, then check id, line
/// and column within each file.
fn machine_order<'a>(issues: &'a [Issue], ctx: &MachineContext) -> Vec<&'a Issue> {
    let ranks = machine_check_ranks(issues, ctx);
    let mut keyed: Vec<(usize, &'a Issue)> = issues
        .iter()
        .map(|issue| (ranks[issue.check.as_str()], issue))
        .collect();
    keyed.sort_by(|(left_rank, left), (right_rank, right)| {
        left.filename
            .cmp(&right.filename)
            .then_with(|| left_rank.cmp(right_rank))
            .then_with(|| none_last(left.line_no, right.line_no))
            .then_with(|| none_last(left.column, right.column))
    });
    keyed.into_iter().map(|(_, issue)| issue).collect()
}

/// Lexicographic rule-id rank per distinct check. Resolving once per check
/// avoids allocating the same rule-id string for every issue.
fn machine_check_ranks<'a>(issues: &'a [Issue], ctx: &MachineContext) -> BTreeMap<&'a str, usize> {
    let mut ids: BTreeMap<&str, String> = BTreeMap::new();
    for issue in issues {
        ids.entry(&issue.check)
            .or_insert_with(|| ctx.rule_id(&issue.check));
    }
    let mut sorted: Vec<(&str, String)> = ids.into_iter().collect();
    sorted.sort_by(|(left_check, left_id), (right_check, right_id)| {
        left_id
            .cmp(right_id)
            .then_with(|| left_check.cmp(right_check))
    });
    sorted
        .into_iter()
        .enumerate()
        .map(|(rank, (check, _))| (check, rank))
        .collect()
}

/// Pinned check `id/0` table for the 120 upstream checks (`ea1ccb9`).
/// Unknown checks fall back to their full module name, mirroring
/// Credo's default `id/0`.
fn pinned_check_id(check: &str) -> Option<&'static str> {
    consistency_id(check)
        .or_else(|| design_id(check))
        .or_else(|| readability_id(check))
        .or_else(|| refactor_id(check))
        .or_else(|| warning_id(check))
}

/// Pinned check ids for the consistency checks.
fn consistency_id(check: &str) -> Option<&'static str> {
    match check {
        "Credo.Check.Consistency.ExceptionNames" => Some("EX1001"),
        "Credo.Check.Consistency.LineEndings" => Some("EX1002"),
        "Credo.Check.Consistency.MultiAliasImportRequireUse" => Some("EX1003"),
        "Credo.Check.Consistency.ParameterPatternMatching" => Some("EX1004"),
        "Credo.Check.Consistency.SpaceAroundOperators" => Some("EX1005"),
        "Credo.Check.Consistency.SpaceInParentheses" => Some("EX1006"),
        "Credo.Check.Consistency.TabsOrSpaces" => Some("EX1007"),
        "Credo.Check.Consistency.UnusedVariableNames" => Some("EX1008"),
        _ => None,
    }
}

/// Pinned check ids for the design checks.
fn design_id(check: &str) -> Option<&'static str> {
    match check {
        "Credo.Check.Design.AliasUsage" => Some("EX2001"),
        "Credo.Check.Design.DeprecatedChecksConfig" => Some("EX2008"),
        "Credo.Check.Design.DuplicatedCode" => Some("EX2002"),
        "Credo.Check.Design.MissingCheckInConfig" => Some("EX2007"),
        "Credo.Check.Design.RedundantConfigComments" => Some("EX2006"),
        "Credo.Check.Design.SkipTestWithoutComment" => Some("EX2003"),
        "Credo.Check.Design.TagFIXME" => Some("EX2004"),
        "Credo.Check.Design.TagTODO" => Some("EX2005"),
        _ => None,
    }
}

/// Pinned check ids for the readability checks.
fn readability_id(check: &str) -> Option<&'static str> {
    match check {
        "Credo.Check.Readability.AliasAs" => Some("EX3001"),
        "Credo.Check.Readability.AliasOrder" => Some("EX3002"),
        "Credo.Check.Readability.BlockPipe" => Some("EX3003"),
        "Credo.Check.Readability.CaptureOperator" => Some("EX3099"),
        "Credo.Check.Readability.FunctionNames" => Some("EX3004"),
        "Credo.Check.Readability.ImplTrue" => Some("EX3036"),
        "Credo.Check.Readability.LargeNumbers" => Some("EX3006"),
        "Credo.Check.Readability.MaxLineLength" => Some("EX3007"),
        "Credo.Check.Readability.ModuleAttributeNames" => Some("EX3008"),
        "Credo.Check.Readability.ModuleDoc" => Some("EX3009"),
        "Credo.Check.Readability.ModuleNames" => Some("EX3010"),
        "Credo.Check.Readability.MultiAlias" => Some("EX3011"),
        "Credo.Check.Readability.NestedFunctionCalls" => Some("EX3012"),
        "Credo.Check.Readability.OneArityFunctionInPipe" => Some("EX3034"),
        "Credo.Check.Readability.OnePipePerLine" => Some("EX3035"),
        "Credo.Check.Readability.ParenthesesInCondition" => Some("EX3013"),
        "Credo.Check.Readability.ParenthesesOnZeroArityDefs" => Some("EX3014"),
        "Credo.Check.Readability.PipeIntoAnonymousFunctions" => Some("EX3015"),
        "Credo.Check.Readability.PredicateFunctionNames" => Some("EX3016"),
        "Credo.Check.Readability.PreferImplicitTry" => Some("EX3017"),
        "Credo.Check.Readability.PreferUnquotedAtoms" => Some("EX3018"),
        "Credo.Check.Readability.RedundantBlankLines" => Some("EX3019"),
        "Credo.Check.Readability.Semicolons" => Some("EX3020"),
        "Credo.Check.Readability.SeparateAliasRequire" => Some("EX3021"),
        "Credo.Check.Readability.SingleFunctionToBlockPipe" => Some("EX3022"),
        "Credo.Check.Readability.SinglePipe" => Some("EX3023"),
        "Credo.Check.Readability.SpaceAfterCommas" => Some("EX3024"),
        "Credo.Check.Readability.SpecParameterNames" => Some("EX3037"),
        "Credo.Check.Readability.Specs" => Some("EX3025"),
        "Credo.Check.Readability.StrictModuleLayout" => Some("EX3026"),
        "Credo.Check.Readability.StringSigils" => Some("EX3027"),
        "Credo.Check.Readability.TrailingBlankLine" => Some("EX3028"),
        "Credo.Check.Readability.TrailingWhiteSpace" => Some("EX3029"),
        "Credo.Check.Readability.UnnecessaryAliasExpansion" => Some("EX3030"),
        "Credo.Check.Readability.UnusedFunctionParameterPattern" => Some("EX5032"),
        "Credo.Check.Readability.VariableNames" => Some("EX3031"),
        "Credo.Check.Readability.WithCustomTaggedTuple" => Some("EX3032"),
        "Credo.Check.Readability.WithSingleClause" => Some("EX3033"),
        _ => None,
    }
}

/// Pinned check ids for the refactor checks.
fn refactor_id(check: &str) -> Option<&'static str> {
    match check {
        "Credo.Check.Refactor.ABCSize" => Some("EX4001"),
        "Credo.Check.Refactor.AppendSingleItem" => Some("EX4002"),
        "Credo.Check.Refactor.Apply" => Some("EX4003"),
        "Credo.Check.Refactor.CaseTrivialMatches" => Some("EX4004"),
        "Credo.Check.Refactor.CondInsteadOfIfElse" => Some("EX4033"),
        "Credo.Check.Refactor.CondStatements" => Some("EX4005"),
        "Credo.Check.Refactor.CyclomaticComplexity" => Some("EX4006"),
        "Credo.Check.Refactor.DoubleBooleanNegation" => Some("EX4007"),
        "Credo.Check.Refactor.FilterCount" => Some("EX4030"),
        "Credo.Check.Refactor.FilterFilter" => Some("EX4008"),
        "Credo.Check.Refactor.FilterReject" => Some("EX4009"),
        "Credo.Check.Refactor.FunctionArity" => Some("EX4010"),
        "Credo.Check.Refactor.IoPuts" => Some("EX4011"),
        "Credo.Check.Refactor.LongQuoteBlocks" => Some("EX4012"),
        "Credo.Check.Refactor.MapInto" => Some("EX4013"),
        "Credo.Check.Refactor.MapJoin" => Some("EX4014"),
        "Credo.Check.Refactor.MapMap" => Some("EX4015"),
        "Credo.Check.Refactor.MatchInCondition" => Some("EX4016"),
        "Credo.Check.Refactor.ModuleDependencies" => Some("EX4017"),
        "Credo.Check.Refactor.NegatedConditionsInUnless" => Some("EX4018"),
        "Credo.Check.Refactor.NegatedConditionsWithElse" => Some("EX4019"),
        "Credo.Check.Refactor.NegatedIsNil" => Some("EX4020"),
        "Credo.Check.Refactor.Nesting" => Some("EX4021"),
        "Credo.Check.Refactor.PassAsyncInTestCases" => Some("EX4031"),
        "Credo.Check.Refactor.PerceivedComplexity" => Some("EX4022"),
        "Credo.Check.Refactor.PipeChainStart" => Some("EX4023"),
        "Credo.Check.Refactor.PreferDateTimeShift" => Some("EX4034"),
        "Credo.Check.Refactor.RedundantWithClauseResult" => Some("EX4024"),
        "Credo.Check.Refactor.RejectFilter" => Some("EX4025"),
        "Credo.Check.Refactor.RejectReject" => Some("EX4026"),
        "Credo.Check.Refactor.UnlessWithElse" => Some("EX4027"),
        "Credo.Check.Refactor.UtcNowTruncate" => Some("EX4032"),
        "Credo.Check.Refactor.VariableRebinding" => Some("EX4028"),
        "Credo.Check.Refactor.WithClauses" => Some("EX4029"),
        _ => None,
    }
}

/// Pinned check ids for the warning checks.
fn warning_id(check: &str) -> Option<&'static str> {
    match check {
        "Credo.Check.Warning.ApplicationConfigInModuleAttribute" => Some("EX5001"),
        "Credo.Check.Warning.BoolOperationOnSameValues" => Some("EX5002"),
        "Credo.Check.Warning.Dbg" => Some("EX5026"),
        "Credo.Check.Warning.ExpensiveEmptyEnumCheck" => Some("EX5003"),
        "Credo.Check.Warning.ForbiddenFunction" => Some("EX5033"),
        "Credo.Check.Warning.ForbiddenModule" => Some("EX5004"),
        "Credo.Check.Warning.IExPry" => Some("EX5005"),
        "Credo.Check.Warning.IoInspect" => Some("EX5006"),
        "Credo.Check.Warning.LazyLogging" => Some("EX5007"),
        "Credo.Check.Warning.LeakyEnvironment" => Some("EX5008"),
        "Credo.Check.Warning.MapGetUnsafePass" => Some("EX5009"),
        "Credo.Check.Warning.MissedMetadataKeyInLoggerConfig" => Some("EX5027"),
        "Credo.Check.Warning.MixEnv" => Some("EX5010"),
        "Credo.Check.Warning.OperationOnSameValues" => Some("EX5011"),
        "Credo.Check.Warning.OperationWithConstantResult" => Some("EX5012"),
        "Credo.Check.Warning.RaiseInsideRescue" => Some("EX5013"),
        "Credo.Check.Warning.SpecWithStruct" => Some("EX5014"),
        "Credo.Check.Warning.StructFieldAmount" => Some("EX5029"),
        "Credo.Check.Warning.UnsafeExec" => Some("EX5015"),
        "Credo.Check.Warning.UnsafeToAtom" => Some("EX5016"),
        "Credo.Check.Warning.UnusedEnumOperation" => Some("EX5017"),
        "Credo.Check.Warning.UnusedFileOperation" => Some("EX5018"),
        "Credo.Check.Warning.UnusedKeywordOperation" => Some("EX5019"),
        "Credo.Check.Warning.UnusedListOperation" => Some("EX5020"),
        "Credo.Check.Warning.UnusedMapOperation" => Some("EX5028"),
        "Credo.Check.Warning.UnusedOperation" => Some("EX5031"),
        "Credo.Check.Warning.UnusedPathOperation" => Some("EX5021"),
        "Credo.Check.Warning.UnusedRegexOperation" => Some("EX5022"),
        "Credo.Check.Warning.UnusedStringOperation" => Some("EX5023"),
        "Credo.Check.Warning.UnusedTupleOperation" => Some("EX5024"),
        "Credo.Check.Warning.WrongTestFileExtension" => Some("EX5025"),
        "Credo.Check.Warning.WrongTestFilename" => Some("EX5030"),
        _ => None,
    }
}

/// Severity letter: first category letter, except `refactor` → `F`.
fn severity_tag(category: Category) -> &'static str {
    match category {
        Category::Consistency => "C",
        Category::Design => "D",
        Category::Readability => "R",
        Category::Refactor => "F",
        Category::Warning => "W",
    }
}

/// Priority arrow from `Priority.to_atom/1` boundaries.
fn priority_arrow(priority: i32) -> char {
    if priority > 19 {
        '↑'
    } else if priority >= 10 {
        '↗'
    } else if priority >= 0 {
        '→'
    } else if priority >= -10 {
        '↘'
    } else {
        '↓'
    }
}

/// `:line_no:column` suffix; a missing column with a present line gives
/// `:line`, both missing give `""`, and the degenerate missing-line case
/// renders `"::col"` because Elixir interpolates `nil` as `""`.
fn pos_suffix(line_no: Option<usize>, column: Option<usize>) -> String {
    match (line_no, column) {
        (None, None) => String::new(),
        (Some(line), None) => format!(":{line}"),
        (Some(line), Some(column)) => format!(":{line}:{column}"),
        (None, Some(column)) => format!("::{column}"),
    }
}

/// `column + String.length(trigger)` when both exist, else absent.
fn column_end(column: Option<usize>, trigger: &IssueTrigger) -> Option<usize> {
    match (column, trigger) {
        (Some(column), IssueTrigger::Text(trigger)) => Some(column + trigger.chars().count()),
        _ => None,
    }
}

/// Trigger text: the raw string, or `""` for the no-trigger sentinel.
fn trigger_text(trigger: &IssueTrigger) -> &str {
    match trigger {
        IssueTrigger::Text(trigger) => trigger,
        IssueTrigger::NoTrigger => "",
    }
}

/// SARIF `level`: errors stay, normal priorities omit the key, lows are notes.
fn sarif_level(priority: i32) -> Option<&'static str> {
    if priority >= 10 {
        Some("error")
    } else if priority < 0 {
        Some("note")
    } else {
        None
    }
}

/// SARIF `rank`: `priority + 11`, clamped to `1` below `-10` and `100`
/// above `89`.
fn sarif_rank(priority: i32) -> i32 {
    if priority < -10 {
        1
    } else if priority > 89 {
        100
    } else {
        priority + 11
    }
}

/// `to_file_uri` of the invocation path: backslashes become slashes,
/// leading and trailing slashes are trimmed, then wrapped.
fn file_uri(root: &Path) -> String {
    let text = root.to_string_lossy().replace('\\', "/");
    let trimmed = text.trim_start_matches('/').trim_end_matches('/');
    format!("file:///{trimmed}/")
}

/// `Path.relative_to(filename, root)`: under-root files relativize (the
/// degenerate equal case yields `"."`), everything else stays verbatim.
fn artifact_uri(filename: &str, root: &Path) -> String {
    match Path::new(filename).strip_prefix(root) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".to_owned(),
        Ok(relative) => relative.to_string_lossy().into_owned(),
        Err(_) => filename.to_owned(),
    }
}

/// Check name as the JSON formatter prints it: the `Elixir.` prefix is
/// stripped when present.
fn json_check_name(check: &str) -> &str {
    check.strip_prefix("Elixir.").unwrap_or(check)
}

/// One native-shape JSON issue; fields are declared alphabetically because
/// Jason enumerates small maps in key order and serde follows declaration
/// order.
#[derive(Debug, serde::Serialize)]
struct JsonIssue<'a> {
    category: &'a str,
    check: &'a str,
    column: Option<usize>,
    column_end: Option<usize>,
    filename: &'a str,
    line_no: Option<usize>,
    message: &'a str,
    priority: i32,
    scope: Option<&'a str>,
    trigger: Option<&'a str>,
}

impl<'a> JsonIssue<'a> {
    fn from_native(issue: &'a Issue) -> Self {
        Self {
            category: issue.category.as_str(),
            check: json_check_name(&issue.check),
            column: issue.column,
            column_end: column_end(issue.column, &issue.trigger),
            filename: &issue.filename,
            line_no: issue.line_no,
            message: &issue.message,
            priority: issue.priority,
            scope: issue.scope.as_deref(),
            trigger: match &issue.trigger {
                IssueTrigger::Text(trigger) => Some(trigger.as_str()),
                IssueTrigger::NoTrigger => None,
            },
        }
    }
}

/// Top-level JSON document: exactly the `issues` key.
#[derive(Debug, serde::Serialize)]
struct JsonDocument<'a> {
    issues: Vec<JsonIssue<'a>>,
}

/// Full SARIF document; every struct declares fields alphabetically to match
/// Jason key order.
#[derive(Debug, serde::Serialize)]
struct SarifDocument {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<SarifRun>,
}

/// Single SARIF run.
#[derive(Debug, serde::Serialize)]
struct SarifRun {
    #[serde(rename = "columnKind")]
    column_kind: &'static str,
    #[serde(rename = "originalUriBaseIds")]
    uri_bases: SarifUriBases,
    results: Vec<SarifResult>,
    tool: SarifTool,
}

/// `originalUriBaseIds` wrapper holding only `ROOTPATH`.
#[derive(Debug, serde::Serialize)]
struct SarifUriBases {
    #[serde(rename = "ROOTPATH")]
    rootpath: SarifUriBase,
}

/// One named URI base.
#[derive(Debug, serde::Serialize)]
struct SarifUriBase {
    uri: String,
}

/// One SARIF result; `level` is omitted for normal priorities.
#[derive(Debug, serde::Serialize)]
struct SarifResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<&'static str>,
    locations: Vec<SarifLocation>,
    message: SarifMessage,
    rank: i32,
    #[serde(rename = "ruleId")]
    rule_id: String,
}

impl SarifResult {
    fn new(issue: &Issue, root: &Path, rule_id: &str) -> Self {
        let trigger = trigger_text(&issue.trigger);
        Self {
            level: sarif_level(issue.priority),
            locations: vec![SarifLocation {
                logical_locations: vec![SarifLogicalLocation {
                    fully_qualified_name: issue.scope.clone(),
                }],
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation {
                        uri: artifact_uri(&issue.filename, root),
                        uri_base_id: "ROOTPATH".to_owned(),
                    },
                    region: SarifRegion {
                        end_column: column_end(issue.column, &issue.trigger),
                        snippet: SarifSnippet {
                            text: trigger.to_owned(),
                        },
                        start_column: issue.column.unwrap_or(1),
                        start_line: issue.line_no.unwrap_or(1),
                    },
                },
            }],
            message: SarifMessage {
                markdown: issue.message.clone(),
                text: issue.message.replace('`', "'"),
            },
            rank: sarif_rank(issue.priority),
            rule_id: rule_id.to_owned(),
        }
    }
}

/// One SARIF location; field order mirrors the native output.
#[derive(Debug, serde::Serialize)]
struct SarifLocation {
    #[serde(rename = "logicalLocations")]
    logical_locations: Vec<SarifLogicalLocation>,
    #[serde(rename = "physicalLocation")]
    physical_location: SarifPhysicalLocation,
}

/// Scope holder; a missing scope encodes as `null`.
#[derive(Debug, serde::Serialize)]
struct SarifLogicalLocation {
    #[serde(rename = "fullyQualifiedName")]
    fully_qualified_name: Option<String>,
}

/// Artifact plus region.
#[derive(Debug, serde::Serialize)]
struct SarifPhysicalLocation {
    #[serde(rename = "artifactLocation")]
    artifact_location: SarifArtifactLocation,
    region: SarifRegion,
}

/// Issue file reference.
#[derive(Debug, serde::Serialize)]
struct SarifArtifactLocation {
    uri: String,
    #[serde(rename = "uriBaseId")]
    uri_base_id: String,
}

/// Line/column region; `endColumn` vanishes without a trigger or column.
#[derive(Debug, serde::Serialize)]
struct SarifRegion {
    #[serde(rename = "endColumn", skip_serializing_if = "Option::is_none")]
    end_column: Option<usize>,
    snippet: SarifSnippet,
    #[serde(rename = "startColumn")]
    start_column: usize,
    #[serde(rename = "startLine")]
    start_line: usize,
}

/// Trigger snippet.
#[derive(Debug, serde::Serialize)]
struct SarifSnippet {
    text: String,
}

/// Markdown/text message pair.
#[derive(Debug, serde::Serialize)]
struct SarifMessage {
    markdown: String,
    text: String,
}

/// Tool wrapper.
#[derive(Debug, serde::Serialize)]
struct SarifTool {
    driver: SarifDriver,
}

/// Credo driver block.
#[derive(Debug, serde::Serialize)]
struct SarifDriver {
    #[serde(rename = "informationUri")]
    information_uri: String,
    name: String,
    rules: Vec<SarifRule>,
    version: String,
}

/// One SARIF rule; `name` is omitted when the id equals the check module.
#[derive(Debug, serde::Serialize)]
struct SarifRule {
    #[serde(rename = "fullDescription")]
    description: SarifMessage,
    #[serde(rename = "helpUri")]
    help_uri: String,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    properties: SarifProperties,
}

impl SarifRule {
    fn new(check: &str, category: &str, rule_id: &str, doc: &RuleDoc) -> Self {
        Self {
            description: SarifMessage {
                markdown: doc.explanation.clone(),
                text: doc.explanation.replace('`', "'"),
            },
            help_uri: doc.help_uri.clone(),
            id: rule_id.to_owned(),
            name: (rule_id != check).then(|| check.to_owned()),
            properties: SarifProperties {
                tags: vec![category.to_owned()],
            },
        }
    }
}

/// Rule category tags.
#[derive(Debug, serde::Serialize)]
struct SarifProperties {
    tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_FILE: &str = "/tmp/p6probe/lib/smells.ex";

    fn fixture_issues() -> Vec<Issue> {
        // Deliberately jumbled: renderers must restore native order.
        vec![
            Issue {
                check: "Credo.Check.Warning.Dbg".to_owned(),
                category: Category::Warning,
                priority: 12,
                severity: 1.0,
                message: "There should be no calls to `dbg/1`.".to_owned(),
                filename: FIXTURE_FILE.to_owned(),
                line_no: Some(4),
                column: Some(5),
                exit_status: 16,
                trigger: IssueTrigger::Text("dbg".to_owned()),
                scope: Some("Smells.f".to_owned()),
            },
            Issue {
                check: "Credo.Check.Readability.ModuleDoc".to_owned(),
                category: Category::Readability,
                priority: 1,
                severity: 1.0,
                message: "Modules should have a @moduledoc tag.".to_owned(),
                filename: FIXTURE_FILE.to_owned(),
                line_no: Some(1),
                column: Some(11),
                exit_status: 4,
                trigger: IssueTrigger::Text("Smells".to_owned()),
                scope: Some("Smells".to_owned()),
            },
            Issue {
                check: "Credo.Check.Warning.IoInspect".to_owned(),
                category: Category::Warning,
                priority: 12,
                severity: 1.0,
                message: "There should be no calls to `IO.inspect/1`.".to_owned(),
                filename: FIXTURE_FILE.to_owned(),
                line_no: Some(3),
                column: Some(5),
                exit_status: 16,
                trigger: IssueTrigger::Text("IO.inspect".to_owned()),
                scope: Some("Smells.f".to_owned()),
            },
        ]
    }

    fn fixture_report() -> RunReport {
        RunReport {
            issues: fixture_issues(),
            exit_status: 20,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        }
    }

    fn fixture_ctx() -> MachineContext {
        MachineContext::new("/work/ref/credo")
    }

    #[test]
    fn oneline_matches_native_three_issue_fixture() {
        let expected = "[R] → /tmp/p6probe/lib/smells.ex:1:11 Modules should have a @moduledoc tag.\n\
                        [W] ↗ /tmp/p6probe/lib/smells.ex:3:5 There should be no calls to `IO.inspect/1`.\n\
                        [W] ↗ /tmp/p6probe/lib/smells.ex:4:5 There should be no calls to `dbg/1`.\n";
        assert_eq!(render_oneline(&fixture_report(), &fixture_ctx()), expected);
    }

    #[test]
    fn flycheck_matches_native_three_issue_fixture() {
        let expected = "/tmp/p6probe/lib/smells.ex:1:11: R: Modules should have a @moduledoc tag.\n\
                        /tmp/p6probe/lib/smells.ex:3:5: W: There should be no calls to `IO.inspect/1`.\n\
                        /tmp/p6probe/lib/smells.ex:4:5: W: There should be no calls to `dbg/1`.\n";
        assert_eq!(render_flycheck(&fixture_report(), &fixture_ctx()), expected);
    }

    #[test]
    fn json_matches_native_three_issue_fixture() {
        let expected = r#"{
  "issues": [
    {
      "category": "readability",
      "check": "Credo.Check.Readability.ModuleDoc",
      "column": 11,
      "column_end": 17,
      "filename": "/tmp/p6probe/lib/smells.ex",
      "line_no": 1,
      "message": "Modules should have a @moduledoc tag.",
      "priority": 1,
      "scope": "Smells",
      "trigger": "Smells"
    },
    {
      "category": "warning",
      "check": "Credo.Check.Warning.IoInspect",
      "column": 5,
      "column_end": 15,
      "filename": "/tmp/p6probe/lib/smells.ex",
      "line_no": 3,
      "message": "There should be no calls to `IO.inspect/1`.",
      "priority": 12,
      "scope": "Smells.f",
      "trigger": "IO.inspect"
    },
    {
      "category": "warning",
      "check": "Credo.Check.Warning.Dbg",
      "column": 5,
      "column_end": 8,
      "filename": "/tmp/p6probe/lib/smells.ex",
      "line_no": 4,
      "message": "There should be no calls to `dbg/1`.",
      "priority": 12,
      "scope": "Smells.f",
      "trigger": "dbg"
    }
  ]
}
"#;
        assert_eq!(render_json(&fixture_report(), &fixture_ctx()), expected);
    }

    fn fixture_ctx_with_docs() -> MachineContext {
        fixture_ctx()
            .with_rule_doc(
                "Credo.Check.Readability.ModuleDoc",
                RuleDoc::new(
                    "EX3009",
                    EX3009_EXPLANATION,
                    "https://hexdocs.pm/credo/Credo.Check.Readability.ModuleDoc.html",
                ),
            )
            .with_rule_doc(
                "Credo.Check.Warning.IoInspect",
                RuleDoc::new(
                    "EX5006",
                    EX5006_EXPLANATION,
                    "https://hexdocs.pm/credo/Credo.Check.Warning.IoInspect.html",
                ),
            )
            .with_rule_doc(
                "Credo.Check.Warning.Dbg",
                RuleDoc::new(
                    "EX5026",
                    EX5026_EXPLANATION,
                    "https://hexdocs.pm/credo/Credo.Check.Warning.Dbg.html",
                ),
            )
    }

    fn tiny_ctx() -> MachineContext {
        MachineContext::new("/root").with_rule_doc(
            "Credo.Check.Warning.IoInspect",
            RuleDoc::new(
                "EX5006",
                "Tiny docs.",
                "https://hexdocs.pm/credo/Credo.Check.Warning.IoInspect.html",
            ),
        )
    }

    fn tiny_issue(check: &str, line: usize, column: usize, priority: i32) -> Issue {
        Issue {
            check: check.to_owned(),
            category: Category::Warning,
            priority,
            severity: 1.0,
            message: "Tiny.".to_owned(),
            filename: "/root/a.ex".to_owned(),
            line_no: Some(line),
            column: Some(column),
            exit_status: 16,
            trigger: IssueTrigger::Text("Io".to_owned()),
            scope: Some("S".to_owned()),
        }
    }

    #[test]
    fn sarif_matches_native_shape_on_three_issue_fixture() {
        // `EXPECTED_SARIF_FIXTURE` is the pinned `/tmp/p6-sarif.out` capture
        // with only the invocation-cwd `ROOTPATH` rewritten to the synthetic
        // test root; artifact URIs stay absolute (outside the root).
        assert_eq!(
            render_sarif(&fixture_report(), &fixture_ctx_with_docs()),
            EXPECTED_SARIF_FIXTURE
        );
    }

    #[test]
    fn oneline_sorts_swap_fixture_by_line_not_check() {
        let mut issues = fixture_issues();
        // Swap the warning bodies so check-id order disagrees with line order.
        issues[0].line_no = Some(3);
        issues[0].message = "There should be no calls to `dbg/1`.".to_owned();
        issues[2].line_no = Some(4);
        issues[2].message = "There should be no calls to `IO.inspect/1`.".to_owned();
        let report = RunReport {
            issues,
            exit_status: 20,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let out = render_oneline(&report, &fixture_ctx());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[1].contains(":3:5 There should be no calls to `dbg/1`."));
        assert!(lines[2].contains(":4:5 There should be no calls to `IO.inspect/1`."));
    }

    #[test]
    fn flycheck_and_json_keep_check_id_order_within_file() {
        // Same swap: native flycheck/json print IoInspect (EX5006, line 4)
        // before Dbg (EX5026, line 3), proving the order is check-id based.
        let mut issues = fixture_issues();
        issues[0].line_no = Some(3);
        issues[2].line_no = Some(4);
        let report = RunReport {
            issues,
            exit_status: 20,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let flycheck = render_flycheck(&report, &fixture_ctx());
        let io_pos = flycheck.find("IO.inspect").expect("ioinspect line");
        let dbg_pos = flycheck.find("dbg/1").expect("dbg line");
        assert!(io_pos < dbg_pos, "{flycheck}");
        let json = render_json(&report, &fixture_ctx());
        let io_pos = json.find("IoInspect").expect("ioinspect entry");
        let dbg_pos = json.find("\"Credo.Check.Warning.Dbg\"").expect("dbg entry");
        assert!(io_pos < dbg_pos, "{json}");
    }

    #[test]
    fn machine_order_groups_filenames_before_check_ids() {
        // A b-file readability issue sorts before an a-file warning even
        // though EX3009 < EX5026 would group checks first.
        let mut issues = fixture_issues();
        issues[1].filename = "/tmp/p6probe/lib/b.ex".to_owned();
        let report = RunReport {
            issues,
            exit_status: 20,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let out = render_flycheck(&report, &fixture_ctx());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("/tmp/p6probe/lib/b.ex:1:11"), "{out}");
        assert!(
            lines[1].starts_with("/tmp/p6probe/lib/smells.ex:3:5"),
            "{out}"
        );
        assert!(
            lines[2].starts_with("/tmp/p6probe/lib/smells.ex:4:5"),
            "{out}"
        );
    }

    #[test]
    fn machine_order_ranks_each_check_once() {
        let issues = vec![
            tiny_issue("Credo.Check.Warning.IoInspect", 3, 1, 0),
            tiny_issue("Credo.Check.Warning.IoInspect", 4, 1, 0),
            tiny_issue("Credo.Check.Warning.Dbg", 2, 1, 0),
        ];
        let ranks = machine_check_ranks(&issues, &fixture_ctx());
        assert_eq!(ranks.len(), 2, "one rank per distinct check");
        assert!(
            ranks["Credo.Check.Warning.IoInspect"] < ranks["Credo.Check.Warning.Dbg"],
            "EX5006 sorts before EX5026"
        );
    }

    #[test]
    fn severity_tags_cover_all_categories() {
        assert_eq!(severity_tag(Category::Consistency), "C");
        assert_eq!(severity_tag(Category::Design), "D");
        assert_eq!(severity_tag(Category::Readability), "R");
        assert_eq!(severity_tag(Category::Refactor), "F");
        assert_eq!(severity_tag(Category::Warning), "W");
    }

    #[test]
    fn priority_arrows_follow_probed_boundaries() {
        for priority in [20, 25, 100] {
            assert_eq!(priority_arrow(priority), '↑', "{priority}");
        }
        for priority in [10, 12, 19] {
            assert_eq!(priority_arrow(priority), '↗', "{priority}");
        }
        for priority in [0, 1, 9] {
            assert_eq!(priority_arrow(priority), '→', "{priority}");
        }
        for priority in [-10, -1, -5] {
            assert_eq!(priority_arrow(priority), '↘', "{priority}");
        }
        for priority in [-11, -100] {
            assert_eq!(priority_arrow(priority), '↓', "{priority}");
        }
    }

    #[test]
    fn sarif_level_follows_probed_mapping() {
        for priority in [10, 12, 19, 20, 95] {
            assert_eq!(sarif_level(priority), Some("error"), "{priority}");
        }
        for priority in [0, 1, 9] {
            assert_eq!(sarif_level(priority), None, "{priority}");
        }
        for priority in [-100, -11, -10, -5, -1] {
            assert_eq!(sarif_level(priority), Some("note"), "{priority}");
        }
    }

    #[test]
    fn sarif_rank_follows_probed_mapping() {
        for priority in [10, 12, 19, 20] {
            assert_eq!(sarif_rank(priority), priority + 11, "{priority}");
        }
        for priority in [0, 1, 9] {
            assert_eq!(sarif_rank(priority), priority + 11, "{priority}");
        }
        assert_eq!(sarif_rank(-5), 6);
        assert_eq!(sarif_rank(-10), 1);
        for priority in [-100, -11] {
            assert_eq!(sarif_rank(priority), 1, "{priority}");
        }
        for priority in [90, 95] {
            assert_eq!(sarif_rank(priority), 100, "{priority}");
        }
    }

    #[test]
    fn json_empty_issues_shape_is_pinned() {
        let report = RunReport {
            issues: Vec::new(),
            exit_status: 0,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        assert_eq!(
            render_json(&report, &fixture_ctx()),
            "{\n  \"issues\": []\n}\n"
        );
    }

    #[test]
    fn json_nil_fields_encode_null() {
        let report = RunReport {
            issues: vec![Issue {
                check: "Credo.Check.Warning.IoInspect".to_owned(),
                category: Category::Warning,
                priority: 12,
                severity: 1.0,
                message: "m".to_owned(),
                filename: "f.ex".to_owned(),
                line_no: None,
                column: None,
                exit_status: 16,
                trigger: IssueTrigger::NoTrigger,
                scope: None,
            }],
            exit_status: 16,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let expected = "{\n  \"issues\": [\n    {\n      \"category\": \"warning\",\n      \"check\": \"Credo.Check.Warning.IoInspect\",\n      \"column\": null,\n      \"column_end\": null,\n      \"filename\": \"f.ex\",\n      \"line_no\": null,\n      \"message\": \"m\",\n      \"priority\": 12,\n      \"scope\": null,\n      \"trigger\": null\n    }\n  ]\n}\n";
        assert_eq!(render_json(&report, &fixture_ctx()), expected);
    }

    #[test]
    fn line_formats_are_empty_without_issues() {
        let report = RunReport {
            issues: Vec::new(),
            exit_status: 0,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        assert_eq!(render_oneline(&report, &fixture_ctx()), "");
        assert_eq!(render_flycheck(&report, &fixture_ctx()), "");
    }

    #[test]
    fn rule_id_table_covers_fixture_and_falls_back() {
        assert_eq!(
            pinned_check_id("Credo.Check.Readability.ModuleDoc"),
            Some("EX3009")
        );
        assert_eq!(
            pinned_check_id("Credo.Check.Warning.IoInspect"),
            Some("EX5006")
        );
        assert_eq!(pinned_check_id("Credo.Check.Warning.Dbg"), Some("EX5026"));
        assert_eq!(pinned_check_id("Credo.Check.Nope"), None);
        // Table ids flow through without any docs registered.
        assert_eq!(fixture_ctx().rule_id("Credo.Check.Warning.Dbg"), "EX5026");
        // Unknown checks fall back to the module name, like Credo's
        // default `id/0`.
        assert_eq!(fixture_ctx().rule_id("My.Check"), "My.Check");
    }

    #[test]
    fn sarif_rules_and_results_agree_without_docs() {
        // No docs registered: rule ids still come from the pinned table in
        // both sections, and the rule keeps its `name`.
        let report = RunReport {
            issues: vec![tiny_issue("Credo.Check.Warning.Dbg", 3, 5, 12)],
            exit_status: 16,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let out = render_sarif(&report, &MachineContext::new("/root"));
        assert!(out.contains("\"ruleId\": \"EX5026\""), "{out}");
        assert!(out.contains("\"id\": \"EX5026\""), "{out}");
        assert!(
            out.contains("\"name\": \"Credo.Check.Warning.Dbg\""),
            "{out}"
        );
    }

    #[test]
    fn pos_suffix_and_uri_edges_match_native() {
        assert_eq!(pos_suffix(Some(1), Some(11)), ":1:11");
        assert_eq!(pos_suffix(Some(1), None), ":1");
        assert_eq!(pos_suffix(None, None), "");
        assert_eq!(pos_suffix(None, Some(5)), "::5");
        assert_eq!(
            file_uri(Path::new("/work/ref/credo")),
            "file:///work/ref/credo/"
        );
        assert_eq!(
            file_uri(Path::new("/work/ref/credo/")),
            "file:///work/ref/credo/"
        );
        assert_eq!(file_uri(Path::new(".")), "file:///./");
        assert_eq!(artifact_uri("/root/a.ex", Path::new("/root")), "a.ex");
        assert_eq!(
            artifact_uri("/elsewhere/b.ex", Path::new("/root")),
            "/elsewhere/b.ex"
        );
        assert_eq!(artifact_uri("rel/c.ex", Path::new("/root")), "rel/c.ex");
        assert_eq!(artifact_uri("/root", Path::new("/root")), ".");
    }

    #[test]
    fn sarif_empty_shape_is_pinned() {
        let report = RunReport {
            issues: Vec::new(),
            exit_status: 0,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let expected = "{\n  \"$schema\": \"https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0-rtm.5.json\",\n  \"version\": \"2.1.0\",\n  \"runs\": [\n    {\n      \"columnKind\": \"utf16CodeUnits\",\n      \"originalUriBaseIds\": {\n        \"ROOTPATH\": {\n          \"uri\": \"file:///root/\"\n        }\n      },\n      \"results\": [],\n      \"tool\": {\n        \"driver\": {\n          \"informationUri\": \"http://credo-ci.org/\",\n          \"name\": \"Credo\",\n          \"rules\": [],\n          \"version\": \"1.8.0-dev\"\n        }\n      }\n    }\n  ]\n}\n";
        assert_eq!(render_sarif(&report, &tiny_ctx()), expected);
    }

    #[test]
    fn sarif_note_level_and_relative_uri() {
        // Low priority omits nothing: it renders `"level": "note"`, and an
        // under-root file relativizes while the message keeps backticks in
        // markdown but quotes in text.
        let issue = Issue {
            message: "Avoid `x`.".to_owned(),
            priority: -5,
            scope: Some("S".to_owned()),
            ..tiny_issue("Credo.Check.Warning.IoInspect", 2, 3, -5)
        };
        let report = RunReport {
            issues: vec![issue],
            exit_status: 16,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let out = render_sarif(&report, &tiny_ctx());
        assert!(out.contains("\"level\": \"note\""), "{out}");
        assert!(out.contains("\"rank\": 6"), "{out}");
        assert!(out.contains("\"uri\": \"a.ex\""), "{out}");
        assert!(out.contains("\"markdown\": \"Avoid `x`.\""), "{out}");
        assert!(out.contains("\"text\": \"Avoid 'x'.\""), "{out}");
        assert!(out.contains("\"endColumn\": 5"), "{out}");
    }

    #[test]
    fn sarif_no_trigger_omits_end_column_and_level_for_normal() {
        let issue = Issue {
            priority: 1,
            trigger: IssueTrigger::NoTrigger,
            column: None,
            line_no: None,
            scope: None,
            ..tiny_issue("Credo.Check.Warning.IoInspect", 2, 3, 1)
        };
        let report = RunReport {
            issues: vec![issue],
            exit_status: 16,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let out = render_sarif(&report, &tiny_ctx());
        assert!(!out.contains("\"level\""), "{out}");
        assert!(!out.contains("endColumn"), "{out}");
        assert!(
            out.contains("\"snippet\": {\n                    \"text\": \"\"\n                  }"),
            "{out}"
        );
        assert!(out.contains("\"startColumn\": 1"), "{out}");
        assert!(out.contains("\"startLine\": 1"), "{out}");
        assert!(out.contains("\"fullyQualifiedName\": null"), "{out}");
    }
    // Auto-extracted from pinned native capture /tmp/p6-sarif.out — DO NOT HAND-EDIT.
    const EX3009_EXPLANATION: &str = r#"Every module should contain comprehensive documentation.

    # preferred

    defmodule MyApp.Web.Search do
      @moduledoc """
      This module provides a public API for all search queries originating
      in the web layer.
      """
    end

    # also okay: explicitly say there is no documentation

    defmodule MyApp.Web.Search do
      @moduledoc false
    end

Many times a sentence or two in plain english, explaining why the module
exists, will suffice. Documenting your train of thought this way will help
both your co-workers and your future-self.

Other times you will want to elaborate even further and show some
examples of how the module's functions can and should be used.

In some cases however, you might not want to document things about a module,
e.g. it is part of a private API inside your project. Since Elixir prefers
explicitness over implicit behaviour, you should "tag" these modules with

    @moduledoc false

to make it clear that there is no intention in documenting it.

Like all `Readability` issues, this one is not a technical concern.
But you can improve the odds of others reading and liking your code by making
it easier to follow.
"#;
    const EX5006_EXPLANATION: &str = r"While calls to IO.inspect might appear in some parts of production code,
most calls to this function are added during debugging sessions.

This check warns about those calls, because they might have been committed
in error.
";
    const EX5026_EXPLANATION: &str = r"Calls to dbg/0 and dbg/2 should mostly be used during debugging sessions.

This check warns about those calls, because they probably have been committed
in error.
";
    const EXPECTED_SARIF_FIXTURE: &str = r#"{
  "$schema": "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0-rtm.5.json",
  "version": "2.1.0",
  "runs": [
    {
      "columnKind": "utf16CodeUnits",
      "originalUriBaseIds": {
        "ROOTPATH": {
          "uri": "file:///work/ref/credo/"
        }
      },
      "results": [
        {
          "level": "error",
          "locations": [
            {
              "logicalLocations": [
                {
                  "fullyQualifiedName": "Smells.f"
                }
              ],
              "physicalLocation": {
                "artifactLocation": {
                  "uri": "/tmp/p6probe/lib/smells.ex",
                  "uriBaseId": "ROOTPATH"
                },
                "region": {
                  "endColumn": 8,
                  "snippet": {
                    "text": "dbg"
                  },
                  "startColumn": 5,
                  "startLine": 4
                }
              }
            }
          ],
          "message": {
            "markdown": "There should be no calls to `dbg/1`.",
            "text": "There should be no calls to 'dbg/1'."
          },
          "rank": 23,
          "ruleId": "EX5026"
        },
        {
          "level": "error",
          "locations": [
            {
              "logicalLocations": [
                {
                  "fullyQualifiedName": "Smells.f"
                }
              ],
              "physicalLocation": {
                "artifactLocation": {
                  "uri": "/tmp/p6probe/lib/smells.ex",
                  "uriBaseId": "ROOTPATH"
                },
                "region": {
                  "endColumn": 15,
                  "snippet": {
                    "text": "IO.inspect"
                  },
                  "startColumn": 5,
                  "startLine": 3
                }
              }
            }
          ],
          "message": {
            "markdown": "There should be no calls to `IO.inspect/1`.",
            "text": "There should be no calls to 'IO.inspect/1'."
          },
          "rank": 23,
          "ruleId": "EX5006"
        },
        {
          "locations": [
            {
              "logicalLocations": [
                {
                  "fullyQualifiedName": "Smells"
                }
              ],
              "physicalLocation": {
                "artifactLocation": {
                  "uri": "/tmp/p6probe/lib/smells.ex",
                  "uriBaseId": "ROOTPATH"
                },
                "region": {
                  "endColumn": 17,
                  "snippet": {
                    "text": "Smells"
                  },
                  "startColumn": 11,
                  "startLine": 1
                }
              }
            }
          ],
          "message": {
            "markdown": "Modules should have a @moduledoc tag.",
            "text": "Modules should have a @moduledoc tag."
          },
          "rank": 12,
          "ruleId": "EX3009"
        }
      ],
      "tool": {
        "driver": {
          "informationUri": "http://credo-ci.org/",
          "name": "Credo",
          "rules": [
            {
              "fullDescription": {
                "markdown": "Every module should contain comprehensive documentation.\n\n    # preferred\n\n    defmodule MyApp.Web.Search do\n      @moduledoc \"\"\"\n      This module provides a public API for all search queries originating\n      in the web layer.\n      \"\"\"\n    end\n\n    # also okay: explicitly say there is no documentation\n\n    defmodule MyApp.Web.Search do\n      @moduledoc false\n    end\n\nMany times a sentence or two in plain english, explaining why the module\nexists, will suffice. Documenting your train of thought this way will help\nboth your co-workers and your future-self.\n\nOther times you will want to elaborate even further and show some\nexamples of how the module's functions can and should be used.\n\nIn some cases however, you might not want to document things about a module,\ne.g. it is part of a private API inside your project. Since Elixir prefers\nexplicitness over implicit behaviour, you should \"tag\" these modules with\n\n    @moduledoc false\n\nto make it clear that there is no intention in documenting it.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
                "text": "Every module should contain comprehensive documentation.\n\n    # preferred\n\n    defmodule MyApp.Web.Search do\n      @moduledoc \"\"\"\n      This module provides a public API for all search queries originating\n      in the web layer.\n      \"\"\"\n    end\n\n    # also okay: explicitly say there is no documentation\n\n    defmodule MyApp.Web.Search do\n      @moduledoc false\n    end\n\nMany times a sentence or two in plain english, explaining why the module\nexists, will suffice. Documenting your train of thought this way will help\nboth your co-workers and your future-self.\n\nOther times you will want to elaborate even further and show some\nexamples of how the module's functions can and should be used.\n\nIn some cases however, you might not want to document things about a module,\ne.g. it is part of a private API inside your project. Since Elixir prefers\nexplicitness over implicit behaviour, you should \"tag\" these modules with\n\n    @moduledoc false\n\nto make it clear that there is no intention in documenting it.\n\nLike all 'Readability' issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n"
              },
              "helpUri": "https://hexdocs.pm/credo/Credo.Check.Readability.ModuleDoc.html",
              "id": "EX3009",
              "name": "Credo.Check.Readability.ModuleDoc",
              "properties": {
                "tags": [
                  "readability"
                ]
              }
            },
            {
              "fullDescription": {
                "markdown": "While calls to IO.inspect might appear in some parts of production code,\nmost calls to this function are added during debugging sessions.\n\nThis check warns about those calls, because they might have been committed\nin error.\n",
                "text": "While calls to IO.inspect might appear in some parts of production code,\nmost calls to this function are added during debugging sessions.\n\nThis check warns about those calls, because they might have been committed\nin error.\n"
              },
              "helpUri": "https://hexdocs.pm/credo/Credo.Check.Warning.IoInspect.html",
              "id": "EX5006",
              "name": "Credo.Check.Warning.IoInspect",
              "properties": {
                "tags": [
                  "warning"
                ]
              }
            },
            {
              "fullDescription": {
                "markdown": "Calls to dbg/0 and dbg/2 should mostly be used during debugging sessions.\n\nThis check warns about those calls, because they probably have been committed\nin error.\n",
                "text": "Calls to dbg/0 and dbg/2 should mostly be used during debugging sessions.\n\nThis check warns about those calls, because they probably have been committed\nin error.\n"
              },
              "helpUri": "https://hexdocs.pm/credo/Credo.Check.Warning.Dbg.html",
              "id": "EX5026",
              "name": "Credo.Check.Warning.Dbg",
              "properties": {
                "tags": [
                  "warning"
                ]
              }
            }
          ],
          "version": "1.8.0-dev"
        }
      }
    }
  ]
}
"#;
}
