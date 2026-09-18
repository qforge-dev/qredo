defmodule Mix.Tasks.Qredo do
  use Mix.Task

  @shortdoc "Runs the native qredo linter"

  @impl Mix.Task
  def run(args) do
    case Qredo.run(args) do
      0 -> :ok
      status -> System.halt(status)
    end
  end
end
