use crate::{Finding, helpers};

/// `EX3018`: prefer unquoted atoms over quoted ones where possible.
pub(crate) fn check(source: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    // Scan raw source for `:"foo"` / `:'foo'` that could be unquoted.
    let bytes = source.as_bytes();
    let mut line_no = 1_usize;
    let mut col_no = 1_usize;
    let mut i = 0_usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line_no += 1;
            col_no = 1;
            i += 1;
            continue;
        }
        if b == b':' && i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'\'') {
            let quote = bytes[i + 1];
            let start_line = line_no;
            let start_col = col_no;
            let mut j = i + 2;
            let mut inner = String::new();
            while j < bytes.len() && bytes[j] != quote && bytes[j] != b'\n' {
                inner.push(bytes[j] as char);
                j += 1;
            }
            if j < bytes.len() && bytes[j] == quote && is_simple_atom(&inner) {
                let trigger = format!(":{}{inner}{}", quote as char, quote as char);
                findings.push(Finding::with_trigger(
                    start_line,
                    Some(start_col),
                    format!("Use unquoted atom `:{inner}` rather than quoted atom `{trigger}`."),
                    trigger,
                ));
            }
            // Advance.
            let consumed = j.saturating_sub(i) + 1;
            i = j + 1;
            col_no += consumed;
            continue;
        }
        // Track columns by chars.
        let ch_len = source[i..].chars().next().map_or(1, char::len_utf8);
        i += ch_len;
        col_no += 1;
        let _ = helpers::mask_strings_comments as fn(&str) -> String;
    }
    findings
}

fn is_simple_atom(inner: &str) -> bool {
    !inner.is_empty()
        && inner
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && inner
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unquoted_is_clean() {
        assert!(check("x = :foo\n").is_empty());
    }
    #[test]
    fn reports_quoted() {
        assert_eq!(check("x = :\"foo\"\n").len(), 1);
    }
}
