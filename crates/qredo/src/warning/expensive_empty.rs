use crate::Finding;

/// `EX5003`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        check_line(line, idx + 1, &mut findings);
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

fn check_line(line: &str, line_no: usize, findings: &mut Vec<Finding>) {
    for token in ["Enum.count", "length"] {
        let mut search = 0_usize;
        while search < line.len() {
            let Some(rel) = line[search..].find(token) else {
                break;
            };
            let base = search + rel;
            search = base + 1;
            if !before_ok(line, base) || !line[base + token.len()..].starts_with('(') {
                continue;
            }
            let Some((arity, close)) = call_arity(line, base + token.len()) else {
                continue;
            };
            if token == "length" && arity != 1 {
                continue;
            }
            if token == "Enum.count" && !(1..=2).contains(&arity) {
                continue;
            }
            if !compared_to_empty(line, base, close) {
                continue;
            }
            let (trigger, message) = suggestion(token, arity);
            findings.push(Finding::with_trigger(
                line_no,
                Some(col_of(line, base)),
                message,
                trigger.to_owned(),
            ));
        }
    }
}

/// Trigger and message for a confirmed empty-comparison.
fn suggestion(token: &str, arity: usize) -> (&'static str, String) {
    if token == "length" {
        (
            "length",
            "Using `length/1` is expensive, prefer comparing against an empty list.".to_owned(),
        )
    } else if arity == 1 {
        (
            "Enum.count",
            "Using `Enum.count/1` is expensive, prefer `Enum.empty?/1`.".to_owned(),
        )
    } else {
        (
            "Enum.count",
            "Using `Enum.count/1` is expensive, prefer `not Enum.any?/2`.".to_owned(),
        )
    }
}

/// The char before the call must not continue another name or remote path.
fn before_ok(line: &str, base: usize) -> bool {
    if base == 0 {
        return true;
    }
    !line[..base]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// Arity and byte index of the closing paren for the call opening at `open`.
fn call_arity(line: &str, open: usize) -> Option<(usize, usize)> {
    let mut depth = 0_usize;
    let mut commas = 0_usize;
    for (rel, chr) in line[open..].char_indices() {
        match chr {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    let close = open + rel;
                    if line[open + 1..close].trim().is_empty() {
                        return Some((0, close));
                    }
                    return Some((commas + 1, close));
                }
            }
            ',' if depth == 1 => commas += 1,
            _ => {}
        }
    }
    None
}

/// Whether the call `[start, close]` is compared against 0 (any operator) or
/// against 1 (`>=`/`<` with the call on the left, `<=` with `1` on the left).
fn compared_to_empty(line: &str, start: usize, close: usize) -> bool {
    if let Some((op, number)) = after_op(line, close + 1) {
        return number == 0 || (number == 1 && (op == ">=" || op == "<"));
    }
    if let Some((number, op)) = before_op(line, start) {
        return number == 0 || (number == 1 && op == "<=");
    }
    false
}

/// Operator and literal right after the call: `call OP 0|1`.
fn after_op(line: &str, from: usize) -> Option<(String, u32)> {
    let rest = line[from..].trim_start();
    let op = ["===", "!==", "==", "!=", ">=", "<=", ">", "<"]
        .into_iter()
        .find(|op| rest.starts_with(op))?;
    // `=` alone (or `=>`) is not a comparison.
    let after = rest[op.len()..].trim_start();
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    if digits != "0" && digits != "1" {
        return None;
    }
    if after[digits.len()..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    Some((op.to_owned(), digits.parse().ok()?))
}

/// Literal and operator right before the call: `0|1 OP call`.
fn before_op(line: &str, start: usize) -> Option<(u32, String)> {
    let before = line[..start].trim_end();
    let op = ["===", "!==", "==", "!=", ">=", "<=", ">", "<"]
        .into_iter()
        .find(|op| before.ends_with(op))?;
    let number = before[..before.len() - op.len()].trim_end();
    let digits: String = number
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if digits != "0" && digits != "1" {
        return None;
    }
    let head = &number[..number.len() - digits.len()];
    if head
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ':')
    {
        return None;
    }
    Some((digits.parse().ok()?, op.to_owned()))
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
        assert!(check_prepared(&crate::batch::Prepared::lazy("Enum.empty?(x)\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("Enum.count(x) == 0\n")).len(),
            1
        );
    }
    #[test]
    fn reports_length_neq_zero_with_empty_list_message() {
        let src = "defmodule CredoSampleModule do\n  def some_function(some_list) do\n    if length(some_list) != 0 do\n      \"not empty\"\n    else\n      \"empty\"\n    end\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
        assert_eq!(out[0].column, Some(8));
        assert_eq!(
            out[0].message,
            "Using `length/1` is expensive, prefer comparing against an empty list."
        );
    }
}
