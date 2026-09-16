use crate::Finding;

/// `EX4024`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let starts = line_starts(masked);
    // Character and byte indexes shared by every scan below.
    let chars: Vec<char> = masked.chars().collect();
    let bytes: Vec<usize> = masked.char_indices().map(|(byte, _)| byte).collect();
    let mut findings = Vec::new();
    for pos in with_positions(masked, &chars, &bytes) {
        if let Some((at, message)) = check_with(masked, &chars, &bytes, pos) {
            let (line_no, line) = line_of(&starts, &lines, at);
            findings.push(Finding::with_trigger(
                line_no,
                trigger_column(line, "with"),
                message,
                "with".to_owned(),
            ));
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

fn check_with(
    masked: &str,
    chars: &[char],
    bytes: &[usize],
    pos: usize,
) -> Option<(usize, String)> {
    let (clauses, body) = split_with(masked, chars, bytes, pos)?;
    if clauses.is_empty() {
        return None;
    }
    let last = clauses[clauses.len() - 1];
    let arrow = top_arrow(last)?;
    let lhs = last[..arrow].trim();
    if lhs.contains("%{") {
        return None;
    }
    if normalize(lhs) != normalize(body.trim()) {
        return None;
    }
    let message = if clauses.len() == 1 {
        "`with` statement is redundant."
    } else {
        "Last clause in `with` is redundant."
    };
    Some((pos, message.to_owned()))
}

/// Split a `with` construct into its `<-` clauses and `do` body.
/// Returns `None` for calls named `with`, missing bodies, and `else` blocks.
fn split_with<'a>(
    masked: &'a str,
    chars: &[char],
    bytes: &[usize],
    pos: usize,
) -> Option<(Vec<&'a str>, &'a str)> {
    let mut clauses = Vec::new();
    let mut depth = 0_usize;
    let mut angle = 0_usize;
    let mut start = pos + "with".len();
    let mut i = masked[..start].chars().count();
    while i < chars.len() {
        if depth == 0 && chars[i] == '<' && chars.get(i + 1) == Some(&'<') {
            angle += 1;
            i += 2;
            continue;
        }
        if depth == 0 && chars[i] == '>' && chars.get(i + 1) == Some(&'>') && angle > 0 {
            angle -= 1;
            i += 2;
            continue;
        }
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 && angle == 0 => {
                clauses.push(masked[start..bytes[i]].trim());
                start = bytes.get(i + 1).copied().unwrap_or(masked.len());
            }
            _ if depth == 0 && angle == 0 && is_word_at(chars, i, b"do") => {
                let after = bytes.get(i + 2).copied().unwrap_or(masked.len());
                let segment = masked[start..bytes[i]].trim();
                if !segment.is_empty() {
                    clauses.push(segment);
                }
                if masked[after..].starts_with(':') {
                    let body = inline_body(&masked[after + 1..])?;
                    return Some((clauses, body));
                }
                let body = block_body(masked, chars, bytes, i)?;
                return Some((clauses, body));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Body of inline `do:` form: text after the colon up to the line end.
/// `None` when an `else` follows on the same line.
fn inline_body(after_colon: &str) -> Option<&str> {
    let line_end = after_colon.find('\n').unwrap_or(after_colon.len());
    let body = after_colon[..line_end].trim();
    if contains_word(body, "else") {
        return None;
    }
    Some(body)
}

/// Body between a block `do` and its matching `end`.
/// `None` when an own-level `else` intervenes or `end` is missing.
fn block_body<'a>(
    masked: &'a str,
    chars: &[char],
    bytes: &[usize],
    do_at: usize,
) -> Option<&'a str> {
    let mut depth = 1_usize;
    let mut i = do_at + 1;
    let mut body_start = bytes.get(do_at + 2).copied();
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if body_start.is_none() && is_word_at(chars, i, b"do") => {
                body_start = Some(bytes.get(i + 2).copied().unwrap_or(masked.len()));
            }
            _ if depth == 1 && is_word_at(chars, i, b"else") => return None,
            _ if is_word_at(chars, i, b"do") || is_word_at(chars, i, b"fn") => {
                depth += 1;
            }
            _ if is_word_at(chars, i, b"end") => {
                depth -= 1;
                if depth == 0 {
                    let start = body_start.unwrap_or(bytes.get(i).copied().unwrap_or(0));
                    return Some(masked[start..bytes[i]].trim());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Byte offset of the first top-level `<-` in a clause.
fn top_arrow(clause: &str) -> Option<usize> {
    let chars: Vec<char> = clause.chars().collect();
    let bytes: Vec<usize> = clause.char_indices().map(|(byte, _)| byte).collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '<' if depth == 0 && chars.get(i + 1) == Some(&'-') => return Some(bytes[i]),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Compare modulo whitespace and redundant surrounding parentheses.
fn normalize(text: &str) -> String {
    let mut current = text.trim();
    loop {
        if !current.starts_with('(') || !current.ends_with(')') {
            break;
        }
        let inner = &current[1..current.len() - 1];
        if balanced(inner) {
            current = inner.trim();
        } else {
            break;
        }
    }
    current.chars().filter(|c| !c.is_whitespace()).collect()
}

fn balanced(text: &str) -> bool {
    let mut depth = 0_usize;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    depth == 0
}

/// Byte offsets of `with` keywords that are not calls or attribute/field uses.
fn with_positions(masked: &str, chars: &[char], bytes: &[usize]) -> Vec<usize> {
    if !masked.contains("with") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < chars.len() {
        if is_word_at(chars, i, b"with") {
            let after = bytes.get(i + "with".len()).copied().unwrap_or(masked.len());
            if !masked[after..]
                .trim_start_matches([' ', '\t'])
                .starts_with('(')
            {
                out.push(bytes[i]);
            }
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
    let mut line_no = 1_usize;
    for (i, start) in starts.iter().enumerate() {
        if *start <= pos {
            line_no = i + 1;
        } else {
            break;
        }
    }
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
    fn normal_with_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "with {:ok, x} <- foo(), {:ok, y} <- bar(), do: {x, y}\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_redundant() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("with x <- y, do: x\n")).len(),
            1
        );
    }
    #[test]
    fn reports_redundant_last_clause() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "def some_function(parameter1, parameter2) do\n  with :ok <- parameter1,\n       :ok <- parameter2 do\n    :ok\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, Some(3));
        assert_eq!(findings[0].message, "Last clause in `with` is redundant.");
    }
    #[test]
    fn reports_redundant_single_block_with() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "def some_function(parameter) do\n  with {:ok, val} <- do_something(parameter) do\n    {:ok, val}\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].message, "`with` statement is redundant.");
    }
    #[test]
    fn ignores_else_block() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("def some_function(parameter1, parameter2) do\n  with :ok <- parameter1,\n       :ok <- parameter2 do\n    :ok\n  else\n    _ -> :error\n  end\nend\n"))
        .is_empty());
    }
    #[test]
    fn reports_redundant_last_clause_after_bitstring() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "def f(bin) do\n  with :ok <- check(),\n       <<a, b>> <- bin do\n    <<a, b>>\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].message, "Last clause in `with` is redundant.");
    }
}
