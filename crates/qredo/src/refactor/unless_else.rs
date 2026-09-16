use crate::Finding;

/// `EX4027`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    // Character and byte indexes built once: every `unless` scan shares
    // them instead of rebuilding per occurrence.
    let chars: Vec<char> = masked.chars().collect();
    let bytes: Vec<usize> = masked.char_indices().map(|(byte, _)| byte).collect();
    let lines: Vec<&str> = masked.split('\n').collect();
    let starts = line_starts(masked);
    let mut findings = Vec::new();
    // No `unless` substring means no keyword to find.
    if !masked.contains("unless") {
        return findings;
    }
    for idx in unless_positions(&chars) {
        if has_else(masked, &chars, &bytes, idx) {
            let (line_no, line) = line_of(&starts, &lines, bytes[idx]);
            findings.push(Finding::with_trigger(
                line_no,
                trigger_column(line, "unless"),
                "Unless conditions should avoid having an `else` block.",
                "unless".to_owned(),
            ));
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Whether the `unless` at char `idx` carries its own `else`, either inline
/// (`do:`) or in its `do`/`end` block.
fn has_else(masked: &str, chars: &[char], bytes: &[usize], idx: usize) -> bool {
    let mut depth = 0_usize;
    let mut i = idx + "unless".len();
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && is_word_at(chars, i, b"do") => {
                let after = bytes.get(i + 2).copied().unwrap_or(masked.len());
                if masked[after..].starts_with(':') {
                    return inline_else(&masked[after + 1..]);
                }
                return block_else(chars, i);
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// `else` on the same line after an inline `do:`.
fn inline_else(after_colon: &str) -> bool {
    let line_end = after_colon.find('\n').unwrap_or(after_colon.len());
    contains_word(&after_colon[..line_end], "else")
}

/// `else` at the own level between a block `do` and its matching `end`.
fn block_else(chars: &[char], do_at: usize) -> bool {
    let mut depth = 1_usize;
    let mut i = do_at + 1;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 1 && is_word_at(chars, i, b"else") => return true,
            _ if is_block_open(chars, i) => depth += 1,
            _ if is_word_at(chars, i, b"end") => {
                depth -= 1;
                if depth == 0 {
                    return false;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

fn is_block_open(chars: &[char], i: usize) -> bool {
    if is_word_at(chars, i, b"fn") && chars.get(i + 2) != Some(&':') {
        return true;
    }
    is_word_at(chars, i, b"do") && chars.get(i + 2) != Some(&':')
}

/// Char indexes of `unless` keywords (not attributes, calls or atoms).
fn unless_positions(chars: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < chars.len() {
        if is_word_at(chars, i, b"unless") {
            out.push(i);
        }
        i += 1;
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '?' || c == '!'
}

fn is_word_at(chars: &[char], i: usize, word: &[u8]) -> bool {
    if chars.len() < i + word.len() {
        return false;
    }
    if !(chars[i..i + word.len()]
        .iter()
        .zip(word.iter())
        .all(|(got, want)| *got == *want as char))
    {
        return false;
    }
    if i > 0 {
        let prev = chars[i - 1];
        if is_name_char(prev) || prev == '.' || prev == ':' || prev == '@' {
            return false;
        }
    }
    chars.get(i + word.len()).is_none_or(|c| !is_name_char(*c))
}

fn contains_word(text: &str, word: &str) -> bool {
    if !text.contains(word) {
        return false;
    }
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len()).any(|i| is_word_at(&chars, i, word.as_bytes()))
}

fn line_starts(masked: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    for (byte, c) in masked.char_indices() {
        if c == '\n' {
            starts.push(byte + 1);
        }
    }
    starts
}

fn line_of<'a>(starts: &[usize], lines: &[&'a str], pos: usize) -> (usize, &'a str) {
    let line_no = starts.partition_point(|start| *start <= pos).max(1);
    (line_no, lines.get(line_no - 1).copied().unwrap_or(""))
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find(trigger) {
        let pos = search + rel;
        if column_boundary_before(line, pos, trigger) && column_boundary_after(line, pos, trigger) {
            return Some(line[..pos].chars().count() + 1);
        }
        search = pos + 1;
    }
    None
}

fn column_boundary_before(line: &str, pos: usize, trigger: &str) -> bool {
    let first = trigger.chars().next();
    match line[..pos].chars().next_back() {
        None => first.is_some_and(is_word_char),
        Some(c) => {
            c.is_whitespace() || c == '(' || c == ')' || c == ',' || boundary_flip(Some(c), first)
        }
    }
}

fn column_boundary_after(line: &str, pos: usize, trigger: &str) -> bool {
    let last = trigger.chars().next_back();
    match line[pos + trigger.len()..].chars().next() {
        None => last.is_some_and(is_word_char),
        Some(c) => {
            c.is_whitespace() || c == '(' || c == ')' || c == ',' || boundary_flip(last, Some(c))
        }
    }
}

fn boundary_flip(left: Option<char>, right: Option<char>) -> bool {
    match (left, right) {
        (Some(l), Some(r)) => is_word_char(l) != is_word_char(r),
        (Some(l), None) => is_word_char(l),
        (None, Some(r)) => is_word_char(r),
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("unless x, do: y\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "unless x do\n a\nelse\n b\nend\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_inline_else() {
        let findings = check_prepared(&crate::batch::Prepared::lazy("unless x, do: y, else: z\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
        assert_eq!(findings[0].column, Some(1));
    }
    #[test]
    fn ignores_nested_try_else() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "unless x do\n try do\n foo()\n else\n bar()\n end\nend\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn unless_substrings_without_keyword_are_clean() {
        // "unless" inside identifiers carries the gated substring but
        // never matches the whole-word scan.
        let src = "my_unless_var = 1\nx = unless_value\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
}
