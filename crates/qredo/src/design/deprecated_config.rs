use crate::Finding;

/// `EX2008`: deprecated check references in config.
pub(crate) fn check(source: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    // Known old names (subset): `Credo.Check.Design.DuplicatedCode` was renamed, etc.
    // Heuristic: `Credo.Check.Readability.ParenthesesInCondition` old?
    // For single-file kernel, look for `Credo.Check.` entries that no longer exist
    // is out of scope; instead detect explicit deprecated markers.
    for (idx, line) in source.split('\n').enumerate() {
        if line.contains("Credo.Check.Consistency.MultiAliasImportRequireUse")
            && line.contains("disabled")
        {
            findings.push(Finding::no_trigger(
                idx + 1,
                "Deprecated check configuration found.".to_owned(),
            ));
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_config() {
        assert!(check("x = 1\n").is_empty());
    }
    #[test]
    fn empty_is_conservative() {
        // Without project config context, kernel is conservative.
        assert!(check("[]\n").is_empty());
    }
}
