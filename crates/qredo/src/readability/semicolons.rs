use crate::Finding;

/// `EX3020`: no `;` separating statements.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch == ';' {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(col + 1),
                    "Don't use `;` to separate statements and expressions.",
                    ";",
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
    fn clean_code_has_no_findings() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("def f do\n  a\n  b\nend\n")).is_empty()
        );
    }

    #[test]
    fn reports_semicolon() {
        let findings = check_prepared(&crate::batch::Prepared::lazy("def f do\n  a; b\nend\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
    }

    #[test]
    fn ignores_semicolon_in_string() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = \";\"\n")).is_empty());
    }
}
