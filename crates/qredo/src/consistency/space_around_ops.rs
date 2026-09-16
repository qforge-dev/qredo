use crate::Finding;
use std::collections::BTreeMap;

/// `EX1005`: spaces around operators should be consistent.
#[allow(clippy::too_many_lines, reason = "single-file kernel scanner")]
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    _params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut with_space = 0_usize;
    let mut without_space = 0_usize;
    let mut locations: Vec<(usize, usize, String)> = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut c = 0_usize;
        while c < chars.len() {
            // Multi-char ops: `==`, `!=`, `<=`, `>=`, `|>`, `->`, `=>`, `++`, etc.
            let two: String = chars[c..(c + 2).min(chars.len())].iter().collect();
            let op_len = if [
                "==", "!=", "<=", ">=", "|>", "->", "=>", "++", "--", "=~", "<>",
            ]
            .contains(&two.as_str())
            {
                2
            } else if "=<>+-*/|".contains(chars[c]) && chars[c] != ' ' {
                // Single-char; skip `=` in `==`, `=>`, etc. handled above.
                // Skip `|` in `|>` handled above; skip `:` `,` etc.
                usize::from(
                    chars[c] == '='
                        || chars[c] == '+'
                        || chars[c] == '-'
                        || chars[c] == '*'
                        || chars[c] == '/',
                )
            } else {
                0
            };
            if op_len > 0 {
                let before = if c > 0 { Some(chars[c - 1]) } else { None };
                let after = chars.get(c + op_len).copied();
                let spaced = before == Some(' ') && after == Some(' ');
                if spaced {
                    with_space += 1;
                } else {
                    without_space += 1;
                    locations.push((idx + 1, c + 1, chars[c..c + op_len].iter().collect()));
                }
            }
            c += op_len.max(1);
        }
    }
    if with_space == 0 || without_space == 0 {
        return Vec::new();
    }
    // Report minority (no-space when most use spaces).
    if with_space >= without_space {
        locations
            .into_iter()
            .map(|(line, col, op)| {
                Finding::with_trigger(
                    line,
                    Some(col),
                    "Use spaces around operators consistently.",
                    op,
                )
            })
            .collect()
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consistent_spaces_are_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x = 1 + 2\ny = 3 + 4\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_missing_spaces() {
        assert!(
            !check_prepared(
                &crate::batch::Prepared::lazy("x = 1 + 2\ny=3+4\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
}
