//! Tree-sitter scope resolution mirroring `Credo.Code.Scope.name/2`.
//!
//! Returns the innermost enclosing `{defmodule, def, defp, defmacro}`
//! scope for a line as a dotted name (`"M"`, `"A.B.f"`), accumulating
//! nested module names. Definitions record without descending (mirroring
//! the `nil` stops in `traverse_defs`); only the scope name is kept
//! because downstream issue building discards the operator. Empty source
//! or out-of-range lines yield `""` (upstream reports `{nil, ""}`).

/// Precomputed per-file scopes mirroring the reversed scope info list.
///
/// Build once per file, then resolve every finding line against it —
/// upstream caches the same way because per-finding recomputation is
/// prohibitive on large files. Scopes resolve from the shared single-walk
/// facts; the per-line table makes `at` O(1) instead of scanning records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scopes {
    scopes: Vec<String>,
    /// Scope id per 1-based source line.
    line_scopes: Vec<u32>,
}

impl Scopes {
    /// Collect scopes for a source file.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    #[must_use]
    pub fn collect(source: &str) -> Self {
        match facts_of(source) {
            Some(facts) => Self::collect_on_facts(&facts),
            None => Self {
                scopes: vec![String::new()],
                line_scopes: Vec::new(),
            },
        }
    }

    /// Collect scopes over shared single-walk facts: the pipeline shares
    /// one parse and one walk per file instead of paying per consumer.
    #[must_use]
    pub fn collect_on_facts(facts: &crate::facts::Facts) -> Self {
        Self {
            scopes: facts.scopes.clone(),
            line_scopes: facts.line_scopes.clone(),
        }
    }

    /// Innermost scope name at `line` (1-based), or `""` outside any scope.
    #[must_use]
    pub fn at(&self, line: usize) -> &str {
        line.checked_sub(1)
            .and_then(|index| self.line_scopes.get(index))
            .and_then(|id| self.scopes.get(*id as usize))
            .map_or("", String::as_str)
    }
}

/// Innermost scope name at `line` (1-based), or `""` outside any scope.
#[must_use]
pub fn scope_for(source: &str, line: usize) -> String {
    Scopes::collect(source).at(line).to_owned()
}

/// Parse and extract with the pinned grammar; `None` when unavailable.
fn facts_of(source: &str) -> Option<crate::facts::Facts> {
    crate::ts_parser::parse(source).map(|tree| crate::facts::extract(&tree, source))
}

/// Scope priority bonuses mirroring `Credo.Priority.scope_priorities/1`.
///
/// Modules contribute 1, or 2 with five or more definitions in their whole
/// subtree; definitions contribute 0/1/2/3 by parameter count, plus their
/// own module's bonus for function scopes. Later same-name scopes overwrite
/// earlier ones, mirroring the map build.
/// Scope bonuses over shared single-walk facts: definition counts come
/// from span containment, arities from the extractor.
pub(crate) fn scope_priorities_on_facts(
    facts: &crate::facts::Facts,
) -> std::collections::BTreeMap<String, i32> {
    let mut own: std::collections::BTreeMap<String, i32> = std::collections::BTreeMap::new();
    for module in &facts.modules {
        let defs = facts
            .defs
            .iter()
            .filter(|def| module.start <= def.start && def.end <= module.end)
            .count();
        own.insert(module.name.clone(), if defs >= 5 { 2 } else { 1 });
    }
    for def in &facts.defs {
        own.insert(
            crate::facts::join_scope(&def.scope, def.name.as_deref()),
            param_bonus(def.arity),
        );
    }
    let lookup = own.clone();
    for (scope, bonus) in &mut own {
        if let Some(module) = mod_name(scope)
            && module != *scope
        {
            *bonus += lookup.get(module).copied().unwrap_or(0);
        }
    }
    own
}

/// Enclosing module name of a function scope (`"A.B"` for `"A.B.g"` when
/// `g` starts lowercase; the whole name otherwise), mirroring `mod_name/1`.
fn mod_name(scope: &str) -> Option<&str> {
    let last = scope.rsplit('.').next().unwrap_or(scope);
    if last.starts_with(|c: char| c == '_' || c.is_ascii_lowercase()) {
        scope.rsplit_once('.').map(|(parent, _)| parent)
    } else {
        Some(scope)
    }
}

