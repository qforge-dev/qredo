# Compatibility evidence

Reference: Credo `ea1ccb9023b44eecbe079dc4bfd48cca4e8b0187`, Elixir 1.20.2 /
OTP 29. The vendored [pins](upstream/pins.tsv) and
[inventory](upstream/inventory.json) retain the complete
rule/parameter inventory. `rules.json` lists the same 120 rule IDs and records
this package's admission separately. The fast test rejects missing inventory
entries and an unimplemented rule returning a successful empty result.

`upstream/trailing_blank_line_test.exs` is an unchanged copy of
`test/credo/check/readability/trailing_blank_line_test.exs` from that pin, under the
included upstream MIT license. The first two JSON cases map its original test
names and input strings to Rust assertions. The positive case also checks line
and message against native Credo, extending the original no-trigger assertion.
The rule implementation is adapted from the corresponding pinned
`lib/credo/check/readability/trailing_blank_line.ex` and Elixir's native line/trim
behavior; the license applies to these adaptations too.

SHA-256 provenance:

- Rule source: `84754c2ec55cdcf77f689c27e64036369dc6587061a964dcab2d0e0562b8c982`.
- Preserved test: `da6979b23eba0f584a56fe4529687b6a42b119d40e966f23fd7fd0d81ff3fdce`.

`trailing_blank_line.json` contains 46 reviewed kernel cases. The native command
compares 36 directly through `Credo.SourceFile.parse/2` and the check's `run/2`,
projecting line, message and the explicit no-trigger sentinel. These include the
two upstream examples, empty text, line endings, final whitespace, Unicode and
all 25 pinned Unicode White_Space codepoints.

Ten additional cases contain invalid Elixir syntax (bare CR after code and
non-trim control/Unicode text). Calling the ordinary native issue formatter on
such a source can fail because no valid AST was stored. These cases explicitly
use the `line-primitive` oracle: native `Credo.Code.to_lines/1` and `String.trim/1`.
They establish text semantics only, not native invalid-source lint results. Even
direct check cases bypass the normal invalid-source filtering pipeline. No parser
acceptance or full diagnostic formatting compatibility follows from this corpus.

Validation performed for this increment:

- Red: the empty first implementation failed `EX3028.upstream.violation`, returning
  zero findings where the upstream example requires one.
- Green: the Rust fixture and inventory contracts pass; the explicit
  native comparison passes 36 check projections and ten line-primitive cases.
- Native comparison: `scripts/credo-kernels PREPARED_DIRECTORY`, using the prepared
  prod reference build and the pinned environment. One VM evaluates the full batch.
- `EX3029` kernels: `trailing_white_space.json` holds 15 reviewed cases mapped
  to upstream test names (both `ignore_strings` modes, heredocs, sigils,
  interpolation, `\r` endings, docs, escaped heredocs). The differential found
  native `run/2` accumulating in reverse line order; kernels emit canonical
  ascending order matching Credo's presentation sort, and the native comparison
  (`scripts/credo-pipeline`, test-env VM) sorts both sides. No content
  mismatches remain.
- `EX3019` kernels: `redundant_blank_lines.json` holds 6 reviewed cases mapped
  to upstream test names (expected code with in-string blanks, heredocs,
  no-empty-lines, violation at line 6, `max_blank_lines` allow/report). The
  differential caught comment-only rows miscounted as blank and in-string
  blanks leaking through the text mask; the kernel now uses the shared
  `syntax::string_interior_rows` fact (pinned grammar) with raw blank rows.
  No content mismatches remain.
- `EX3020` kernels: `semicolons.json` holds 6 reviewed cases (upstream
  expected code and violation, plus string/comment/charlist/multi boundaries).
  Differential passes with no content mismatches (both sides sorted).
- `EX3024` kernels: `space_after_commas.json` holds 13 reviewed cases mapped
  to upstream test names (spacing, interpolations, binaries, newlines, sigils,
  `?`-literals, quote-in-comment, violation, multi-comma with exact columns).
  The differential caught sigil contents leaking through the text mask,
  char-literal masking hiding real commas and trailing-`?` names masked as
  literals; shared masking now covers sigils and only treats `?` as a literal
  at expression starts, with triggers read from raw text. No content
  mismatches remain.
- `EX3007` kernels: `max_line_length.json` holds 17 reviewed cases mapped to
  upstream test names (all seven ignore flags, heredocs, `~H` sigils,
  interpolation, specs, definitions, URLs, violations with exact columns and
  messages). The differential caught over-broad string ignoring (comment URL
  lines and late-starting concatenated strings wrongly excused); the kernel
  now mirrors the native token rule over the masked line (nearest-to-EOL
  string among the last two tokens, starting before the limit). No content
  mismatches remain.
