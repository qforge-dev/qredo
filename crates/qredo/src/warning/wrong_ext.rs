use crate::Finding;

/// `EX5025`: test files should end with `_test.exs` (filename context needed;
/// single-file kernel without filename is conservative).
pub(crate) fn check(_source: &str) -> Vec<Finding> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conservative_empty() {
        assert!(check("x = 1\n").is_empty());
    }
}
