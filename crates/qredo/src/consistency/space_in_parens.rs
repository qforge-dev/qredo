use crate::Finding;
use std::collections::BTreeMap;

/// `EX1006`: no spaces inside parens (or consistent per file).
///
/// `allow_empty_enums: true` exempts empty `()` pairs (upstream also
/// covers `[]`/`%{}`, which this paren-only kernel never scans).
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let allow_empty = crate::helpers::param_bool(params, "allow_empty_enums", false);
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for w in 0..chars.len() {
            if chars[w] == '('
                && let Some(&next) = chars.get(w + 1)
                && next == ' '
                && !(allow_empty && chars.get(w + 2) == Some(&')'))
            {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(w + 1),
                    "There should be no space after `(`.",
                    "(".to_owned(),
                ));
            }
            if chars[w] == ')'
                && w > 0
                && chars[w - 1] == ' '
                && !(allow_empty && w >= 2 && chars[w - 2] == '(')
            {
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
    #[test]
    fn allow_empty_enums_exempts_empty_parens() {
        let mut params = BTreeMap::new();
        params.insert("allow_empty_enums".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy("foo( )\n"), &params).is_empty());
        // Non-empty calls still report.
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("foo( x )\n"), &params).len(),
            2
        );
    }
    #[test]
    fn empty_parens_report_by_default() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("foo( )\n"), &BTreeMap::new()).len(),
            2
        );
    }
}
