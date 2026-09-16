use crate::{Finding, Trigger};
use std::collections::BTreeMap;

/// `EX5004`: uses of configured forbidden modules.
///
/// Without configured `modules`, the kernel is conservative (no findings).
/// Every `alias` node is visited (definitions, calls, directives), matching
/// upstream's prewalk; grouped `alias Base.{A, B}` parts resolve against
/// their base with the short name as trigger.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let forbidden = parse_modules(params);
    if forbidden.is_empty() {
        return Vec::new();
    }
    let facts = prepared.facts();
    let mut findings = Vec::new();
    for (start, end) in &facts.aliases {
        let Some(full) = slice(source, *start, *end) else {
            continue;
        };
        if let Some(message) = forbidden.get(full) {
            let (line, column) = line_col(source, *start as usize);
            findings.push(issue(line, column, message.as_ref(), full));
        }
    }
    for directive in &facts.alias_directives {
        for grouped in &directive.grouped_all {
            let Some(base) = slice(source, grouped.base_start, grouped.base_end) else {
                continue;
            };
            for (part_start, part_end) in &grouped.parts {
                let Some(part) = slice(source, *part_start, *part_end) else {
                    continue;
                };
                if let Some(message) = forbidden.get(&format!("{base}.{part}")) {
                    let (line, column) = line_col(source, *part_start as usize);
                    findings.push(issue(line, column, message.as_ref(), part));
                }
            }
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

#[must_use]
fn issue(line: usize, column: usize, message: Option<&String>, trigger: &str) -> Finding {
    Finding {
        line,
        column: Some(column),
        message: message
            .cloned()
            .unwrap_or_else(|| format!("The `{trigger}` module is not allowed.")),
        trigger: Trigger::Text(trigger.to_owned()),
        severity: None,
    }
}

/// Parse the `modules` param: compact JSON array of module atoms (bare
/// `Elixir.Foo` names) or `{module, message}` tuples.
fn parse_modules(params: &BTreeMap<String, String>) -> BTreeMap<String, Option<String>> {
    let mut out = BTreeMap::new();
    let Some(raw) = params.get("modules") else {
        return out;
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return out;
    };
    for item in &items {
        if let Some(name) = item.as_str() {
            out.insert(full_name(name), None);
        } else if let Some(parts) = item.get("tuple").and_then(serde_json::Value::as_array)
            && parts.len() == 2
            && let (Some(name), Some(message)) = (parts[0].as_str(), parts[1].as_str())
        {
            out.insert(full_name(name), Some(message.to_owned()));
        }
    }
    out
}

/// Strip the `Elixir.` atom prefix to the dotted module name.
/// Corpus atoms arrive colon-marked (`:Elixir.Foo`); matching is tolerant.
fn full_name(name: &str) -> String {
    let bare = name.trim_start_matches(':');
    bare.strip_prefix("Elixir.").unwrap_or(bare).to_owned()
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
    #[test]
    fn conservative_empty() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("x = 1\n"), &BTreeMap::new()).is_empty()
        );
    }
    #[test]
    fn reports_inline_forbidden_module() {
        let mut params = BTreeMap::new();
        params.insert(
            "modules".to_owned(),
            r#"["Elixir.CredoSampleModule.ForbiddenModule"]"#.to_owned(),
        );
        let src = "defmodule CredoSampleModule do\n  def some_function, do: CredoSampleModule.ForbiddenModule.another_function()\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (2, Some(26)));
    }
    #[test]
    fn reports_grouped_alias_parts_with_short_trigger() {
        let mut params = BTreeMap::new();
        params.insert(
            "modules".to_owned(),
            r#"["Elixir.CredoSampleModule.ForbiddenModule","Elixir.CredoSampleModule.ForbiddenModule2"]"#
                .to_owned(),
        );
        let src = "defmodule CredoSampleModule do\n  alias CredoSampleModule.{AllowedModule, ForbiddenModule, ForbiddenModule2}\n  def some_function, do: ForbiddenModule.another_function()\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 2);
        assert_eq!(
            findings[0].trigger,
            Trigger::Text("ForbiddenModule".to_owned())
        );
        assert_eq!((findings[0].line, findings[0].column), (2, Some(43)));
    }
    #[test]
    fn reports_custom_message() {
        let mut params = BTreeMap::new();
        params.insert(
            "modules".to_owned(),
            r#"[{"tuple":["Elixir.CredoSampleModule.ForbiddenModule","my message"]}]"#.to_owned(),
        );
        let src = "defmodule CredoSampleModule do\n  def some_function, do:\n    CredoSampleModule.ForbiddenModule.another_function()\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].message, "my message");
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
    }

    #[test]
    fn broken_source_stays_clean() {
        let mut params = BTreeMap::new();
        params.insert("modules".to_owned(), r#"["Elixir.Foo"]"#.to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy("def foo( do\n"), &params).is_empty());
    }
}
