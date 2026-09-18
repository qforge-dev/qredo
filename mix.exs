defmodule Qredo.MixProject do
  use Mix.Project

  @version File.read!(Path.join(__DIR__, "VERSION")) |> String.trim()

  def project do
    [
      app: :qredo,
      version: @version,
      elixir: "~> 1.15",
      description: "Native Elixir linting with Credo-compatible behavior",
      source_url: "https://github.com/qforge-dev/qredo",
      homepage_url: "https://github.com/qforge-dev/qredo",
      package: package(),
      docs: [
        main: "readme",
        extras: [
          "README.md",
          "crates/qredo/ROADMAP.md",
          "crates/qredo/ARCHITECTURE.md",
          "crates/qredo/AGENTS.md"
        ]
      ],
      deps: deps()
    ]
  end

  def application do
    [extra_applications: [:crypto, :inets, :ssl]]
  end

  defp deps do
    [
      {:ex_doc, "~> 0.40.4", only: :dev, runtime: false}
    ]
  end

  defp package do
    [
      licenses: ["MIT"],
      links: %{"GitHub" => "https://github.com/qforge-dev/qredo"},
      files: [
        "lib",
        "crates/qredo/src",
        "crates/qredo/compatibility/rules.json",
        "crates/qredo/compatibility/README.md",
        "crates/qredo/compatibility/upstream/inventory.json",
        "crates/qredo/compatibility/upstream/credo_default_config.exs",
        "crates/qredo/Cargo.toml",
        "crates/qredo/ROADMAP.md",
        "crates/qredo/ARCHITECTURE.md",
        "crates/qredo/AGENTS.md",
        "mix.exs",
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
