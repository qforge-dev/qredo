use crate::{Finding, Trigger};
use std::collections::BTreeMap;

/// `EX4017`: modules should not depend on too many other modules.
///
/// Dependencies are every dotted alias referenced in the `defmodule`
/// body, minus the module's own name and `alias` directive targets (single
/// and grouped forms). `alias` targets additionally resolve short
/// references to full names. `max_deps`, `dependency_namespaces`, and
/// `excluded_namespaces` params are honored; filename-based
/// `excluded_paths` live at pipeline level and are ignored here.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let facts = prepared.facts();
    let max_deps: usize = params
        .get("max_deps")
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);
    let keep_namespaces = string_list(params.get("dependency_namespaces"));
    let skip_namespaces = string_list(params.get("excluded_namespaces"));
    let limits = Limits {
        max_deps,
        keep_namespaces,
        skip_namespaces,
    };
    let mut findings = Vec::new();
    for module in &facts.modules {
        if let Some(finding) = check_module(module, source, facts, &limits) {
            findings.push(finding);
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// Resolved per-module inventory: dependencies, exclusions, resolutions.
struct ModuleInventory {
    deps: Vec<String>,
    exclusions: Vec<String>,
    resolutions: Vec<String>,
}

/// Dependency finding for one `defmodule`, or `None` when within limits.
fn check_module(
    module: &crate::facts::ModuleFact,
    source: &str,
    facts: &crate::facts::Facts,
    limits: &Limits,
) -> Option<Finding> {
    let (alias_start, _) = module.alias?;
    let name = module.name.clone();
    if limits.skip_namespaces.iter().any(|ns| name.starts_with(ns)) {
        return None;
    }
    let inventory = module_inventory(module, source, facts);
    let mut deps = inventory.deps;
    deps.retain(|dep| dep != &name);
    deps.retain(|dep| !inventory.exclusions.iter().any(|excluded| excluded == dep));
    let mut deps: Vec<String> = deps
        .into_iter()
        .map(|dep| resolve(&dep, &inventory.resolutions))
        .filter(|dep| {
            limits.keep_namespaces.is_empty()
                || limits.keep_namespaces.iter().any(|ns| dep.starts_with(ns))
        })
        .collect();
    deps.sort();
    deps.dedup();
    if deps.len() > limits.max_deps {
        let (line, column) = line_col(source, alias_start as usize);
        Some(Finding {
            line,
            column: Some(column),
            message: format!(
                "Module has too many dependencies: {} (max is {})",
                deps.len(),
                limits.max_deps
            ),
            trigger: Trigger::Text(name),
            severity: None,
        })
    } else {
        None
    }
}

/// Effective per-check limits from params.
struct Limits {
    max_deps: usize,
    keep_namespaces: Vec<String>,
    skip_namespaces: Vec<String>,
}

/// Per-module inventory by span containment, in walk order: every dotted
/// alias text plus alias-directive exclusions and short-name resolutions.
/// Document order matches the upstream prewalk.
fn module_inventory(
    module: &crate::facts::ModuleFact,
    source: &str,
    facts: &crate::facts::Facts,
) -> ModuleInventory {
    let mut inventory = ModuleInventory {
        deps: Vec::new(),
        exclusions: Vec::new(),
        resolutions: Vec::new(),
    };
    for (start, end) in &facts.aliases {
        if module.start <= *start
            && *end <= module.end
            && let Some(text) = slice(source, *start, *end)
        {
            inventory.deps.push(text.to_owned());
        }
    }
    for directive in &facts.alias_directives {
        if directive.start < module.start || directive.end > module.end {
            continue;
        }
        if directive.has_top_comma {
            continue;
        }
        if let Some((start, end)) = directive.single
            && let Some(target) = slice(source, start, end)
        {
            inventory.exclusions.push(target.to_owned());
            inventory.resolutions.push(target.to_owned());
        }
        if let Some(grouped) = &directive.grouped_top {
            let Some(base) = slice(source, grouped.base_start, grouped.base_end) else {
                continue;
            };
            inventory.exclusions.push(base.to_owned());
            for (part_start, part_end) in &grouped.parts {
                if let Some(part) = slice(source, *part_start, *part_end) {
                    inventory.exclusions.push(part.to_owned());
                    inventory.resolutions.push(format!("{base}.{part}"));
                }
            }
        }
    }
    inventory
}

/// Resolve a short dependency through `alias` targets: the first target
/// ending with the short name wins; otherwise the name stands.
fn resolve(dep: &str, aliases: &[String]) -> String {
    aliases
        .iter()
        .find(|alias| alias.ends_with(dep))
        .cloned()
        .unwrap_or_else(|| dep.to_owned())
}

/// Parse a compact-JSON string list param (atoms arrive bare).
fn string_list(raw: Option<&String>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// 1-based `(line, column)` with the column counted in characters.
fn line_col(source: &str, byte: usize) -> (usize, usize) {
    let before = source.get(..byte).unwrap_or("");
    let line = before.bytes().filter(|&byte| byte == b'\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

/// Source slice for fact spans; `None` on invalid boundaries.
fn slice(source: &str, start: u32, end: u32) -> Option<&str> {
    source.get(start as usize..end as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    /// Single-source entry: the same lazy parse the old `check` built.
    fn check(src: &str, params: &BTreeMap<String, String>) -> Vec<Finding> {
        check_prepared(&crate::batch::Prepared::lazy(src), params)
    }

    #[test]
    fn few_deps_clean() {
        assert!(check("defmodule M do\n alias A\nend\n", &BTreeMap::new()).is_empty());
    }
    #[test]
    fn reports_many_deps() {
        // Alias directives alone are not dependencies (upstream excludes
        // their targets); direct references are.
        let mut src = String::from("defmodule M do\n  def f do\n    [\n");
        for i in 0..12 {
            let _ = writeln!(src, "      Mod{i},");
        }
        src.push_str("    ]\n  end\nend\n");
        assert_eq!(check(&src, &BTreeMap::new()).len(), 1);
    }
    #[test]
    fn reports_referenced_modules_without_directives() {
        let src = "defmodule CredoSampleModule do\n  def some_function() do\n    [\n      DateTime,\n      Kernel,\n      GenServer,\n      GenEvent,\n      File,\n      Time,\n      IO,\n      Logger,\n      URI,\n      Path,\n      String\n    ]\n  end\nend\n";
        let findings = check(src, &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (1, Some(11)));
    }
    #[test]
    fn resolves_aliased_modules() {
        let src = "defmodule CredoSampleModule do\n  alias Foo.Bar.DateTime\n  alias Foo.Bar.Kernel\n  def some_function() do\n    [DateTime, Kernel]\n  end\nend\n";
        let findings = check(src, &BTreeMap::new());
        assert!(findings.is_empty());
    }
    #[test]
    fn respects_namespaces() {
        let src = "defmodule CredoSample.Excluded.Module do\n  def some_function() do\n    [Foo.Bar.DateTime, Foo.Bar.Kernel, URI, Path, String]\n  end\nend\n";
        let mut params = BTreeMap::new();
        params.insert(
            "excluded_namespaces".to_owned(),
            r#"["CredoSample.Excluded"]"#.to_owned(),
        );
        assert!(check(src, &params).is_empty());
        let mut params = BTreeMap::new();
        params.insert(
            "dependency_namespaces".to_owned(),
            r#"["Foo.Bar"]"#.to_owned(),
        );
        assert!(check(src, &params).is_empty());
    }

    #[test]
    fn broken_source_stays_clean() {
        assert!(check("def foo( do\n", &BTreeMap::new()).is_empty());
    }
}
