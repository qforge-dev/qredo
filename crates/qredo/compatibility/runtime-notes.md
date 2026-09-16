# Message parity notes (P4) and runtime matrix (P5)

Pinned reference: Credo `ea1ccb9`, Elixir 1.20.2 / OTP 29.

## Messages: 109 match, 11 decided

Fixed in this phase (unit-tested, corpus-green):

- EX5033 `ForbiddenFunction`: nil-message entries fall back to
  `` "Calls to `{trigger}` are not allowed." `` like upstream
  (`message || default`). 2-tuples stay conservative-empty (upstream
  crashes them → warn-and-skip; no issues either way).
- EX3018 `PreferUnquotedAtoms`: trigger/message always render `:"atom"`
  regardless of source quote.
- EX2002 `DuplicatedCode` single-file wording follows the collector
  template with the checked filename (threaded through the kernel lane;
  the anonymous API path keeps the generic rendering).

Accepted deviations (documented, not silent):

- Per-file consistency kernels (EX1001/1003/1004/1005/1006/1008) use
  approximation messages (e.g. `"Use spaces around operators
  consistently."`): the single-file path cannot claim a project majority,
  so it must not emit `"most of the time"` wording. Project collectors
  carry the exact upstream templates and match verbatim.
- EX2007 `MissingCheckInConfig`: conservative empty (needs project config
  inventory; no false message emitted).
- EX2008 single-file kernel keeps its placeholder: without config-shape
  context it cannot choose between the two upstream templates. The exact
  templates live in `config_checks::deprecated_config`, corpus-pinned via
  `tests/config.rs`.

## Runtime matrix (P5)

Version gates (execution-level only; native `run/2` bypasses them, like
`check_kernel`): on the pinned toolchain native skips EX5007
(`< 1.7.0`), EX3018 (`< 1.7.0-dev`) and EX4013 (`< 1.8.0`) with zero
issues. `version_skipped_on_pinned_toolchain` (`check_meta.rs`) mirrors
this in `run_check_entry`; EX5026 (`>= 1.14.0-dev`) and EX4034
(`>= 1.17.0`) run on both sides.

Ambient Logger config (EX5027) is the one documented carve-out: with an
empty ambient config qredo matches (approximation pinned by unit
contracts); a non-empty real `default_formatter`/`console` metadata
config, or ambient `:all`, suppresses native issues qredo still reports.
Bridging live application env is future work (see P6 executable-config
decision); until then this is a stated limitation, not a silent gap.
`metadata_keys` params (including `:all`) match unconditionally.

Name-only checks (EX5001/EX5008/EX5010) match: pure syntax on both sides,
no runtime reads.
