# qredo

Native Elixir linting with Credo-compatible behavior, written in Rust.

`qredo` runs Credo checks without the BEAM: a rule-kernel library, a
7-stage pipeline runner (selection, scope, priority, suppression, exit
status) and a `qredo` CLI that reads your project's `.credo.exs` and
prints issues with OR-combined exit codes, like `mix credo`.

Status: early. 120 rule kernels exist and the admitted compatibility
corpora pass; the CLI serves static default-parameter configs and fails
closed with an explicit reason on anything else. See [ROADMAP](crates/qredo/ROADMAP.md).

## Install

Requires Rust 1.91.1 (pinned in `rust-toolchain.toml`) and no Elixir
installation for linting itself:

```sh
cargo install --path crates/qredo
```

## Usage

```sh
qredo [PATH] [--config-file FILE] [--strict] [--format text|json]
```

```sh
qredo apps/my_app --strict
```

On an unsupported configuration qredo prints the reason and exits 2
instead of running partial analysis.

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

See [AGENTS.md](crates/qredo/AGENTS.md) for the contribution workflow,
[ARCHITECTURE.md](crates/qredo/ARCHITECTURE.md) for the design and
[compatibility](crates/qredo/compatibility/README.md) for provenance.

## License

MIT. Compatibility corpora derived from Credo's test suite keep their
upstream MIT license and attribution; see
[compatibility](crates/qredo/compatibility/README.md). The `upstream`
directory additionally carries Credo's own `LICENSE` file.
