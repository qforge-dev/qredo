//! Batch evaluation: shared per-file facts across all rule kernels.
//!
//! One-shot `check_kernel` calls recompute masked text and syntax trees per
//! rule. `check_all_kernels` prepares each fact once per file and shares it.
//! Single-rule calls stay lazy (facts compute on first use), so the tested
//! `check_kernel` path pays nothing extra.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::{Finding, UnsupportedRule};

/// One rule's findings from a batch pass over a single source.
#[derive(Debug, PartialEq, Clone)]
pub struct RuleOutcome {
    /// Ledger rule ID, e.g. `"Credo.Check.Readability.ModuleDoc"`.
    pub rule: &'static str,
    /// Kernel findings in ascending `(line, column)` order.
    pub findings: Vec<Finding>,
}

/// Source text with facts shared by every kernel in a batch pass.
///
/// `masked` is `helpers::mask_strings_comments` output; `tree` is the single
/// tree-sitter parse with the pinned grammar (no timeout), `None` when the
/// grammar is unavailable or parsing yields nothing. `facts` is the single
/// [`crate::facts::Facts`] walk over that tree. Kernels keep their own
/// error policies (e.g. `has_error` handling) on the shared tree.
pub(crate) struct Prepared<'src> {
    source: &'src str,
    masked: OnceLock<String>,
    tree: OnceLock<Option<tree_sitter::Tree>>,
    facts: OnceLock<crate::facts::Facts>,
}

impl<'src> Prepared<'src> {
    /// Lazily prepared facts for single-rule calls.
    pub(crate) fn lazy(source: &'src str) -> Self {
        Self {
            source,
            masked: OnceLock::new(),
            tree: OnceLock::new(),
            facts: OnceLock::new(),
        }
    }

    /// Eagerly prepared facts for batch passes: each fact computed once.
    pub(crate) fn eager(source: &'src str) -> Self {
        let prepared = Self::lazy(source);
        let _ = prepared.masked();
        let _ = prepared.tree();
        prepared
    }

    /// Eagerly prepared facts around an already-parsed tree: the pipeline's
    /// syntax gate parses once, then shares that tree instead of reparsing.
    pub(crate) fn with_tree(source: &'src str, tree: Option<tree_sitter::Tree>) -> Self {
        let prepared = Self::lazy(source);
        let _ = prepared.masked();
        let _ = prepared.tree.set(tree);
        prepared
    }

    /// Original source text.
    pub(crate) fn source(&self) -> &'src str {
        self.source
    }

    /// Masked source, computed at most once.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn masked(&self) -> &str {
        self.masked
            .get_or_init(|| crate::helpers::mask_strings_comments(self.source))
    }

    /// Shared parse tree, computed at most once.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn tree(&self) -> Option<&tree_sitter::Tree> {
        self.tree.get_or_init(|| parse(self.source)).as_ref()
    }

    /// Shared single-walk syntax facts, computed at most once.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn facts(&self) -> &crate::facts::Facts {
        self.facts.get_or_init(|| match self.tree() {
            Some(tree) => crate::facts::extract(tree, self.source),
            None => crate::facts::Facts::empty(),
        })
    }
}

/// One pinned-grammar parse on the calling thread's reused parser;
/// `None` when unavailable.
fn parse(source: &str) -> Option<tree_sitter::Tree> {
    crate::ts_parser::parse(source)
}

/// Run every rule kernel over one source with one parameter map.
///
/// Per-rule findings match `check_kernel_with_params` called rule by rule;
/// see `check_all_matches_per_rule_kernels`.
#[must_use]
pub fn check_all_kernels(source: &str, params: &BTreeMap<String, String>) -> Vec<RuleOutcome> {
    let prepared = Prepared::eager(source);
    rule_ids()
        .iter()
        .map(|rule| {
            #[cfg(feature = "hotpath")]
            let findings =
                hotpath::measure_block!(rule, run_one(rule, &prepared, params).unwrap_or_default());
            #[cfg(not(feature = "hotpath"))]
            let findings = run_one(rule, &prepared, params).unwrap_or_default();
            RuleOutcome { rule, findings }
        })
        .collect()
}

/// True when `rule` dispatches to a kernel (used to report unknown rules
/// explicitly instead of silently producing nothing).
#[must_use]
pub(crate) fn knows_rule(rule: &str) -> bool {
    rule_ids().contains(&rule)
}

