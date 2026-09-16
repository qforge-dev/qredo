use crate::Finding;

/// `EX3024`: exactly one space after each comma (outside strings/comments).
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, (mask_line, raw_line)) in masked.split('\n').zip(source.split('\n')).enumerate() {
        // Mask and raw share char offsets: masking replaces whole
        // characters with spaces, so comma positions line up.
        let mask_chars: Vec<char> = mask_line.chars().collect();
        let raw_chars: Vec<char> = raw_line.chars().collect();
        let mut col = 0_usize;
        while col < mask_chars.len() {
            if mask_chars[col] == ',' {
                // Trailing comma at EOL and `, ` are fine; Credo only reports
                // a missing space before the next token. The trigger shows the
                // raw next character (e.g. `,?` before a char literal).
                if let Some(next) = raw_chars.get(col + 1)
                    && *next != ' '
                    && *next != '\t'
                {
                    let trigger: String = [',', raw_chars[col + 1]].iter().collect();
                    findings.push(Finding::with_trigger(
                        idx + 1,
                        Some(col + 1),
                        "Space missing after comma.",
                        trigger,
                    ));
                }
            }
            col += 1;
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_list_has_no_findings() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = [1, 2, 3]\n")).is_empty());
    }

    #[test]
    fn reports_missing_space() {
        let findings = check_prepared(&crate::batch::Prepared::lazy("x = [1,2]\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(7));
    }

    #[test]
    fn ignores_comma_in_string() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = \",\"\n")).is_empty());
    }

    #[test]
    fn ignores_comma_in_sigil() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "Regex.match?(~r/^\\d,\\d$/, value)\n"
            ))
            .is_empty()
        );
    }

    #[test]
    fn reports_comma_before_char_literal() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "  @some_char_codes [?,,?;]\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(23));
    }

    #[test]
    fn reports_comma_after_predicate_name() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "  @attribute [question?,answer]\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(24));
    }
}
