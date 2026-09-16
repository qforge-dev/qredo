use crate::Finding;

/// `EX4007`: `!!x` double negation.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let mut ops = negation_ops(line);
        ops.sort();
        let mut consumed = 0_usize;
        while consumed + 1 < ops.len() {
            let first_end = ops[consumed].1;
            let second_start = ops[consumed + 1].0;
            if only_whitespace_between(line, first_end, second_start) {
                let trigger = negation_trigger(&ops[consumed].2, &ops[consumed + 1].2);
                findings.push(Finding::with_trigger(
                    idx + 1,
                    credo_column(line, &trigger),
                    "Double boolean negation found.",
                    trigger,
                ));
                consumed += 2;
            } else {
                consumed += 1;
            }
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

fn negation_trigger(first: &str, second: &str) -> String {
    if first == "!" && second == "!" {
        "!!".to_owned()
    } else {
        format!("{first} {second}")
    }
}

/// `(byte_start, byte_end, text)` of `!`/`not` operators on the line.
fn negation_ops(line: &str) -> Vec<(usize, usize, String)> {
    let mut ops = Vec::new();
    let bytes = line.as_bytes();
    let mut idx = 0_usize;
    while idx < bytes.len() {
        if bytes[idx] == b'!'
            && bytes.get(idx + 1) != Some(&b'=')
            && prev_is_operator_position(bytes, idx)
        {
            ops.push((idx, idx + 1, "!".to_owned()));
            idx += 1;
        } else if line.get(idx..).is_some_and(|rest| rest.starts_with("not"))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + 3)
            && prev_allows(bytes, idx)
            && bytes.get(idx + 3) != Some(&b':')
        {
            ops.push((idx, idx + 3, "not".to_owned()));
            idx += 3;
        } else {
            idx += 1;
        }
    }
    ops
}

/// `!` is an operator (not a `foo!` name suffix).
fn prev_is_operator_position(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    !(prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'?')
}

fn word_boundary(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 || idx >= bytes.len() {
        return true;
    }
    !is_name_byte(bytes[idx]) || !is_name_byte(bytes[idx - 1])
}

fn prev_allows(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    bytes[idx - 1] != b':'
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

fn only_whitespace_between(line: &str, first_end: usize, second_start: usize) -> bool {
    line.get(first_end..second_start)
        .is_some_and(|gap| gap.chars().all(char::is_whitespace))
}

/// Credo `SourceFile.column/3`: 1-based column of `trigger` when surrounded by
/// whitespace, parens, commas or word boundaries; `None` otherwise.
fn credo_column(line: &str, trigger: &str) -> Option<usize> {
    if trigger.is_empty() {
        return None;
    }
    let lchars: Vec<char> = line.chars().collect();
    let tchars: Vec<char> = trigger.chars().collect();
    if lchars.len() < tchars.len() {
        return None;
    }
    for idx in 0..=lchars.len() - tchars.len() {
        if lchars[idx..idx + tchars.len()] != tchars[..] {
            continue;
        }
        let before_ok = if idx == 0 {
            is_word(tchars[0])
        } else {
            before_ok(lchars[idx - 1], tchars[0])
        };
        let after = idx + tchars.len();
        let after_ok = if after == lchars.len() {
            is_word(tchars[tchars.len() - 1])
        } else {
            after_ok(tchars[tchars.len() - 1], lchars[after])
        };
        if before_ok && after_ok {
            return Some(idx + 1);
        }
    }
    None
}

fn before_ok(prev: char, first: char) -> bool {
    prev.is_whitespace()
        || prev == '('
        || prev == ')'
        || prev == ','
        || is_word(prev) != is_word(first)
}

fn after_ok(last: char, next: char) -> bool {
    next.is_whitespace()
        || next == '('
        || next == ')'
        || next == ','
        || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_negation_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("!x\n")).is_empty());
    }
    #[test]
    fn reports_double() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("!!x\n")).len(),
            1
        );
    }
    #[test]
    fn reports_without_column_like_upstream() {
        // EX4007.upstream.violation: `!!` at the line start has no column.
        let findings = check_prepared(&crate::batch::Prepared::lazy("!!true\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, None);
        assert_eq!(findings[0].trigger, crate::Trigger::Text("!!".to_owned()));
    }
    #[test]
    fn reports_not_not_with_column() {
        // EX4007.upstream.violation-2.
        let findings = check_prepared(&crate::batch::Prepared::lazy("not not true\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(1));
        assert_eq!(
            findings[0].trigger,
            crate::Trigger::Text("not not".to_owned())
        );
    }
}
