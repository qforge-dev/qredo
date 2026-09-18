# qredo

Native Elixir linting with Credo-compatible behavior, written in Rust.

`qredo` runs Credo checks without the BEAM: a rule-kernel library, a
7-stage pipeline runner (selection, scope, priority, suppression, exit
status) and a `qredo` CLI that reads your project's `.credo.exs` and
prints issues with OR-combined exit codes, like `mix credo`.

Status: early. 120 rule kernels exist and the admitted compatibility
corpora pass; the CLI serves static configs with all 81 documented
rule-specific parameters and all five general parameters validated by schema,
including explicit `nil` values that select Credo defaults, and fails closed
with an explicit reason on anything else. See
[ROADMAP](crates/qredo/ROADMAP.md).

## Install with Mix

Add qredo from Hex as a development dependency:

```elixir
defp deps do
  [
    {:qredo, "~> 0.1.0", only: [:dev, :test], runtime: false}
  ]
end
```

Then run it through Mix; the first invocation downloads and SHA-256 verifies
the native executable for the current platform:

```sh
mix deps.get
mix qredo --strict
```

Set `QREDO_BUILD=source` or run `mix qredo.install --source` to compile with
the pinned Rust toolchain instead. `QREDO_BINARY_PATH` selects an existing
binary, and `QREDO_OFFLINE=1` prevents downloads.

## Install with Cargo

Requires Rust 1.91.1 (pinned in `rust-toolchain.toml`) and no Elixir
installation for linting itself:

```sh
cargo install --path crates/qredo
```

## Usage

```sh
qredo [PATH] [--config-file FILE] [--config-name NAME] [--strict] [--min-priority N] [--mute-exit-status] [--only CHECK,...] [--ignore CHECK,...] [--format text|json] [--stale]
```

```sh
qredo apps/my_app --strict
qredo --only IoInspect,Dbg --format json
qredo --stale  # reuse cached results for unchanged files
```

Files come from the config's `files.included` (defaulting to `lib/` and
`test/`), minus `files.excluded`; build directories are never
descended into. On an unsupported configuration qredo prints the
reason and exits 2 instead of running partial analysis.

## Incremental runs (`--stale`)

`qredo suggest --stale` (and `list --stale`) reuse cached results for
unchanged files: per-file issues are keyed by content hash, consistency
votes by per-file counts, and the global majority is recomputed from
merged counts without re-parsing cached files. The cache lives under
`~/.cache/qredo/` (or `$XDG_CACHE_HOME/qredo`), keyed by project root
plus a fingerprint over tool version, config bytes, environment snapshot,
check list, selection and minimum priority. Any mismatch — or a flipped
consistency majority — fails open to a full run, so `--stale` output
always matches a fresh run.

## Library

```rust
use qredo::check_kernel;

let findings = check_kernel(
    "Credo.Check.Readability.TrailingBlankLine",
    "defmodule Example do\nend",
).unwrap();
assert_eq!(findings[0].line, 2);
```

Full-pipeline runs go through `integration::execute`, which serves
supported configs and reports an explicit `Fallback` reason otherwise.

## Development

```sh
scripts/check  # rustfmt, Clippy (-D warnings), tests
```

## Releases

Every push to `main` runs the complete quality gate, builds five native targets
on two runners, and replaces the `dev` GitHub prerelease. Development asset
names include the exact Git commit consumed by a Mix git dependency.

Publishing a non-prerelease GitHub release such as `v0.1.0` verifies that its
tag matches both `VERSION` and `crates/qredo/Cargo.toml`, uploads immutable
native assets plus `SHA256SUMS`, marks it latest, and publishes the matching
Mix package to Hex. The repository must define a `HEX_API_KEY` Actions secret
with package publishing permission.

See [AGENTS.md](crates/qredo/AGENTS.md) for the contribution workflow,
[ARCHITECTURE.md](crates/qredo/ARCHITECTURE.md) for the design and
[compatibility](crates/qredo/compatibility/README.md) for provenance.

## License

MIT. Compatibility corpora derived from Credo's test suite keep their
upstream MIT license and attribution; see
[compatibility](crates/qredo/compatibility/README.md). The `upstream`
directory additionally carries Credo's own `LICENSE` file.
