# CLI parity table (P1)

Pinned reference: Credo `ea1ccb9`, Elixir 1.20.2 / OTP 29.
Probes: `work/p1-priority-probe.md`, `work/p1-flags-probe.md`,
`work/p1-formats-probe.md` (scratch, gitignored); every row below was
observed against the pinned checkout, then covered by a Rust test or an
explicit carve-out.

## Priority filtering

| Flag | Native semantics | qredo |
|---|---|---|
| (default) | `min_priority = 0`, keep `priority >= 0` | `resolve_min_priority` → 0, unit-tested |
| `--strict`, `--all-priorities`, `-A` | `-99` (alias) | both flags + `-A`, unit-tested |
| `-a` / `--all` | display only: lifts 5-per-category truncation, no filter change | accepted, stored; truncation is a P6 formatter concern |
| `--min-priority LEVEL` | names `higher/high/normal/low/ignore` = 20/10/1/-10/-100, else integers, inclusive `>=`; explicit value beats `--strict`/`-A` regardless of order; invalid → `RuntimeError` crash, exit 1 | same table + precedence + `** (RuntimeError)` shape, unit-tested |
| check-level pre-exclusion | checks with base `< min_priority - 9` never run | engine runs all served checks; issue-level `>=` filtering matches observed sets (low checks served: TrailingWS etc. verified in differential) |
| `--mute-exit-status` | issues listed, exit 0 | same, in `report_exit` |

## Selection

| Flag | Native semantics | qredo |
|---|---|---|
| `--only` / `--checks` / `-c` | comma-split, repeatable (union), case-insensitive regex vs check name; invalid regex → `MatchError` crash exit 1 | `Selection.only` + `compile_selection` pre-check, unit-tested |
| `--ignore` / `--ignore-checks` / `-i` | same shape; wins over `--only` | same, unit-tested |
| `--checks-with-tag` | repeatable union over `:formatter` (12) / `:controversial` (22) / `:experimental` (1); unknown tag matches nothing | `check_tags` table + `Selection.checks_with_tag`, unit-tested |
| `--checks-without-tag` | **no-op** at this pin (writes unread key) | accepted, ignored, unit-tested via parse |
| `--enable-disabled-checks` | comma-split case-insensitive regex; matches move from `disabled` to enabled | `Selection.enable_disabled` + `runner_of` re-enable, integration-tested |
| `--files-included` | **dead** for `suggest`/`list` (positional handling wipes it); live for `info`/`diff` | accepted, ignored for `suggest` |
| `--files-excluded` | repeatable union, live everywhere | applied to relative + absolute names, discovery-tested |

## Discovery

| Input | Native semantics | qredo |
|---|---|---|
| no positionals | config `files.included` (default `lib/` + `test/`) | same, discovery-tested |
| first positional is a dir | becomes working dir; rest are file patterns | `split_root`, tested via `run_with` |
| file positionals | union | same |
| globs | expanded | `expand_glob`, discovery-tested |
| missing `*.ex`/`*.exs` literal | `File.Error` crash, exit 1 (first line deterministic) | `unreadable_file`, tested |
| missing anything else | contributes nothing; empty set → `No files found!`, exit 0 | same, tested |
| `--working-dir` | resolution root | same |

## Exit codes

| Situation | Native | qredo |
|---|---|---|
| issues | OR of category bits (consistency 1, design 2, readability 4, refactor 8, warning 16) over post-filter issues | `report.exit_status`, differential-verified |
| clean | 0 | same |
| `--mute-exit-status` | 0 with issues listed | same |
| missing `--config-file` | 129 `** (config) Given config file does not exist:` | same shape + code, tested |
| malformed config | 129 | 129 via `parse_config` failure (message shape differs; native prints an Elixir warning + stack — carve-out, see below) |
| wrong-shape config / bad regex / bad priority | crash, exit 1 | exit 1 with first-line shapes, tested |
| unknown switch / missing flag value | exit 130 `** (credo) Unknown switch` | same shape + code, tested |
| unserved config (custom params, project checks, unknown checks) | n/a (native runs) | fail-closed 2 with reason |
| `version` / `help` | `1.8.0-dev` / help text, exit 0 | version prints crate version, help texts present, exit 0 |
| `list categories info explain diff gen.check gen.config` | full subcommands | explicit `Unimplemented` refusal, exit 2 (P1d follow-up per command) |

## Carve-outs (documented, not silent)

- Default human formatter bytes (`Checking …`, `┃` layout, `Analysis took …`,
  summary counts, `--all` truncation): placeholder oneline output until P6.
  `--format json` stays differential JSONL (native single-object shape is P6).
- Malformed-config stderr prints qredo's static-parse reason, not the native
  Elixir warning + stack trace; exit code 129 matches.
- Missing-file crash prints the first `** (File.Error)` line only, not the
  process-specific `Task` frames.
- `version` prints the qredo crate version, not `1.8.0-dev`.
- Sibling subcommands (`list`, `explain`, `info`, `categories`, `diff`,
  `gen.check`, `gen.config`) refuse with exit 2 pending per-command work.
