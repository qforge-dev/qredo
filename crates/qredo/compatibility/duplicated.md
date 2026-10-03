# EX2002 native duplication

Reference: Credo `ea1ccb9023b44eecbe079dc4bfd48cca4e8b0187`, Elixir 1.20.2,
OTP 29. Elixir is used only to produce/verify development evidence.

## Contract

- Full-project and direct single-file evaluation share one Rust engine.
- Structural identity ignores metadata/formatting, preserves names and values,
  and normalizes numeric spellings, strings, heredocs, interpolation, sigils,
  maps/structs, calls/captures, operators, bitstrings and clauses.
- Mass counts quoted tuple nodes using Macro traversal rules, not characters.
- `mass_threshold` defaults to 40, `nodes_threshold` to 2, and
  `excluded_macros` to `[]`. Occurrences within the same file count.
- Pruning uses the upstream fixed default threshold 40, even when candidate
  collection uses a different threshold. Exclusions happen after pruning.
- One issue per file per surviving identity; other occurrences in that file
  remain in the peer list. Severity is the group's occurrence count. Repeated
  identical-looking issues from distinct identities are preserved.
- Nil locations remain nil. JSON preserves the explicit `["__no_trigger__"]`
  sentinel. Peer names follow discovery's relative/absolute filename form.
- Whole-project context is required; subset execution remains unsupported.

### Deterministic ordering

Pinned Credo processes 30-file chunks asynchronously with `ordered: false`.
In a 61-file probe, three native runs produced different peer-list orders:
the final one-file chunk completed first, with the two other chunks changing
places. Consequently byte-identical peer messages across arbitrary native
runs are not a meaningful requirement.

Qredo preserves reverse encounter order within each chunk and uses discovery
order across chunks. This is an explicit difference, not a hidden oracle
normalization. Tests pin deterministic output at 29/30/31/60/61 files. The
strict differential script still reports ordering differences; scale evidence
compares peer multisets separately while retaining every other field.

## Evidence and reproduction

- `duplicated-shapes.json`: 69 reviewed pinned structural identity/mass cases.
  Identity digests are test artifacts over a canonical JSON projection; the
  production engine does not hash serialized subtrees.
- `duplicated.json`: exact same-file, mixed-peer, block-location and normalized
  literal diagnostics. Existing `cases/EX2002.json` remains unchanged and passes.
- `scripts/check-duplicated [PINNED_CHECKOUT]` verifies expectations without
  overwriting them, then runs 120 generated native project comparisons covering
  syntax, zero/default/boundary thresholds, occurrence counts and exclusions.
- A development campaign compared complete normalized structures and mass for
  277 Credo/Jason library files. It exposed signed-range grammar ambiguity and
  raw/interpolated heredoc edge cases; minimized regressions are committed.
- Stale tests cover cold/unchanged runs, edit/revert, rename, delete/add,
  invalid syntax and suppression, comparing full fresh and incremental reports.
  Cache v4 stores portable descriptors and reporting scopes; unchanged-file
  syntax is reused even when its peer findings change.
- Full JSON differential runs use identical working directories/configs and
  compare every JSON issue field plus exit status, preserving multiplicity.
  Three runs on Credo's `lib/` matched all 12 issues and exit status 2 exactly.
  Three 61-file runs matched every diagnostic field and peer multiset (61
  issues, exit status 2); only upstream's nondeterministic peer order differed.

## Performance

Release probe (Apple Silicon, three runs at each size; timings are observations,
not a flaky CI wall-clock assertion):

| Files × 30 functions | Previous collector | Compact engine, initial measurement |
|---|---:|---:|
| 30 | 122 ms | 29 ms |
| 120 | 396 ms | 69 ms |
| 480 | 1,566 ms | 247 ms |

Reproduce with:

```sh
cargo test --release --test duplicated ex2002_performance -- --ignored --nocapture
```

Mass and identity are computed bottom-up; pruning walks the identity DAG.
There are no recursive subtree copies or repeated subtree serializations.
The detector adds no normalization work when disabled. Peer-message output
itself necessarily grows with the number of reported peers.
