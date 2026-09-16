use crate::{Finding, Trigger};

/// `EX4029`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let text = masked;
    let raw: Vec<&str> = source.split('\n').collect();
    let mut findings = Vec::new();
    let mut search = 0_usize;
    while let Some(base) = find_with(text, search) {
        let clause_start = base + "with".len();
        if let Some(clauses) = collect_clauses(text, clause_start) {
            let line_no = text[..base].matches('\n').count() + 1;
            let first_arrow = clauses.first().is_some_and(|c| is_arrow(c));
            if !first_arrow {
                findings.push(with_finding(
                    line_no,
                    raw.get(line_no - 1).unwrap_or(&""),
                    "`with` doesn't start with a <- clause, move the non-pattern <- clauses outside of the `with`.",
                ));
            }
            if clauses.len() > 1 && !clauses.last().is_some_and(|c| is_arrow(c)) {
                findings.push(with_finding(
                    line_no,
                    raw.get(line_no - 1).unwrap_or(&""),
                    "`with` doesn't end with a <- clause, move the non-pattern <- clauses inside the body of the `with`.",
                ));
            }
        }
        search = base + "with".len();
    }
    findings
}

fn with_finding(line_no: usize, raw_line: &str, message: &str) -> Finding {
    Finding {
        line: line_no,
        column: fallback_column(raw_line, "with"),
        message: message.to_owned(),
        trigger: Trigger::Text("with".to_owned()),
        severity: None,
    }
}

/// Byte index of a `with` special form (not a call, attribute or identifier).
fn find_with(text: &str, from: usize) -> Option<usize> {
    let mut search = from;
    while let Some(rel) = text[search..].find("with") {
        let base = search + rel;
        if before_ok(text, base) && after_ok(text, base + "with".len()) {
            return Some(base);
        }
        search = base + 1;
    }
    None
}

fn before_ok(text: &str, base: usize) -> bool {
    if base == 0 {
        return true;
    }
    let prev = text[..base].chars().next_back().unwrap_or(' ');
    !(prev.is_alphanumeric() || prev == '_' || prev == '.' || prev == ':' || prev == '@')
}

fn after_ok(text: &str, end: usize) -> bool {
    let rest: String = text[end..]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let after = &text[end + rest.len()..];
    // A parenthesized group is a call (`with(x)`) unless clauses follow it
    // (`with (x = y) do ...`, `with(x) do ...` quotes as the special form).
    if after.starts_with('(') {
        return paren_opens_clauses(after);
    }
    if after.starts_with('=') {
        return false;
    }
    match text[end..].chars().next() {
        Some(next) => !(next.is_alphanumeric() || next == '_' || next == '?' || next == '!'),
        None => false,
    }
}

