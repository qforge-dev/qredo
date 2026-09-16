use crate::Finding;

/// `EX4002`: `a ++ [b]` single-item append is inefficient.
///
/// `[a] ++ b` (and any non-single right side) is clean.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let raw: Vec<&str> = source.split('\n').collect();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let mut search = 0_usize;
        while let Some(rel) = line[search..].find("++") {
            let base = search + rel;
            if is_single_append(&lines, idx, base) {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    fallback_column(raw[idx], "++"),
                    "Appending a single item to a list is inefficient, use `[head | tail]` notation (and `Enum.reverse/1` when order matters).",
                    "++".to_owned(),
                ));
            }
            search = base + 2;
            if search >= line.len() {
                break;
            }
        }
    }
    findings
}

/// True when `++` appends one list item: the right side is a single-element
/// list literal (which may open on a following line) and the left side is
/// not a single-element list literal.
fn is_single_append(lines: &[&str], idx: usize, base: usize) -> bool {
    if left_is_single_list(lines[idx], base) {
        return false;
    }
    let Some(text) = right_side_text(lines, idx, base) else {
        return false;
    };
    is_single_list_text(&text)
}

/// Text after `++`, extended over following lines while the list literal is
/// still open; `None` when no list literal opens here.
fn right_side_text(lines: &[&str], idx: usize, base: usize) -> Option<String> {
    let mut text = lines[idx][base + 2..].to_owned();
    let mut next = idx + 1;
    if !text.trim_start().starts_with('[') {
        if !text.trim().is_empty() {
            return None;
        }
        // `++` ends the line: pull lines through the first non-blank one,
        // which must open the list literal.
        let mut opened = false;
        while next < lines.len() {
            text.push('\n');
            text.push_str(lines[next]);
            next += 1;
            if !lines[next - 1].trim().is_empty() {
                opened = true;
                break;
            }
        }
        if !opened || !text.trim_start().starts_with('[') {
            return None;
        }
    }
    // A multi-line list: pull lines until its brackets balance (or EOF).
    while !list_closed(&text) && next < lines.len() {
        text.push('\n');
        text.push_str(lines[next]);
        next += 1;
    }
    Some(text)
}

