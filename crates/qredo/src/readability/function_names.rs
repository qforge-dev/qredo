use crate::{Finding, helpers};
use std::collections::BTreeMap;

const DEF_OPS: [&str; 6] = [
    "def ",
    "defp ",
    "defmacro ",
    "defmacrop ",
    "defguard ",
    "defguardp ",
];

/// `EX3004`: function/macro/guard names must be `snake_case`.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let allow_acronyms = helpers::param_bool(params, "allow_acronyms", false);
    // One issue per `name/arity` signature; later clauses overwrite earlier ones.
    let mut by_signature: BTreeMap<String, Finding> = BTreeMap::new();
    for (line, col, op, name, arity) in def_entries(prepared.masked()) {
        if name == "unquote" || is_operator_name(&name) {
            continue;
        }
        if is_sigil_exception(&op, &name) {
            continue;
        }
        if is_snake_case(&name, allow_acronyms) {
            continue;
        }
        let key = format!("{name}/{arity}");
        by_signature.insert(
            key,
            Finding::with_trigger(
                line,
                Some(col),
                "Function/macro/guard names should be written in snake_case.",
                name,
            ),
        );
    }
    let mut findings: Vec<Finding> = by_signature.into_values().collect();
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Credo `Name.snake_case?/1`, plus the `allow_acronyms` segment rule.
fn is_snake_case(name: &str, allow_acronyms: bool) -> bool {
    if is_plain_snake(name) {
        return true;
    }
    allow_acronyms && is_acronym_snake(name)
}

fn is_plain_snake(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_lowercase() || c.is_ascii_digit() || c == '_' || c == '?' || c == '!')
}

/// Each `_`-separated segment must be all-lower/digit or all-upper/digit,
/// with one optional trailing `?`/`!`.
fn is_acronym_snake(name: &str) -> bool {
    !name.is_empty()
        && name.split('_').all(|segment| {
            let segment = segment.trim_end_matches(['?', '!']);
            !segment.is_empty()
                && (segment
                    .chars()
                    .all(|c| c.is_lowercase() || c.is_ascii_digit())
                    || segment
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
        })
}

/// Single-letter `sigil_X` definitions (except guards) and multi-letter
/// all-uppercase `sigil_XX` definitions are exempt.
fn is_sigil_exception(op: &str, name: &str) -> bool {
    let Some(letters) = name.strip_prefix("sigil_") else {
        return false;
    };
    if !letters.is_empty() && letters.chars().all(|c| c.is_ascii_uppercase()) {
        return true;
    }
    let mut chars = letters.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_alphabetic())
        && matches!(op, "def" | "defp" | "defmacro" | "defmacrop")
}

/// Operator definitions (`++`, `&&`, `@`, ...) carry no `snake_case` name.
fn is_operator_name(name: &str) -> bool {
    !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!' || c == '.')
}

/// `(line, column, op, name, arity)` for each `def`-family definition.
fn def_entries(masked: &str) -> Vec<(usize, usize, String, String, usize)> {
    // Operator tables built once: the per-position matcher below must not
    // allocate (it runs on every character of every line).
    let op_table: Vec<(Vec<char>, &str)> = DEF_OPS
        .iter()
        .map(|candidate| (candidate.chars().collect(), candidate.trim_end()))
        .collect();
    let mut out = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        // Every operator starts with "def": lines without it cannot match.
        if !line.contains("def") {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut pos = 0_usize;
        while pos < chars.len() {
            let Some((op, op_end)) = match_def_op(&chars, pos, &op_table) else {
                pos += 1;
                continue;
            };
            let mut name_start = op_end;
            while name_start < chars.len() && chars[name_start] == ' ' {
                name_start += 1;
            }
            let mut name_end = name_start;
            while name_end < chars.len()
                && (chars[name_end].is_alphanumeric() || matches!(chars[name_end], '_' | '?' | '!'))
            {
                name_end += 1;
            }
            if name_end == name_start {
                pos = op_end;
                continue;
            }
            let name: String = chars[name_start..name_end].iter().collect();
            let arity = def_arity(&chars, name_end);
            out.push((idx + 1, name_start + 1, op, name, arity));
            pos = name_end.max(op_end);
        }
    }
    out
}

/// Match a whole-word `def`-family operator at `pos`; returns `(op, end)`.
fn match_def_op(
    chars: &[char],
    pos: usize,
    table: &[(Vec<char>, &str)],
) -> Option<(String, usize)> {
    for (op_chars, op) in table {
        if chars[pos..].starts_with(op_chars)
            && (pos == 0 || !(chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_'))
        {
            return Some(((*op).to_owned(), pos + op_chars.len()));
        }
    }
    None
}

/// Arity from the parenthesised argument list, or zero without parens.
fn def_arity(chars: &[char], mut pos: usize) -> usize {
    while pos < chars.len() && chars[pos] == ' ' {
        pos += 1;
    }
    if pos >= chars.len() || chars[pos] != '(' {
        return 0;
    }
    let mut depth = 0_usize;
    let mut commas = 0_usize;
    let mut nonempty = false;
    let mut i = pos;
    while i < chars.len() {
        match chars[i] {
            '(' => {
                depth += 1;
                if depth > 1 {
                    nonempty = true;
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => commas += 1,
            c if depth == 1 && !c.is_whitespace() => nonempty = true,
            _ => {}
        }
        i += 1;
    }
    if nonempty { commas + 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def handle_message(x), do: x\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn reports_camel_case() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("def handleMessage(x), do: x\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn multi_letter_sigil_def_is_clean() {
        let src = "def sigil_ZZO(input, args) do\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn acronym_with_digit_is_allowed() {
        let src = "def clean_HTTP2_url(0), do: :ok\n";
        let mut params = BTreeMap::new();
        params.insert("allow_acronyms".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn repeated_clauses_report_once_at_last_line() {
        let src = "def credoSampleFunction(0), do: :ok\ndef credoSampleFunction(1), do: :ok\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
    }
    #[test]
    fn def_inside_identifiers_is_ignored() {
        // `define`, `abcdef` and `my_defx` contain "def" but are not definitions.
        let src = "define = 1\nabcdef = 2\nmy_defx = 3\ndef ok_name, do: 1\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
}
