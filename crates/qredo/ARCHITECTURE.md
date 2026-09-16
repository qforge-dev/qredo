# Architecture

One Rust crate (`crates/qredo`) holds a library plus the `qredo` binary.
There is no daemon, no cache store and no BEAM dependency at runtime.

## Pipeline

`runner::run_checks` runs the full pipeline over `RunnerFile`s and a
`RunnerConfig`: syntax gating (invalid files are skipped with a
complaint, never counted clean), per-check file selection, kernel /
project / filename lanes, config-comment suppression, priority
filtering, relevant ordering and OR-combined exit status.

`integration::{select, execute}` is the product contract: static
default-parameter configs whose checks are all implemented are served
natively; anything else yields an explicit `Fallback` reason
(`unsupported-check:*`, `custom-check-params:*`,
`project-scope-check:*`, `needs-validated-config:*`,
`unsupported-credo-config:*`, `native-pipeline-errors`). The CLI maps
that to an error and a non-zero exit instead of partial analysis.

## Facts: one walk per file

`Prepared` shares per-file facts across all checks in a run: masked
text (comment/string-aware scanning), one tree-sitter parse with the
pinned grammar (`tree-sitter-elixir 0.3.5`), and one `facts::Facts`
extraction walk. Rule kernels query facts instead of walking trees;
each walk removed is verified against the pinned native behavior
before it lands.

`Facts` covers call inventory (heads, arguments, arities, keyword
pairs), `def`/`quote` ranges, scope tables, module/def inventory with
names and arities, alias inventory with directive decomposition,
module body statements, string ranges and binding regions. Checks
needing deeper shapes (dataflow over parents, argument-content
analysis, duplication models) keep explicit tree logic; the
`residual_tree_walks` test pins exactly which files those are, so new
tree walks fail loudly.

## Compatibility method

Every check preserves its upstream Credo assertions: positive and
negative cases, parameters and diagnostic shape live in
`compatibility/cases/` and gate the build through `tests/cases.rs`.
`compatibility/rules.json` ledgers all 120 pinned checks;
`compatibility/admitted*.txt` promote corpora only after they fully
pass. Intentional differences have explicit contracts and separate
reporting — never silently updated expectations.