/// Parameter-count bonus of a definition: 0, 1 (1-2 params), 2 (3-4) or 3.
fn param_bonus(arity: u32) -> i32 {
    match arity {
        0 => 0,
        1 | 2 => 1,
        3 | 4 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Bonuses over shared facts (the pipeline path under test).
    fn prios(source: &str) -> BTreeMap<String, i32> {
        match crate::ts_parser::parse(source) {
            Some(tree) => {
                let facts = crate::facts::extract(&tree, source);
                scope_priorities_on_facts(&facts)
            }
            None => BTreeMap::new(),
        }
    }

    #[test]
    fn module_and_function_scopes() {
        let source = "defmodule M do\n  def foo, do: 1\nend\n";
        assert_eq!(scope_for(source, 1), "M");
        assert_eq!(scope_for(source, 2), "M.foo");
        assert_eq!(scope_for(source, 3), "M.foo");
    }

    #[test]
    fn nested_modules_accumulate() {
        let source =
            "defmodule A do\n  defmodule B do\n    def f, do: 1\n  end\n  def g, do: 2\nend\n";
        assert_eq!(scope_for(source, 1), "A");
        assert_eq!(scope_for(source, 2), "A.B");
        assert_eq!(scope_for(source, 3), "A.B.f");
        assert_eq!(scope_for(source, 5), "A.g");
    }

    #[test]
    fn top_level_code_has_empty_scope() {
        assert_eq!(scope_for("x = 1\n", 1), "");
    }

    #[test]
    fn definition_kinds_share_naming() {
        let source = "defmodule M do\n  def foo(a, b), do: a\n  defp bar, do: 1\n  defmacro baz, do: 2\nend\n";
        assert_eq!(scope_for(source, 2), "M.foo");
        assert_eq!(scope_for(source, 3), "M.bar");
        assert_eq!(scope_for(source, 4), "M.baz");
    }

    #[test]
    fn one_liner_reports_definition_scope() {
        let source = "defmodule M, do: (def foo, do: 1)\n";
        assert_eq!(scope_for(source, 1), "M.foo");
    }

    #[test]
    fn when_guards_unwrap_to_function_name() {
        let source = "defmodule M do\n  def foo(x) when is_list(x), do: x\nend\n";
        assert_eq!(scope_for(source, 2), "M.foo");
    }

    #[test]
    fn operator_definition_keeps_operator_name() {
        let source = "defmodule M do\n  def a + b, do: a\nend\n";
        assert_eq!(scope_for(source, 2), "M.+");
    }

    #[test]
    fn unknown_module_heads_match_upstream() {
        let source = "defmodule __MODULE__ do\n  def f, do: 1\nend\n";
        assert_eq!(scope_for(source, 1), "<Unknown Module Name>");
        assert_eq!(scope_for(source, 2), "<Unknown Module Name>.f");
    }

    #[test]
    fn scope_priorities_match_native_values() {
        let source = "defmodule M do\n  def a, do: 1\n  def b, do: 2\n  def c, do: 3\n  def d, do: 4\n  def e, do: 5\nend\n";
        let expected: BTreeMap<String, i32> = [
            ("M".to_owned(), 2),
            ("M.a".to_owned(), 2),
            ("M.b".to_owned(), 2),
            ("M.c".to_owned(), 2),
            ("M.d".to_owned(), 2),
            ("M.e".to_owned(), 2),
        ]
        .into_iter()
        .collect();
        assert_eq!(prios(source), expected);
    }

    #[test]
    fn scope_priorities_count_defaulted_params() {
        let source = "defmodule M do\n  def f(a \\\\ 1, b \\\\ 2, c \\\\ 3, d \\\\ 4, e \\\\ 5), do: 1\nend\n";
        let expected: BTreeMap<String, i32> = [("M".to_owned(), 1), ("M.f".to_owned(), 4)]
            .into_iter()
            .collect();
        assert_eq!(prios(source), expected);
    }

    #[test]
    fn scope_priorities_nest_modules() {
        let source = "defmodule A do\n  def f, do: 1\n  defmodule B do\n    def g(a, b, c), do: 1\n  end\nend\n";
        let expected: BTreeMap<String, i32> = [
            ("A".to_owned(), 1),
            ("A.B".to_owned(), 1),
            ("A.B.g".to_owned(), 3),
            ("A.f".to_owned(), 1),
        ]
        .into_iter()
        .collect();
        assert_eq!(prios(source), expected);
    }

    #[test]
    fn scope_priorities_count_nested_defs_for_module_bonus() {
        let source = "defmodule A do\n  def a1, do: 1\n  def a2, do: 1\n  def a3, do: 1\n  def a4, do: 1\n  defmodule B do\n    def g1, do: 1\n    def g2, do: 1\n  end\nend\n";
        let prios = prios(source);
        assert_eq!(prios.get("A"), Some(&2));
        assert_eq!(prios.get("A.B"), Some(&1));
    }

    #[test]
    fn on_facts_variants_match_source_variants() {
        // Sharing the prepare-phase facts must not change scopes,
        // including on unparseable input (empty results). Bonus values
        // are pinned by the reviewed `scope_priorities_*` tests above.
        for source in [
            "defmodule A do\n  def f(a, b, c), do: 1\n  def g, do: 2\nend\n",
            "x = 1\n",
            "def foo( do\n",
        ] {
            if let Some(facts) = facts_of(source) {
                assert_eq!(Scopes::collect_on_facts(&facts), Scopes::collect(source));
            } else {
                // Without a grammar both variants report empty results.
                assert_eq!(Scopes::collect(source), Scopes::collect(""));
            }
        }
    }
}
