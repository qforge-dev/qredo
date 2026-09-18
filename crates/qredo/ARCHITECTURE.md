# Architecture

One Rust crate (`crates/qredo`) holds a library plus the `qredo` binary.
There is no daemon and no BEAM dependency at runtime. The only disk
state is the opt-in `--stale` incremental cache (`~/.cache/qredo/`,
see below); plain runs are stateless.

## Pipeline

`runner::run_checks` runs the full pipeline over `RunnerFile`s and a
`RunnerConfig`: syntax gating (invalid files are skipped with a
complaint, never counted clean), per-check file selection, kernel /
project / filename lanes, config-comment suppression, priority
filtering, relevant ordering and OR-combined exit status.
File-selection globs and regexes compile once per check and retain
upstream's lazy pattern-error ordering when matched across files.
Machine-format ordering resolves the Credo rule id once per distinct check,
then sorts issues by compact numeric ranks rather than per-issue strings.

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

## Incremental cache (`--stale`)

`stale::execute_stale` serves `suggest --stale` / `list --stale` with
output identical to a fresh run. Per file it stores the content hash,
per-check consistency vote counts and validation outcomes; globally it
stores the sorted final issues, config fingerprint and per-check majority
winners (`stale_cache::DiskCache`, schema `cache-v2.json`). Comment-validation
outcomes, filename-dependent pattern validation and the final globally sorted
report are hash-bound cache data too: an exact filename/order and content-hash
hit returns that report directly without rescanning comments, rematching every
check, rebuilding project state or resorting issues. Ownership moves the
cached issue vector into the report, and machine-format path absolutization
mutates it in place, avoiding two full diagnostic clones.

Each consistency collector exposes `collect_file` (per-file votes),
`counts_of`, `winner` (same force normalization and suppression as
`run`) and `emit_with_winner`, so the stale path merges cached counts
with fresh-file votes, recomputes the majority and emits only fresh
files — unchanged files are never parsed. A flipped winner, missing
votes, or any fingerprint mismatch (tool version, config bytes, env
snapshot, checks, selection, priority) fails open to a full run.
An unchanged hit leaves the existing cache file untouched rather than
serializing and atomically rewriting an identical payload.

## Compatibility method

Every check preserves its upstream Credo assertions: positive and
negative cases, parameters and diagnostic shape live in
`compatibility/cases/` and gate the build through `tests/cases.rs`.
`compatibility/rules.json` ledgers all 120 pinned checks;
`compatibility/admitted*.txt` promote corpora only after they fully
pass. Intentional differences have explicit contracts and separate
reporting — never silently updated expectations.
