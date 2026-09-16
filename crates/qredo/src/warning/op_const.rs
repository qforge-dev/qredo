use crate::Finding;

/// `EX5012`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        check_line(line, idx, &lines, &mut findings);
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

fn check_line(line: &str, idx: usize, lines: &[&str], findings: &mut Vec<Finding>) {
    let stripped = line.trim_start();
    if stripped == "@spec" || stripped.starts_with("@spec ") || stripped.starts_with("@spec(") {
        return;
    }
    let mut pos = 0_usize;
    while pos < line.len() {
        let Some(rel) = line[pos..].find('*') else {
            break;
        };
        let base = pos + rel;
        pos = base + 1;
        if doubled(line, base) || !has_lhs(line, base) || in_capture(line, base) {
            continue;
        }
        let Some(zero) = rhs_zero_or_one(lines, idx, base + 1) else {
            continue;
        };
        let message = if zero {
            "Operation will always return zero."
        } else {
            "Operation will always return the left side of the expression."
        };
        findings.push(Finding::with_trigger(
            idx + 1,
            Some(col_of(line, base)),
            message.to_owned(),
            "*".to_owned(),
        ));
    }
}

/// Whether the `*` is doubled (`**` is not part of this check).
fn doubled(line: &str, base: usize) -> bool {
    line[..base].ends_with('*') || line[base + 1..].starts_with('*')
}

/// Whether some expression precedes the `*` on this line.
fn has_lhs(line: &str, base: usize) -> bool {
    !line[..base].trim_end().is_empty()
}

/// Whether the `*` sits inside a `&` capture (skipped upstream).
fn in_capture(line: &str, base: usize) -> bool {
    let before = &line[..base];
    let mut depth = 0_usize;
    let mut open = None;
    for (idx, chr) in before.char_indices() {
        match chr {
            '(' | '[' | '{' => {
                if depth == 0 {
                    open = Some(idx);
                }
                depth += 1;
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    open = None;
                }
            }
            _ => {}
        }
    }
    if let Some(idx) = open {
        return before[..idx].trim_end().ends_with('&');
    }
    has_lone_ampersand(before)
}

/// Whether the text holds a lone `&` (capture) outside brackets.
fn has_lone_ampersand(before: &str) -> bool {
    let mut depth = 0_usize;
    let mut adjacent_amp = false;
    let mut saw_lone = false;
    for chr in before.chars() {
        match chr {
            '(' | '[' | '{' => {
                depth += 1;
                adjacent_amp = false;
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                adjacent_amp = false;
            }
            '&' if depth == 0 => {
                if adjacent_amp {
                    return false;
                }
                adjacent_amp = true;
                saw_lone = true;
            }
            _ => adjacent_amp = false,
        }
    }
    saw_lone
}

/// Right operand literal (`0`/`1`) after the `*`, possibly on later lines.
fn rhs_zero_or_one(lines: &[&str], idx: usize, end: usize) -> Option<bool> {
    let rest = lines[idx][end..].trim_start();
    if !rest.is_empty() {
        return literal(rest);
    }
    for line in &lines[idx + 1..] {
        if !line.trim().is_empty() {
            return literal(line.trim_start());
        }
    }
    None
}

/// Whether the text starts with a bare `0` or `1` literal.
fn literal(rest: &str) -> Option<bool> {
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits != "0" && digits != "1" {
        return None;
    }
    if rest[digits.len()..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    Some(digits == "0")
}

/// Column (1-based, characters) of the byte offset (which must be a boundary).
fn col_of(line: &str, byte_pos: usize) -> usize {
    line[..byte_pos].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x + y\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("x * 0\n")).len(),
            1
        );
    }
    #[test]
    fn left_zero_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("0 * x\n")).is_empty());
    }
    #[test]
    fn zero_message() {
        let out = check_prepared(&crate::batch::Prepared::lazy("x * 0\n"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "Operation will always return zero.");
        assert_eq!(out[0].column, Some(3));
    }
}
