//! Per-check documentation transcribed from the pinned upstream Credo
//! (`work/ref/credo`, `ea1ccb9`).
//!
//! Each entry mirrors one check's `use Credo.Check` block: the check `id`,
//! `category`, `base_priority` and `tags`, the `check:` explanation prose
//! byte-verbatim, and every `param_defaults` entry with its default
//! rendering plus the `params:` doc string.
//!
//! Conventions (all verified against the pinned checkout):
//!
//! * `category` is the explicit `category:` opt when present (10 checks);
//!   otherwise it is the module-path segment lowercased, which is exactly
//!   what upstream `category_body(nil)` derives at compile time.
//! * `base_priority` is the `base_priority:` atom name (`higher`, `high`,
//!   `normal`, `low`); checks without the opt report `"0"`, matching the
//!   integer upstream `base_priority/0` returns by default.
//! * `tags` is the `tags:` atom list, empty for the 85 untagged checks.
//! * `params` keeps `param_defaults` order. `default` renders the runtime
//!   value with Elixir `inspect/2` (`limit: :infinity`), so sigils and
//!   alias lists round-trip (`~r/.../`, `[Credo.Check, ...]`).
//! * `doc` is `""` for the 6 params upstream leaves undocumented
//!   (`files` on four checks, `ignore_heredocs`, `parens`).
//!
//! Verification: all 120 entries (ids, explanation bytes, categories,
//! bases, tags, param names/defaults/docs) were byte-compared against the
//! compiled checkout's runtime (`id/0`, `explanations/0`, `category/0`,
//! `base_priority/0`, `tags/0`, `param_defaults/0`) with zero mismatches.
//! See `crates/qredo/compatibility/upstream/inventory.json` for the
//! pinned 120-check ledger this table covers.

/// Documentation for one check parameter: its name, default rendering
/// and doc string (empty when upstream documents none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamDoc {
    /// Parameter name, e.g. `max_length`.
    pub name: &'static str,
    /// Default rendered with Elixir `inspect/2`, e.g. `120`, `true`,
    /// `~r/foo$/`.
    pub default: &'static str,
    /// Doc string from the `params:` explanations, or `""`.
    pub doc: &'static str,
}

/// Documentation for one upstream check, transcribed verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckDoc {
    /// Full check module name, e.g. `Credo.Check.Readability.ModuleDoc`.
    pub module: &'static str,
    /// Check `id/0`, e.g. `EX3009`.
    pub id: &'static str,
    /// Lowercase category name, e.g. `readability`.
    pub category: &'static str,
    /// Base priority name (`higher`, `high`, `normal`, `low`) or `"0"`.
    pub base_priority: &'static str,
    /// Tag atoms as strings, e.g. `formatter`.
    pub tags: &'static [&'static str],
    /// Verbatim `check:` explanation prose.
    pub explanation: &'static str,
    /// Parameter docs in `param_defaults` order.
    pub params: &'static [ParamDoc],
}

