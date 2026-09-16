use crate::Finding;

/// `EX3036`: `@impl true` should be `@impl Module`.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let mut search = 0_usize;
        while let Some(pos) = line[search..].find("@impl") {
            let base = search + pos;
            let rest = line[base + "@impl".len()..].trim_start();
            if let Some(after_true) = rest.strip_prefix("true") {
                let after = after_true.chars().next();
                if after.is_none_or(|c| !c.is_alphanumeric() && c != '_') {
                    findings.push(Finding::with_trigger(
                        idx + 1,
                        Some(base + 1),
                        "`@impl true` should be `@impl MyBehaviour`.",
                        "@impl",
                    ));
                }
            }
            search = base + 5;
            if search >= line.len() {
                break;
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_impl_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "@impl MyBehaviour\ndef f, do: 1\n"
            ))
            .is_empty()
        );
    }

    #[test]
    fn reports_impl_true() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("@impl true\ndef f, do: 1\n")).len(),
            1
        );
    }
}