/// True once the first `[` in `text` (ignoring leading whitespace) closes.
fn list_closed(text: &str) -> bool {
    let mut chars = text.chars();
    for char in chars.by_ref() {
        if !char.is_whitespace() {
            if char != '[' {
                return false;
            }
            break;
        }
    }
    let mut depth = 1_usize;
    for char in chars {
        match char {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// True when `text` (starting after `++`) is a single-element list literal
/// that is not itself the left side of a chained `++`.
fn is_single_list_text(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
    if chars.next() != Some('[') {
        return false;
    }
    is_single_append_rest(&mut chars)
}

/// Shared tail of the single-list verdict: bracket scan from inside the
/// opening `[`, then the right-associativity guard on the remainder.
fn is_single_append_rest(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    let mut depth = 1_usize;
    let mut top_commas = 0_usize;
    let mut top_bars = 0_usize;
    let mut len = 0_usize;
    let mut rest = String::new();
    let mut done = false;
    for char in chars {
        if done {
            rest.push(char);
            continue;
        }
        match char {
            '[' | '(' | '{' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    done = true;
                }
            }
            ')' | '}' => {
                depth = depth.saturating_sub(1);
            }
            ',' if depth == 1 => top_commas += 1,
            '|' if depth == 1 => top_bars += 1,
            c if !c.is_whitespace() => len += 1,
            _ => {}
        }
    }
    if !done {
        return false;
    }
    // `++` is right-associative: `a ++ [b] ++ [c]` appends a call result,
    // not a single-item literal.
    if rest.trim_start().starts_with("++") {
        return false;
    }
    len > 0 && top_commas == 0 && top_bars == 0
}

/// True when the `++` operand on the left is a single-element list literal
/// (`[x] ++ y` is explicitly allowed upstream).
fn left_is_single_list(line: &str, base: usize) -> bool {
    let before: Vec<char> = line[..base].chars().collect();
    let mut idx = before.len();
    while idx > 0 && before[idx - 1].is_whitespace() {
        idx -= 1;
    }
    if idx == 0 || before[idx - 1] != ']' {
        return false;
    }
    let mut depth = 0_usize;
    let mut top_commas = 0_usize;
    let mut top_bars = 0_usize;
    let mut len = 0_usize;
    while idx > 0 {
        idx -= 1;
        match before[idx] {
            ']' | ')' | '}' => depth += 1,
            '[' => {
                if depth == 1 {
                    return len > 0 && top_commas == 0 && top_bars == 0;
                }
                depth = depth.saturating_sub(1);
            }
            '(' | '{' => {
                depth = depth.saturating_sub(1);
            }
            ',' if depth == 1 => top_commas += 1,
            '|' if depth == 1 => top_bars += 1,
            c if depth == 1 && !c.is_whitespace() => len += 1,
            _ => {}
        }
    }
    false
}

/// Native column fallback: first `++` occurrence with Credo's boundary
/// rule (`SourceFile.column/3`), as a 1-based byte column. Native computes
/// one column per line, so same-line issues share it.
fn fallback_column(line: &str, trigger: &str) -> Option<usize> {
    let first = trigger.chars().next()?;
    let last = trigger.chars().next_back()?;
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find(trigger) {
        let base = search + rel;
        if before_ok(line, base, first) && after_ok(line, base + trigger.len(), last) {
            return Some(base + 1);
        }
        search = base + 1;
    }
    None
}

fn before_ok(line: &str, base: usize, first: char) -> bool {
    if base == 0 {
        return first.is_alphanumeric() || first == '_';
    }
    let prev = line[..base].chars().next_back().unwrap_or(' ');
    prev.is_whitespace() || "()[],".contains(prev) || is_word(prev) != is_word(first)
}

fn after_ok(line: &str, end: usize, last: char) -> bool {
    if end >= line.len() {
        // `++` often ends the line when its list opens below; nothing can
        // extend an operator past end of line. (`++` is this file's only
        // trigger, so no word-trigger case reaches this branch.)
        let _ = last;
        return true;
    }
    let next = line[end..].chars().next().unwrap_or(' ');
    next.is_whitespace() || "()[],".contains(next) || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multi_append_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x ++ [a, b]\n")).is_empty());
    }
    #[test]
    fn reports_single_append() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("x ++ [a]\n")).len(),
            1
        );
    }
    #[test]
    fn reports_list_opening_on_next_line() {
        // Triage: `++` with the list opening on the following line.
        let src = "value = Map.get(container, \"env\", []) ++\n  [new_item]\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (1, Some(39)));
    }
    #[test]
    fn next_line_non_list_is_clean() {
        // `x ++ y` followed by an unrelated list is not an append.
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = a ++\n  b\n")).is_empty());
    }
    #[test]
    fn reports_multiline_list_opening_on_next_line() {
        // The single-element list itself may span lines (knative_spec shape).
        let src = "\"env\" =>\n  Map.get(container, \"env\", []) ++\n    [\n      %{\"name\" => \"x\"}\n    ]\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (2, Some(33)));
    }
    #[test]
    fn chained_append_across_lines_is_clean() {
        // Right-associativity applies when the chain continues below.
        assert!(check_prepared(&crate::batch::Prepared::lazy("x ++\n  [a] ++ [c]\n")).is_empty());
    }
    #[test]
    fn single_item_left_list_is_clean() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    [parameter1] ++ [parameter2]\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn chained_append_is_clean() {
        // `++` is right-associative: the outer append takes a call result.
        assert!(check_prepared(&crate::batch::Prepared::lazy("a ++ [b] ++ [c]\n")).is_empty());
    }
    #[test]
    fn chained_append_flags_inner_literal() {
        // `x ++ ([a, b] ++ [c])`: the inner append takes a single item.
        // Native reports the line's first `++` column for the issue.
        let findings = check_prepared(&crate::batch::Prepared::lazy("x ++ [a, b] ++ [c]\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(3));
    }
    #[test]
    fn same_line_issues_share_first_column() {
        let findings = check_prepared(&crate::batch::Prepared::lazy("a ++ [b]; c ++ [d]\n"));
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].column, Some(3));
        assert_eq!(findings[1].column, Some(3));
    }
    #[test]
    fn reports_with_exact_message_and_column() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    parameter1 ++ [parameter2]\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src)),
            vec![Finding::with_trigger(
                3,
                Some(16),
                "Appending a single item to a list is inefficient, use `[head | tail]` notation (and `Enum.reverse/1` when order matters).",
                "++",
            )]
        );
    }
}
