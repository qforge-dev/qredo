defmodule Mix.Tasks.Qredo.Install do
  use Mix.Task

  @shortdoc "Downloads or builds the native qredo executable"

  @impl Mix.Task
  def run(args) do
    {opts, rest} =
      OptionParser.parse!(args,
        strict: [force: :boolean, source: :boolean]
      )

    if rest != [], do: Mix.raise("unexpected arguments: #{Enum.join(rest, " ")}")

    path =
      Qredo.install!(
        force: Keyword.get(opts, :force, false),
        source: Keyword.get(opts, :source, false)
      )

    Mix.shell().info(path)
  end
end
