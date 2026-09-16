use crate::Finding;

/// `EX3035`: at most one `|>` per line.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        if line.matches("|>").count() > 1 {
            let col = line.find("|>").unwrap_or(0) + 1;
            findings.push(Finding::with_trigger(
                idx + 1,
                Some(col),
                "Avoid using multiple pipes (`|>`) on the same line.",
                "|>",
            ));
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_pipe_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x |> foo()\n")).is_empty());
    }
    #[test]
    fn reports_two_pipes() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("x |> foo() |> bar()\n")).len(),
            1
        );
    }
}