/// Bounded pool size: all logical cores, eight on lookup failure.
#[must_use]
pub(crate) fn worker_count(items: usize) -> usize {
    let threads = std::thread::available_parallelism().map_or(8, std::num::NonZero::get);
    threads.max(1).min(items.max(1))
}

/// Evaluate items on a bounded pool with dynamic work sharing (uneven
/// item sizes must not strand work); returns `(item position, output)`
/// pairs in input order regardless of thread scheduling.
///
/// # Panics
/// Panics if a worker panics, matching serial evaluation on the input.
pub(crate) fn parallel_map<'s, I, T, F>(items: &'s [I], task: F) -> Vec<(usize, T)>
where
    I: Sync,
    T: Send,
    F: Sync + Fn(usize, &'s I) -> T,
{
    if items.is_empty() {
        return Vec::new();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..worker_count(items.len()) {
            handles.push(scope.spawn(|| {
                let mut mine = Vec::new();
                loop {
                    let position = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = items.get(position) else {
                        break;
                    };
                    mine.push((position, task(position, item)));
                }
                mine
            }));
        }
        let mut outcomes = Vec::new();
        for handle in handles {
            outcomes.extend(handle.join().expect("batch worker panicked"));
        }
        outcomes.sort_by_key(|(position, _)| *position);
        outcomes
    })
}

/// One file's findings from a parallel batch pass, in input order.
#[derive(Debug, PartialEq, Clone)]
pub struct FileOutcome {
    /// Index into the input slice.
    pub file: usize,
    /// Per-rule outcomes in ledger order.
    pub rules: Vec<RuleOutcome>,
}

