use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3007`: line length with Credo defaults.
#[allow(clippy::too_many_lines, reason = "single-file kernel scanner")]
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let max_length = helpers::param_usize(params, "max_length", 120);
    let ignore_definitions = helpers::param_bool(params, "ignore_definitions", true);
    let ignore_heredocs = helpers::param_bool(params, "ignore_heredocs", true);
    let ignore_specs = helpers::param_bool(params, "ignore_specs", false);
    let ignore_sigils = helpers::param_bool(params, "ignore_sigils", true);
    let ignore_strings = helpers::param_bool(params, "ignore_strings", true);
    let ignore_urls = helpers::param_bool(params, "ignore_urls", true);

    let masked = prepared.masked();
    let mut findings = Vec::new();
    let mut in_heredoc = false;
    let mut heredoc_delim = String::new();
    // Masked rows collected once: indexing per line keeps this linear
    // instead of re-splitting the whole source on every row.
    let mask_lines: Vec<&str> = masked.split('\n').collect();
    for (idx, line) in source.split('\n').enumerate() {
        let line_no = idx + 1;
        let mask_line = mask_lines.get(idx).copied().unwrap_or("");
        // Track heredoc state from raw line.
        if !in_heredoc {
            if let Some(delim) = find_heredoc_open(line) {
                in_heredoc = true;
                heredoc_delim = delim;
            }
        } else if line.contains(&heredoc_delim) {
            in_heredoc = false;
        }
        let length = line.chars().count();
        if length <= max_length {
            continue;
        }
        if ignore_heredocs && in_heredoc {
            continue;
        }
        if ignore_specs && mask_line.trim_start().starts_with("@spec") {
            continue;
        }
        if ignore_definitions && is_definition(mask_line) {
            continue;
        }
        if ignore_sigils && contains_sigil(mask_line, line) {
            continue;
        }
        if ignore_strings && ignored_by_strings(mask_line, max_length) {
            continue;
        }
        if ignore_urls && contains_url(line) {
            continue;
        }
        let column = max_length + 1;
        let trigger: String = line.chars().skip(max_length).collect();
        findings.push(Finding::with_trigger(
            line_no,
            Some(column),
            format!("Line is too long (max is {max_length}, was {length})."),
            trigger,
        ));
    }
    findings
}

fn find_heredoc_open(line: &str) -> Option<String> {
    for delim in ["\"\"\"", "'''"] {
        if line.contains(delim) {
            // Opening without closing on the same line starts a heredoc.
            let count = line.matches(delim).count();
            if count % 2 == 1 {
                return Some(delim.to_owned());
            }
        }
    }
    None
}

fn is_definition(mask_line: &str) -> bool {
    let trimmed = mask_line.trim_start();
    trimmed.starts_with("def ")
        || trimmed.starts_with("defp ")
        || trimmed.starts_with("defmacro ")
        || trimmed.starts_with("defmacrop ")
        || trimmed.starts_with("defguard ")
        || trimmed.starts_with("defdelegate ")
}

fn contains_sigil(mask_line: &str, raw: &str) -> bool {
    // Masked line keeps `~` and sigil delimiters; raw contains `~<letter>`.
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '~'
            && let Some(&next) = chars.peek()
            && next.is_ascii_alphabetic()
        {
            return true;
        }
    }
    mask_line.contains('~')
}

/// Whether a line is excused by `ignore_strings`, mirroring the native
/// token rule: the line is ignored when the string nearest to end-of-line is
/// the last or second-to-last token and it starts before `max_length`.
/// `mask_line` is the masked view, so comments and sigil contents cannot
/// produce phantom literals.
fn ignored_by_strings(mask_line: &str, max_length: usize) -> bool {
    let chars: Vec<char> = mask_line.chars().collect();
    // Literal spans as `(start, end)` char offsets of the delimiters.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut col = 0_usize;
    while col < chars.len() {
        if (chars[col] == '"' || chars[col] == '\'')
            && chars.get(col + 1) == Some(&chars[col])
            && chars.get(col + 2) == Some(&chars[col])
        {
            col += 3;
            continue;
        }
        if chars[col] == '"' || chars[col] == '\'' {
            let quote = chars[col];
            let start = col;
            col += 1;
            while col < chars.len() && chars[col] != quote {
                col += 1;
            }
            spans.push((start, col.min(chars.len().saturating_sub(1))));
            col += 1;
            continue;
        }
        col += 1;
    }
    let Some((start, end)) = spans.pop() else {
        return false;
    };
    let remainder: String = chars[end + 1..].iter().collect();
    if !remainder.trim().is_empty() && !is_single_token(remainder.trim()) {
        return false;
    }
    start + 1 < max_length
}

/// A single trailing token after a string literal (`do`, `)`, `,`).
fn is_single_token(remainder: &str) -> bool {
    if remainder.is_empty() {
        return true;
    }
    if remainder.chars().count() == 1 {
        return true;
    }
    remainder
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn contains_url(line: &str) -> bool {
    line.contains("http://") || line.contains("https://") || line.contains("www.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn short_lines_are_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = 1\n"), &defaults()).is_empty());
    }

    #[test]
    fn reports_long_line() {
        let long = format!("x = \"{}\"\n", "a".repeat(130));
        let mut params = BTreeMap::new();
        params.insert("ignore_strings".to_owned(), "false".to_owned());
        params.insert("ignore_urls".to_owned(), "false".to_owned());
        let findings = check_prepared(&crate::batch::Prepared::lazy(&long), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(121));
    }

    #[test]
    fn ignores_definitions_by_default() {
        let long_def = format!("def {}(x), do: x\n", "a".repeat(130));
        assert!(check_prepared(&crate::batch::Prepared::lazy(&long_def), &defaults()).is_empty());
    }

    #[test]
    fn reports_concatenated_strings_starting_late() {
        let long = format!("x = {}\n", vec!["\"a\""; 30].join(" <> "));
        assert!(long.chars().count() > 120);
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(&long), &defaults()).len(),
            1
        );
    }

    #[test]
    fn reports_url_comment_when_urls_checked() {
        let long = format!("# see https://example.com/{}\n", "y".repeat(120));
        let mut params = BTreeMap::new();
        params.insert("ignore_urls".to_owned(), "false".to_owned());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(&long), &params).len(),
            1
        );
    }

    #[test]
    fn custom_max_length_is_honored() {
        let src = "x = [1, 2, 3, 4, 5, 6, 7]\n";
        assert!(src.chars().count() > 10);
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &defaults()).is_empty());
        let mut params = BTreeMap::new();
        params.insert("max_length".to_owned(), "10".to_owned());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params).len(),
            1
        );
    }

    #[test]
    fn definitions_report_when_not_ignored() {
        let long_def = format!("def {}(x), do: x\n", "a".repeat(130));
        let mut params = BTreeMap::new();
        params.insert("ignore_definitions".to_owned(), "false".to_owned());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(&long_def), &params).len(),
            1
        );
    }

    #[test]
    fn heredocs_report_when_not_ignored() {
        let long_body = "a".repeat(130);
        let src = format!("x = \"\"\"\n{long_body}\n\"\"\"\n");
        assert!(check_prepared(&crate::batch::Prepared::lazy(&src), &defaults()).is_empty());
        let mut params = BTreeMap::new();
        params.insert("ignore_heredocs".to_owned(), "false".to_owned());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(&src), &params).len(),
            1
        );
    }

    #[test]
    fn specs_ignored_when_configured() {
        let long_spec = format!("@spec {}(integer) :: integer\n", "a".repeat(130));
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(&long_spec), &defaults()).len(),
            1
        );
        let mut params = BTreeMap::new();
        params.insert("ignore_specs".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(&long_spec), &params).is_empty());
    }

    #[test]
    fn sigils_report_when_not_ignored() {
        let long_sigil = format!("x = ~s({})\n", "a".repeat(130));
        assert!(check_prepared(&crate::batch::Prepared::lazy(&long_sigil), &defaults()).is_empty());
        let mut params = BTreeMap::new();
        params.insert("ignore_sigils".to_owned(), "false".to_owned());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(&long_sigil), &params).len(),
            1
        );
    }
}
