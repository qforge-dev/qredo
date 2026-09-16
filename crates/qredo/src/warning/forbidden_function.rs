use crate::{Finding, Trigger};
use std::collections::BTreeMap;

/// `EX5033`: calls to configured `{module, function, message}` triples.
///
/// Without configured `functions`, the kernel is conservative (no findings).
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let forbidden = parse_functions(params);
    if forbidden.is_empty() {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for call in &prepared.facts().calls {
        let Some(crate::facts::HeadFact::Remote {
            mod_start,
            mod_end,
            mod_kind,
            fun_start,
            fun_end,
        }) = &call.head
        else {
            continue;
        };
        let module = match mod_kind {
            crate::facts::ModKind::Alias => slice(source, *mod_start, *mod_end),
            crate::facts::ModKind::Atom => {
                slice(source, *mod_start, *mod_end).map(|text| text.trim_start_matches(':'))
            }
            crate::facts::ModKind::Other => None,
        };
        let (Some(module), Some(fun)) = (module, slice(source, *fun_start, *fun_end)) else {
            continue;
        };
        if let Some(entry) = forbidden
            .iter()
            .find(|entry| entry.module == module && entry.fun == fun)
        {
            let (line, column) = line_col(source, *mod_start as usize);
            findings.push(Finding {
                line,
                column: Some(column),
                message: entry.message.clone(),
                trigger: Trigger::Text(format!("{}.{}", entry.display, entry.fun)),
                severity: None,
            });
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// A configured forbidden function: `module` is the match key (`erlang` for
/// Erlang modules, dotted name otherwise), `display` renders the trigger.
struct Forbidden {
    module: String,
    display: String,
    fun: String,
    message: String,
}

/// Parse the `functions` param: compact JSON array of `{mod, fun, message}`
/// tuples whose atoms arrive as bare names (`Elixir.Foo`, `erlang`).
fn parse_functions(params: &BTreeMap<String, String>) -> Vec<Forbidden> {
    let Some(raw) = params.get("functions") else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in &items {
        let Some(parts) = item.get("tuple").and_then(serde_json::Value::as_array) else {
            continue;
        };
        if parts.len() != 3 {
            continue;
        }
        let (Some(module), Some(fun)) = (parts[0].as_str(), parts[1].as_str()) else {
            continue;
        };
        let (key, display) = split_module(module);
        let fun = fun.strip_prefix(':').unwrap_or(fun);
        let message = match &parts[2] {
            serde_json::Value::String(message) => message.clone(),
            serde_json::Value::Null => {
                format!("Calls to `{display}.{fun}` are not allowed.")
            }
            _ => continue,
        };
        out.push(Forbidden {
            module: key,
            display,
            fun: fun.to_owned(),
            message,
        });
    }
    out
}

/// Split an atom name into match key and trigger display.
/// Corpus atoms arrive colon-marked (`:erlang`, `:Elixir.Foo.Bar`);
/// `Elixir.Foo.Bar` matches dotted calls shown as `Foo.Bar`;
/// any other atom (e.g. `erlang`) matches `:atom` calls shown with a colon.
fn split_module(name: &str) -> (String, String) {
    let bare = name.trim_start_matches(':');
    if let Some(dotted) = bare.strip_prefix("Elixir.") {
        (dotted.to_owned(), dotted.to_owned())
    } else {
        (bare.to_owned(), format!(":{bare}"))
    }
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
    fn reports_configured_erlang_function() {
        let mut params = BTreeMap::new();
        params.insert(
            "functions".to_owned(),
            r#"[{"tuple":["erlang","binary_to_term","Use safe alternative."]}]"#.to_owned(),
        );
        let src = "defmodule MyModule do\n  def decode(data) do\n    :erlang.binary_to_term(data)\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
    }
    #[test]
    fn reports_aliased_module_with_trigger() {
        let mut params = BTreeMap::new();
        params.insert(
            "functions".to_owned(),
            r#"[{"tuple":["Elixir.SomeModule","dangerous_function","This function is dangerous."]}]"#
                .to_owned(),
        );
        let src = "defmodule MyModule do\n  def dangerous do\n    SomeModule.dangerous_function(\"foo\")\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
        assert_eq!(findings[0].column, Some(5));
        assert_eq!(
            findings[0].trigger,
            Trigger::Text("SomeModule.dangerous_function".to_owned())
        );
    }
    #[test]
    fn ignores_other_functions_and_magic_modules() {
        let mut params = BTreeMap::new();
        params.insert(
            "functions".to_owned(),
            r#"[{"tuple":["Elixir.Some.Nested.Module","forbidden_func","Don't use this."]}]"#
                .to_owned(),
        );
        let src = "defmodule MyModule do\n  def call do\n    Some.Nested.Module.forbidden_func()\n    __MODULE__.Nested.Module.fun()\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].trigger,
            Trigger::Text("Some.Nested.Module.forbidden_func".to_owned())
        );
    }

    #[test]
    fn broken_source_stays_clean() {
        let mut params = BTreeMap::new();
        params.insert(
            "functions".to_owned(),
            r#"[{"tuple":["erlang","binary_to_term","Use safe alternative."]}]"#.to_owned(),
        );
        assert!(check_prepared(&crate::batch::Prepared::lazy("def foo( do\n"), &params).is_empty());
    }
    #[test]
    fn null_message_uses_default_template_erlang() {
        let mut params = BTreeMap::new();
        params.insert(
            "functions".to_owned(),
            r#"[{"tuple":[":erlang",":binary_to_term",null]}]"#.to_owned(),
        );
        let src = "defmodule MyModule do\n  def decode(data) do\n    :erlang.binary_to_term(data)\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Calls to `:erlang.binary_to_term` are not allowed."
        );
        assert_eq!(
            findings[0].trigger,
            Trigger::Text(":erlang.binary_to_term".to_owned())
        );
    }
    #[test]
    fn null_message_uses_default_template_elixir_module() {
        let mut params = BTreeMap::new();
        params.insert(
            "functions".to_owned(),
            r#"[{"tuple":["Elixir.SomeModule","dangerous_function",null]}]"#.to_owned(),
        );
        let src = "defmodule MyModule do\n  def dangerous do\n    SomeModule.dangerous_function(\"foo\")\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Calls to `SomeModule.dangerous_function` are not allowed."
        );
        assert_eq!(
            findings[0].trigger,
            Trigger::Text("SomeModule.dangerous_function".to_owned())
        );
    }
}
