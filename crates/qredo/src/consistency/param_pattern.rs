use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX1004`: parameter pattern matching position consistency.
#[allow(clippy::too_many_lines, reason = "single-file kernel scanner")]
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let force = helpers::param_str(params, "force", "");
    let masked = prepared.masked();
    let mut before = 0_usize;
    let mut after = 0_usize;
    for line in masked.split('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("def ") || trimmed.starts_with("defp ") {
            // Heuristic: `%{...} = var` (before) vs `var = %{...}` (after) in params.
            if let Some(paren) = line.find('(')
                && let Some(close) = line.find(')')
            {
                let inside = &line[paren..close];
                if inside.contains('=') {
                    let eq = inside.find('=').unwrap_or(0);
                    let left = &inside[..eq];
                    if left.contains("%{") || left.contains('{') {
                        before += 1;
                    } else {
                        after += 1;
                    }
                }
            }
        }
    }
    if before == 0 && after == 0 {
        return Vec::new();
    }
    let expected = if force == "before" {
        "before"
    } else if force == "after" {
        "after"
    } else if before >= after {
        "before"
    } else {
        "after"
    };
    // Single-file kernel: without force and with consistent style, no issue.
    if force.is_empty() && (before == 0 || after == 0) {
        return Vec::new();
    }
    // Report lines not matching expected (only when forced or mixed).
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("def ") || trimmed.starts_with("defp ")) {
            continue;
        }
        if let Some(paren) = line.find('(')
            && let Some(close) = line.find(')')
        {
            let inside = &line[paren..close];
            if inside.contains('=') {
                let eq = inside.find('=').unwrap_or(0);
                let left = &inside[..eq];
                let is_before = left.contains("%{") || left.contains('{');
                let matches = (expected == "before") == is_before;
                if !matches {
                    findings.push(Finding::no_trigger(
                        idx + 1,
                        format!("Use parameter pattern matching `{expected}` consistently."),
                    ));
                }
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consistent_is_clean() {
        let src = "def foo(%{a: a} = m), do: a\ndef bar(%{b: b} = m), do: b\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn force_after_reports_before() {
        let mut p = BTreeMap::new();
        p.insert("force".to_owned(), "after".to_owned());
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo(%{a: a} = m), do: a\n"),
                &p
            )
            .len(),
            1
        );
    }
}
