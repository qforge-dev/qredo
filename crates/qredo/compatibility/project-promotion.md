# Project-check promotion (P3)

Pinned reference: Credo `ea1ccb9`, Elixir 1.20.2 / OTP 29.

## Gate split

Project-lane checks aggregate votes across the full file set, so one gate
cannot serve both paths:

- `select()` (full runs: `execute`, `execute_selected`, the CLI) serves
  promoted checks via `promoted_project_check` (`runner.rs`).
- `select_subset()` (`execute_files`) refuses every project-lane check
  with `project-scope-check:*`, promoted or not: a lone minority file is
  clean natively but blamed in-project (proven: lone `tabs/c.ex` → 0
  issues natively vs 4 in the 3-file project).
- `supports_per_file()` stays false for all project-lane checks (subset
  unsoundness is a property of the check, not of promotion).

Unsupported check params stay fail-closed (`custom-check-params:*`). All five
general parameters and all 81 documented rule-specific options are validated
against scalar and structured schemas; configured `tags` also drive CLI tag
selection. Parameters do not bypass the separate project/config check gates.
The evidence below covers default project-check params only.

## Promoted (8 consistency checks)

| Check | Synthetic probes | Real target (labqoat-web `lib/`, 580 files) | Corpus | Tie/revert |
|---|---|---|---|---|
| EX1001 ExceptionNames | file/line/column/scope/priority/category/trigger/message MATCH | 0/0 clean both sides | admitted_project passes | tie test |
| EX1002 LineEndings | MATCH | 0/0 | passes | tie test |
| EX1003 MultiAliasImportRequireUse | MATCH | 46/46, 6-field agreement | passes | tie test |
| EX1004 ParameterPatternMatching | MATCH | 0/0 | passes | tie test |
| EX1005 SpaceAroundOperators | MATCH (after operator-atom fix) | 0/0 after fix (was 76 qredo-only) | passes | tie test |
| EX1006 SpaceInParentheses | MATCH | 0/0 | passes | tie test |
| EX1007 TabsOrSpaces | MATCH incl. 4v4 tie → smallest key | 0/0 | passes | tie test |
| EX1008 UnusedVariableNames | MATCH | 211/211, 6-field agreement | passes | tie test |

Probe details (synthetic 3-file majorities, invalidation sequences,
verbatim native outputs): `work/p3-evidence.md`, `work/p3-real-target.md`
(scratch). Majority semantics verified: highest count wins, ties go to the
smallest key, `force` overrides, empty votes → clean, single-style files
never blamed (except ExceptionNames single-match suppression, mirrored).

## Kept gated

- `Credo.Check.Design.DuplicatedCode` (`project-scope-check:*`): exact
  match on small probes, but message peer paths are absolute, chunk-scale
  ordering is unverified, and near-threshold edits flip whole groups.
  Promote after Jason-scale differential + edit/revert blame stability +
  message path relativization.
- `needs-validated-config` (EX2007/EX2008): require full project config
  inventory; exact message templates already live in `config_checks`
  (corpus-pinned via `tests/config.rs`), served by the real-Credo fallback.
- Custom params on promoted checks: lane-level corpus proof exists, but
  flow-level discovery/selection interplay per param is unproven.
