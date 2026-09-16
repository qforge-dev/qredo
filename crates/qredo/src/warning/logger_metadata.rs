use crate::{Finding, Trigger};
use std::collections::BTreeMap;

/// Logger calls with remote `Logger.` prefix.
const LOGGER_FUNS: [&str; 10] = [
    "alert",
    "critical",
    "debug",
    "emergency",
    "error",
    "info",
    "notice",
    "warn",
    "warning",
    "metadata",
];

/// `EX5027`: logger metadata keys must exist in the logger config.
///
/// Single-source best effort: natively supported keys (`ansi_color`,
/// `report_cb`) plus the configured `metadata_keys` param are allow-listed.
/// The ambient logger application config is unavailable here; an absent or
/// empty param behaves like an empty ambient config. `metadata_keys: :all`
/// skips the check, mirroring upstream.
/// `Logger.log/2` and single-argument calls never carry metadata.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let allowed = configured_keys(params);
    let Some(allowed) = allowed else {
        return Vec::new();
    };
    let facts = prepared.facts();
    let mut findings = Vec::new();
    for call in &facts.calls {
        if let Some(finding) = call_finding(call, source, &allowed) {
            findings.push(finding);
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// One finding for a `Logger` call carrying disallowed metadata keys.
fn call_finding(
    call: &crate::facts::CallFact,
    source: &str,
    allowed: &[String],
) -> Option<Finding> {
    let keys = metadata_keys(call, source)?;
    let missed: Vec<(String, (u32, u32))> = keys
        .into_iter()
        .filter(|(key, _)| !allowed.iter().any(|name| name == key))
        .collect();
    if missed.is_empty() {
        None
    } else {
        Some(missed_issue(source, &missed))
    }
}

/// Keyword metadata keys of one `Logger` call with their spans, if it
/// carries a keyword list: second argument for levels, only argument
/// for `metadata/1`, third for `log/3`. `None` otherwise.
fn metadata_keys(call: &crate::facts::CallFact, source: &str) -> Option<Vec<(String, (u32, u32))>> {
    let Some(crate::facts::HeadFact::Remote {
        mod_start,
        mod_end,
        mod_kind,
        fun_start,
        fun_end,
    }) = &call.head
    else {
        return None;
    };
    if *mod_kind != crate::facts::ModKind::Alias {
        return None;
    }
    let (Some(module), Some(fun)) = (
        slice(source, *mod_start, *mod_end),
        slice(source, *fun_start, *fun_end),
    ) else {
        return None;
    };
    if module != "Logger" || (!LOGGER_FUNS.contains(&fun) && fun != "log") {
        return None;
    }
    let positionals: Vec<&crate::facts::ArgFact> =
        call.args.iter().filter(|arg| arg.named).collect();
    let keys = match fun {
        "metadata" if positionals.len() == 1 => &positionals[0].keys,
        "log" if positionals.len() == 3 => &positionals[2].keys,
        "log" => return None,
        _ if positionals.len() == 2 => &positionals[1].keys,
        _ => return None,
    };
    if keys.is_empty() {
        return None;
    }
    Some(
        keys.iter()
            .filter_map(|pair| {
                let key = slice(source, pair.key_start, pair.key_end)?;
                Some((
                    key.trim_end_matches([':', ' ']).to_owned(),
                    (pair.key_start, pair.key_end),
                ))
            })
            .collect(),
    )
}

/// Allow-listed keys: native keys plus the `metadata_keys` param.
/// `None` when the param selects `:all` (check skipped).
fn configured_keys(params: &BTreeMap<String, String>) -> Option<Vec<String>> {
    let mut allowed = vec!["ansi_color".to_owned(), "report_cb".to_owned()];
    let Some(raw) = params.get("metadata_keys") else {
        return Some(allowed);
    };
    if raw == ":all" || raw == "all" {
        return None;
    }
    if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
        for value in values {
            if let serde_json::Value::String(name) = value {
                allowed.push(name.strip_prefix(':').unwrap_or(&name).to_owned());
            }
        }
    }
    Some(allowed)
}

/// One issue per offending call: the first missed key is the trigger and
/// the message joins all missed keys, at the first key's position.
fn missed_issue(source: &str, missed: &[(String, (u32, u32))]) -> Finding {
    let (line, column) = line_col(source, missed[0].1.0 as usize);
    let names: Vec<&str> = missed.iter().map(|(key, _)| key.as_str()).collect();
    Finding {
        line,
        column: Some(column),
        message: format!(
            "Logger metadata key {} not found in Logger config.",
            names.join(", ")
        ),
        trigger: Trigger::Text(names[0].to_owned()),
        severity: None,
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
    fn params() -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    #[test]
    fn conservative_empty() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = 1\n"), &params()).is_empty());
    }
    #[test]
    fn reports_unknown_metadata_key() {
        let src = "defmodule CredoSampleModule do\n\n  def some_function(parameter1, parameter2) do\n    var_1 = \"Hello world\"\n    Logger.alert(\"The module: #{var1}\", key: \"value\")\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (5, Some(41)));
    }
    #[test]
    fn allows_native_keys_and_bare_calls() {
        let src = "defmodule M do\n  def f do\n    Logger.debug(\"test\", ansi_color: :yellow)\n    Logger.info(message)\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params()).is_empty());
    }
    #[test]
    fn configured_keys_are_allowed() {
        let src =
            "defmodule M do\n  def f do\n    Logger.alert(\"hi\", account_id: 1)\n  end\nend\n";
        let mut configured = BTreeMap::new();
        configured.insert("metadata_keys".to_owned(), "[\"account_id\"]".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &configured).is_empty());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params()).len(),
            1
        );
    }
    #[test]
    fn all_selection_skips() {
        let src =
            "defmodule M do\n  def f do\n    Logger.alert(\"hi\", account_id: 1)\n  end\nend\n";
        let mut all = BTreeMap::new();
        all.insert("metadata_keys".to_owned(), ":all".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &all).is_empty());
    }

    #[test]
    fn broken_source_stays_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("def foo( do\n"), &params()).is_empty()
        );
    }
}