/// Pinned per-check documentation for all 120 upstream checks.
static ALL_DOCS: &[CheckDoc] = &[
    // --- consistency ---
    CheckDoc {
        module: "Credo.Check.Consistency.ExceptionNames",
        id: "EX1001",
        category: "consistency",
        base_priority: "high",
        tags: &[],
        explanation: "Exception names should end with a common suffix like \"Error\".\n\nTry to name your exception modules consistently:\n\n    defmodule BadCodeError do\n      defexception [:message]\n    end\n\n    defmodule ParserError do\n      defexception [:message]\n    end\n\nInconsistent use should be avoided:\n\n    defmodule BadHTTPResponse do\n      defexception [:message]\n    end\n\n    defmodule HTTPHeaderException do\n      defexception [:message]\n    end\n\nWhile this is not necessarily a concern for the correctness of your code,\nyou should use a consistent style throughout your codebase.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Consistency.LineEndings",
        id: "EX1002",
        category: "consistency",
        base_priority: "high",
        tags: &["formatter"],
        explanation: "Windows and Linux/macOS systems use different line-endings in files.\n\nIt seems like a good idea not to mix these in the same codebase.\n\nWhile this is not necessarily a concern for the correctness of your code,\nyou should use a consistent style throughout your codebase.\n",
        params: &[ParamDoc {
            name: "force",
            default: "nil",
            doc: "Force a choice, values can be `:unix` or `:windows`.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Consistency.MultiAliasImportRequireUse",
        id: "EX1003",
        category: "consistency",
        base_priority: "high",
        tags: &["controversial"],
        explanation: "When using alias, import, require or use for multiple names from the same\nnamespace, you have two options:\n\nUse single instructions per name:\n\n    alias Ecto.Query\n    alias Ecto.Schema\n    alias Ecto.Multi\n\nor use one multi instruction per namespace:\n\n    alias Ecto.{Query, Schema, Multi}\n\nWhile this is not necessarily a concern for the correctness of your code,\nyou should use a consistent style throughout your codebase.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Consistency.ParameterPatternMatching",
        id: "EX1004",
        category: "consistency",
        base_priority: "high",
        tags: &[],
        explanation: "When capturing a parameter using pattern matching you can either put the parameter name before or after the value\ni.e.\n\n    def parse({:ok, values} = pair)\n\nor\n\n    def parse(pair = {:ok, values})\n\nNeither of these is better than the other, but it seems a good idea not to mix the two patterns in the same codebase.\n\nWhile this is not necessarily a concern for the correctness of your code,\nyou should use a consistent style throughout your codebase.\n",
        params: &[ParamDoc {
            name: "force",
            default: "nil",
            doc: "Force a choice, values can be `:after` or `:before`.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Consistency.SpaceAroundOperators",
        id: "EX1005",
        category: "consistency",
        base_priority: "high",
        tags: &["formatter"],
        explanation: "Use spaces around operators like `+`, `-`, `*` and `/`. This is the\n**preferred** way, although other styles are possible, as long as it is\napplied consistently.\n\n    # preferred\n\n    1 + 2 * 4\n\n    # also okay\n\n    1+2*4\n\nWhile this is not necessarily a concern for the correctness of your code,\nyou should use a consistent style throughout your codebase.\n",
        params: &[ParamDoc {
            name: "ignore",
            default: "[:|]",
            doc: "List of operators to be ignored for this check.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Consistency.SpaceInParentheses",
        id: "EX1006",
        category: "consistency",
        base_priority: "high",
        tags: &["formatter"],
        explanation: "Don't use spaces after `(`, `[`, and `{` or before `}`, `]`, and `)`. This is\nthe **preferred** way, although other styles are possible, as long as it is\napplied consistently.\n\n    # preferred\n\n    Helper.format({1, true, 2}, :my_atom)\n\n    # also okay\n\n    Helper.format( { 1, true, 2 }, :my_atom )\n\nWhile this is not necessarily a concern for the correctness of your code,\nyou should use a consistent style throughout your codebase.\n",
        params: &[ParamDoc {
            name: "allow_empty_enums",
            default: "false",
            doc: "Allows [], %{} and similar empty enum values to be used regardless of spacing throughout the codebase.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Consistency.TabsOrSpaces",
        id: "EX1007",
        category: "consistency",
        base_priority: "high",
        tags: &["formatter"],
        explanation: "Tabs should be used consistently.\n\nNOTE: This check does not verify the indentation depth, but checks whether\nor not soft/hard tabs are used consistently across all source files.\n\nIt is very common to use 2 spaces wide soft-tabs, but that is not a strict\nrequirement and you can use hard-tabs if you like that better.\n\nWhile this is not necessarily a concern for the correctness of your code,\nyou should use a consistent style throughout your codebase.\n",
        params: &[ParamDoc {
            name: "force",
            default: "nil",
            doc: "Force a choice, values can be `:spaces` or `:tabs`.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Consistency.UnusedVariableNames",
        id: "EX1008",
        category: "consistency",
        base_priority: "high",
        tags: &[],
        explanation: "Elixir allows us to use `_` as a name for variables that are not meant to be\nused. But it’s a common practice to give these variables meaningful names\nanyway (`_user` instead of `_`), but some people prefer to name them all anonymously (`_`).\n\nA single style should be present in the same codebase.\n",
        params: &[ParamDoc {
            name: "force",
            default: "nil",
            doc: "Force a choice, values can be `:meaningful` or `:anonymous`.",
        }],
    },
    // --- design ---
    CheckDoc {
        module: "Credo.Check.Design.AliasUsage",
        id: "EX2001",
        category: "design",
        base_priority: "normal",
        tags: &[],
        explanation: "Functions from other modules should be used via an alias if the module's\nnamespace is not top-level.\n\nWhile this is completely fine:\n\n    defmodule MyApp.Web.Search do\n      def twitter_mentions do\n        MyApp.External.TwitterAPI.search(...)\n      end\n    end\n\n... you might want to refactor it to look like this:\n\n    defmodule MyApp.Web.Search do\n      alias MyApp.External.TwitterAPI\n\n      def twitter_mentions do\n        TwitterAPI.search(...)\n      end\n    end\n\nThe thinking behind this is that you can see the dependencies of your module\nat a glance. So if you are attempting to build a medium to large project,\nthis can help you to get your boundaries/layers/contracts right.\n\nAs always: This is just a suggestion. Check the configuration options for\ntweaking or disabling this check.\n",
        params: &[
            ParamDoc {
                name: "excluded_namespaces",
                default: "[\"File\", \"IO\", \"Inspect\", \"Kernel\", \"Macro\", \"Supervisor\", \"Task\", \"Version\"]",
                doc: "List of namespaces to be excluded for this check.",
            },
            ParamDoc {
                name: "excluded_lastnames",
                default: "[\"Access\", \"Agent\", \"Application\", \"Atom\", \"Base\", \"Behaviour\", \"Bitwise\", \"Code\", \"Date\", \"DateTime\", \"Dict\", \"Enum\", \"Exception\", \"File\", \"Float\", \"GenEvent\", \"GenServer\", \"HashDict\", \"HashSet\", \"Integer\", \"IO\", \"Kernel\", \"Keyword\", \"List\", \"Macro\", \"Map\", \"MapSet\", \"Module\", \"NaiveDateTime\", \"Node\", \"OptionParser\", \"Path\", \"Port\", \"Process\", \"Protocol\", \"Range\", \"Record\", \"Regex\", \"Registry\", \"Set\", \"Stream\", \"String\", \"StringIO\", \"Supervisor\", \"System\", \"Task\", \"Time\", \"Tuple\", \"URI\", \"Version\"]",
                doc: "List of lastnames to be excluded for this check.",
            },
            ParamDoc {
                name: "if_called_more_often_than",
                default: "0",
                doc: "Only raise an issue if a module is called more often than this.",
            },
            ParamDoc {
                name: "if_nested_deeper_than",
                default: "0",
                doc: "Only raise an issue if a module is nested deeper than this.",
            },
            ParamDoc {
                name: "if_referenced",
                default: "false",
                doc: "Raise an issue if a module is referenced by name, e.g. as an argument in a function call.",
            },
            ParamDoc {
                name: "only",
                default: "nil",
                doc: "Regex or a list of regexes that specifies which modules to include for this check.\n\n`excluded_namespaces` and `excluded_lastnames` take precedence over this parameter.\n",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Design.DeprecatedChecksConfig",
        id: "EX2008",
        category: "design",
        base_priority: "normal",
        tags: &[],
        explanation: "Checks for an old `.credo.exs` config file format regarding checks.\n\nInstead of using a list for checks and deactivating them by setting the params to `false`:\n\n    %{\n      configs: [\n        %{\n          name: \"default\",\n          checks: [\n            # ...\n            {Credo.Check.Readability.LargeNumbers, false}\n          ]\n        }\n      ]\n    }\n\nUse a map with `:enabled` and `:disabled` keys. This way, the check's params get preserved:\n\n    %{\n      configs: [\n        %{\n          name: \"default\",\n          checks: %{\n            enabled: [\n              # ...\n            ],\n            diabled: [\n              {Credo.Check.Readability.LargeNumbers, only_greater_than: 99_999}\n            ]\n          }\n        }\n      ]\n    }\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Design.DuplicatedCode",
        id: "EX2002",
        category: "design",
        base_priority: "higher",
        tags: &["controversial"],
        explanation: "Code should not be copy-pasted in a codebase when there is room to abstract\nthe copied functionality in a meaningful way.\n\nThat said, you should by no means \"ABSTRACT ALL THE THINGS!\".\n\nSometimes it can serve a purpose to have code be explicit in two places, even\nif it means the snippets are nearly identical. A good example for this are\nDatabase Adapters in a project like Ecto, where you might have nearly\nidentical functions for things like `order_by` or `limit` in both the\nPostgres and MySQL adapters.\n\nIn this case, introducing an `AbstractAdapter` just to avoid code duplication\nmight cause more trouble down the line than having a bit of duplicated code.\n\nLike all `Software Design` issues, this is just advice and might not be\napplicable to your project/situation.\n",
        params: &[
            ParamDoc {
                name: "mass_threshold",
                default: "40",
                doc: "The minimum mass which a part of code has to have to qualify for this check.",
            },
            ParamDoc {
                name: "nodes_threshold",
                default: "2",
                doc: "The number of nodes that need to be found to raise an issue.",
            },
            ParamDoc {
                name: "excluded_macros",
                default: "[]",
                doc: "List of macros to be excluded for this check.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Design.MissingCheckInConfig",
        id: "EX2007",
        category: "design",
        base_priority: "high",
        tags: &[],
        explanation: "Knowing if a Credo config covers all existing checks can be difficult,\nespecially over time, when new checks are introduced.\n\nEnabling/disabling all relevant checks explicitly can help staying on top of this.\n",
        params: &[ParamDoc {
            name: "compare_to",
            default: ":credo_checks",
            doc: "Which set of checks should be considered when looking for missing checks:\n\n- `:all` - all checks ever mentioned in any transitive config\n- `:credo_checks` - all checks from Credo\n- `:credo_checks_enabled_by_default` - all checks from Credo that are enabled by default\n",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Design.RedundantConfigComments",
        id: "EX2006",
        category: "design",
        base_priority: "normal",
        tags: &[],
        explanation: "Config comments are sometimes left unchecked and become redundant.\n\nThis can happen because a comment ignored a check for a line that\nchanged or where the check was disabled via the config.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Design.SkipTestWithoutComment",
        id: "EX2003",
        category: "design",
        base_priority: "normal",
        tags: &[],
        explanation: "Skipped tests should have a comment documenting why the test is skipped.\n\nTests are often skipped using `@tag :skip` when some issue arises that renders\nthe test temporarily broken or unable to run. This temporary skip often becomes\na permanent one because the reason for the test being skipped is not documented.\n\nA comment should exist on the line prior to the skip tag describing why the test is\nskipped.\n\nExample:\n\n    # john: skipping this since our credentials expired, working on getting new ones\n    @tag :skip\n    test \"vendor api returns data\" do\n      # ...\n    end\n\nWhile the pure existence of a comment does not change anything per se, a thoughtful\ncomment can improve the odds for future iteration on the issue.\n",
        params: &[ParamDoc {
            name: "files",
            default: "%{included: [\"test/**/*_test.exs\", \"apps/**/test/**/*_test.exs\"]}",
            doc: "",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Design.TagFIXME",
        id: "EX2004",
        category: "design",
        base_priority: "high",
        tags: &[],
        explanation: "FIXME comments are used to indicate places where source code needs fixing.\n\nExample:\n\n    # FIXME: this does no longer work, research new API url\n    defp fun do\n      # ...\n    end\n\nThe premise here is that FIXME should indeed be fixed as soon as possible and\nare therefore reported by Credo.\n\nLike all `Software Design` issues, this is just advice and might not be\napplicable to your project/situation.\n",
        params: &[ParamDoc {
            name: "include_doc",
            default: "true",
            doc: "Set to `true` to also include tags from @doc attributes.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Design.TagTODO",
        id: "EX2005",
        category: "design",
        base_priority: "0",
        tags: &[],
        explanation: "TODO comments are used to remind yourself of source code related things.\n\nExample:\n\n    # TODO: move this to a Helper module\n    defp fun do\n      # ...\n    end\n\nThe premise here is that TODO should be dealt with in the near future and\nare therefore reported by Credo.\n\nLike all `Software Design` issues, this is just advice and might not be\napplicable to your project/situation.\n",
        params: &[ParamDoc {
            name: "include_doc",
            default: "true",
            doc: "Set to `true` to also include tags from @doc attributes.",
        }],
    },
    // --- readability ---
    CheckDoc {
        module: "Credo.Check.Readability.AliasAs",
        id: "EX3001",
        category: "readability",
        base_priority: "low",
        tags: &["experimental"],
        explanation: "Aliases can be \"renamed\" using the `:as` option, but that sometimes\nmakes the code more difficult to read.\n\n    # preferred\n\n    def MyModule do\n      alias MyApp.Module1\n\n      def my_function(foo) do\n        Module1.run(foo)\n      end\n    end\n\n    # NOT preferred\n\n    def MyModule do\n      alias MyApp.Module1, as: M1\n\n      def my_function(foo) do\n        # what the heck is `M1`?\n        M1.run(foo)\n      end\n    end\n\nPlease note that you might want to deactivate this check for cases in which you have an alias that\nis used tons throughout your codebase.\n\nIf, for example, you are using a third-party module named `FlupsyTopsyDataRetentionServiceServer`\nin half your modules, it is of course reasonable to alias it to `Server`.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "ignore",
            default: "[]",
            doc: "List of modules to ignore and allow to `alias Module, as: ...`",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.AliasOrder",
        id: "EX3002",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "Alphabetically ordered lists are more easily scannable by the reader.\n\n    # preferred\n\n    alias ModuleA\n    alias ModuleB\n    alias ModuleC\n\n    # NOT preferred\n\n    alias ModuleA\n    alias ModuleC\n    alias ModuleB\n\nAlias should be alphabetically ordered among their group:\n\n    # preferred\n\n    alias ModuleC\n    alias ModuleD\n\n    alias ModuleA\n    alias ModuleB\n\n    # NOT preferred\n\n    alias ModuleC\n    alias ModuleD\n\n    alias ModuleB\n    alias ModuleA\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "sort_method",
            default: ":alpha",
            doc: "The ordering method to use.\n\nOptions\n- `:alpha` - Alphabetical case-insensitive sorting.\n- `:ascii` - Case-sensitive sorting where upper case characters are ordered\n              before their lower case equivalent.\n",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.BlockPipe",
        id: "EX3003",
        category: "readability",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "Pipes (`|>`) should not be used with blocks.\n\nThe code in this example ...\n\n    list\n    |> Enum.take(5)\n    |> Enum.sort()\n    |> case do\n      [[_h | _t] | _] -> true\n      _ -> false\n    end\n\n... should be refactored to look like this:\n\n    maybe_nested_lists =\n      list\n      |> Enum.take(5)\n      |> Enum.sort()\n\n    case maybe_nested_lists do\n      [[_h | _t] | _] -> true\n      _ -> false\n    end\n\n... or create a new function:\n\n    list\n    |> Enum.take(5)\n    |> Enum.sort()\n    |> contains_nested_list?()\n\nPiping to blocks may be harder to read because it can be said that it obscures intentions\nand increases cognitive load on the reader. Instead, prefer introducing variables to your code or\nnew functions when it may be a sign that your function is getting too complicated and/or has too many concerns.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "exclude",
            default: "[]",
            doc: "Do not raise an issue for these macros and functions.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.CaptureOperator",
        id: "EX3099",
        category: "readability",
        base_priority: "0",
        tags: &[],
        explanation: "Using the capture operator can be a powerful short-hand, but sometimes this\nmakes the code more difficult to read.\n\n    # preferred\n\n    permissions = Enum.map(users, fn user -> {user.username, user.allowed?} end)\n\n    # NOT preferred\n\n    permissions = Enum.map(users, &{&1.username, &1.allowed?})\n\nWhile veterans understand the first snippet easily, folks new to the language or team\nmight have a better chance at grapsing what is happening.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[
            ParamDoc {
                name: "allow_field_access",
                default: "false",
                doc: "Allow very short captures only accessing a field using `& &1.foo` or `& &1[:foo]`.",
            },
            ParamDoc {
                name: "allow_function_with_arity",
                default: "false",
                doc: "Allow plain captures of functions using their arity `&String.downcase/1`.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Readability.FunctionNames",
        id: "EX3004",
        category: "readability",
        base_priority: "high",
        tags: &[],
        explanation: "Function, macro, and guard names are always written in snake_case in Elixir.\n\n    # snake_case\n\n    def handle_incoming_message(message) do\n    end\n\n    # not snake_case\n\n    def handleIncomingMessage(message) do\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "allow_acronyms",
            default: "false",
            doc: "Allows acronyms like HTTP or OTP in function names.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.ImplTrue",
        id: "EX3036",
        category: "readability",
        base_priority: "normal",
        tags: &[],
        explanation: "`@impl true` is a shortform so you don't have to write the actual behaviour that is being implemented.\nThis can make code harder to comprehend.\n\n# preferred\n\n    @impl MyBehaviour\n    def my_funcion() do\n      # ...\n    end\n\n# NOT preferred\n\n    @impl true\n    def my_funcion() do\n      # ...\n    end\n\nWhen implementing behaviour callbacks, `@impl true` indicates that a function implements a callback, but\na more explicit way is to use the actual behaviour being implemented, for example `@impl MyBehaviour`.\n\nThis not only improves readability, but adds extra validation in cases where multiple behaviours are\nimplemented in a single module.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.LargeNumbers",
        id: "EX3006",
        category: "readability",
        base_priority: "high",
        tags: &["formatter"],
        explanation: "Numbers can contain underscores for readability purposes.\nThese do not affect the value of the number, but can help read large numbers\nmore easily.\n\n    141592654 # how large is this number?\n\n    141_592_654 # ah, it's in the hundreds of millions!\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[
            ParamDoc {
                name: "only_greater_than",
                default: "9999",
                doc: "The check only reports numbers greater than this.",
            },
            ParamDoc {
                name: "trailing_digits",
                default: "[]",
                doc: "The check allows for the given number of trailing digits (can be a number, range or list)",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Readability.MaxLineLength",
        id: "EX3007",
        category: "readability",
        base_priority: "low",
        tags: &["formatter"],
        explanation: "Checks for the length of lines.\n\nIgnores function definitions and (multi-)line strings by default.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[
            ParamDoc {
                name: "max_length",
                default: "120",
                doc: "The maximum number of characters a line may consist of.",
            },
            ParamDoc {
                name: "ignore_definitions",
                default: "true",
                doc: "Set to `true` to ignore lines including function definitions.",
            },
            ParamDoc {
                name: "ignore_heredocs",
                default: "true",
                doc: "",
            },
            ParamDoc {
                name: "ignore_specs",
                default: "false",
                doc: "Set to `true` to ignore lines including `@spec`s.",
            },
            ParamDoc {
                name: "ignore_sigils",
                default: "true",
                doc: "Set to `true` to ignore lines that are sigils, e.g. regular expressions.",
            },
            ParamDoc {
                name: "ignore_strings",
                default: "true",
                doc: "Set to `true` to ignore lines that are strings or in heredocs.",
            },
            ParamDoc {
                name: "ignore_urls",
                default: "true",
                doc: "Set to `true` to ignore lines that contain urls.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Readability.ModuleAttributeNames",
        id: "EX3008",
        category: "readability",
        base_priority: "high",
        tags: &[],
        explanation: "Module attribute names are always written in snake_case in Elixir.\n\n    # snake_case\n\n    @inbox_name \"incoming\"\n\n    # not snake_case\n\n    @inboxName \"incoming\"\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.ModuleDoc",
        id: "EX3009",
        category: "readability",
        base_priority: "0",
        tags: &[],
        explanation: "Every module should contain comprehensive documentation.\n\n    # preferred\n\n    defmodule MyApp.Web.Search do\n      @moduledoc \"\"\"\n      This module provides a public API for all search queries originating\n      in the web layer.\n      \"\"\"\n    end\n\n    # also okay: explicitly say there is no documentation\n\n    defmodule MyApp.Web.Search do\n      @moduledoc false\n    end\n\nMany times a sentence or two in plain english, explaining why the module\nexists, will suffice. Documenting your train of thought this way will help\nboth your co-workers and your future-self.\n\nOther times you will want to elaborate even further and show some\nexamples of how the module's functions can and should be used.\n\nIn some cases however, you might not want to document things about a module,\ne.g. it is part of a private API inside your project. Since Elixir prefers\nexplicitness over implicit behaviour, you should \"tag\" these modules with\n\n    @moduledoc false\n\nto make it clear that there is no intention in documenting it.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[
            ParamDoc {
                name: "ignore_names",
                default: "[~r/(\\.\\w+Controller|\\.Endpoint|\\.\\w+Live(\\.\\w+)?|\\.Repo|\\.Router|\\.\\w+Socket|\\.\\w+View|\\.\\w+HTML|\\.\\w+JSON|\\.Telemetry|\\.Layouts|\\.Mailer)$/]",
                doc: "List of modules to ignore based on their name. Accepts atoms, strings and regexes.",
            },
            ParamDoc {
                name: "ignore_modules_using",
                default: "[Credo.Check, Ecto.Schema, Phoenix.LiveView, ~r/\\.Web$/]",
                doc: "List of modules to ignore based on their `use` declarations. Accepts atoms, strings and regexes.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Readability.ModuleNames",
        id: "EX3010",
        category: "readability",
        base_priority: "high",
        tags: &[],
        explanation: "Module names are always written in PascalCase in Elixir.\n\n    # PascalCase\n\n    defmodule MyApp.WebSearchController do\n      # ...\n    end\n\n    # not PascalCase\n\n    defmodule MyApp.Web_searchController do\n      # ...\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of other reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "ignore",
            default: "[]",
            doc: "List of ignored module names and patterns e.g. `[~r/Sample_Module/, \"Credo.Sample_Module\"]`",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.MultiAlias",
        id: "EX3011",
        category: "readability",
        base_priority: "low",
        tags: &["controversial"],
        explanation: "Multi alias expansion makes module uses harder to search for in large code bases.\n\n    # preferred\n\n    alias Module.Foo\n    alias Module.Bar\n\n    # NOT preferred\n\n    alias Module.{Foo, Bar}\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.NestedFunctionCalls",
        id: "EX3012",
        category: "readability",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "A function call should not be nested inside another function call.\n\nSo while this is fine:\n\n    Enum.shuffle([1,2,3])\n\nThe code in this example ...\n\n    Enum.shuffle(Enum.uniq([1,2,3,3]))\n\n... should be refactored to look like this:\n\n    [1,2,3,3]\n    |> Enum.uniq()\n    |> Enum.shuffle()\n\nNested function calls make the code harder to read. Instead, break the\nfunction calls out into a pipeline.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "min_pipeline_length",
            default: "2",
            doc: "Set a minimum pipeline length",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.OneArityFunctionInPipe",
        id: "EX3034",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "Use parentheses for one-arity functions when using the pipe operator (|>).\n\n    # not preferred\n    some_string |> String.downcase |> String.trim\n\n    # preferred\n    some_string |> String.downcase() |> String.trim()\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.OnePipePerLine",
        id: "EX3035",
        category: "readability",
        base_priority: "0",
        tags: &[],
        explanation: "Don't use multiple pipes (|>) in the same line.\nEach function in the pipe should be in it's own line.\n\n    # preferred\n\n    foo\n    |> bar()\n    |> baz()\n\n    # NOT preferred\n\n    foo |> bar() |> baz()\n\nThe code in this example ...\n\n    1 |> Integer.to_string() |> String.to_integer()\n\n... should be refactored to look like this:\n\n    1\n    |> Integer.to_string()\n    |> String.to_integer()\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.ParenthesesInCondition",
        id: "EX3013",
        category: "readability",
        base_priority: "high",
        tags: &["formatter"],
        explanation: "Because `if` and `unless` are macros, the preferred style is to not use\nparentheses around conditions.\n\n    # preferred\n\n    if valid?(username) do\n      # ...\n    end\n\n    # NOT preferred\n\n    if( valid?(username) ) do\n      # ...\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.ParenthesesOnZeroArityDefs",
        id: "EX3014",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "Either use parentheses or not when defining a function with no arguments.\n\nBy default, this check enforces no parentheses, so zero-arity function\nand macro definitions should look like this:\n\n    def summer? do\n      # ...\n    end\n\nIf the `:parens` param is set to `true` for this check, then the check\nenforces zero-arity function and macro definitions to have parens:\n\n    def summer?() do\n      # ...\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "parens",
            default: "false",
            doc: "",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.PipeIntoAnonymousFunctions",
        id: "EX3015",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "Avoid piping into anonymous functions.\n\nThe code in this example ...\n\n    def my_fun(foo) do\n      foo\n      |> (fn i -> i * 2 end).()\n      |> my_other_fun()\n    end\n\n... should be refactored to define a private function:\n\n    def my_fun(foo) do\n      foo\n      |> times_2()\n      |> my_other_fun()\n    end\n\n    defp times_2(i), do: i * 2\n\n... or use `then/1`:\n\n    def my_fun(foo) do\n      foo\n      |> then(fn i -> i * 2 end)\n      |> my_other_fun()\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.PredicateFunctionNames",
        id: "EX3016",
        category: "readability",
        base_priority: "high",
        tags: &[],
        explanation: "Predicate functions/macros should be named accordingly:\n\n* For functions, they should end in a question mark.\n\n      # preferred\n\n      defp user?(cookie) do\n      end\n\n      defp has_attachment?(mail) do\n      end\n\n      # NOT preferred\n\n      defp is_user?(cookie) do\n      end\n\n      defp is_user(cookie) do\n      end\n\n* For guard-safe macros they should have the prefix `is_` and not end in a question mark.\n\n      # preferred\n\n      defmacro is_user(cookie) do\n      end\n\n      # NOT preferred\n\n      defmacro is_user?(cookie) do\n      end\n\n      defmacro user?(cookie) do\n      end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.PreferImplicitTry",
        id: "EX3017",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "Prefer using an implicit `try` rather than explicit `try` if you try to rescue\nanything the function does.\n\nFor example, this:\n\n    def failing_function(first) do\n      try do\n        to_string(first)\n      rescue\n        _ -> :rescued\n      end\n    end\n\nCan be rewritten without `try` as below:\n\n    def failing_function(first) do\n      to_string(first)\n    rescue\n      _ -> :rescued\n    end\n\nThis emphazises that you really want to try/rescue anything the function does,\nwhich might be important for other contributors so they can reason about adding\ncode to the function.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.PreferUnquotedAtoms",
        id: "EX3018",
        category: "readability",
        base_priority: "high",
        tags: &[],
        explanation: "Prefer unquoted atoms unless quotes are necessary.\nThis is helpful because a quoted atom can be easily mistaken for a string.\n\n    # preferred\n\n    :x\n    [x: 1]\n    %{x: 1}\n\n    # NOT preferred\n\n    :\"x\"\n    [\"x\": 1]\n    %{\"x\": 1}\n\nThe primary case where this can become an issue is when using atoms or\nstrings for keys in a Map or Keyword list.\n\nFor example, this:\n\n    %{\"x\": 1}\n\nCan easily be mistaken for this:\n\n    %{\"x\" => 1}\n\nBecause a string key cannot be used to access a value with the equivalent\natom key, this can lead to subtle bugs which are hard to discover.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.RedundantBlankLines",
        id: "EX3019",
        category: "readability",
        base_priority: "low",
        tags: &["formatter"],
        explanation: "Files should not have two or more consecutive blank lines.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "max_blank_lines",
            default: "1",
            doc: "The maximum number of tolerated consecutive blank lines.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.Semicolons",
        id: "EX3020",
        category: "readability",
        base_priority: "high",
        tags: &["formatter"],
        explanation: "Don't use ; to separate statements and expressions.\nStatements and expressions should be separated by lines.\n\n    # preferred\n\n    a = 1\n    b = 2\n\n    # NOT preferred\n\n    a = 1; b = 2\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.SeparateAliasRequire",
        id: "EX3021",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "All instances of `alias` should be consecutive within a file.\nLikewise, all instances of `require` should be consecutive within a file.\n\nFor example:\n\n    defmodule Foo do\n      require Logger\n      alias Foo.Bar\n\n      alias Foo.Baz\n      require Integer\n\n      # ...\n    end\n\nshould be changed to:\n\n    defmodule Foo do\n      require Integer\n      require Logger\n\n      alias Foo.Bar\n      alias Foo.Baz\n\n      # ...\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.SingleFunctionToBlockPipe",
        id: "EX3022",
        category: "readability",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "A single pipe (`|>`) should not be used to pipe into blocks.\n\nThe code in this example ...\n\n    list\n    |> length()\n    |> case do\n      0 -> :none\n      1 -> :one\n      _ -> :many\n    end\n\n... should be refactored to look like this:\n\n    case length(list) do\n      0 -> :none\n      1 -> :one\n      _ -> :many\n    end\n\nIf you want to disallow piping into blocks altogether, use\n`Credo.Check.Readability.BlockPipe`.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.SinglePipe",
        id: "EX3023",
        category: "readability",
        base_priority: "high",
        tags: &["controversial"],
        explanation: "Pipes (`|>`) should only be used when piping data through multiple calls.\n\nSo while this is fine:\n\n    list\n    |> Enum.take(5)\n    |> Enum.shuffle\n    |> evaluate()\n\nThe code in this example ...\n\n    list\n    |> evaluate()\n\n... should be refactored to look like this:\n\n    evaluate(list)\n\nUsing a single |> to invoke functions makes the code harder to read. Instead,\nwrite a function call when a pipeline is only one function long.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[
            ParamDoc {
                name: "allow_0_arity_functions",
                default: "false",
                doc: "Allow 0-arity functions",
            },
            ParamDoc {
                name: "allow_blocks",
                default: "true",
                doc: "Allow block functions/macro like `for`, `if` or `case`",
            },
            ParamDoc {
                name: "allow_lists",
                default: "false",
                doc: "Allow single pipes where the value being piped is a list literal",
            },
            ParamDoc {
                name: "allow_maps",
                default: "false",
                doc: "Allow single pipes where the value being piped is a map literal (including structs)",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Readability.SpaceAfterCommas",
        id: "EX3024",
        category: "readability",
        base_priority: "0",
        tags: &["formatter"],
        explanation: "You can use white-space after commas to make items of lists,\ntuples and other enumerations easier to separate from one another.\n\n    # preferred\n\n    alias Project.{Alpha, Beta}\n\n    def some_func(first, second, third) do\n      list = [1, 2, 3, 4, 5]\n      # ...\n    end\n\n    # NOT preferred - items are harder to separate\n\n    alias Project.{Alpha,Beta}\n\n    def some_func(first,second,third) do\n      list = [1,2,3,4,5]\n      # ...\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.SpecParameterNames",
        id: "EX3037",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "Parameters in `@spec` and `@callback` declarations should be named.\n\nUsing the `name :: type` syntax, naming parameters makes specs self-documenting:\nreaders and ExDoc see what each argument is for, not just its type.\nThis is especially valuable when several parameters share the same type.\n\n    # preferred\n\n    @spec create_user(attrs :: map(), email :: String.t()) :: {:ok, User.t()}\n\n    @callback handle_event(event :: String.t(), params :: map(), socket :: Socket.t()) ::\n                {:noreply, Socket.t()}\n\n    # NOT preferred\n\n    @spec create_user(map(), String.t()) :: {:ok, User.t()}\n\n    @callback handle_event(String.t(), map(), Socket.t()) :: {:noreply, Socket.t()}\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.Specs",
        id: "EX3025",
        category: "readability",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "Functions, callbacks and macros need typespecs.\n\nAdding typespecs gives tools like Dialyzer more information when performing\nchecks for type errors in function calls and definitions.\n\n    @spec add(integer, integer) :: integer\n    def add(a, b), do: a + b\n\nFunctions with multiple arities need to have a spec defined for each arity:\n\n    @spec foo(integer) :: boolean\n    @spec foo(integer, integer) :: boolean\n    def foo(a), do: a > 0\n    def foo(a, b), do: a > b\n\nThe check only considers whether the specification is present, it doesn't\nperform any actual type checking.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "include_defp",
            default: "false",
            doc: "Include private functions.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.StrictModuleLayout",
        id: "EX3026",
        category: "readability",
        base_priority: "low",
        tags: &["controversial"],
        explanation: "Provide module parts in a required order.\n\n    # preferred\n\n    defmodule MyMod do\n      @moduledoc \"moduledoc\"\n      use Foo\n      import Bar\n      alias Baz\n      require Qux\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[
            ParamDoc {
                name: "order",
                default: "[:shortdoc, :moduledoc, :behaviour, :use, :import, :alias, :require]",
                doc: "List of atoms identifying the desired order of module parts.\n\nSupported values are:\n\n- `:moduledoc` - `@moduledoc` module attribute\n- `:shortdoc` - `@shortdoc` module attribute\n- `:behaviour` - `@behaviour` module attribute\n- `:use` - `use` expression\n- `:import` - `import` expression\n- `:alias` - `alias` expression\n- `:require` - `require` expression\n- `:defstruct` - `defstruct` expression\n- `:opaque` - `@opaque` module attribute\n- `:type` - `@type` module attribute\n- `:typep` - `@typep` module attribute\n- `:callback` - `@callback` module attribute\n- `:macrocallback` - `@macrocallback` module attribute\n- `:optional_callbacks` - `@optional_callbacks` module attribute\n- `:module_attribute` - other module attribute\n- `:public_fun` - public function\n- `:private_fun` - private function or a public function marked with `@doc false`\n- `:public_macro` - public macro\n- `:private_macro` - private macro or a public macro marked with `@doc false`\n- `:callback_impl` - public function or macro marked with `@impl`\n- `:public_guard` - public guard\n- `:private_guard` - private guard or a public guard marked with `@doc false`\n- `:module` - inner module definition (`defmodule` expression inside a module)\n\nNotice that the desired order always starts from the top.\n\nFor example, if you provide the order `~w/public_fun private_fun/a`,\nit means that everything else (e.g. `@moduledoc`) must appear after\nfunction definitions.\n",
            },
            ParamDoc {
                name: "ignore",
                default: "[]",
                doc: "List of atoms identifying the module parts which are not checked, and may\ntherefore appear anywhere in the module. Supported values are the same as\nin the `:order` param.\n",
            },
            ParamDoc {
                name: "ignore_module_attributes",
                default: "[]",
                doc: "List of atoms identifying the module attributes which are not checked, and may\ntherefore appear anywhere in the module. Useful for custom DSLs that use attributes\nbefore function heads.\n\nFor example, if you provide `~w/trace/a`, all `@trace` attributes will be ignored.\n",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Readability.StringSigils",
        id: "EX3027",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "If you used quoted strings that contain quotes, you might want to consider\nswitching to the use of sigils instead.\n\n    # okay\n\n    \"<a href=\\\"http://elixirweekly.net\\\">#\\{text}</a>\"\n\n    # not okay, lots of escaped quotes\n\n    \"<a href=\\\"http://elixirweekly.net\\\" target=\\\"_blank\\\">#\\{text}</a>\"\n\n    # refactor to\n\n    ~S(<a href=\"http://elixirweekly.net\" target=\"_blank\">#\\{text}</a>)\n\nThis allows us to remove the noise which results from the need to escape\nquotes within quotes.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "maximum_allowed_quotes",
            default: "3",
            doc: "The maximum amount of escaped quotes you want to tolerate.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.TrailingBlankLine",
        id: "EX3028",
        category: "readability",
        base_priority: "low",
        tags: &["formatter"],
        explanation: "Files should end in a trailing blank line.\n\nThis is mostly for historical reasons: every text file should end with a \\n,\nor newline since this acts as `eol` or the end of the line character.\n\nSee also: http://pubs.opengroup.org/onlinepubs/9699919799/basedefs/V1_chap03.html#tag_03_206\n\nMost text editors ensure this \"final newline\" automatically.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.TrailingWhiteSpace",
        id: "EX3029",
        category: "readability",
        base_priority: "low",
        tags: &["formatter"],
        explanation: "There should be no white-space (i.e. tabs, spaces) at the end of a line.\n\nMost text editors provide a way to remove them automatically.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[ParamDoc {
            name: "ignore_strings",
            default: "true",
            doc: "Set to `false` to check lines that are strings or in heredocs",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Readability.UnnecessaryAliasExpansion",
        id: "EX3030",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "Alias expansion is useful but when aliasing a single module,\nit can be harder to read with unnecessary braces.\n\n    # preferred\n\n    alias ModuleA.Foo\n    alias ModuleA.{Foo, Bar}\n\n    # NOT preferred\n\n    alias ModuleA.{Foo}\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.UnusedFunctionParameterPattern",
        id: "EX5032",
        category: "readability",
        base_priority: "normal",
        tags: &[],
        explanation: "Pattern matches in function parameters that are immediately ignored should be avoided.\n\nFor example, the pattern match `= _user_params` is unnecessary because the variable cannot be used.\n\n    def valid?(%{} = _user_params) do\n      # ...\n    end\n\nIf you want to use the name as a form of documentation, try a type specification:\n\n    @spec valid?(user_params :: map()) :: term()\n    def valid?(%{}) do\n      # ...\n    end\n\nor:\n\n    @spec valid?(user_params) :: term() when user_params: map()\n    def valid?(%{}) do\n      # ...\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.VariableNames",
        id: "EX3031",
        category: "readability",
        base_priority: "high",
        tags: &[],
        explanation: "Variable names are always written in snake_case in Elixir.\n\n    # snake_case:\n\n    incoming_result = handle_incoming_message(message)\n\n    # not snake_case\n\n    incomingResult = handle_incoming_message(message)\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.WithCustomTaggedTuple",
        id: "EX3032",
        category: "readability",
        base_priority: "low",
        tags: &[],
        explanation: "Avoid using custom tags for error reporting from `with` macros.\n\nThis code injects tuple_tag tags such as `:resource` and `:authz` for the purpose of error\nreporting.\n\n    with {:resource, {:ok, resource}} <- {:resource, Resource.fetch(user)},\n         {:authz, :ok} <- {:authz, Resource.authorize(resource, user)} do\n      do_something_with(resource)\n    else\n      {:resource, _} -> {:error, :not_found}\n      {:authz, _} -> {:error, :unauthorized}\n    end\n\nInstead, extract each validation into a separate helper function which returns error\ninformation immediately:\n\n    defp find_resource(user) do\n      with :error <- Resource.fetch(user), do: {:error, :not_found}\n    end\n\n    defp authorize(resource, user) do\n      with :error <- Resource.authorize(resource, user), do: {:error, :unauthorized}\n    end\n\nAt this point, the validation chain in `with` becomes clearer and easier to understand:\n\n    with {:ok, resource} <- find_resource(user),\n         :ok <- authorize(resource, user),\n         do: do_something(user)\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Readability.WithSingleClause",
        id: "EX3033",
        category: "readability",
        base_priority: "0",
        tags: &[],
        explanation: "`with` statements are useful when you need to chain a sequence\nof pattern matches, stopping at the first one that fails.\n\nIf the `with` has a single pattern matching clause and no `else`\nbranch, it means that if the clause doesn't match than the whole\n`with` will return the value of that clause.\n\nHowever, if that `with` has also an `else` clause, then you're using `with` exactly\nlike a `case` and a `case` should be used instead.\n\nTake this code:\n\n    with {:ok, user} <- User.create(make_ref()) do\n      user\n    else\n      {:error, :db_down} ->\n        raise \"DB is down!\"\n\n      {:error, reason} ->\n        raise \"error: #{inspect(reason)}\"\n    end\n\nIt can be rewritten with a clearer use of `case`:\n\n    case User.create(make_ref()) do\n      {:ok, user} ->\n        user\n\n      {:error, :db_down} ->\n        raise \"DB is down!\"\n\n      {:error, reason} ->\n        raise \"error: #{inspect(reason)}\"\n    end\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n",
        params: &[],
    },
    // --- refactor ---
    CheckDoc {
        module: "Credo.Check.Refactor.ABCSize",
        id: "EX4001",
        category: "refactor",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "The ABC size describes a metric based on assignments, branches and conditions.\n\nA high ABC size is a hint that a function might be doing \"more\" than it\nshould.\n\nAs always: Take any metric with a grain of salt. Since this one was originally\nintroduced for C, C++ and Java, we still have to see whether or not this can\nbe a useful metric in a declarative language like Elixir.\n",
        params: &[
            ParamDoc {
                name: "max_size",
                default: "30",
                doc: "The maximum ABC size a function should have.",
            },
            ParamDoc {
                name: "excluded_functions",
                default: "[]",
                doc: "All functions listed will be ignored.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.AppendSingleItem",
        id: "EX4002",
        category: "refactor",
        base_priority: "low",
        tags: &["controversial"],
        explanation: "When building up large lists, it is faster to prepend than\nappend. Therefore: It is sometimes best to prepend to the list\nduring iteration and call Enum.reverse/1 at the end, as it is quite\nfast.\n\nExample:\n\n    list = list_so_far ++ [new_item]\n\n    # refactoring it like this can make the code faster:\n\n    list = [new_item] ++ list_so_far\n    # ...\n    Enum.reverse(list)\n\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.Apply",
        id: "EX4003",
        category: "refactor",
        base_priority: "low",
        tags: &[],
        explanation: "Prefer calling functions directly if the number of arguments is known\nat compile time instead of using `apply/2` and `apply/3`.\n\nExample:\n\n    # preferred\n\n    fun.(arg_1, arg_2, ..., arg_n)\n\n    module.function(arg_1, arg_2, ..., arg_n)\n\n    # NOT preferred\n\n    apply(fun, [arg_1, arg_2, ..., arg_n])\n\n    apply(module, :function, [arg_1, arg_2, ..., arg_n])\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.CaseTrivialMatches",
        id: "EX4004",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "PLEASE NOTE: This check is deprecated as it might do more harm than good.\n\nRelated discussion: https://github.com/rrrene/credo/issues/65\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.CondInsteadOfIfElse",
        id: "EX4033",
        category: "refactor",
        base_priority: "low",
        tags: &[],
        explanation: "Prefer `cond` over `if/else` blocks.\n\nSo while this is fine:\n\n    if allowed? do\n      :ok\n    end\n\nThe use of `else` could impact readability:\n\n    if allowed? do\n      :ok\n    else\n      :error\n    end\n\nand could be improved to:\n\n    cond do\n      allowed? -> :ok\n      true -> :error\n    end\n\nThere's no technical reason for this; it's a matter of preferred code style.\n\nNOTE: This check is mutually exclusive with `Credo.Check.Refactor.CondStatements`,\nwhich recommends the opposite. Enable only one of these checks.\n",
        params: &[ParamDoc {
            name: "allow_one_liners",
            default: "false",
            doc: "Allow one-liner `if/else` expressions (e.g., `if x, do: y, else: z`).",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.CondStatements",
        id: "EX4005",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "Each cond statement should have 3 or more statements including the\n\"always true\" statement.\n\nConsider an `if`/`else` construct if there is only one condition and the\n\"always true\" statement, since it will more accessible to programmers\nnew to the codebase (and possibly new to Elixir).\n\nExample:\n\n    cond do\n      x == y -> 0\n      true -> 1\n    end\n\n    # should be written as\n\n    if x == y do\n      0\n    else\n      1\n    end\n\nNOTE: This check is mutually exclusive with `Credo.Check.Refactor.CondInsteadOfIfElse`,\nwhich recommends the opposite. Enable only one of these checks.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.CyclomaticComplexity",
        id: "EX4006",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "Cyclomatic complexity (CC) is a software complexity metric closely\ncorrelated with coding errors.\n\nIf a function feels like it's gotten too complex, it more often than not also\nhas a high CC value. So, if anything, this is useful to convince team members\nand bosses of a need to refactor parts of the code based on \"objective\"\nmetrics.\n",
        params: &[ParamDoc {
            name: "max_complexity",
            default: "9",
            doc: "The maximum cyclomatic complexity a function should have.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.DoubleBooleanNegation",
        id: "EX4007",
        category: "refactor",
        base_priority: "low",
        tags: &["controversial"],
        explanation: "Having double negations in your code can obscure the parameter's original value.\n\n    # NOT preferred\n\n    !!var\n\nThis will return `false` for `false` and `nil`, and `true` for anything else.\n\nAt first this seems like an extra clever shorthand to cast anything truthy to\n`true` and anything non-truthy to `false`. But in most scenarios you want to\nbe explicit about your input parameters (because it is easier to reason about\nedge-cases, code-paths and tests).\nAlso: `nil` and `false` do mean two different things.\n\nA scenario where you want this kind of flexibility, however, is parsing\nexternal data, e.g. a third party JSON-API where a value is sometimes `null`\nand sometimes `false` and you want to normalize that before handing it down\nin your program.\n\nIn these case, you would be better off making the cast explicit by introducing\na helper function:\n\n    # preferred\n\n    defp present?(nil), do: false\n    defp present?(false), do: false\n    defp present?(_), do: true\n\nThis makes your code more explicit than relying on the implications of `!!`.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.FilterCount",
        id: "EX4030",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "`Enum.count/2` is more efficient than `Enum.filter/2 |> Enum.count/1`.\n\nThis should be refactored:\n\n    [1, 2, 3, 4, 5]\n    |> Enum.filter(fn x -> rem(x, 3) == 0 end)\n    |> Enum.count()\n\nto look like this:\n\n    Enum.count([1, 2, 3, 4, 5], fn x -> rem(x, 3) == 0 end)\n\nThe reason for this is performance, because the two separate calls\nto `Enum.filter/2` and `Enum.count/1` require two iterations whereas\n`Enum.count/2` performs the same work in one pass.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.FilterFilter",
        id: "EX4008",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "One `Enum.filter/2` is more efficient than `Enum.filter/2 |> Enum.filter/2`.\n\nThis should be refactored:\n\n    [\"a\", \"b\", \"c\"]\n    |> Enum.filter(&String.contains?(&1, \"x\"))\n    |> Enum.filter(&String.contains?(&1, \"a\"))\n\nto look like this:\n\n    Enum.filter([\"a\", \"b\", \"c\"], fn letter ->\n      String.contains?(letter, \"x\") && String.contains?(letter, \"a\")\n    end)\n\nThe reason for this is performance, because the two separate calls\nto `Enum.filter/2` require two iterations whereas doing the\nfunctions in the single `Enum.filter/2` only requires one.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.FilterReject",
        id: "EX4009",
        category: "refactor",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "One `Enum.filter/2` is more efficient than `Enum.filter/2 |> Enum.reject/2`.\n\nThis should be refactored:\n\n    [\"a\", \"b\", \"c\"]\n    |> Enum.filter(&String.contains?(&1, \"x\"))\n    |> Enum.reject(&String.contains?(&1, \"a\"))\n\nto look like this:\n\n    Enum.filter([\"a\", \"b\", \"c\"], fn letter ->\n      String.contains?(letter, \"x\") && !String.contains?(letter, \"a\")\n    end)\n\nThe reason for this is performance, because the two calls to\n`Enum.reject/2` and `Enum.filter/2` require two iterations whereas\ndoing the functions in the single `Enum.filter/2` only requires one.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.FunctionArity",
        id: "EX4010",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "A function can take as many parameters as needed, but even in a functional\nlanguage there can be too many parameters.\n\nCan optionally ignore private functions (check configuration options).\n",
        params: &[
            ParamDoc {
                name: "max_arity",
                default: "8",
                doc: "The maximum number of parameters which a function should take.",
            },
            ParamDoc {
                name: "ignore_defp",
                default: "false",
                doc: "Set to `true` to ignore private functions.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.IoPuts",
        id: "EX4011",
        category: "refactor",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "Prefer using Logger statements over using `IO.puts/1`.\n\nThis is a situational check.\n\nAs such, it might be a great help for e.g. Phoenix projects, but\na clear mismatch for CLI projects.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.LongQuoteBlocks",
        id: "EX4012",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "Long `quote` blocks are generally an indication that too much is done inside\nthem.\n\nLet's look at why this is problematic:\n\n    defmodule MetaCommand do\n      def __using__(opts \\\\ []) do\n        modes = opts[:modes]\n        command_name = opts[:command_name]\n\n        quote do\n          def run(filename) do\n            contents =\n              if File.exists?(filename) do\n                {:ok, file} = File.open(filename, unquote(modes))\n                {:ok, contents} = IO.read(file, :line)\n                File.close(file)\n                contents\n              else\n                \"\"\n              end\n\n            case contents do\n              \"\" ->\n                # ...\n              unquote(command_name) <> rest ->\n                # ...\n            end\n          end\n\n          # ...\n        end\n      end\n    end\n\nA cleaner solution would be to call \"regular\" functions outside the\n`quote` block to perform the actual work.\n\n    defmodule MyMetaCommand do\n      def __using__(opts \\\\ []) do\n        modes = opts[:modes]\n        command_name = opts[:command_name]\n\n        quote do\n          def run(filename) do\n            MyMetaCommand.run_on_file(filename, unquote(modes), unquote(command_name))\n          end\n\n          # ...\n        end\n      end\n\n      def run_on_file(filename, modes, command_name) do\n        contents =\n          # actual implementation\n      end\n    end\n\nThis way it is easier to reason about what is actually happening. And to debug\nit.\n",
        params: &[
            ParamDoc {
                name: "max_line_count",
                default: "150",
                doc: "The maximum number of lines a quote block should be allowed to have.",
            },
            ParamDoc {
                name: "ignore_comments",
                default: "false",
                doc: "Ignores comments when counting the lines of a `quote` block.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.MapInto",
        id: "EX4013",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "`Enum.into/3` is more efficient than `Enum.map/2 |> Enum.into/2`.\n\nThis should be refactored:\n\n    [:apple, :banana, :carrot]\n    |> Enum.map(&({&1, to_string(&1)}))\n    |> Enum.into(%{})\n\nto look like this:\n\n    Enum.into([:apple, :banana, :carrot], %{}, &({&1, to_string(&1)}))\n\nThe reason for this is performance, because the separate calls to\n`Enum.map/2` and `Enum.into/2` require two iterations whereas\n`Enum.into/3` only requires one.\n\n**NOTE**: This check is only available in Elixir < 1.8 since performance\nimprovements have since made this check obsolete.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.MapJoin",
        id: "EX4014",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "`Enum.map_join/3` is more efficient than `Enum.map/2 |> Enum.join/2`.\n\nThis should be refactored:\n\n    [\"a\", \"b\", \"c\"]\n    |> Enum.map(&String.upcase/1)\n    |> Enum.join(\", \")\n\nto look like this:\n\n    Enum.map_join([\"a\", \"b\", \"c\"], \", \", &String.upcase/1)\n\nThe reason for this is performance, because the two separate calls\nto `Enum.map/2` and `Enum.join/2` require two iterations whereas\n`Enum.map_join/3` performs the same work in one pass.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.MapMap",
        id: "EX4015",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "One `Enum.map/2` is more efficient than `Enum.map/2 |> Enum.map/2`.\n\nThis should be refactored:\n\n    [:a, :b, :c]\n    |> Enum.map(&inspect/1)\n    |> Enum.map(&String.upcase/1)\n\nto look like this:\n\n    Enum.map([:a, :b, :c], fn letter ->\n      letter\n      |> inspect()\n      |> String.upcase()\n    end)\n\nThe reason for this is performance, because the two separate calls\nto `Enum.map/2` require two iterations whereas doing the functions\nin the single `Enum.map/2` only requires one.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.MatchInCondition",
        id: "EX4016",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "Pattern matching should only ever be used for simple assignments\ninside `if` and `unless` clauses.\n\nWhile this fine:\n\n    # okay, simple wildcard assignment:\n\n    if contents = File.read!(\"foo.txt\") do\n      do_something(contents)\n    end\n\nthe following should be avoided, since it mixes a pattern match with a\ncondition and do/else blocks.\n\n    # considered too \"complex\":\n\n    if {:ok, contents} = File.read(\"foo.txt\") do\n      do_something(contents)\n    end\n\n    # also considered \"complex\":\n\n    if allowed? && ( contents = File.read!(\"foo.txt\") ) do\n      do_something(contents)\n    end\n\nIf you want to match for something and execute another block otherwise,\nconsider using a `case` statement:\n\n    case File.read(\"foo.txt\") do\n      {:ok, contents} ->\n        do_something()\n      _ ->\n        do_something_else()\n    end\n\n",
        params: &[
            ParamDoc {
                name: "allow_tagged_tuples",
                default: "false",
                doc: "Allow tagged tuples in conditions, e.g. `if {:ok, contents} = File.read( \"foo.txt\") do`",
            },
            ParamDoc {
                name: "allow_operators",
                default: "false",
                doc: "Allow operators in conditions, e.g. `if contents = File.read(input <> \".txt\") do`",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.ModuleDependencies",
        id: "EX4017",
        category: "refactor",
        base_priority: "normal",
        tags: &["controversial"],
        explanation: "This module might be doing too much. Consider limiting the number of\nmodule dependencies.\n\nAs always: This is just a suggestion. Check the configuration options for\ntweaking or disabling this check.\n",
        params: &[
            ParamDoc {
                name: "max_deps",
                default: "10",
                doc: "Maximum number of module dependencies.",
            },
            ParamDoc {
                name: "dependency_namespaces",
                default: "[]",
                doc: "List of dependency namespaces to include in this check",
            },
            ParamDoc {
                name: "excluded_namespaces",
                default: "[]",
                doc: "List of namespaces to exclude from this check",
            },
            ParamDoc {
                name: "excluded_paths",
                default: "[~r/\\/test\\//, \"test\"]",
                doc: "List of paths or regex to exclude from this check",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.NegatedConditionsInUnless",
        id: "EX4018",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "Unless blocks should avoid having a negated condition.\n\nThe code in this example ...\n\n    unless !allowed? do\n      proceed_as_planned()\n    end\n\n... should be refactored to look like this:\n\n    if allowed? do\n      proceed_as_planned()\n    end\n\nThe reason for this is not a technical but a human one. It is pretty difficult\nto wrap your head around a block of code that is executed if a negated\ncondition is NOT met. See what I mean?\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.NegatedConditionsWithElse",
        id: "EX4019",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "An `if` block with a negated condition should not contain an else block.\n\nSo while this is fine:\n\n    if not allowed? do\n      raise \"Not allowed!\"\n    end\n\nThe code in this example ...\n\n    if not allowed? do\n      raise \"Not allowed!\"\n    else\n      proceed_as_planned()\n    end\n\n... should be refactored to look like this:\n\n    if allowed? do\n      proceed_as_planned()\n    else\n      raise \"Not allowed!\"\n    end\n\nThe same goes for negation through `!` instead of `not`.\n\nThe reason for this is not a technical but a human one. It is easier to wrap\nyour head around a positive condition and then thinking \"and else we do ...\".\n\nIn the above example raising the error in case something is not allowed\nmight seem so important to put it first. But when you revisit this code a\nwhile later or have to introduce a colleague to it, you might be surprised\nhow much clearer things get when the \"happy path\" comes first.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.NegatedIsNil",
        id: "EX4020",
        category: "refactor",
        base_priority: "low",
        tags: &["controversial"],
        explanation: "We should avoid negating the `is_nil` predicate function.\n\nFor example, the code here ...\n\n    def fun(%{external_id: external_id, id: id}) when not is_nil(external_id) do\n       # ...\n    end\n\n... can be refactored to look like this:\n\n    def fun(%{external_id: nil, id: id}) do\n      # ...\n    end\n\n    def fun(%{external_id: external_id, id: id}) do\n      # ...\n    end\n\n... or even better, can match on what you were expecting on the first place:\n\n    def fun(%{external_id: external_id, id: id}) when is_binary(external_id) do\n      # ...\n    end\n\n    def fun(%{external_id: nil, id: id}) do\n      # ...\n    end\n\n    def fun(%{external_id: external_id, id: id}) do\n      # ...\n    end\n\nSimilar to negating `unless` blocks, the reason for this check is not\ntechnical, but a human one. If we can use the positive, more direct and human\nfriendly case, we should.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.Nesting",
        id: "EX4021",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "Code should not be nested more than once inside a function.\n\n    defmodule CredoSampleModule do\n      def some_function(parameter1, parameter2) do\n        Enum.reduce(var1, list, fn({_hash, nodes}, list) ->\n          filenames = nodes |> Enum.map(&(&1.filename))\n\n          Enum.reduce(list, [], fn(item, acc) ->\n            if item.filename do\n              item               # <-- this is nested 3 levels deep\n            end\n            acc ++ [item]\n          end)\n        end)\n      end\n    end\n\nAt this point it might be a good idea to refactor the code to separate the\ndifferent loops and conditions.\n",
        params: &[ParamDoc {
            name: "max_nesting",
            default: "2",
            doc: "The maximum number of levels code should be nested.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.PassAsyncInTestCases",
        id: "EX4031",
        category: "refactor",
        base_priority: "normal",
        tags: &[],
        explanation: "Test modules marked `async: true` are run concurrently, speeding up the\ntest suite and improving productivity. This should always be done when\npossible.\n\nLeaving off the `async:` option silently defaults to `false`, which may make\na test suite slower for no real reason.\n\nTest modules which cannot be run concurrently should be explicitly marked\n`async: false`, ideally with a comment explaining why.\n",
        params: &[
            ParamDoc {
                name: "files",
                default: "%{included: [\"test/**/*_test.exs\", \"apps/**/test/**/*_test.exs\"]}",
                doc: "",
            },
            ParamDoc {
                name: "force_comment_on_explicit_false",
                default: "false",
                doc: "Force adding a comment when `async: false` is used.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.PerceivedComplexity",
        id: "EX4022",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "Cyclomatic complexity is a software complexity metric closely correlated with\ncoding errors.\n\nIf a function feels like it's gotten too complex, it more often than not also\nhas a high CC value. So, if anything, this is useful to convince team members\nand bosses of a need to refactor parts of the code based on \"objective\"\nmetrics.\n",
        params: &[ParamDoc {
            name: "max_complexity",
            default: "9",
            doc: "The maximum cyclomatic complexity a function should have.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.PipeChainStart",
        id: "EX4023",
        category: "refactor",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "Pipes (`|>`) can become more readable by starting with a \"raw\" value.\n\nSo while this is easily comprehendable:\n\n    list\n    |> Enum.take(5)\n    |> Enum.shuffle\n    |> pick_winner()\n\nThis might be harder to read:\n\n    Enum.take(list, 5)\n    |> Enum.shuffle\n    |> pick_winner()\n\nAs always: This is just a suggestion. Check the configuration options for\ntweaking or disabling this check.\n",
        params: &[
            ParamDoc {
                name: "excluded_argument_types",
                default: "[]",
                doc: "All pipes with argument types listed will be ignored.",
            },
            ParamDoc {
                name: "excluded_functions",
                default: "[]",
                doc: "All functions listed will be ignored.",
            },
        ],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.PreferDateTimeShift",
        id: "EX4034",
        category: "refactor",
        base_priority: "low",
        tags: &[],
        explanation: "For `Date`, `DateTime`, `NaiveDateTime` and `Time` prefer `shift/2`\nover `add/2`, as it provides a more ergonomic API.\n\nThis should be refactored:\n\n    Date.add(date, 1)\n    DateTime.add(dt, 1, :hour)\n    NaiveDateTime.add(dt, 7, :day)\n    Time.add(time, 30, :minute)\n\nto look like this:\n\n    Date.shift(date, day: 1)\n    DateTime.shift(dt, hour: 1)\n    NaiveDateTime.shift(dt, day: 7)\n    Time.shift(time, minute: 30)\n\nSee https://hexdocs.pm/elixir/NaiveDateTime.html#add/3\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.RedundantWithClauseResult",
        id: "EX4024",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "`with` statements are useful when you need to chain a sequence\nof pattern matches, stopping at the first one that fails.\n\nIf the match of the last clause in a `with` statement is identical to the expression in the\nin its body, the code should be refactored to remove the redundant expression.\n\nThis should be refactored:\n\n    with {:ok, map} <- check(input),\n         {:ok, result} <- something(map) do\n      {:ok, result}\n    end\n\nto look like this:\n\n    with {:ok, map} <- check(input) do\n      something(map)\n    end\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.RejectFilter",
        id: "EX4025",
        category: "refactor",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "One `Enum.filter/2` is more efficient than `Enum.reject/2 |> Enum.filter/2`.\n\nThis should be refactored:\n\n    [\"a\", \"b\", \"c\"]\n    |> Enum.reject(&String.contains?(&1, \"x\"))\n    |> Enum.filter(&String.contains?(&1, \"a\"))\n\nto look like this:\n\n    Enum.filter([\"a\", \"b\", \"c\"], fn letter ->\n      !String.contains?(letter, \"x\") && String.contains?(letter, \"a\")\n    end)\n\nThe reason for this is performance, because the two calls to\n`Enum.reject/2` and `Enum.filter/2` require two iterations whereas\ndoing the functions in the single `Enum.filter/2` only requires one.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.RejectReject",
        id: "EX4026",
        category: "refactor",
        base_priority: "0",
        tags: &[],
        explanation: "One `Enum.reject/2` is more efficient than `Enum.reject/2 |> Enum.reject/2`.\n\nThis should be refactored:\n\n    [\"a\", \"b\", \"c\"]\n    |> Enum.reject(&String.contains?(&1, \"x\"))\n    |> Enum.reject(&String.contains?(&1, \"a\"))\n\nto look like this:\n\n    Enum.reject([\"a\", \"b\", \"c\"], fn letter ->\n      String.contains?(letter, \"x\") || String.contains?(letter, \"a\")\n    end)\n\nThe reason for this is performance, because the two separate calls\nto `Enum.reject/2` require two iterations whereas doing the\nfunctions in the single `Enum.reject/2` only requires one.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.UnlessWithElse",
        id: "EX4027",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "An `unless` block should not contain an else block.\n\nSo while this is fine:\n\n    unless allowed? do\n      raise \"Not allowed!\"\n    end\n\nThis should be refactored:\n\n    unless allowed? do\n      raise \"Not allowed!\"\n    else\n      proceed_as_planned()\n    end\n\nto look like this:\n\n    if allowed? do\n      proceed_as_planned()\n    else\n      raise \"Not allowed!\"\n    end\n\nThe reason for this is not a technical but a human one. The `else` in this\ncase will be executed when the condition is met, which is the opposite of\nwhat the wording seems to imply.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.UtcNowTruncate",
        id: "EX4032",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "`DateTime.utc_now/1` is more efficient than `DateTime.utc_now/0 |> DateTime.truncate/1`.\n\nFor example, the code here ...\n\n    DateTime.utc_now() |> DateTime.truncate(:second)\n    NaiveDateTime.utc_now() |> NaiveDateTime.truncate(:second)\n\n... can be refactored to look like this:\n\n    DateTime.utc_now(:second)\n    NaiveDateTime.utc_now(:second)\n\nThe reason for this is not just performance, because no separate function\ncall is required, but also brevity of the resulting code.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.VariableRebinding",
        id: "EX4028",
        category: "refactor",
        base_priority: "0",
        tags: &["controversial"],
        explanation: "You might want to refrain from rebinding variables.\n\nAlthough technically fine, rebinding to the same name can lead to less\nprecise naming.\n\nConsider this example:\n\n    def find_a_good_time do\n      time = MyApp.DateTime.now\n      time = MyApp.DateTime.later(time, 5, :days)\n      {:ok, time} = verify_available_time(time)\n\n      time\n    end\n\nWhile there is nothing wrong with this, many would consider the following\nimplementation to be easier to comprehend:\n\n    def find_a_good_time do\n      today = DateTime.now\n      proposed_time = DateTime.later(today, 5, :days)\n      {:ok, verified_time} = verify_available_time(proposed_time)\n\n      verified_time\n    end\n\nIn some rare cases you might really want to rebind a variable.  This can be\nenabled \"opt-in\" on a per-variable basis by setting the :allow_bang option\nto true and adding a bang suffix sigil to your variable.\n\n    def uses_mutating_parameters(params!) do\n      params! = do_a_thing(params!)\n      params! = do_another_thing(params!)\n      params! = do_yet_another_thing(params!)\n    end\n",
        params: &[ParamDoc {
            name: "allow_bang",
            default: "false",
            doc: "Variables with a bang suffix will be ignored.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Refactor.WithClauses",
        id: "EX4029",
        category: "refactor",
        base_priority: "high",
        tags: &[],
        explanation: "`with` statements are useful when you need to chain a sequence\nof pattern matches, stopping at the first one that fails.\n\nBut sometimes, we go a little overboard with them (pun intended).\n\nIf the first or last clause in a `with` statement is not a `<-` clause,\nit still compiles and works, but is not really utilizing what the `with`\nmacro provides and can be misleading.\n\n    with ref = make_ref(),\n         {:ok, user} <- User.create(ref),\n         :ok <- send_email(user),\n         Logger.debug(\"Created user: #{inspect(user)}\") do\n      user\n    end\n\nHere, both the first and last clause are actually not matching anything.\n\nIf we move them outside of the `with` (the first ones) or inside the body\nof the `with` (the last ones), the code becomes more focused and .\n\nThis `with` should be refactored like this:\n\n    ref = make_ref()\n\n    with {:ok, user} <- User.create(ref),\n         :ok <- send_email(user) do\n      Logger.debug(\"Created user: #{inspect(user)}\")\n      user\n    end\n",
        params: &[],
    },
    // --- warning ---
    CheckDoc {
        module: "Credo.Check.Warning.ApplicationConfigInModuleAttribute",
        id: "EX5001",
        category: "warning",
        base_priority: "high",
        tags: &["controversial"],
        explanation: "Module attributes are evaluated at compile time and not at run time. As\na result, certain configuration read calls made in your module attributes\nmay work as expected during local development, but may break once in a\ndeployed context.\n\nThis check analyzes all of the module attributes present within a module,\nand validates that there are no unsafe calls.\n\nThese unsafe calls include:\n\n- `Application.fetch_env/2`\n- `Application.fetch_env!/2`\n- `Application.get_all_env/1`\n- `Application.get_env/3`\n- `Application.get_env/2`\n\nAs of Elixir 1.10 you can leverage `Application.compile_env/3` and\n`Application.compile_env!/2` if you wish to set configuration at\ncompile time using module attributes.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.BoolOperationOnSameValues",
        id: "EX5002",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Boolean operations with identical values on the left and right side are\nmost probably a logical fallacy or a copy-and-paste error.\n\nExamples:\n\n    x && x\n    x || x\n    x and x\n    x or x\n\nEach of these cases behaves the same as if you were just writing `x`.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.Dbg",
        id: "EX5026",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Calls to dbg/0 and dbg/2 should mostly be used during debugging sessions.\n\nThis check warns about those calls, because they probably have been committed\nin error.\n",
        params: &[ParamDoc {
            name: "allow_captures",
            default: "false",
            doc: "Allow using a capture, e.g. `&dbg/1`.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.ExpensiveEmptyEnumCheck",
        id: "EX5003",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Checking if the size of the enum is `0` (or not `0`) can be very expensive,\nsince you are determining the exact count of elements.\n\nChecking if an enum is empty should be done by using\n\n    Enum.empty?(enum)\n\nor\n\n    list == []\n\n\nFor `Enum.count/2`: Checking if an enum doesn't contain specific elements should\nbe done by using\n\n    not Enum.any?(enum, condition)\n\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.ForbiddenFunction",
        id: "EX5033",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Some functions that are included by a package or public in your project\nmay be hazardous if used directly.\n\nUse this check to forbid specific functions from being called directly\nby your application (while allowing other functions from the same module).\n\nThis check is similar to `Credo.Check.Warning.ForbiddenModule`, but for\nspecific functions within a module rather than the entire module.\n\nExample:\n\n`:erlang.binary_to_term/1` is vulnerable to arbitrary code execution exploits\nwhen deserializing untrusted data; you may want to point developers to\n`Plug.Crypto.non_executable_binary_to_term/2` instead, which\ndisallows anonymous functions in the deserialized term.\n",
        params: &[ParamDoc {
            name: "functions",
            default: "[]",
            doc: "List of `{module, function, error_message}` tuples specifying forbidden functions.\n\nExample:\n\n    functions: [\n      {:erlang, :binary_to_term, \"Use `Plug.Crypto.non_executable_binary_to_term/2` instead.\"}\n    ]\n",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.ForbiddenModule",
        id: "EX5004",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Some modules that are included by a package may be hazardous\nif used by your application.\n\nUse this check to allow these modules in your dependencies but\nforbid them to be used in your application.\n\nExamples:\n\nThe `:ecto_sql` package includes the `Ecto.Adapters.SQL` module,\nbut direct usage of the `Ecto.Adapters.SQL.query/4` function, and related functions, may\ncause issues when using Ecto's dynamic repositories.\n",
        params: &[ParamDoc {
            name: "modules",
            default: "[]",
            doc: "List of modules or `{Module, \"Error message\"}` tuples that must not be used.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.IExPry",
        id: "EX5005",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "While calls to IEx.pry might appear in some parts of production code,\nmost calls to this function are added during debugging sessions.\n\nThis check warns about those calls, because they might have been committed\nin error.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.IoInspect",
        id: "EX5006",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "While calls to IO.inspect might appear in some parts of production code,\nmost calls to this function are added during debugging sessions.\n\nThis check warns about those calls, because they might have been committed\nin error.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.LazyLogging",
        id: "EX5007",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Ensures laziness of Logger calls.\n\nYou will want to wrap expensive logger calls into a zero argument\nfunction (`fn -> \"string that gets logged\" end`).\n\nExample:\n\n    # preferred\n\n    Logger.debug fn ->\n      \"This happened: #{expensive_calculation(arg1, arg2)}\"\n    end\n\n    # NOT preferred\n    # the interpolation is executed whether or not the info is logged\n\n    Logger.debug \"This happened: #{expensive_calculation(arg1, arg2)}\"\n",
        params: &[ParamDoc {
            name: "ignore",
            default: "[:error, :warn, :info]",
            doc: "Do not raise an issue for these Logger calls.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.LeakyEnvironment",
        id: "EX5008",
        category: "warning",
        base_priority: "high",
        tags: &["controversial"],
        explanation: "OS child processes inherit the environment of their parent process. This\nincludes sensitive configuration parameters, such as credentials. To\nminimize the risk of such values leaking, clear or overwrite them when\nspawning executables.\n\nThe functions `System.cmd/2` and `System.cmd/3` allow environment variables be cleared by\nsetting their value to `nil`:\n\n    System.cmd(\"env\", [], env: %{\"DB_PASSWORD\" => nil})\n\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.MapGetUnsafePass",
        id: "EX5009",
        category: "warning",
        base_priority: "normal",
        tags: &["controversial"],
        explanation: "`Map.get/2` can lead into runtime errors if the result is passed into a pipe\nwithout a proper default value. This happens when the next function in the\npipe cannot handle `nil` values correctly.\n\nExample:\n\n    %{foo: [1, 2 ,3], bar: [4, 5, 6]}\n    |> Map.get(:missing_key)\n    |> Enum.each(&IO.puts/1)\n\nThis will cause a `Protocol.UndefinedError`, since `nil` isn't `Enumerable`.\nOften times while iterating over enumerables zero iterations is preferable\nto being forced to deal with an exception. Had there been a `[]` default\nparameter this could have been averted.\n\nIf you are sure the value exists and can't be nil, please use `Map.fetch!/2`.\nIf you are not sure, `Map.get/3` can help you provide a safe default value.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.MissedMetadataKeyInLoggerConfig",
        id: "EX5027",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Ensures custom metadata keys are included in logger config.\n\nNote that all metadata is optional and may not always be available.\n\nFor example, you might wish to include a custom `:error_code` metadata in your logs:\n\n    Logger.error(\"We have a problem\", [error_code: :pc_load_letter])\n\nIn your app's logger configuration, you would need to include the `:error_code` key:\n\n    config :logger, :default_formatter,\n      format: \"[$level] $message $metadata\\n\",\n      metadata: [:error_code, :file]\n\nThat way your logs might then receive lines like this:\n\n    [error] We have a problem error_code=pc_load_letter file=lib/app.ex\n\nIf you want to allow any metadata to be printed, you can use `:all` in the logger's\nmetadata config.\n",
        params: &[ParamDoc {
            name: "metadata_keys",
            default: "[]",
            doc: "Do not raise an issue for these Logger metadata keys.\n\nBy default, we read the metadata keys configured as the current environment's\n`:default_formatter` (or `:console` for older versions of Elixir).\n\nYou can use this parameter to dynamically load the environment/backend you care about,\nvia `.credo.exs` (e.g. reading the `:file_log` config from `config/prod.exs`):\n\n    {Credo.Check.Warning.MissedMetadataKeyInLoggerConfig,\n      [\n        metadata_keys:\n          \"config/prod.exs\"\n          |> Config.Reader.read!()\n          |> get_in([:logger, :file_log, :metadata])\n      ]}\n",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.MixEnv",
        id: "EX5010",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Mix is a build tool and, as such, it is not expected to be available in production.\nTherefore, it is recommended to access Mix.env only in configuration files and inside\nmix.exs, never in your application code (lib).\n\n(from the Elixir docs)\n",
        params: &[ParamDoc {
            name: "excluded_paths",
            default: "[]",
            doc: "List of paths or regex to exclude from this check",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.OperationOnSameValues",
        id: "EX5011",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Operations on the same values always yield the same result and therefore make\nlittle sense in production code.\n\nExamples:\n\n    x == x  # always returns true\n    x <= x  # always returns true\n    x >= x  # always returns true\n    x != x  # always returns false\n    x > x   # always returns false\n    y / y   # always returns 1\n    y - y   # always returns 0\n\nIn practice they are likely the result of a debugging session or were made by\nmistake.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.OperationWithConstantResult",
        id: "EX5012",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Some numerical operations always yield the same result and therefore make\nlittle sense in production code.\n\nExamples:\n\n    x * 1   # always returns x\n    x * 0   # always returns 0\n\nIn practice they are likely the result of a debugging session or were made by\nmistake.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.RaiseInsideRescue",
        id: "EX5013",
        category: "warning",
        base_priority: "0",
        tags: &[],
        explanation: "Using `Kernel.raise` inside of a `rescue` block creates a new stacktrace.\n\nMost of the time, this is not what you want to do since it obscures the cause of the original error.\n\nExample:\n\n    # preferred\n\n    try do\n      raise \"oops\"\n    rescue\n      error ->\n        Logger.warn(\"An exception has occurred\")\n\n        reraise error, System.stacktrace\n    end\n\n    # NOT preferred\n\n    try do\n      raise \"oops\"\n    rescue\n      error ->\n        Logger.warn(\"An exception has occurred\")\n\n        raise error\n    end\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.SpecWithStruct",
        id: "EX5014",
        category: "warning",
        base_priority: "normal",
        tags: &[],
        explanation: "Structs create compile-time dependencies between modules.  Using a struct in a spec\nwill cause the module to be recompiled whenever the struct's module changes.\n\nIt is preferable to define and use `MyModule.t()` instead of `%MyModule{}` in specs.\n\nExample:\n\n    # preferred\n    @spec a_function(MyModule.t()) :: any\n\n    # NOT preferred\n    @spec a_function(%MyModule{}) :: any\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.StructFieldAmount",
        id: "EX5029",
        category: "warning",
        base_priority: "normal",
        tags: &[],
        explanation: "Avoid structs with 32 or more fields.\n\nStructs in Elixir are implemented as compile-time maps, which have a\npredefined amount of fields.\n\nWhen structs have 32 or more fields, their internal representation in\nthe Erlang Virtual Machines changes, potentially leading to bloating\nand higher memory usage.\n\nSee https://hexdocs.pm/elixir/1.19.0/code-anti-patterns.html#structs-with-32-fields-or-more\n",
        params: &[ParamDoc {
            name: "max_fields",
            default: "31",
            doc: "The maximum number of field a struct should be allowed to have.",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnsafeExec",
        id: "EX5015",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Spawning external commands can lead to command injection vulnerabilities.\n\nUse a safe API where arguments are passed as an explicit list, rather\nthan unsafe APIs that run a shell to parse the arguments from a single\nstring.\n\nSafe APIs include:\n\n  * `System.cmd/2,3`\n  * `:erlang.open_port/2`, passing `{:spawn_executable, file_name}` as the\n    first parameter, and any arguments using the `:args` option\n\nUnsafe APIs include:\n\n  * `:os.cmd/1,2`\n  * `:erlang.open_port/2`, passing `{:spawn, command}` as the first\n    parameter\n\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnsafeToAtom",
        id: "EX5016",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Creating atoms from unknown or external input is a security risk, not just a\nstyle preference.\n\nAtoms are not garbage-collected by the runtime and the number of atoms a node\ncan hold is capped (1_048_576 by default). Any code path that turns\nattacker-controlled input into atoms can therefore exhaust the atom table and\ncrash the entire VM. That is a denial-of-service vulnerability, and in a\nweb-facing application it is a CVE waiting to happen.\n\nCreating an atom from a string or charlist should be done by using\n\n    String.to_existing_atom(string)\n\nor\n\n    List.to_existing_atom(charlist)\n\nModule aliases should be constructed using\n\n    Module.safe_concat(prefix, suffix)\n\nor\n\n    Module.safe_concat([prefix, infix, suffix])\n\nJason.decode/Jason.decode! should be called using `keys: :atoms!` (*not* `keys: :atoms`):\n\n    Jason.decode(str, keys: :atoms!)\n\nor `:keys` should be omitted (which defaults to `:strings`):\n\n    Jason.decode(str)\n\nFor more on atom exhaustion as an attack vector, see:\n\nhttps://erlef.org/blog/security/atom-exhaustion\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedEnumOperation",
        id: "EX5017",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "With the exception of `Enum.each/2`, the result of a call to the\nEnum module's functions has to be used.\n\nWhile this is correct ...\n\n    def prepend_my_username(my_username, usernames) do\n      usernames = Enum.reject(usernames, &is_nil/1)\n\n      [my_username] ++ usernames\n    end\n\n... we forgot to save the downcased username in this example:\n\n    # This is bad because it does not modify the usernames variable!\n\n    def prepend_my_username(my_username, usernames) do\n      Enum.reject(usernames, &is_nil/1)\n\n      [my_username] ++ usernames\n    end\n\nSince Elixir variables are immutable, Enum operations never work on the\nvariable you pass in, but return a new variable which has to be used somehow\n(the exception being `Enum.each/2` which iterates a list and returns `:ok`).\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedFileOperation",
        id: "EX5018",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to the File module's functions has to be used.\n\nWhile this is correct ...\n\n    def read_from_cwd(filename) do\n      # TODO: use Path.join/2\n      filename = File.cwd!() <> \"/\" <> filename\n\n      File.read(filename)\n    end\n\n... we forgot to save the result in this example:\n\n    def read_from_cwd(filename) do\n      File.cwd!() <> \"/\" <> filename\n\n      File.read(filename)\n    end\n\nSince Elixir variables are immutable, many File operations don't work on the\nvariable you pass in, but return a new variable which has to be used somehow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedKeywordOperation",
        id: "EX5019",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to the Keyword module's functions has to be used.\n\nWhile this is correct ...\n\n    def clean_and_verify_options!(keywords) do\n      keywords = Keyword.delete(keywords, :debug)\n\n      if Enum.length(keywords) == 0, do: raise \"OMG!!!1\"\n\n      keywords\n    end\n\n... we forgot to save the result in this example:\n\n    def clean_and_verify_options!(keywords) do\n      Keyword.delete(keywords, :debug)\n\n      if Enum.length(keywords) == 0, do: raise \"OMG!!!1\"\n\n      keywords\n    end\n\nKeyword operations never work on the variable you pass in, but return a new\nvariable which has to be used somehow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedListOperation",
        id: "EX5020",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to the List module's functions has to be used.\n\nWhile this is correct ...\n\n    def sort_usernames(usernames) do\n      usernames = List.flatten(usernames)\n\n      List.sort(usernames)\n    end\n\n... we forgot to save the result in this example:\n\n    def sort_usernames(usernames) do\n      List.flatten(usernames)\n\n      List.sort(usernames)\n    end\n\nList operations never work on the variable you pass in, but return a new\nvariable which has to be used somehow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedMapOperation",
        id: "EX5028",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to the Map module's functions has to be used.\n\nWhile this is correct ...\n\n    def clean_and_verify_options!(map) do\n      map = Map.delete(map, :debug)\n\n      if Enum.length(map) == 0, do: raise \"OMG!!!1\"\n\n      map\n    end\n\n... we forgot to save the result in this example:\n\n    def clean_and_verify_options!(map) do\n      Map.delete(map, :debug)\n\n      if Enum.length(map) == 0, do: raise \"OMG!!!1\"\n\n      map\n    end\n\nMap operations never work on the variable you pass in, but return a new\nvariable which has to be used somehow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedOperation",
        id: "EX5031",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to some functions has to be used.\n\nThis is a generic check that you can configure to your needs.\nWith checks like `UnusedEnumOperation` you can catch instances where you call\ne.g. `Enum.reject/1`, but accidentally do not use the result:\n\n    def prepend_my_username(my_username, usernames) do\n      Enum.reject(usernames, &is_nil/1)\n\n      [my_username] ++ usernames\n    end\n\nWith this check you can do the same for your modules and functions.\n",
        params: &[ParamDoc {
            name: "modules",
            default: "[]",
            doc: "The modules and functions that should trigger this check.\n\nFormat: `{module, functions}` or `{module, functions, issue_message}`\n\n`functions` can be a list of functions names as atoms or `:all` to include all functions of `module`.\n",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedPathOperation",
        id: "EX5021",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to the Path module's functions has to be used.\n\nWhile this is correct ...\n\n    def read_from_cwd(filename) do\n      filename = Path.join(cwd, filename)\n\n      File.read(filename)\n    end\n\n... we forgot to save the result in this example:\n\n    def read_from_cwd(filename) do\n      Path.join(cwd, filename)\n\n      File.read(filename)\n    end\n\nPath operations never work on the variable you pass in, but return a new\nvariable which has to be used somehow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedRegexOperation",
        id: "EX5022",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to the Regex module's functions has to be used.\n\nWhile this is correct ...\n\n    def extract_username_and_salute(regex, string) do\n      [string] = Regex.run(regex, string)\n\n      \"Hi #{string}\"\n    end\n\n... we forgot to save the downcased username in this example:\n\n    def extract_username_and_salute(regex, string) do\n      Regex.run(regex, string)\n\n      \"Hi #{string}\"\n    end\n\nRegex operations never work on the variable you pass in, but return a new\nvariable which has to be used somehow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedStringOperation",
        id: "EX5023",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to the String module's functions has to be used.\n\nWhile this is correct ...\n\n    def salutation(username) do\n      username = String.downcase(username)\n\n      \"Hi #{username}\"\n    end\n\n... we forgot to save the downcased username in this example:\n\n    # This is bad because it does not modify the username variable!\n\n    def salutation(username) do\n      String.downcase(username)\n\n      \"Hi #{username}\"\n    end\n\nSince Elixir variables are immutable, String operations never work on the\nvariable you pass in, but return a new variable which has to be used somehow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.UnusedTupleOperation",
        id: "EX5024",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "The result of a call to the Tuple module's functions has to be used.\n\nWhile this is correct ...\n\n    def remove_magic_item!(tuple) do\n      tuple = Tuple.delete_at(tuple, 0)\n\n      if Enum.length(tuple) == 0, do: raise \"OMG!!!1\"\n\n      tuple\n    end\n\n... we forgot to save the result in this example:\n\n    def remove_magic_item!(tuple) do\n      Tuple.delete_at(tuple, 0)\n\n      if Enum.length(tuple) == 0, do: raise \"OMG!!!1\"\n\n      tuple\n    end\n\nTuple operations never work on the variable you pass in, but return a new\nvariable which has to be used somehow.\n",
        params: &[],
    },
    CheckDoc {
        module: "Credo.Check.Warning.WrongTestFileExtension",
        id: "EX5025",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Invoking mix test from the command line will run the tests in each file\nmatching the pattern `*_test.exs` found in the test directory of your project.\n\n(from the `ex_unit` docs)\n\nThis check ensures that test files are not ending with `_test.ex` (which would cause them to be skipped).\n",
        params: &[ParamDoc {
            name: "files",
            default: "%{included: [\"test/**/*_test.ex\", \"apps/**/test/**/*_test.ex\"]}",
            doc: "",
        }],
    },
    CheckDoc {
        module: "Credo.Check.Warning.WrongTestFilename",
        id: "EX5030",
        category: "warning",
        base_priority: "high",
        tags: &[],
        explanation: "Invoking mix test from the command line will run the tests in each file\nmatching the pattern `*_test.exs` found in the test directory of your project.\n\n(from the `ex_unit` docs)\n\nThis test ensures that files containing `use ExUnit.Case` and related cases are only\nused in files ending with `_test.exs`.\n\nIf you have a file named differently (say, `test_my_module.exs`), you will be able to\nrun `mix test test/test_my_module.exs` and see the tests run. This can mislead you\ninto believing that subsequently running the full test suite (`mix test`) will also\ntest your file.\n",
        params: &[ParamDoc {
            name: "files",
            default: "%{included: [\"test/\"], excluded: [\"test/**/*_test.exs\", \"apps/**/test/**/*_test.exs\"]}",
            doc: "",
        }],
    },
];

/// Documentation for a check module, or `None` for unknown modules.
#[must_use]
pub fn doc_for(module: &str) -> Option<&'static CheckDoc> {
    ALL_DOCS.iter().find(|doc| doc.module == module)
}

/// All 120 pinned check documents, grouped by category.
#[must_use]
pub fn all_docs() -> &'static [CheckDoc] {
    ALL_DOCS
}

/// SARIF rule document for a check module: the pinned id, the verbatim
/// explanation and the default hexdocs URI (no check overrides
/// `docs_uri` at the pinned commit). Unknown modules fall back exactly
/// like [`crate::format_machine::RuleDoc::fallback`], so this composes
/// directly with `MachineContext::with_rule_doc`.
#[must_use]
pub fn sarif_rule_doc(module: &str) -> crate::format_machine::RuleDoc {
    match doc_for(module) {
        Some(doc) => crate::format_machine::RuleDoc::new(
            doc.id,
            doc.explanation,
            format!("https://hexdocs.pm/credo/{module}.html"),
        ),
        None => crate::format_machine::RuleDoc::fallback(module),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check_meta::{CheckBase, base_priority, category_for, check_tags};

    #[test]
    fn table_holds_120_checks() {
        assert_eq!(all_docs().len(), 120);
    }

    #[test]
    fn every_inventory_id_appears_exactly_once() {
        let text = include_str!("../compatibility/upstream/inventory.json");
        let json: serde_json::Value = serde_json::from_str(text).expect("valid inventory JSON");
        let rules = json["rules"].as_array().expect("rules array");
        assert_eq!(rules.len(), 120);
        for rule in rules {
            let want = rule["upstream_id"].as_str().expect("upstream id");
            let hits = all_docs().iter().filter(|doc| doc.id == want).count();
            assert_eq!(hits, 1, "{want}");
        }
    }

    #[test]
    fn modules_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for doc in all_docs() {
            assert!(seen.insert(doc.module), "duplicate {}", doc.module);
        }
    }

    #[test]
    fn moduledoc_explanation_is_verbatim() {
        let doc = doc_for("Credo.Check.Readability.ModuleDoc").expect("moduledoc doc");
        assert_eq!(doc.id, "EX3009");
        assert_eq!(
            doc.explanation,
            "Every module should contain comprehensive documentation.\n\n    # preferred\n\n    defmodule MyApp.Web.Search do\n      @moduledoc \"\"\"\n      This module provides a public API for all search queries originating\n      in the web layer.\n      \"\"\"\n    end\n\n    # also okay: explicitly say there is no documentation\n\n    defmodule MyApp.Web.Search do\n      @moduledoc false\n    end\n\nMany times a sentence or two in plain english, explaining why the module\nexists, will suffice. Documenting your train of thought this way will help\nboth your co-workers and your future-self.\n\nOther times you will want to elaborate even further and show some\nexamples of how the module's functions can and should be used.\n\nIn some cases however, you might not want to document things about a module,\ne.g. it is part of a private API inside your project. Since Elixir prefers\nexplicitness over implicit behaviour, you should \"tag\" these modules with\n\n    @moduledoc false\n\nto make it clear that there is no intention in documenting it.\n\nLike all `Readability` issues, this one is not a technical concern.\nBut you can improve the odds of others reading and liking your code by making\nit easier to follow.\n"
        );
    }

    #[test]
    fn fields_agree_with_check_meta() {
        for doc in all_docs() {
            let category = category_for(doc.module).expect("known category");
            assert_eq!(doc.category, category.as_str(), "{}", doc.module);
            assert_eq!(doc.tags, check_tags(doc.module), "{}", doc.module);
            let expected_base = match base_priority(doc.module) {
                Some(CheckBase::Higher) => "higher",
                Some(CheckBase::High) => "high",
                Some(CheckBase::Normal) => "normal",
                Some(CheckBase::Low) => "low",
                Some(CheckBase::DefaultZero) => "0",
                None => panic!("unknown check {}", doc.module),
            };
            assert_eq!(doc.base_priority, expected_base, "{}", doc.module);
        }
    }

    #[test]
    fn doc_for_unknown_is_none() {
        assert_eq!(doc_for("Credo.Check.Nope"), None);
    }

    #[test]
    fn sarif_rule_doc_composes() {
        let rule = sarif_rule_doc("Credo.Check.Readability.ModuleDoc");
        assert_eq!(rule.id, "EX3009");
        assert!(!rule.explanation.is_empty());
        assert_eq!(
            rule.help_uri,
            "https://hexdocs.pm/credo/Credo.Check.Readability.ModuleDoc.html"
        );
        let fallback = sarif_rule_doc("Credo.Check.Nope");
        assert_eq!(fallback.id, "Credo.Check.Nope");
        assert!(fallback.explanation.is_empty());
    }

    #[test]
    fn sarif_ids_match_pinned_table_for_all_docs() {
        for doc in all_docs() {
            assert_eq!(sarif_rule_doc(doc.module).id, doc.id, "{}", doc.module);
        }
    }
}
