# Roadmap: 100% Credo CLI compatibility

Target: for any Elixir project whose config uses no custom check
*code*, `qredo` and `mix credo` produce identical observable behavior:
same issues (check, file, line, column, message), same exit codes,
same CLI surface, same failure modes.

Compatibility target: Credo `ea1ccb9` on Elixir 1.20.2 / OTP 29.
"Compatible" means identical behavior on served inputs — never a
relabelled discrepancy. New Credo versions need their own reviewed
inventory.

## Scope

In scope: all 120 checks in-pipeline, all parameters, full
`.credo.exs` semantics, the complete CLI surface, message text,
output formats byte-exact, runtime-dependent checks, edge behaviors.

Out of scope: plugins, custom check modules and executable extension
APIs stay fail-closed with an explicit reason. One open question is
tracked in P6: executable `.credo.exs` bodies beyond a declared pure
subset (see below).

Distribution is a thin Mix package around the Rust executable: development
Git dependencies consume commit-addressed assets from a rolling `dev` release,
while tagged releases publish immutable assets and the matching Hex version.

## Phases

Each phase ends in a measurable gate; no phase starts concealing the
previous one. Standing rules: pinned reference, inventory-driven work,
Red → Green → Refactor → Commit, upstream assertions preserved and
never auto-updated, full diagnostics compared (never bare counts),
`scripts/check` green at every commit.

- **P0 Differential harness.** `scripts/differential APPROOT` runs the
  release `qredo` binary and pinned `mix credo` over the same project
  and config, normalizes both sides and diffs issue sets, exit codes
  and stderr shapes. Seed corpus: the `tests/fixtures` app plus
  real-world Mix projects. *Gate: green-on-green runs agree with
  themselves; disagreements land as a triage list, not surprises.*
- **P1 CLI surface.** Missing flags, discovery rules, selection
  semantics, exit-code matrix, invalid-input errors — verified against
  `mix credo --help` behavior and error cases, not happy paths.
  *Gate: flag-level parity table, every row verified.* Note: `--stale`
  is an intentional qredo-only extension (no upstream counterpart);
  its contract is equivalence with a fresh `qredo` run, not Credo
  parity.
- **P2 Parameters.** Per-check audits from the inventory (81
  rule-specific + 5 general): types, defaults, invalid values,
  interactions, each with differential cases in its corpus.
  All five general parameters and all 81 documented rule-specific options are
  admitted with per-check type/shape validation, including explicit `nil`
  values that select upstream defaults. Configured `tags` replace or extend
  built-in tags during CLI selection, including `:__initial__`.
  *Gate: every parameter covered by passing assertions.*
- **P3 Check promotion.** Project-lane and validated-config checks,
  one at a time: differential campaign on real targets, then flip its
  gate entry. Kernels unchanged; only the promotion is new.
  *Gate: zero fallback refusals on served corpus configs.*
- **P4 Message parity.** Transcribe the 120 message catalogs
  verbatim; differential asserts on message text.
  *Gate: message-level diff clean on the corpus.*
- **P5 Runtime-dependent checks.** Pin capture semantics (logger
  config, `Mix.env`, application config), implement, verify across an
  env matrix. *Gate: env-matrix green.*
- **P6 Formats + executable configs.** Byte-exact formatters
  (default human layout, oneline, flycheck, JSON shape and ordering).
  Inventory what real configs execute dynamically; serve the static
  subset plus a declared pure list (`Mix.env`, `System.get_env` with
  defaults); anything beyond stays the single documented carve-out.
  *Gate: byte diff clean; carve-out (if any) written down with
  evidence.*
- **P7 Admission.** Expanded project set, reviewed report, version
  bump. *Gate: the report, not a feeling.*

## Costing (honest)

P0–P1 are days. P2–P4 are the bulk (weeks): message catalogs and
param edges are numerous, mechanical and unforgiving. P5–P6 hold the
only real unknowns. Any phase whose inventory explodes gets its scope
cut explicitly rather than quietly — the same fail-closed principle
as the product.