/// True when the parenthesized group after `with` is followed by more clauses
/// or the body `do` (rather than ending the call).
fn paren_opens_clauses(after: &str) -> bool {
    let chars: Vec<char> = after.chars().collect();
    let mut depth = 0_usize;
    let mut idx = 0_usize;
    while idx < chars.len() {
        match chars[idx] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let tail: String = chars[idx + 1..].iter().collect();
                    let tail = tail.trim_start();
                    return tail.starts_with(',') || is_do_word(tail);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    false
}

/// True for a `do` keyword (not `done`, `do:` is accepted here as the body).
fn is_do_word(tail: &str) -> bool {
    if !tail.starts_with("do") {
        return false;
    }
    !tail["do".len()..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

/// Collect `with` clauses (between `with` and its body `do`), split at
/// top-level commas. Returns `None` for calls and `do`-less fragments.
fn collect_clauses(text: &str, from: usize) -> Option<Vec<String>> {
    let mut depth = 0_usize;
    let mut current = String::new();
    let mut clauses: Vec<String> = Vec::new();
    let tail = &text[from..];
    let chars: Vec<char> = tail.chars().collect();
    let mut idx = 0_usize;
    while idx < chars.len() {
        let char = chars[idx];
        match char {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(char);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(char);
            }
            ',' if depth == 0 => {
                clauses.push(current.trim().to_owned());
                current = String::new();
            }
            'd' if depth == 0 && is_body_do(&chars, idx, &current) => {
                clauses.push(current.trim().to_owned());
                clauses.retain(|c| !c.is_empty());
                return if clauses.is_empty() {
                    None
                } else {
                    Some(clauses)
                };
            }
            _ => current.push(char),
        }
        idx += 1;
    }
    None
}

/// True at a depth-0 `do` word opening the `with` body.
fn is_body_do(chars: &[char], idx: usize, current: &str) -> bool {
    if chars.get(idx + 1) != Some(&'o') {
        return false;
    }
    if chars
        .get(idx + 2)
        .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
    {
        return false;
    }
    match current.chars().next_back() {
        None => true,
        // `:do`, `.do` and `@do` are atoms/fields, not the body keyword.
        Some(prev) => {
            !(prev.is_alphanumeric()
                || prev == '_'
                || prev == '?'
                || prev == '!'
                || prev == ':'
                || prev == '.'
                || prev == '@')
        }
    }
}

/// True when a clause is a `<-` pattern clause (top-level `<-`,
/// ignoring balanced outer parentheses like `(a <- b)`).
fn is_arrow(clause: &str) -> bool {
    let mut rest = clause.trim();
    while rest.starts_with('(') && rest.ends_with(')') && rest.len() > 2 {
        rest = rest[1..rest.len() - 1].trim();
    }
    let mut depth = 0_usize;
    let chars: Vec<char> = rest.chars().collect();
    let mut idx = 0_usize;
    while idx + 1 < chars.len() {
        match chars[idx] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '<' if depth == 0 && chars[idx + 1] == '-' => return true,
            _ => {}
        }
        idx += 1;
    }
    false
}

/// Native column fallback: first `trigger` occurrence with Credo's boundary
/// rule (`SourceFile.column/3`), as a 1-based byte column.
fn fallback_column(line: &str, trigger: &str) -> Option<usize> {
    let first = trigger.chars().next()?;
    let last = trigger.chars().next_back()?;
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find(trigger) {
        let base = search + rel;
        if before_col_ok(line, base, first) && after_col_ok(line, base + trigger.len(), last) {
            return Some(base + 1);
        }
        search = base + 1;
    }
    None
}

fn before_col_ok(line: &str, base: usize, first: char) -> bool {
    if base == 0 {
        return first.is_alphanumeric() || first == '_';
    }
    let prev = line[..base].chars().next_back().unwrap_or(' ');
    prev.is_whitespace() || "()[],".contains(prev) || is_word(prev) != is_word(first)
}

fn after_col_ok(line: &str, end: usize, last: char) -> bool {
    if end >= line.len() {
        return last.is_alphanumeric() || last == '_';
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
    fn arrow_start_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "with {:ok, x} <- foo(), do: x\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_non_arrow_start() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "with x = foo(), {:ok, y} <- bar(), do: y\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_missing_leading_arrow() {
        let src = "def some_function(parameter1, parameter2) do\n  with IO.puts(\"not a <- clause\"),\n       :ok <- parameter1 do\n    parameter2\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, Some(3));
        assert_eq!(findings[0].trigger, crate::Trigger::Text("with".to_owned()));
        assert_eq!(
            findings[0].message,
            "`with` doesn't start with a <- clause, move the non-pattern <- clauses outside of the `with`."
        );
    }
    #[test]
    fn reports_missing_trailing_arrow() {
        let src = "def some_function(parameter1, parameter2) do\n  with :ok <- parameter1,\n       IO.puts(\"not a <- clause\") do\n    parameter2\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, Some(3));
        assert_eq!(
            findings[0].message,
            "`with` doesn't end with a <- clause, move the non-pattern <- clauses inside the body of the `with`."
        );
    }
    #[test]
    fn reports_parenthesized_non_arrow_first_clause() {
        let src = "def f(y) do\n  with (x = foo()) do y end\nend\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 1);
    }
    #[test]
    fn parenthesized_arrow_first_clause_is_clean() {
        let src = "def f(y) do\n  with (a <- y) do a end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn call_with_do_block_reports_start_issue() {
        let src = "def f(x) do\n  with(x) do y end\nend\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 1);
    }
}
