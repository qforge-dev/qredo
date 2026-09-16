# Roadmap

Compatibility target: Credo `ea1ccb9` on Elixir 1.20.2 / OTP 29.
"Compatible" means identical configured checks, parameters, file
handling, diagnostics and exit behavior on served inputs — never a
relabelled discrepancy. New Credo versions need their own reviewed
inventory.

## Now

- 120 rule kernels with default parameters; admitted corpora green in
  `cargo test`.
- 7-stage pipeline runner with scope, priority, suppression and exit
  status.
- `qredo` CLI over configured files with `--strict`, `--format`,
  `--only`/`--ignore`, `--config-name`, `--min-priority` and
  `--mute-exit-status`, failing closed on unsupported configs.
- Per-file shared parse and facts; residual tree walks pinned by test.

## Next

- Custom check parameters behind the differential gate (real-target
  native-vs-qredo comparisons authorize each expansion).
- Remaining tree-walk migrations (statement-role and argument-content
  facts) per the residual list in `src/facts.rs`.
- Deeper CLI parity: default human-readable formatter matching Credo's
  layout, `mix credo` exit-code edge cases, stdin input, umbrella
  project handling.
- Differential campaigns on real projects: identical issue sets and
  exit codes vs `mix credo --strict`, with work counts beside timings.
- Distribution: versioned releases with prebuilt binaries; `cargo
  install` works today.

## Non-goals

Executable `.credo.exs` extensions, plugins and custom checks execute
Elixir and stay out of scope for the native engine: they fail closed
with a reason. Full upstream transcription beyond differential demand
is explicitly not pursued — rule work follows real-target failures.
