use crate::{Finding, helpers};

/// `EX1001`: exception modules should share a suffix/prefix within the file.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let modules = helpers::module_names(source);
    let masked = prepared.masked();
    // Find modules using `defexception`.
    let mut exceptions: Vec<(usize, String)> = Vec::new();
    for (line, _col, name) in modules {
        // Look ahead a few lines for `defexception`.
        let lines: Vec<&str> = masked.split('\n').collect();
        let idx = line - 1;
        let window = lines[idx..(idx + 8).min(lines.len())].join("\n");
        if window.contains("defexception") {
            let short = name.rsplit('.').next().unwrap_or(&name).to_owned();
            exceptions.push((line, short));
        }
    }
    if exceptions.len() < 2 {
        return Vec::new();
    }
    // Determine majority suffix: `Error` vs `Exception` vs other.
    let suffix = |name: &str| -> String {
        if name.ends_with("Error") {
            "Error".to_owned()
        } else if name.ends_with("Exception") {
            "Exception".to_owned()
        } else {
            "other".to_owned()
        }
    };
    let mut counts = std::collections::BTreeMap::new();
    for (_, name) in &exceptions {
        *counts.entry(suffix(name)).or_insert(0_usize) += 1;
    }
    let expected = counts.into_iter().max_by_key(|(_, c)| *c).map(|(s, _)| s);
    let Some(expected) = expected else {
        return Vec::new();
    };
    if expected == "other" {
        return Vec::new();
    }
    exceptions
        .into_iter()
        .filter(|(_, name)| suffix(name) != expected)
        .map(|(line, name)| {
            Finding::no_trigger(
                line,
                format!(
                    "Exception modules should be named consistently. It seems your strategy is to have `{expected}` as a suffix, but `{name}` does not follow that convention."
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consistent_suffix_is_clean() {
        let src = "defmodule FooError do\n defexception [:message]\nend\ndefmodule BarError do\n defexception [:message]\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_inconsistent_suffix() {
        let src = "defmodule FooError do\n defexception [:message]\nend\ndefmodule BarException do\n defexception [:message]\nend\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 1);
    }
}
