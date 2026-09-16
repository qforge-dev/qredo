use crate::Finding;

/// `EX1003`: multi-alias vs single-alias consistency within the file.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut multi = 0_usize;
    let mut single = 0_usize;
    for line in masked.split('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("alias ")
            || trimmed.starts_with("import ")
            || trimmed.starts_with("require ")
            || trimmed.starts_with("use ")
        {
            if trimmed.contains('{') {
                multi += 1;
            } else {
                single += 1;
            }
        }
    }
    if multi == 0 || single == 0 {
        return Vec::new();
    }
    // Report minority style lines.
    let mut findings = Vec::new();
    let expect_multi = multi >= single;
    for (idx, line) in masked.split('\n').enumerate() {
        let trimmed = line.trim_start();
        let is_directive = trimmed.starts_with("alias ")
            || trimmed.starts_with("import ")
            || trimmed.starts_with("require ")
            || trimmed.starts_with("use ");
        if !is_directive {
            continue;
        }
        let is_multi = trimmed.contains('{');
        if expect_multi != is_multi {
            findings.push(Finding::no_trigger(
                idx + 1,
                "Use multi-alias syntax consistently.".to_owned(),
            ));
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consistent_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "alias Foo.Bar\nalias Foo.Baz\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_mixed_styles() {
        assert!(
            !check_prepared(&crate::batch::Prepared::lazy(
                "alias Foo.{Bar, Baz}\nalias Foo.Qux\n"
            ))
            .is_empty()
        );
    }
}
