%{
  configs: [
    %{
      name: "default",
      files: %{included: ["lib/", "test/"]},
      checks: %{
        enabled: [
          {Credo.Check.Warning.IoInspect, []},
          {Credo.Check.Warning.Dbg, []},
          {Credo.Check.Readability.ModuleDoc, []}
        ]
      }
    }
  ]
}
