use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX1007`: tabs vs spaces consistency.
#[allow(clippy::too_many_lines, reason = "single-file kernel scanner")]
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let force = helpers::param_str(params, "force", "");
    let masked = prepared.masked();
    let mut tab_lines: Vec<usize> = Vec::new();
    let mut space_lines: Vec<usize> = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        if line.starts_with('\t') {
            tab_lines.push(idx + 1);
        } else if line.starts_with("  ")
            || line.starts_with(' ') && line.trim_start().len() != line.len()
        {
            // Indented with spaces.
            if line.starts_with(' ') {
                space_lines.push(idx + 1);
            }
        }
    }
    if force == "spaces" {
        return tab_lines
            .into_iter()
            .map(|line| {
                Finding::with_trigger(
                    line,
                    Some(1),
                    "File is using tabs while most of the files use spaces for indentation.",
                    "\t".to_owned(),
                )
            })
            .collect();
    }
    if force == "tabs" {
        return space_lines
            .into_iter()
            .map(|line| {
                Finding::with_trigger(
                    line,
                    Some(1),
                    "File is using spaces while most of the files use tabs for indentation.",
                    " ".to_owned(),
                )
            })
            .collect();
    }
    // Without force: mixed indentation in one file is reported.
    if !tab_lines.is_empty() && !space_lines.is_empty() {
        // Report minority.
        if tab_lines.len() <= space_lines.len() {
            return tab_lines
                .into_iter()
                .map(|line| {
                    Finding::with_trigger(
                        line,
                        Some(1),
                        "File is using tabs while most of the files use spaces for indentation.",
                        "\t".to_owned(),
                    )
                })
                .collect();
        }
        return space_lines
            .into_iter()
            .map(|line| {
                Finding::with_trigger(
                    line,
                    Some(1),
                    "File is using spaces while most of the files use tabs for indentation.",
                    " ".to_owned(),
                )
            })
            .collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spaces_only_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def f do\n  x\nend\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_mixed() {
        assert!(
            !check_prepared(
                &crate::batch::Prepared::lazy("def f do\n  x\n\tend\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
}
