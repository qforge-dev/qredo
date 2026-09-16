use crate::Finding;
use std::collections::{BTreeMap, HashMap};

/// `EX2002`: intra-file duplicate code (cross-file needs project context).
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    check_prepared_with_filename(prepared, params, "file")
}

/// Intra-file duplicate code naming the checked file, mirroring the
/// upstream `"Duplicate code found in #{filenames} (mass: #{node_mass})."`
/// template (`duplicated_code.ex:244`).
pub(crate) fn check_prepared_with_filename(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
    filename: &str,
) -> Vec<Finding> {
    let mass_threshold: usize = params
        .get("mass_threshold")
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    // Sliding window of 4+ non-empty lines; hash normalized content.
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut findings = Vec::new();
    let window = 4_usize;
    if lines.len() < window {
        return findings;
    }
    for start in 0..=lines.len() - window {
        let chunk: Vec<&str> = lines[start..start + window]
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        if chunk.len() < window {
            continue;
        }
        let key = chunk.join("\n");
        // Mass ~ characters.
        if key.len() < mass_threshold {
            continue;
        }
        if let Some(first) = seen.get(&key) {
            let _ = first;
            findings.push(Finding::no_trigger(
                start + 1,
                format!("Duplicate code found in {filename} (mass: {}).", key.len()),
            ));
            break;
        }
        seen.insert(key, start + 1);
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unique_code_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def a, do: 1\ndef b, do: 2\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_duplicate_block() {
        let block = "  x = 1_000_000_000_000_000_000_000_000\n  y = 2_000_000_000_000_000_000_000_000\n  z = x + y + 1_000_000_000_000_000_000_000\n  w = z * 2_000_000_000_000_000_000_000_000\n";
        let src = format!("def a do\n{block}end\ndef b do\n{block}end\n");
        assert!(!check_prepared(&crate::batch::Prepared::lazy(&src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn mass_threshold_param_suppresses_small_dup() {
        let block = "  x = 1_000_000_000_000_000_000_000_000\n  y = 2_000_000_000_000_000_000_000_000\n  z = x + y + 1_000_000_000_000_000_000_000\n  w = z * 2_000_000_000_000_000_000_000_000\n";
        let src = format!("def a do\n{block}end\ndef b do\n{block}end\n");
        let mut params = BTreeMap::new();
        params.insert("mass_threshold".to_owned(), "100000".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(&src), &params).is_empty());
    }
    #[test]
    fn single_file_message_names_checked_file() {
        let block = "  x = 1_000_000_000_000_000_000_000_000\n  y = 2_000_000_000_000_000_000_000_000\n  z = x + y + 1_000_000_000_000_000_000_000\n  w = z * 2_000_000_000_000_000_000_000_000\n";
        let src = format!("def a do\n{block}end\ndef b do\n{block}end\n");
        let found = check_prepared_with_filename(
            &crate::batch::Prepared::lazy(&src),
            &BTreeMap::new(),
            "lib/a.ex",
        );
        assert_eq!(found.len(), 1);
        assert!(
            found[0]
                .message
                .starts_with("Duplicate code found in lib/a.ex (mass: "),
            "unexpected message: {}",
            found[0].message
        );
    }
}
