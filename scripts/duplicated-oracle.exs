# Development-only oracle. Run in the pinned Credo checkout:
# mix run /path/to/scripts/duplicated-oracle.exs INPUT.json > OUTPUT.json
# Writes to stdout only; reviewed expectations are never overwritten.
defmodule DuplicatedOracle do
  alias Credo.Check.Design.DuplicatedCode, as: Check

  def term(value) when is_atom(value), do: ["atom", Atom.to_string(value)]
  def term(value) when is_binary(value), do: ["bytes", :binary.bin_to_list(value)]
  def term(value) when is_integer(value), do: ["integer", Integer.to_string(value)]

  def term(value) when is_float(value) do
    <<bits::unsigned-64>> = <<value::float-64>>
    ["float", Integer.to_string(bits)]
  end

  def term(value) when is_list(value), do: ["list", Enum.map(value, &term/1)]
  def term({left, right}), do: ["pair", term(left), term(right)]

  def term({head, _meta, args}),
    do: ["call", term(head), if(is_list(args), do: Enum.map(args, &term/1), else: nil)]

  def shape(source) do
    {:ok, ast} = Credo.Code.ast(source)
    identity = :crypto.hash(:sha256, Jason.encode!(term(ast))) |> Base.encode16(case: :lower)
    result = %{"source" => source, "mass" => Check.mass(ast), "identity" => identity}
    if System.get_env("QREDO_ORACLE_TERMS"), do: Map.put(result, "term", term(ast)), else: result
  end

  def project(entry) do
    files =
      Enum.map(entry["sources"], fn f -> Credo.SourceFile.parse(f["source"], f["filename"]) end)

    params =
      Enum.map(entry["params"] || %{}, fn {key, value} ->
        value =
          if key == "excluded_macros",
            do: Enum.map(value, &String.to_atom(String.trim_leading(&1, ":"))),
            else: value

        {String.to_atom(key), value}
      end)

    issues = Credo.Test.CheckRunner.run_check(files, Check, params)

    findings =
      Enum.map(issues, fn i ->
        %{
          "filename" => i.filename,
          "line" => i.line_no,
          "column" => i.column,
          "message" => i.message,
          "severity" => i.severity,
          "trigger" => "no_trigger"
        }
      end)
      |> Enum.sort_by(&{&1["filename"], &1["line"], &1["message"]})

    Map.put(entry, "findings", findings)
  end
end

[input] = System.argv()
data = input |> File.read!() |> Jason.decode!()

result = %{
  "pin" => "ea1ccb9023b44eecbe079dc4bfd48cca4e8b0187",
  "shapes" =>
    Enum.map(data["shapes"] || [], fn
      source when is_binary(source) -> DuplicatedOracle.shape(source)
      %{"source" => source} -> DuplicatedOracle.shape(source)
    end),
  "projects" => Enum.map(data["projects"] || [], &DuplicatedOracle.project/1)
}

IO.puts(Jason.encode!(result))
