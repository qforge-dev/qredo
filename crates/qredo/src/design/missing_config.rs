use crate::Finding;

/// `EX2007`: missing checks in config (needs project config; conservative).
pub(crate) fn check(source: &str) -> Vec<Finding> {
    // Single-file kernel without full config inventory cannot decide;
    // report nothing (pipeline remains unsupported for project aggregation).
    let _ = source;
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conservative_empty() {
        assert!(check("[]\n").is_empty());
    }
}
