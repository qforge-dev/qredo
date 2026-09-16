use crate::Finding;

/// `EX4034`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        // Longest module names first to avoid `Time.add` matching inside `DateTime.add`.
        let mut reported = false;
        for module in ["NaiveDateTime", "DateTime", "Date", "Time"] {
            if reported {
                break;
            }
            let call = format!("{module}.add");
            let mut search = 0_usize;
            while let Some(pos) = line[search..].find(&call) {
                let base = search + pos;
                let before_ok = base == 0
                    || !line[..base]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_');
                if before_ok {
                    findings.push(Finding::with_trigger(
                        idx + 1,
                        Some(base + 1),
                        format!("Prefer `{module}.shift/2` over `{module}.add`."),
                        call.clone(),
                    ));
                    reported = true;
                    break;
                }
                search = base + call.len();
                if search >= line.len() {
                    break;
                }
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shift_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("DateTime.shift(x, day: 1)\n")).is_empty()
        );
    }
    #[test]
    fn reports_add() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("DateTime.add(x, 1, :day)\n")).len(),
            1
        );
    }
}
