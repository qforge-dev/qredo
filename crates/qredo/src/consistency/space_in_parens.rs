use crate::Finding;
use std::collections::BTreeMap;

/// `EX1006`: no spaces inside parens (or consistent per file).
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    _params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for w in 0..chars.len() {
            if chars[w] == '('
                && let Some(&next) = chars.get(w + 1)
                && next == ' '
            {
                // Allow empty `()`? `allow_empty_enums` covers `[]`/`%{}`; keep simple.
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(w + 1),
                    "There should be no space after `(`.",
                    "(".to_owned(),
                ));
            }
            if chars[w] == ')' && w > 0 && chars[w - 1] == ' ' {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(w + 1),
                    "There should be no space before `)`.",
                    ")".to_owned(),
                ));
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_call() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("foo(x)\n"), &BTreeMap::new()).is_empty()
        );
    }
    #[test]
    fn reports_space() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("foo( x )\n"),
                &BTreeMap::new()
            )
            .len(),
            2
        );
    }
}
