defmodule Qredo.MixProject do
  use Mix.Project

  @version File.read!(Path.join(__DIR__, "VERSION")) |> String.trim()

  def project do
    [
      app: :qredo,
      version: @version,
      elixir: "~> 1.15",
      description: "Native Elixir linting with Credo-compatible behavior",
      package: package(),
      deps: []
    ]
  end

  def application do
    [extra_applications: [:crypto, :inets, :ssl]]
  end

  defp package do
    [
      licenses: ["MIT"],
      links: %{"GitHub" => "https://github.com/qforge-dev/qredo"},
      files: [
        "lib",
        "crates/qredo/src",
        "crates/qredo/compatibility/rules.json",
        "crates/qredo/compatibility/upstream/inventory.json",
        "crates/qredo/compatibility/upstream/credo_default_config.exs",
        "crates/qredo/Cargo.toml",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "VERSION",
        "README.md",
        "LICENSE-MIT"
      ]
    ]
  end
end
