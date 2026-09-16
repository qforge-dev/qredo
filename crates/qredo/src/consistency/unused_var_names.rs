use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX1008`: unused variable names should be consistently `_`-prefixed or not.
pub(crate) fn check(source: &str, params: &BTreeMap<String, String>) -> Vec<Finding> {
    let force = helpers::param_str(params, "force", "");
    let mut underscore = 0_usize;
    let mut plain = 0_usize;
    let mut locations: Vec<(usize, usize, String, bool)> = Vec::new();
    for (line, col, name) in helpers::variable_tokens(source) {
        // Without dataflow, only `_`-prefixed names are tracked; bare names
        // cannot be proven unused in a single-file kernel.
        if name.starts_with('_') {
            underscore += 1;
            locations.push((line, col, name, true));
        } else {
            plain += 1;
        }
    }
    if force == "meaningful" {
        // `_`-prefixed should be plain.
        return locations
            .into_iter()
            .filter(|(_, _, n, _)| n.starts_with('_'))
            .map(|(line, col, name, _)| {
                Finding::with_trigger(
                    line,
                    Some(col),
                    "Unused variables should use meaningful names.",
                    name,
                )
            })
            .collect();
    }
    if force == "anonymous" {
        // Unused should be `_`-prefixed; without dataflow we cannot know which
        // are unused, so report nothing (conservative).
        return Vec::new();
    }
    let _ = plain;
    // Without force and without dataflow, no findings (conservative).
    let _ = underscore;
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_without_force() {
        assert!(check("def f(_x), do: 1\n", &BTreeMap::new()).is_empty());
    }
    #[test]
    fn force_meaningful_reports_underscore() {
        let mut p = BTreeMap::new();
        p.insert("force".to_owned(), "meaningful".to_owned());
        assert!(!check("def f(_x), do: 1\n", &p).is_empty());
    }
}