/// Run every rule kernel over many sources on a bounded thread pool.
///
/// Files are handed out dynamically (shared atomic counter), so an uneven
/// mix of file sizes cannot strand work on one straggler chunk. Output
/// preserves input order regardless of thread scheduling. Each worker
/// prepares and drops its own facts, so no syntax tree crosses threads.
/// `max_threads` is clamped to at least 1 and at most the input length.
///
/// # Panics
/// Panics if a worker panics, matching serial `check_all_kernels` behavior
/// on the same input.
#[must_use]
pub fn check_sources_parallel(
    sources: &[&str],
    params: &BTreeMap<String, String>,
    max_threads: usize,
) -> Vec<FileOutcome> {
    if sources.is_empty() {
        return Vec::new();
    }
    let workers = max_threads.max(1).min(sources.len());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..workers {
            let counter = &next;
            handles.push(scope.spawn(move || {
                let mut mine = Vec::new();
                loop {
                    let index = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index >= sources.len() {
                        break;
                    }
                    mine.push(FileOutcome {
                        file: index,
                        rules: check_all_kernels(sources[index], params),
                    });
                }
                mine
            }));
        }
        let mut outcomes = Vec::with_capacity(sources.len());
        for handle in handles {
            outcomes.extend(handle.join().expect("batch worker panicked"));
        }
        outcomes.sort_by_key(|file| file.file);
        outcomes
    })
}
/// All dispatchable ledger rule IDs, in batch order.
#[allow(
    clippy::too_many_lines,
    reason = "120 ledger IDs mirroring the dispatch table; guarded by rule_coverage_matches_ledger"
)]
fn rule_ids() -> Vec<&'static str> {
    vec![
        "Credo.Check.Readability.TrailingBlankLine",
        "Credo.Check.Readability.TrailingWhiteSpace",
        "Credo.Check.Readability.RedundantBlankLines",
        "Credo.Check.Readability.Semicolons",
        "Credo.Check.Readability.SpaceAfterCommas",
        "Credo.Check.Readability.MaxLineLength",
        "Credo.Check.Design.TagTODO",
        "Credo.Check.Design.TagFIXME",
        "Credo.Check.Readability.AliasAs",
        "Credo.Check.Readability.AliasOrder",
        "Credo.Check.Readability.BlockPipe",
        "Credo.Check.Readability.CaptureOperator",
        "Credo.Check.Readability.FunctionNames",
        "Credo.Check.Readability.ImplTrue",
        "Credo.Check.Readability.LargeNumbers",
        "Credo.Check.Readability.ModuleAttributeNames",
        "Credo.Check.Readability.ModuleDoc",
        "Credo.Check.Readability.ModuleNames",
        "Credo.Check.Readability.MultiAlias",
        "Credo.Check.Readability.NestedFunctionCalls",
        "Credo.Check.Readability.OneArityFunctionInPipe",
        "Credo.Check.Readability.OnePipePerLine",
        "Credo.Check.Readability.ParenthesesInCondition",
        "Credo.Check.Readability.ParenthesesOnZeroArityDefs",
        "Credo.Check.Readability.PipeIntoAnonymousFunctions",
        "Credo.Check.Readability.PredicateFunctionNames",
        "Credo.Check.Readability.PreferImplicitTry",
        "Credo.Check.Readability.PreferUnquotedAtoms",
        "Credo.Check.Readability.SeparateAliasRequire",
        "Credo.Check.Readability.SingleFunctionToBlockPipe",
        "Credo.Check.Readability.SinglePipe",
        "Credo.Check.Readability.SpecParameterNames",
        "Credo.Check.Readability.Specs",
        "Credo.Check.Readability.StrictModuleLayout",
        "Credo.Check.Readability.StringSigils",
        "Credo.Check.Readability.UnnecessaryAliasExpansion",
        "Credo.Check.Readability.UnusedFunctionParameterPattern",
        "Credo.Check.Readability.VariableNames",
        "Credo.Check.Readability.WithCustomTaggedTuple",
        "Credo.Check.Readability.WithSingleClause",
        "Credo.Check.Consistency.ExceptionNames",
        "Credo.Check.Consistency.LineEndings",
        "Credo.Check.Consistency.MultiAliasImportRequireUse",
        "Credo.Check.Consistency.ParameterPatternMatching",
        "Credo.Check.Consistency.SpaceAroundOperators",
        "Credo.Check.Consistency.SpaceInParentheses",
        "Credo.Check.Consistency.TabsOrSpaces",
        "Credo.Check.Consistency.UnusedVariableNames",
        "Credo.Check.Design.AliasUsage",
        "Credo.Check.Design.DeprecatedChecksConfig",
        "Credo.Check.Design.DuplicatedCode",
        "Credo.Check.Design.MissingCheckInConfig",
        "Credo.Check.Design.RedundantConfigComments",
        "Credo.Check.Design.SkipTestWithoutComment",
        "Credo.Check.Refactor.ABCSize",
        "Credo.Check.Refactor.AppendSingleItem",
        "Credo.Check.Refactor.Apply",
        "Credo.Check.Refactor.CaseTrivialMatches",
        "Credo.Check.Refactor.CondInsteadOfIfElse",
        "Credo.Check.Refactor.CondStatements",
        "Credo.Check.Refactor.CyclomaticComplexity",
        "Credo.Check.Refactor.DoubleBooleanNegation",
        "Credo.Check.Refactor.FilterCount",
        "Credo.Check.Refactor.FilterFilter",
        "Credo.Check.Refactor.FilterReject",
        "Credo.Check.Refactor.FunctionArity",
        "Credo.Check.Refactor.IoPuts",
        "Credo.Check.Refactor.LongQuoteBlocks",
        "Credo.Check.Refactor.MapInto",
        "Credo.Check.Refactor.MapJoin",
        "Credo.Check.Refactor.MapMap",
        "Credo.Check.Refactor.MatchInCondition",
        "Credo.Check.Refactor.ModuleDependencies",
        "Credo.Check.Refactor.NegatedConditionsInUnless",
        "Credo.Check.Refactor.NegatedConditionsWithElse",
        "Credo.Check.Refactor.NegatedIsNil",
        "Credo.Check.Refactor.Nesting",
        "Credo.Check.Refactor.PassAsyncInTestCases",
        "Credo.Check.Refactor.PerceivedComplexity",
        "Credo.Check.Refactor.PipeChainStart",
        "Credo.Check.Refactor.PreferDateTimeShift",
        "Credo.Check.Refactor.RedundantWithClauseResult",
        "Credo.Check.Refactor.RejectFilter",
        "Credo.Check.Refactor.RejectReject",
        "Credo.Check.Refactor.UnlessWithElse",
        "Credo.Check.Refactor.UtcNowTruncate",
        "Credo.Check.Refactor.VariableRebinding",
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
        "Credo.Check.Warning.MapGetUnsafePass",
        "Credo.Check.Warning.MissedMetadataKeyInLoggerConfig",
        "Credo.Check.Warning.MixEnv",
        "Credo.Check.Warning.OperationOnSameValues",
        "Credo.Check.Warning.OperationWithConstantResult",
        "Credo.Check.Warning.RaiseInsideRescue",
        "Credo.Check.Warning.SpecWithStruct",
        "Credo.Check.Warning.StructFieldAmount",
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
    ]
}

