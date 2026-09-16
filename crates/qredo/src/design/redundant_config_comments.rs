use crate::Finding;

/// `EX2006`: redundant `# credo:disable-for-...` comments.
pub(crate) fn check(source: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (idx, line) in source.split('\n').enumerate() {
        if line.contains("# credo:disable-for-this-file") && !line.contains("credo:") {
            continue;
        }
        if line.trim_start().starts_with("# credo:") {
            // Heuristic: comment without a check reference is redundant.
            if !line.contains("Credo.Check") && !line.contains("EX") {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(line.find("# credo:").unwrap_or(0) + 1),
                    "This config comment does not ignore any issue.",
                    "# credo:".to_owned(),
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
    fn code_without_comments_is_clean() {
        assert!(check("x = 1\n").is_empty());
    }
    #[test]
    fn reports_bare_comment() {
        assert_eq!(check("# credo: foo\n").len(), 1);
    }
}
