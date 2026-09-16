use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX1002`: consistent line endings; single-file kernel checks internal
/// mixing and `force` (`unix`/`windows`).
pub(crate) fn check(source: &str, params: &BTreeMap<String, String>) -> Vec<Finding> {
    let force = helpers::param_str(params, "force", "");
    let has_crlf = source.contains("\r\n");
    let has_lf = source.replace("\r\n", "").contains('\n');
    let has_bare_cr = source.replace("\r\n", "").contains('\r');
    if force == "unix" {
        if has_crlf {
            let line = source
                .split('\n')
                .position(|l| l.ends_with('\r'))
                .map_or(1, |i| i + 1);
            return vec![Finding::with_trigger(
                line,
                None,
                "File is using windows line endings while most of the files use unix line endings.",
                "\r\n".to_owned(),
            )];
        }
        return Vec::new();
    }
    if force == "windows" {
        if has_lf {
            return vec![Finding::with_trigger(
                1,
                None,
                "File is using unix line endings while most of the files use windows line endings.",
                "\n".to_owned(),
            )];
        }
        return Vec::new();
    }
    // Without force: report only mixed endings within the file.
    if (has_crlf && has_lf) || has_bare_cr {
        let line = source
            .split('\n')
            .position(|l| l.ends_with('\r'))
            .map_or(1, |i| i + 1);
        return vec![Finding::with_trigger(
            line,
            None,
            "File is using windows line endings while most of the files use unix line endings.",
            "\r\n".to_owned(),
        )];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unix_only_is_clean() {
        assert!(check("a\nb\n", &BTreeMap::new()).is_empty());
    }
    #[test]
    fn reports_mixed() {
        assert_eq!(check("a\r\nb\n", &BTreeMap::new()).len(), 1);
    }
    #[test]
    fn force_unix_reports_crlf() {
        let mut p = BTreeMap::new();
        p.insert("force".to_owned(), "unix".to_owned());
        assert_eq!(check("a\r\n", &p).len(), 1);
    }
    #[test]
    fn force_windows_reports_lf() {
        let mut p = BTreeMap::new();
        p.insert("force".to_owned(), "windows".to_owned());
        assert_eq!(check("a\n", &p).len(), 1);
        assert!(check("a\r\n", &p).is_empty());
    }
}
