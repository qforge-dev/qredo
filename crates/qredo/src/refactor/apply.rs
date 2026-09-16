use crate::Finding;

/// `EX4003`: avoid `apply/2,3` with known args.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let raw: Vec<&str> = source.split('\n').collect();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with('@') {
            continue;
        }
        let mut search = 0_usize;
        while let Some(base) = find_apply(line, search) {
            if let Some(call) = parse_call(&lines, idx, base)
                && is_violation(&call)
            {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    fallback_column(raw[idx], "apply"),
                    "Avoid `apply/2` and `apply/3` when the number of arguments is known.",
                    "apply".to_owned(),
                ));
            }
            search = base + "apply".len();
        }
    }
    findings
}

/// A parsed `apply(...)` call: its top-level args and optional pipe subject.
struct ApplyCall {
    args: Vec<String>,
    piped_subject: Option<String>,
}

/// Byte index of an `apply` call name at or after `from`.
fn find_apply(line: &str, from: usize) -> Option<usize> {
    let mut search = from;
    while let Some(rel) = line[search..].find("apply") {
        let base = search + rel;
        if before_ok(line, base) && is_call_open(line, base + "apply".len()) {
            return Some(base);
        }
        search = base + 1;
    }
    None
}

fn before_ok(line: &str, base: usize) -> bool {
    if base == 0 {
        return true;
    }
    let prev = line[..base].chars().next_back().unwrap_or(' ');
    !(prev.is_alphanumeric() || prev == '_' || prev == '.' || prev == ':' || prev == '@')
}

fn is_call_open(line: &str, mut pos: usize) -> bool {
    let bytes = line.as_bytes();
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    bytes.get(pos) == Some(&b'(')
}

/// Parse the argument list of the `apply` call at `base` on line `idx`.
fn parse_call(lines: &[&str], idx: usize, base: usize) -> Option<ApplyCall> {
    let line = lines[idx];
    let mut pos = base + "apply".len();
    let bytes = line.as_bytes();
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    if bytes.get(pos) != Some(&b'(') {
        return None;
    }
    let mut depth = 0_usize;
    let mut current = String::new();
    let mut args: Vec<String> = Vec::new();
    let chars: Vec<char> = line[pos..].chars().collect();
    let mut cursor = 0_usize;
    while cursor < chars.len() {
        match chars[cursor] {
            '(' | '[' | '{' => {
                depth += 1;
                if depth > 1 {
                    current.push(chars[cursor]);
                }
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    args.push(current.trim().to_owned());
                    let piped_subject = pipe_subject(lines, idx, base);
                    return Some(ApplyCall {
                        args,
                        piped_subject,
                    });
                }
                current.push(chars[cursor]);
            }
            ',' if depth == 1 => {
                args.push(current.trim().to_owned());
                current = String::new();
            }
            char => current.push(char),
        }
        cursor += 1;
    }
    None
}

/// Pipe subject when `apply` is called as `subject |> apply(...)`.
/// The subject may sit on a previous line when `|>` starts the line.
fn pipe_subject(lines: &[&str], idx: usize, base: usize) -> Option<String> {
    let before = lines[idx][..base].trim_end();
    let head = before.strip_suffix("|>")?;
    let head = head.trim();
    if !head.is_empty() {
        return Some(head.to_owned());
    }
    for prev in lines[..idx].iter().rev() {
        let subject = prev.trim();
        if !subject.is_empty() {
            return Some(subject.to_owned());
        }
    }
    Some(String::new())
}

/// True for flaggable `apply` calls: known list-literal args with a
/// variable fun (apply/2) or an atom fun (apply/3).
fn is_violation(call: &ApplyCall) -> bool {
    let mut effective: Vec<String> = Vec::new();
    if let Some(subject) = &call.piped_subject {
        effective.push(subject.clone());
    }
    effective.extend(call.args.iter().cloned());
    match effective.as_slice() {
        [fun, args] => is_var(fun) && is_proper_list(args),
        [module, fun, args] => {
            module.trim() != "__MODULE__" && is_atom(fun) && is_proper_list(args)
        }
        _ => false,
    }
}

/// Bare variable or local call name (lowercase/underscore start).
fn is_var(text: &str) -> bool {
    let text = text.trim();
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first == '_' => {}
        _ => return false,
    }
    text.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

/// Literal atom fun like `:function`.
fn is_atom(text: &str) -> bool {
    let Some(name) = text.trim().strip_prefix(':') else {
        return false;
    };
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    name.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

/// Proper list literal (`[...]` without a top-level `|`).
fn is_proper_list(text: &str) -> bool {
    let text = text.trim();
    if text.len() < 2 || !text.starts_with('[') || !text.ends_with(']') {
        return false;
    }
    let inner: String = text.chars().skip(1).collect();
    let inner = inner[..inner.len().saturating_sub(1)].to_owned();
    let mut depth = 0_usize;
    let chars: Vec<char> = inner.chars().collect();
    for (idx, char) in chars.iter().enumerate() {
        match char {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => {
                depth = depth.saturating_sub(1);
                // A closer ending the outer level early means the text is an
                // expression (`[a] ++ [b]`), not one literal.
                if depth == 0 && *char == ']' && idx + 1 < chars.len() {
                    return false;
                }
            }
            '|' if depth == 0 => return false,
            _ => {}
        }
    }
    // Balanced brackets are guaranteed by the call parser.
    true
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
    fn direct_call_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("foo(a, b)\n")).is_empty());
    }
    #[test]
    fn reports_apply() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("apply(foo, :bar, [x])\n")).len(),
            1
        );
    }
    #[test]
    fn var_args_are_clean() {
        let src = "defmodule Test do\n  def some_function(fun, args) do\n    apply(fun, args)\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_apply2_with_exact_position() {
        let src = "defmodule Test do\n  def some_function(fun, arg1, arg2) do\n    apply(fun, [arg1, arg2])\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src)),
            vec![Finding::with_trigger(
                3,
                Some(5),
                "Avoid `apply/2` and `apply/3` when the number of arguments is known.",
                "apply",
            )]
        );
    }
}