- Config context (`EX2006`/`EX2007`/`EX2008`): `tests/config.rs` evaluates
  validated-config data and registered config comments without executing
  Elixir. `EX2007` compares the validated check set against the pinned
  115-module standard / 77-module enabled inventories (`src/config_data.rs`,
  generated from the real tool); `EX2008` flags list-form configs and
  `false`-disabled checks; `EX2006` reports registered comments ignored by no
  issue (sharing `suppression::config_comments`). Missing configs yield no
  findings, never clean credit. All corpus entries pass.
- `EX2005`/`EX2004` kernels: `tag_todo.json` (9) and `tag_fixme.json` (5)
  hold reviewed cases mapped to upstream test names (both `include_doc`
  modes, `@doc`/`@moduledoc`/`@shortdoc`, lowercase tags, couples). The
  differential reworked tag matching to native semantics (case-insensitive
  `#`-anchored triggers, comment-hash columns, heredoc dedent by closing
  indent, trigger-searched column backfill, sigil and `?"` handling in comment
  scanning) plus canonical ascending kernel order. No content mismatches
  remain.
- `EX4022` kernel: `perceived_complexity.json` holds 7 reviewed cases.
  Upstream ships zero test cases, so each case was verified against native
  `run` on the pinned checkout (7/7 OK) before committing: trivial clean,
  nine-`if` violation (`CC is 10`, `foo` at 2:7), eight-`if` boundary clean,
  custom-max violation, `__using__` exemption, 3-clause `case` weighing
  `round(1 + 0.9) = 2` (clean where cyclomatic reports 4), 2-clause `cond`
  clean. Tenths arithmetic matches Elixir `round/1` without float casts.
- `EX5025` filename selection: `wrong_test_file_extension.json` holds 3
  reviewed cases. Upstream defines zero test cases (its test module documents
  `files.included` selection as untestable in isolation); native `run/2`
  always reports line 1 with no trigger, and the committed cases lock that
  issue shape behind the default selection (`test/**/*_test.ex` and
  `apps/**/test/**/*_test.ex` report, `lib/` does not).
- Pipeline slice (committed): `trailing_blank_line_pipeline.json` holds 50
  reviewed full-issue expectations (36 native `check` fixtures plus 14
  selection/filter cases: suppression file/next-line, files included/excluded,
  general overrides including numeric priority, execution only/ignore/regex/
  invalid-pattern, executable config, invalid source). The Rust
  contract `ex3028_pipeline_matches_reviewed_full_issue_expectations` reads
  them with no live Credo. `scripts/credo-pipeline PREPARED_DIRECTORY`
  re-verifies them in one native VM (pin, upstream test unchanged, Rust suite,
  full-issue comparison where applicable, `ConfigComment` suppression
  spot-check, selection regex semantics) without
  regenerating expectations. Base expectations were generated from the real
  tool, inspected (0 mismatches, priority fix: general priority overrides base
  only, scope bonus still applies; selection fix: native uses
  case-insensitive regexes, `ignore` wins), then committed.
- Remaining for promotion: config discovery/inheritance and execution-level
  selection (only/ignore/strict); scope approximation is single-file
  (multi-module edge cases may differ).

Expectations are committed review artifacts, not automatically refreshed snapshots.
The script verifies the reference source pin and unchanged tracked check/test
sources; it requires the laboratory's correctly prepared build. Kernel status
`verified` means the check passes its corpus: 96 EXIDs gate `tests/cases.rs`
via `compatibility/admitted.txt`, 9 more (`EX2004`, `EX2005`, `EX3007`,
`EX3019`, `EX3020`, `EX3024`, `EX3028`, `EX3029`, `EX4022`) pass committed
per-check corpora with native comparisons, 9 checks (`EX1001`–`EX1008` plus
`EX2002`) gate `compatibility/admitted_project.txt` through multi-file project
evaluation, 5 filename-aware checks (`EX2003`, `EX3009`, `EX4017`,
`EX5010`, `EX5030`) gate `compatibility/admitted_filename.txt`, 3
config-context checks (`EX2006`, `EX2007`, `EX2008`) pass `tests/config.rs`
over validated-config data, and `EX5025` passes a committed filename-selection
corpus (`compatibility/wrong_test_file_extension.json`, native `run/2` issue
shape behind default `files.included` selection) — 120 of 120
checks with passing evidence (`EX3009`, `EX4017`, `EX5010` are covered by two
gates for different entries). Upstream ships zero test cases for `EX4022` and
`EX5025`; their committed corpora were verified against native `run` on the
pinned checkout before committing (7/7 and shape-match) and are reviewed
artifacts, not transcribed upstream assertions. Bulk-marked ledger entries beyond passing
evidence must not be read as verified; the gates are the source of truth.
Pipeline status remains `unsupported` for all 120 checks until full rule
configuration, scope, priority, suppression, invalid-source and observable
output parity are verified. Consistency/project checks are single-file kernels:
forced-parameter and intra-file inconsistency cases are covered, while
multi-file majority aggregation remains pipeline work.
