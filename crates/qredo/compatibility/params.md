# Upstream case param encoding

`compatibility/cases/*.json` records upstream `run_check` params as JSON values
mirroring the Elixir literals:

| Elixir literal | JSON encoding | Kernel string param |
| --- | --- | --- |
| `true` / `false` | `true` / `false` | `"true"` / `"false"` |
| integer (e.g. `4`, `120`) | number | decimal rendering |
| atom (e.g. `:unix`, `:alpha`, `Map`) | `":unix"`, `":alpha"`, `":Elixir.Map"` | atom name without the colon |
| plain string | string | verbatim, EXCEPT a leading `:` is stripped (see below) |
| `~w()` word-list strings (e.g. `~w(:crypto.hash)`) | `":crypto.hash"` (indistinguishable from an atom) | kernels compare colon-tolerantly |
| list | array (elements encoded recursively) | compact JSON |
| tuple (e.g. MFA `{Mod, :fun, "msg"}`) | `{"tuple": [...]}` | compact JSON |
| regex (e.g. `~r/.../`) | `{"regex": "<source>"}` | compact JSON |
| range (e.g. `2..4`) | `{"range": [2, 4]}` | compact JSON |
| `nil` | `null` | `"null"` at the gate, then omitted so the check default applies |

The generic harness (`tests/cases.rs`) converts each value with the middle
column rule; per-check kernels parse the resulting strings. A leading-`:`
string is ambiguous between an atom encoding and a genuine `~w()` colon-word,
so kernels MUST compare such values colon-tolerantly (accept with or without
the colon). Structured params (regexes, tuples, ranges, lists) are admitted
only when their check implements and validates the exact shape. General `tags`
is an atom list in this encoding; `:__initial__` expands to the check's
built-in tags during CLI tag selection, matching Credo.
