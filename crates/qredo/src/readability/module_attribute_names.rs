use crate::Finding;

/// `EX3008`: module attribute names must be `snake_case`.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut pos = 0_usize;
        while pos < chars.len() {
            if chars[pos] != '@' {
                pos += 1;
                continue;
            }
            let mut end = pos + 1;
            while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            if end > pos + 1 {
                let name: String = chars[pos + 1..end].iter().collect();
                if !is_snake_case(&name) {
                    findings.push(Finding::with_trigger(
                        idx + 1,
                        Some(pos + 1),
                        "Module attribute names should be written in snake_case.",
                        format!("@{name}"),
                    ));
                }
            }
            pos = end.max(pos + 1);
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Credo `Name.snake_case?/1`.
fn is_snake_case(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '?' | '!'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_attr_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("@my_attr 1\n")).is_empty());
    }

    #[test]
    fn reports_camel_case_attr() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("@myAttr 1\n")).len(),
            1
        );
    }

    #[test]
    fn column_counts_chars_before_multibyte() {
        let findings = check_prepared(&crate::batch::Prepared::lazy("λ = @myAttr\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(5));
    }
}
