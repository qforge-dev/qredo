use crate::Finding;
use std::collections::BTreeMap;

/// `EX3003`: do not pipe into a block (`|> call do ... end`, any macro or
/// function taking a `do` block; upstream matches the piped AST call).
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let excluded = excluded_names(params);
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let mut search = 0_usize;
        while let Some(pos) = line[search..].find("|>") {
            let base = search + pos;
            if let Some(name) = piped_block_name(&line[base + 2..])
                && !excluded.iter().any(|e| e == &name)
            {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(base + 1),
                    "Use a variable or create a new function instead of piping to a block.",
                    "|>",
                ));
            }
            search = base + 2;
            if search >= line.len() {
                break;
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// If the pipe target is a call followed by a `do` block keyword, return the
/// called function name (last dotted segment); otherwise `None`.
fn piped_block_name(after: &str) -> Option<String> {
    let gap = after
        .find(|c: char| !c.is_whitespace())
        .map_or(after.len(), |p| p);
    let mut idx = gap;
    while let Some(c) = after[idx..].chars().next() {
        if !(c.is_alphanumeric() || c == '_' || c == '.' || c == '?' || c == '!') {
            break;
        }
        idx += c.len_utf8();
    }
    if idx == gap {
        return None;
    }
    let callee = after[gap..idx].to_owned();
    let tail = after[idx..].trim_start();
    // `do` block opener: `do` word boundary or `do:`.
    let is_do = tail == "do"
        || tail.starts_with("do:")
        || tail
            .strip_prefix("do")
            .is_some_and(|t| t.starts_with(|c: char| !c.is_alphanumeric() && c != '_'));
    if !is_do {
        return None;
    }
    Some(callee.rsplit('.').next().unwrap_or(&callee).to_owned())
}

/// Parse the `exclude` list param (compact JSON array of atom names).
fn excluded_names(params: &BTreeMap<String, String>) -> Vec<String> {
    let Some(raw) = params.get("exclude") else {
        return Vec::new();
    };
    serde_json::from_str::<serde_json::Value>(raw).map_or_else(
        |_| {
            raw.strip_prefix(':')
                .unwrap_or(raw)
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        },
        |v| {
            v.as_array().map_or_else(Vec::new, |items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str())
                    .map(|s| s.strip_prefix(':').unwrap_or(s).to_owned())
                    .collect()
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_pipe_has_no_findings() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x |> foo() |> bar()\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn reports_block_pipe() {
        // `|> case do` pipes into a call with a `do` block.
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("x |> case do\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }

    #[test]
    fn bare_fn_value_is_clean() {
        // Upstream only matches calls with a `do` block, not an `fn` literal.
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x |> fn y -> y end\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn reports_try_block_and_honors_exclude() {
        let src = "defmodule M do\n  def f(v) do\n    v\n    |> try do\n    end\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
        let mut params = BTreeMap::new();
        params.insert("exclude".to_owned(), "[\"case\"]".to_owned());
        let case_src =
            "defmodule M do\n  def f(v) do\n    v\n    |> case do\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(case_src), &params).is_empty());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params).len(),
            1
        );
    }
}
