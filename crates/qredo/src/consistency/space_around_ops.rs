use crate::Finding;
use std::collections::BTreeMap;

/// `EX1005`: spaces around operators should be consistent.
///
/// The `ignore` param lists skipped operators (default `["|"]`, matching
/// upstream `ignore: [:|]`); values arrive as compact JSON atom lists.
#[allow(clippy::too_many_lines, reason = "single-file kernel scanner")]
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let ignored = ignored_operators(params);
    let masked = prepared.masked();
    let mut with_space = 0_usize;
    let mut without_space = 0_usize;
    let mut locations: Vec<(usize, usize, String)> = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut c = 0_usize;
        while c < chars.len() {
            // Multi-char ops: `==`, `!=`, `<=`, `>=`, `|>`, `->`, `=>`, `++`, etc.
            // `**` is a power-op token, never an operator occurrence.
            let two: String = chars[c..(c + 2).min(chars.len())].iter().collect();
            if two == "**" {
                c += 2;
                continue;
            }
            let op_len = if [
                "==", "!=", "<=", ">=", "|>", "->", "=>", "++", "--", "=~", "<>",
            ]
            .contains(&two.as_str())
            {
                2
            } else if "=<>+-*/|".contains(chars[c]) && chars[c] != ' ' {
                // Single-char; skip `=` in `==`, `=>`, etc. handled above.
                // Skip `:` `,` etc.
                usize::from(
                    chars[c] == '='
                        || chars[c] == '+'
                        || chars[c] == '-'
                        || chars[c] == '*'
                        || chars[c] == '/'
                        || chars[c] == '|',
                )
            } else {
                0
            };
            if op_len > 0 {
                let op: String = chars[c..c + op_len].iter().collect();
                if ignored.iter().any(|item| item == &op) {
                    c += op_len;
                    continue;
                }
                // Operator atoms (`:*`) and remote call names (`Kernel.||`)
                // are not operators (native `{:atom, …}` /
                // `{:paren_identifier, …}`); `..` ranges keep their sign.
                if c > 0 && (chars[c - 1] == ':' || chars[c - 1] == '.') {
                    let range_dot = chars[c - 1] == '.' && c > 1 && chars[c - 2] == '.';
                    if !range_dot {
                        c += op_len;
                        continue;
                    }
                }
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

/// Operators skipped by the `ignore` param (default `["|"]`, matching
/// upstream `ignore: [:|]`); values arrive as compact JSON atom lists.
fn ignored_operators(params: &BTreeMap<String, String>) -> Vec<String> {
    let Some(raw) = params.get("ignore") else {
        return vec!["|".to_owned()];
    };
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .map(|item| item.strip_prefix(':').unwrap_or(&item).to_owned())
        .collect()
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
    #[test]
    fn pipe_is_ignored_by_default() {
        // Upstream default `ignore: [:|]`.
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x = foo(a|b)\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn empty_ignore_restores_pipe_reports() {
        let mut params = BTreeMap::new();
        params.insert("ignore".to_owned(), "[]".to_owned());
        let findings = check_prepared(&crate::batch::Prepared::lazy("x = foo(a|b)\n"), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].trigger, crate::Trigger::Text("|".to_owned()));
    }
    #[test]
    fn operator_atoms_and_remote_calls_are_not_operators() {
        // Native lexes `:*` as `{:atom, …}` and `Kernel.||` as
        // `{:paren_identifier, …}` — neither is an operator occurrence.
        // Each source mixes spaced operators (majority) with the atom/call
        // form, which must not vote or report.
        for source in [
            "x = 1 + 2\nmatch :*, \"/notes\"\n",
            "x = a |> b\nquery = uri.query |> Kernel.||(\"\")\n",
            "x = 1 + 2\n@conditions [:==, :!=]\n",
        ] {
            assert!(
                check_prepared(&crate::batch::Prepared::lazy(source), &BTreeMap::new()).is_empty(),
                "findings for {source:?}"
            );
        }
    }

    #[test]
    fn power_operator_is_not_an_operator() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("x = 1 + 2\ny = 3 * 2 ** (n - 1)\n"),
            &BTreeMap::new(),
        );
        assert!(
            findings.iter().all(|finding| !matches!(
                &finding.trigger,
                crate::Trigger::Text(text) if text == "*"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn custom_ignore_suppresses_listed_operators() {
        let mut params = BTreeMap::new();
        params.insert("ignore".to_owned(), "[\"+\"]".to_owned());
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x = 1+2\ny = 3 + 4\n"),
                &params
            )
            .is_empty()
        );
    }
}