/// One rule over prepared facts; `Err` only for unknown IDs.
#[allow(
    clippy::too_many_lines,
    reason = "120-arm kernel dispatch table; each arm delegates to a small rule module"
)]
pub(crate) fn run_one(
    rule: &str,
    prepared: &Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Result<Vec<Finding>, UnsupportedRule> {
    match rule {
        "Credo.Check.Readability.TrailingBlankLine" => {
            Ok(crate::trailing_blank_line::check(prepared.source()))
        }
        "Credo.Check.Readability.TrailingWhiteSpace" => Ok(
            crate::readability::trailing_white_space::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.RedundantBlankLines" => Ok(
            crate::readability::redundant_blank_lines::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.Semicolons" => {
            Ok(crate::readability::semicolons::check_prepared(prepared))
        }
        "Credo.Check.Readability.SpaceAfterCommas" => Ok(
            crate::readability::space_after_commas::check_prepared(prepared),
        ),
        "Credo.Check.Readability.MaxLineLength" => Ok(
            crate::readability::max_line_length::check_prepared(prepared, params),
        ),
        "Credo.Check.Design.TagTODO" => {
            Ok(crate::design::tags::check_todo(prepared.source(), params))
        }
        "Credo.Check.Design.TagFIXME" => {
            Ok(crate::design::tags::check_fixme(prepared.source(), params))
        }
        "Credo.Check.Readability.AliasAs" => Ok(crate::readability::alias_as::check_prepared(
            prepared, params,
        )),
        "Credo.Check.Readability.AliasOrder" => Ok(
            crate::readability::alias_order::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.BlockPipe" => Ok(crate::readability::block_pipe::check_prepared(
            prepared, params,
        )),
        "Credo.Check.Readability.CaptureOperator" => Ok(
            crate::readability::capture_operator::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.FunctionNames" => Ok(
            crate::readability::function_names::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.ImplTrue" => {
            Ok(crate::readability::impl_true::check_prepared(prepared))
        }
        "Credo.Check.Readability.LargeNumbers" => Ok(
            crate::readability::large_numbers::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.ModuleAttributeNames" => Ok(
            crate::readability::module_attribute_names::check_prepared(prepared),
        ),
        "Credo.Check.Readability.ModuleDoc" => Ok(crate::readability::module_doc::check_prepared(
            prepared, params,
        )),
        "Credo.Check.Readability.ModuleNames" => Ok(
            crate::readability::module_names::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.MultiAlias" => {
            Ok(crate::readability::multi_alias::check_prepared(prepared))
        }
        "Credo.Check.Readability.NestedFunctionCalls" => Ok(
            crate::readability::nested_calls::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.OneArityFunctionInPipe" => {
            Ok(crate::readability::one_arity_pipe::check_prepared(prepared))
        }
        "Credo.Check.Readability.OnePipePerLine" => Ok(
            crate::readability::one_pipe_per_line::check_prepared(prepared),
        ),
        "Credo.Check.Readability.ParenthesesInCondition" => Ok(
            crate::readability::parens_in_condition::check_prepared(prepared),
        ),
        "Credo.Check.Readability.ParenthesesOnZeroArityDefs" => Ok(
            crate::readability::parens_zero_arity::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.PipeIntoAnonymousFunctions" => {
            Ok(crate::readability::pipe_anon::check_prepared(prepared))
        }
        "Credo.Check.Readability.PredicateFunctionNames" => Ok(
            crate::readability::predicate_names::check_prepared(prepared),
        ),
        "Credo.Check.Readability.PreferImplicitTry" => Ok(
            crate::readability::prefer_implicit_try::check_prepared(prepared),
        ),
        "Credo.Check.Readability.PreferUnquotedAtoms" => Ok(
            crate::readability::prefer_unquoted::check(prepared.source()),
        ),
        "Credo.Check.Readability.SeparateAliasRequire" => Ok(
            crate::readability::separate_alias_require::check_prepared(prepared),
        ),
        "Credo.Check.Readability.SingleFunctionToBlockPipe" => Ok(
            crate::readability::single_to_block::check_prepared(prepared),
        ),
        "Credo.Check.Readability.SinglePipe" => Ok(
            crate::readability::single_pipe::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.SpecParameterNames" => Ok(
            crate::readability::spec_param_names::check_prepared(prepared),
        ),
        "Credo.Check.Readability.Specs" => {
            Ok(crate::readability::specs::check_prepared(prepared, params))
        }
        "Credo.Check.Readability.StrictModuleLayout" => Ok(
            crate::readability::strict_layout::check_prepared(prepared, params),
        ),
        "Credo.Check.Readability.StringSigils" => Ok(crate::readability::string_sigils::check(
            prepared.source(),
            params,
        )),
        "Credo.Check.Readability.UnnecessaryAliasExpansion" => Ok(
            crate::readability::unnecessary_alias::check_prepared(prepared),
        ),
        "Credo.Check.Readability.UnusedFunctionParameterPattern" => Ok(
            crate::readability::unused_param_pattern::check_prepared(prepared),
        ),
        "Credo.Check.Readability.VariableNames" => {
            Ok(crate::readability::variable_names::check_prepared(prepared))
        }
        "Credo.Check.Readability.WithCustomTaggedTuple" => {
            Ok(crate::readability::with_tagged::check_prepared(prepared))
        }
        "Credo.Check.Readability.WithSingleClause" => {
            Ok(crate::readability::with_single::check_prepared(prepared))
        }
        "Credo.Check.Consistency.ExceptionNames" => Ok(
            crate::consistency::exception_names::check_prepared(prepared),
        ),
        "Credo.Check.Consistency.LineEndings" => Ok(crate::consistency::line_endings::check(
            prepared.source(),
            params,
        )),
        "Credo.Check.Consistency.MultiAliasImportRequireUse" => Ok(
            crate::consistency::multi_alias_use::check_prepared(prepared),
        ),
        "Credo.Check.Consistency.ParameterPatternMatching" => Ok(
            crate::consistency::param_pattern::check_prepared(prepared, params),
        ),
        "Credo.Check.Consistency.SpaceAroundOperators" => Ok(
            crate::consistency::space_around_ops::check_prepared(prepared, params),
        ),
        "Credo.Check.Consistency.SpaceInParentheses" => Ok(
            crate::consistency::space_in_parens::check_prepared(prepared, params),
        ),
        "Credo.Check.Consistency.TabsOrSpaces" => Ok(
            crate::consistency::tabs_or_spaces::check_prepared(prepared, params),
        ),
        "Credo.Check.Consistency.UnusedVariableNames" => Ok(
            crate::consistency::unused_var_names::check(prepared.source(), params),
        ),
        "Credo.Check.Design.AliasUsage" => {
            Ok(crate::design::alias_usage::check_prepared(prepared, params))
        }
        "Credo.Check.Design.DeprecatedChecksConfig" => {
            Ok(crate::design::deprecated_config::check(prepared.source()))
        }
        "Credo.Check.Design.DuplicatedCode" => {
            Ok(crate::design::duplicated::check_prepared(prepared, params))
        }
        "Credo.Check.Design.MissingCheckInConfig" => {
            Ok(crate::design::missing_config::check(prepared.source()))
        }
        "Credo.Check.Design.RedundantConfigComments" => Ok(
            crate::design::redundant_config_comments::check(prepared.source()),
        ),
        "Credo.Check.Design.SkipTestWithoutComment" => {
            Ok(crate::design::skip_test::check_prepared(prepared))
        }
        "Credo.Check.Refactor.ABCSize" => {
            Ok(crate::refactor::abc_size::check_prepared(prepared, params))
        }
        "Credo.Check.Refactor.AppendSingleItem" => {
            Ok(crate::refactor::append_single::check_prepared(prepared))
        }
        "Credo.Check.Refactor.Apply" => Ok(crate::refactor::apply::check_prepared(prepared)),
        "Credo.Check.Refactor.CaseTrivialMatches" => {
            Ok(crate::refactor::case_trivial::check_prepared(prepared))
        }
        "Credo.Check.Refactor.CondInsteadOfIfElse" => Ok(
            crate::refactor::cond_if_else::check_prepared(prepared, params),
        ),
        "Credo.Check.Refactor.CondStatements" => {
            Ok(crate::refactor::cond_statements::check_prepared(prepared))
        }
        "Credo.Check.Refactor.CyclomaticComplexity" => Ok(
            crate::refactor::cyclomatic::check_prepared(prepared, params),
        ),
        "Credo.Check.Refactor.DoubleBooleanNegation" => {
            Ok(crate::refactor::double_negation::check_prepared(prepared))
        }
        "Credo.Check.Refactor.FilterCount" => {
            Ok(crate::refactor::filter_count::check_prepared(prepared))
        }
        "Credo.Check.Refactor.FilterFilter" => {
            Ok(crate::refactor::filter_filter::check_prepared(prepared))
        }
        "Credo.Check.Refactor.FilterReject" => {
            Ok(crate::refactor::filter_reject::check_prepared(prepared))
        }
        "Credo.Check.Refactor.FunctionArity" => Ok(
            crate::refactor::function_arity::check_prepared(prepared, params),
        ),
        "Credo.Check.Refactor.IoPuts" => Ok(crate::refactor::io_puts::check_prepared(prepared)),
        "Credo.Check.Refactor.LongQuoteBlocks" => Ok(crate::refactor::long_quote::check_prepared(
            prepared, params,
        )),
        "Credo.Check.Refactor.MapInto" => Ok(crate::refactor::map_into::check_prepared(prepared)),
        "Credo.Check.Refactor.MapJoin" => Ok(crate::refactor::map_join::check_prepared(prepared)),
        "Credo.Check.Refactor.MapMap" => Ok(crate::refactor::map_map::check_prepared(prepared)),
        "Credo.Check.Refactor.MatchInCondition" => Ok(
            crate::refactor::match_in_condition::check_prepared(prepared, params),
        ),
        "Credo.Check.Refactor.ModuleDependencies" => Ok(
            crate::refactor::module_deps::check_prepared(prepared, params),
        ),
        "Credo.Check.Refactor.NegatedConditionsInUnless" => {
            Ok(crate::refactor::negated_unless::check_prepared(prepared))
        }
        "Credo.Check.Refactor.NegatedConditionsWithElse" => {
            Ok(crate::refactor::negated_else::check_prepared(prepared))
        }
        "Credo.Check.Refactor.NegatedIsNil" => {
            Ok(crate::refactor::negated_is_nil::check_prepared(prepared))
        }
        "Credo.Check.Refactor.Nesting" => {
            Ok(crate::refactor::nesting::check_prepared(prepared, params))
        }
        "Credo.Check.Refactor.PassAsyncInTestCases" => Ok(
            crate::refactor::pass_async::check_prepared(prepared, params),
        ),
        "Credo.Check.Refactor.PerceivedComplexity" => {
            Ok(crate::refactor::perceived::check_prepared(prepared, params))
        }
        "Credo.Check.Refactor.PipeChainStart" => Ok(crate::refactor::pipe_start::check_prepared(
            prepared, params,
        )),
        "Credo.Check.Refactor.PreferDateTimeShift" => {
            Ok(crate::refactor::datetime_shift::check_prepared(prepared))
        }
        "Credo.Check.Refactor.RedundantWithClauseResult" => {
            Ok(crate::refactor::redundant_with::check_prepared(prepared))
        }
        "Credo.Check.Refactor.RejectFilter" => {
            Ok(crate::refactor::reject_filter::check_prepared(prepared))
        }
        "Credo.Check.Refactor.RejectReject" => {
            Ok(crate::refactor::reject_reject::check_prepared(prepared))
        }
        "Credo.Check.Refactor.UnlessWithElse" => {
            Ok(crate::refactor::unless_else::check_prepared(prepared))
        }
        "Credo.Check.Refactor.UtcNowTruncate" => {
            Ok(crate::refactor::utc_truncate::check_prepared(prepared))
        }
        "Credo.Check.Refactor.VariableRebinding" => {
            Ok(crate::refactor::rebinding::check_prepared(prepared, params))
        }
        "Credo.Check.Refactor.WithClauses" => {
            Ok(crate::refactor::with_clauses::check_prepared(prepared))
        }
        "Credo.Check.Warning.ApplicationConfigInModuleAttribute" => {
            Ok(crate::warning::app_config_attr::check_prepared(prepared))
        }
        "Credo.Check.Warning.BoolOperationOnSameValues" => {
            Ok(crate::warning::bool_same::check_prepared(prepared))
        }
        "Credo.Check.Warning.Dbg" => Ok(crate::warning::dbg::check_prepared(prepared, params)),
        "Credo.Check.Warning.ExpensiveEmptyEnumCheck" => {
            Ok(crate::warning::expensive_empty::check_prepared(prepared))
        }
        "Credo.Check.Warning.ForbiddenFunction" => Ok(
            crate::warning::forbidden_function::check_prepared(prepared, params),
        ),
        "Credo.Check.Warning.ForbiddenModule" => Ok(
            crate::warning::forbidden_module::check_prepared(prepared, params),
        ),
        "Credo.Check.Warning.IExPry" => Ok(crate::warning::iex_pry::check_prepared(prepared)),
        "Credo.Check.Warning.IoInspect" => {
            Ok(crate::warning::io_inspect::check_prepared(prepared, params))
        }
        "Credo.Check.Warning.LazyLogging" => Ok(crate::warning::lazy_logging::check_prepared(
            prepared, params,
        )),
        "Credo.Check.Warning.LeakyEnvironment" => {
            Ok(crate::warning::leaky_env::check_prepared(prepared))
        }
        "Credo.Check.Warning.MapGetUnsafePass" => {
            Ok(crate::warning::map_get::check_prepared(prepared))
        }
        "Credo.Check.Warning.MissedMetadataKeyInLoggerConfig" => Ok(
            crate::warning::logger_metadata::check_prepared(prepared, params),
        ),
        "Credo.Check.Warning.MixEnv" => Ok(crate::warning::mix_env::check_prepared(prepared)),
        "Credo.Check.Warning.OperationOnSameValues" => {
            Ok(crate::warning::op_same::check_prepared(prepared))
        }
        "Credo.Check.Warning.OperationWithConstantResult" => {
            Ok(crate::warning::op_const::check_prepared(prepared))
        }
        "Credo.Check.Warning.RaiseInsideRescue" => {
            Ok(crate::warning::raise_rescue::check_prepared(prepared))
        }
        "Credo.Check.Warning.SpecWithStruct" => {
            Ok(crate::warning::spec_struct::check_prepared(prepared))
        }
        "Credo.Check.Warning.StructFieldAmount" => Ok(
            crate::warning::struct_fields::check_prepared(prepared, params),
        ),
        "Credo.Check.Warning.UnsafeExec" => {
            Ok(crate::warning::unsafe_exec::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnsafeToAtom" => {
            Ok(crate::warning::unsafe_atom::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnusedEnumOperation" => Ok(
            crate::warning::unused_enum::check_prepared(prepared, params),
        ),
        "Credo.Check.Warning.UnusedFileOperation" => {
            Ok(crate::warning::unused_file::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnusedKeywordOperation" => {
            Ok(crate::warning::unused_keyword::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnusedListOperation" => {
            Ok(crate::warning::unused_list::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnusedMapOperation" => {
            Ok(crate::warning::unused_map::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnusedOperation" => {
            Ok(crate::warning::unused_op::check_prepared(prepared, params))
        }
        "Credo.Check.Warning.UnusedPathOperation" => {
            Ok(crate::warning::unused_path::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnusedRegexOperation" => {
            Ok(crate::warning::unused_regex::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnusedStringOperation" => {
            Ok(crate::warning::unused_string::check_prepared(prepared))
        }
        "Credo.Check.Warning.UnusedTupleOperation" => {
            Ok(crate::warning::unused_tuple::check_prepared(prepared))
        }
        "Credo.Check.Warning.WrongTestFileExtension" => {
            Ok(crate::warning::wrong_ext::check(prepared.source()))
        }
        "Credo.Check.Warning.WrongTestFilename" => {
            Ok(crate::warning::wrong_name::check(prepared.source()))
        }
        _ => Err(UnsupportedRule(rule.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_facts_share_across_threads() {
        fn assert_sync<T: Sync>() {}
        // The parallel kernel lane shares one `Prepared` per file across
        // worker threads; `std::cell::OnceCell` cannot cross threads.
        assert_sync::<Prepared<'_>>();
    }

    #[test]
    fn with_tree_shares_gate_parse_without_reparsing() {
        let source = "defmodule M do\n  def foo(x), do: x\nend\n";
        let tree = crate::ts_parser::parse(source).expect("source parses");
        let sexp = tree.root_node().to_sexp();
        let prepared = Prepared::with_tree(source, Some(tree));
        assert_eq!(
            prepared.tree().map(|tree| tree.root_node().to_sexp()),
            Some(sexp)
        );
        assert_eq!(prepared.source(), source);
        assert!(!prepared.masked().is_empty());
        let empty = Prepared::with_tree(source, None);
        assert!(empty.tree().is_none());
    }

    /// Diverse small sources exercising text, syntax and failure paths.
    fn samples() -> Vec<&'static str> {
        vec![
            "",
            "x = 1\n",
            "defmodule M do\n  def foo(x), do: x\nend\n",
            "def caf\u{e9}(x), do: \"h\u{e9}llo\" # \u{30b3}\u{30e1}\u{30f3}\u{30c8}\n",
            "def foo( do\n",
            "# only a comment\n\n   \n",
        ]
    }

    fn ledger_ids() -> Vec<String> {
        let text = include_str!("../compatibility/rules.json");
        let json: serde_json::Value = serde_json::from_str(text).expect("valid JSON");
        json["rules"]
            .as_array()
            .expect("rules array")
            .iter()
            .map(|rule| rule["id"].as_str().expect("id").to_owned())
            .collect()
    }

    #[test]
    fn check_all_matches_per_rule_kernels() {
        let ledger = ledger_ids();
        assert_eq!(rule_ids().len(), ledger.len());
        for source in samples() {
            let params = BTreeMap::new();
            let outcomes = check_all_kernels(source, &params);
            assert_eq!(outcomes.len(), ledger.len());
            for outcome in &outcomes {
                let expected = crate::check_kernel_with_params(outcome.rule, source, &params)
                    .expect("ledger rule dispatches");
                assert_eq!(outcome.findings, expected, "{}", outcome.rule);
            }
        }
    }

    #[test]
    fn check_all_matches_per_rule_kernels_with_params() {
        let params: BTreeMap<String, String> = [
            ("max_complexity".to_owned(), "2".to_owned()),
            ("max_length".to_owned(), "40".to_owned()),
        ]
        .into_iter()
        .collect();
        for source in samples() {
            let outcomes = check_all_kernels(source, &params);
            for outcome in &outcomes {
                let expected = crate::check_kernel_with_params(outcome.rule, source, &params)
                    .expect("ledger rule dispatches");
                assert_eq!(outcome.findings, expected, "{}", outcome.rule);
            }
        }
    }

    #[test]
    fn rule_coverage_matches_ledger() {
        let mut batched: Vec<String> = rule_ids().iter().map(ToString::to_string).collect();
        let mut ledger = ledger_ids();
        batched.sort();
        ledger.sort();
        assert_eq!(batched, ledger);
    }

    #[test]
    fn parallel_matches_serial_in_order() {
        let sources = samples();
        let params = BTreeMap::new();
        let serial: Vec<Vec<RuleOutcome>> = sources
            .iter()
            .map(|source| check_all_kernels(source, &params))
            .collect();
        let parallel = check_sources_parallel(&sources, &params, 4);
        assert_eq!(parallel.len(), serial.len());
        for (index, file) in parallel.iter().enumerate() {
            assert_eq!(file.file, index);
            assert_eq!(file.rules, serial[index]);
        }
    }

    #[test]
    fn parallel_handles_edges() {
        let params = BTreeMap::new();
        assert!(check_sources_parallel(&[], &params, 4).is_empty());
        let single = check_sources_parallel(&["x = 1\n"], &params, 0);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].file, 0);
        assert_eq!(single[0].rules.len(), rule_ids().len());
    }

    #[test]
    fn parallel_matches_serial_on_uneven_workload() {
        // A few large sources among many tiny ones: scheduling must not
        // drop, duplicate, or reorder files.
        let big = "defmodule M do\n  def foo(x), do: x\nend\n".repeat(50);
        let mut sources = vec!["x = 1\n"; 50];
        sources.push(&big);
        sources.push(&big);
        let params = BTreeMap::new();
        let serial: Vec<Vec<RuleOutcome>> = sources
            .iter()
            .map(|source| check_all_kernels(source, &params))
            .collect();
        let parallel = check_sources_parallel(&sources, &params, 8);
        assert_eq!(parallel.len(), serial.len());
        for (index, file) in parallel.iter().enumerate() {
            assert_eq!(file.file, index);
            assert_eq!(file.rules, serial[index]);
        }
    }
}
