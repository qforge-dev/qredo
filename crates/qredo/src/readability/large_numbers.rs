use crate::Finding;
use std::collections::BTreeMap;

/// A decimal number literal with its source fragment and value.
struct NumberToken {
    line: usize,
    column: usize,
    source: String,
    int_digits: String,
    frac: Option<String>,
    value: f64,
}

/// `EX3006`: large numbers should use underscores.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let threshold: f64 = params
        .get("only_greater_than")
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(9999.0);
    let trailing = parse_trailing(params);
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for token in number_tokens(masked) {
        if token.value <= threshold {
            continue;
        }
        let expected = expected_renderings(&token, &trailing);
        if !expected.contains(&token.source) {
            findings.push(Finding::with_trigger(
                token.line,
                Some(token.column),
                format!(
                    "Numbers larger than {} should be written with underscores: {}",
                    format_threshold(params),
                    expected.join(" or ")
                ),
                token.source,
            ));
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

fn format_threshold(params: &BTreeMap<String, String>) -> String {
    match params.get("only_greater_than") {
        Some(raw) => raw.clone(),
        None => "9999".to_owned(),
    }
}

/// `trailing_digits` arrives as compact JSON: number, list or `{"range":[a,b]}`.
fn parse_trailing(params: &BTreeMap<String, String>) -> Vec<usize> {
    let Some(raw) = params.get("trailing_digits") else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    match &value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .map(|n| vec![usize::try_from(n).unwrap_or(usize::MAX)])
            .unwrap_or_default(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.as_u64()
                    .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
            })
            .collect(),
        serde_json::Value::Object(map) => match map.get("range") {
            Some(serde_json::Value::Array(bounds)) if bounds.len() == 2 => {
                let start = bounds[0]
                    .as_u64()
                    .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
                let end = bounds[1]
                    .as_u64()
                    .map_or(0, |n| usize::try_from(n).unwrap_or(0));
                (start..=end).collect()
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Correctly grouped renderings of the token (standard plus trailing-digit
/// variants), with the fractional part kept verbatim.
fn expected_renderings(token: &NumberToken, trailing: &[usize]) -> Vec<String> {
    let mut bases = vec![group(&token.int_digits)];
    for digits in trailing {
        if *digits > 0 && token.int_digits.len() > *digits {
            let split = token.int_digits.len() - digits;
            let head = group(&token.int_digits[..split]);
            let tail = &token.int_digits[split..];
            let variant = format!("{head}_{tail}");
            if !bases.contains(&variant) {
                bases.push(variant);
            }
        }
    }
    match &token.frac {
        Some(frac) => bases.iter().map(|base| format!("{base}.{frac}")).collect(),
        None => bases,
    }
}

/// Group decimal digits in threes from the right (`1000000` -> `1_000_000`).
fn group(digits: &str) -> String {
    let chars: Vec<char> = digits.chars().collect();
    let mut out = String::new();
    for (from_right, c) in chars.iter().rev().enumerate() {
        if from_right > 0 && from_right % 3 == 0 && from_right != chars.len() {
            out.push('_');
        }
        out.push(*c);
    }
    out.chars().rev().collect()
}

/// Decimal integer/float literals; skips hex/octal/binary and names.
fn number_tokens(masked: &str) -> Vec<NumberToken> {
    let mut out = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        line_numbers(line, idx + 1, &mut out);
    }
    out
}

/// Number literals on one masked line.
fn line_numbers(line: &str, line_no: usize, out: &mut Vec<NumberToken>) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            i += 1;
            continue;
        }
        if chars[i] == '0' && matches!(chars.get(i + 1), Some('x' | 'X' | 'o' | 'O' | 'b' | 'B')) {
            i = skip_word(&chars, i + 2);
            continue;
        }
        if let Some((token, next)) = read_number(&chars, i, line_no) {
            out.push(token);
            i = next;
        } else {
            i += 1;
        }
    }
}

/// Read the number starting at digit `start`; returns the token and end.
fn read_number(chars: &[char], start: usize, line_no: usize) -> Option<(NumberToken, usize)> {
    let mut i = start;
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
        i += 1;
    }
    let mut int_end = i;
    let mut frac: Option<String> = None;
    if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
        let frac_start = i + 1;
        let mut j = frac_start;
        while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '_') {
            j += 1;
        }
        frac = Some(chars[frac_start..j].iter().collect());
        int_end = j;
    }
    let source: String = chars[start..int_end].iter().collect();
    let int_digits: String = chars[start..i]
        .iter()
        .filter(|c| c.is_ascii_digit())
        .collect();
    if int_digits.is_empty() {
        return None;
    }
    let value = token_value(&int_digits, frac.as_deref());
    Some((
        NumberToken {
            line: line_no,
            column: start + 1,
            source,
            int_digits,
            frac,
            value,
        },
        int_end,
    ))
}

/// Numeric value for the threshold comparison.
fn token_value(int_digits: &str, frac: Option<&str>) -> f64 {
    match frac {
        Some(frac_digits) => {
            let clean_frac: String = frac_digits.chars().filter(char::is_ascii_digit).collect();
            format!("{int_digits}.{clean_frac}").parse().unwrap_or(0.0)
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "thresholds sit far below float precision limits; the conversion cannot flip the comparison"
        )]
        None => int_digits
            .parse::<i128>()
            .map(|n| n as f64)
            .unwrap_or(f64::MAX),
    }
}

fn skip_word(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_numbers_are_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x = 9999\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn reports_large_number() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("x = 1000000\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }

    #[test]
    fn message_shows_underscored_expectation() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("x = 1000000\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Numbers larger than 9999 should be written with underscores: 1_000_000"
        );
        assert_eq!(findings[0].column, Some(5));
    }

    #[test]
    fn underscored_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x = 1_000_000\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn hex_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x = 0x123456\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn only_greater_than_raises_threshold() {
        let src = "x = 1000000\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
        let mut params = BTreeMap::new();
        params.insert("only_greater_than".to_owned(), "10000000".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn trailing_digits_allows_variant_grouping() {
        let src = "x = 10_000_00\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
        let mut params = BTreeMap::new();
        params.insert("trailing_digits".to_owned(), "[2]".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
}
