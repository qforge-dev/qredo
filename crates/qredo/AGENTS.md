# Contributing to qredo

Work in small, complete increments: Red → Green → Refactor → Commit.

1. **Red.** Pick one behavior, give it a stable test ID, write the test
   first and watch it fail for the right reason (missing behavior, not
   broken setup).
2. **Green.** Implement the minimum production code for that contract.
   Run the focused test, then the affected suite.
3. **Refactor.** Simplify while green. New behavior needs a new Red
   first.
4. **Commit.** Run `scripts/check` from the repo root, stage one
   coherent increment and commit. Reference the test ID, the observed
   Red failure and the Green checks in the commit body. Never bypass
   the gate or commit broken work.

Rules:

- Production changes need dedicated tests. Developer scripts and docs
  validate with syntax checks and focused exercises instead.
- Fixes need a reproducing test before the fix.
- Compatibility expectations come from pinned Credo behavior. Never
  auto-update them after a mismatch; mismatches mean the candidate is
  wrong until proven otherwise.
- Keep `cargo fmt`, Clippy with warnings denied, and the full test
  suite green at all times.
- Shell for developer scripts. No new dependencies without a concrete
  need.
- Update README, ARCHITECTURE and ROADMAP in the same commit when a
  change affects them.
