# Generate development probes, not expectations. Pipe through duplicated-oracle
# in the pinned checkout, then run the ignored EX2002 project/shape contracts.
root = Path.expand("..", __DIR__)

shapes =
  root
  |> Path.join("crates/qredo/compatibility/duplicated-shapes.json")
  |> File.read!()
  |> :json.decode()

sources = fn body ->
  for name <- ["A", "B"] do
    %{
      "filename" => String.downcase(name) <> ".ex",
      "source" => "defmodule #{name} do\n def f(x) do\n#{body}\n end\nend\n"
    }
  end
end

body = Enum.map_join(1..20, "\n", fn i -> "x = call(x, #{i})" end)
base = sources.(body)

thresholds =
  for mass <- [0, 1, 3, 39, 40, 41, 82, 83, 84, 1000], nodes <- [0, 1, 2, 3] do
    %{
      "id" => "EX2002.threshold.#{mass}.#{nodes}",
      "sources" => base,
      "params" => %{"mass_threshold" => mass, "nodes_threshold" => nodes}
    }
  end

syntax =
  for {shape, i} <- Enum.with_index(shapes) do
    %{
      "id" => "EX2002.syntax.#{i}",
      "sources" => sources.(shape["source"]),
      "params" => %{"mass_threshold" => 1}
    }
  end

excluded =
  for macro <- ["def", "test", "if", "@", "__block__"], threshold <- [1, 40] do
    %{
      "id" => "EX2002.excluded.#{macro}.#{threshold}",
      "sources" => sources.("test \"x\" do\nif x do\n#{body}\nend\nend"),
      "params" => %{"mass_threshold" => threshold, "excluded_macros" => [macro]}
    }
  end

nested = %{
  "id" => "EX2002.excluded.nested-keyword-value",
  "sources" => sources.("foo(do: [do: bar(x)])"),
  "params" => %{"mass_threshold" => 1, "excluded_macros" => ["bar"]}
}

IO.puts(:json.encode(%{"projects" => thresholds ++ syntax ++ excluded ++ [nested]}))
