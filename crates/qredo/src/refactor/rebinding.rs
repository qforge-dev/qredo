use crate::{Finding, helpers};
use std::collections::BTreeMap;

const KEYWORDS: &[&str] = &[
    "def",
    "defp",
    "defmodule",
    "do",
    "end",
    "if",
    "unless",
    "case",
    "cond",
    "with",
    "for",
    "fn",
    "in",
    "not",
    "and",
    "or",
    "true",
    "false",
    "nil",
    "when",
    "else",
    "try",
    "rescue",
    "catch",
    "after",
    "quote",
    "unquote",
    "super",
    "receive",
];

/// `EX4028`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut scan = Scan {
        stack: Vec::new(),
        findings: Vec::new(),
        allow_bang: helpers::param_bool(params, "allow_bang", false),
    };
    for (idx, line) in lines.iter().enumerate() {
        scan.process_line(line, idx + 1, &lines);
    }
    while let Some(scope) = scan.stack.pop() {
        scope.finish(scan.allow_bang, &lines, &mut scan.findings);
    }
    scan.findings
        .sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    scan.findings
}

struct Scope {
    checked: bool,
    vars: Vec<(String, usize)>,
}

struct Scan {
    stack: Vec<Scope>,
    findings: Vec<Finding>,
    allow_bang: bool,
}

impl Scan {
    fn process_line(&mut self, line: &str, line_no: usize, lines: &[&str]) {
        let scan = scan_line(line);
        let record = if line.contains("->") {
            None
        } else if scan.opens.is_empty() {
            Some(line)
        } else {
            scan.after_do
        };
        for is_fn in scan.opens {
            self.stack.push(Scope {
                checked: !is_fn,
                vars: Vec::new(),
            });
        }
        if let Some(text) = record {
            self.record_checked(text, line_no);
        }
        for _ in 0..scan.closes {
            if let Some(scope) = self.stack.pop() {
                scope.finish(self.allow_bang, lines, &mut self.findings);
            }
        }
    }

    fn record_checked(&mut self, text: &str, line_no: usize) {
        let Some(top) = self.stack.last_mut() else {
            return;
        };
        if !top.checked {
            return;
        }
        for stmt in text.split(';') {
            record_statement(stmt, line_no, top);
        }
    }
}

impl Scope {
    fn finish(self, allow_bang: bool, lines: &[&str], findings: &mut Vec<Finding>) {
        let mut done: Vec<&str> = Vec::new();
        for (name, line_no) in self.vars.iter().rev() {
            if done.contains(&name.as_str()) {
                continue;
            }
            done.push(name);
            if allow_bang && name.ends_with('!') {
                continue;
            }
            if self.vars.iter().filter(|(other, _)| other == name).count() >= 2 {
                let line = lines.get(line_no - 1).copied().unwrap_or("");
                findings.push(Finding::with_trigger(
                    *line_no,
                    trigger_column(line, name),
                    format!("Variable \"{name}\" was declared more than once."),
                    name.clone(),
                ));
            }
        }
    }
}

/// Block structure of one line, computed in a single left-to-right pass:
/// `fn` (`true`) and block-`do` (`false`) openers in order, `end` closers,
/// and text after the first block `do` for same-line recording.
struct LineScan<'line> {
    opens: Vec<bool>,
    closes: usize,
    after_do: Option<&'line str>,
}

/// Single scan replacing `openers`/`count_ends`/`after_block_do` (which each
/// rebuilt char and byte vectors per line with backward scans per position).
/// Word boundaries mirror `is_word_at`: before must avoid name chars plus
/// `./:/@`, after must avoid name chars; `do`/`end` reject a `:` suffix.
fn scan_line(line: &str) -> LineScan<'_> {
    let chars: Vec<char> = line.chars().collect();
    let mut scan = LineScan {
        opens: Vec::new(),
        closes: 0,
        after_do: None,
    };
    let mut byte = 0_usize;
    let mut prev: Option<char> = None;
    let mut i = 0_usize;
    while i < chars.len() {
        let word_at = |word: &[u8]| {
            chars.len() >= i + word.len()
                && chars[i..i + word.len()]
                    .iter()
                    .zip(word.iter())
                    .all(|(got, want)| *got == *want as char)
        };
        let before_ok = prev.is_none_or(|c| !is_name_char(c) && c != '.' && c != ':' && c != '@');
        let after_ok = |len: usize| chars.get(i + len).is_none_or(|c| !is_name_char(*c));
        if before_ok && word_at(b"fn") && chars.get(i + 2) != Some(&':') && after_ok(2) {
            scan.opens.push(true);
        } else if before_ok
            && word_at(b"do")
            && value_start(prev)
            && chars.get(i + 2) != Some(&':')
            && after_ok(2)
        {
            scan.opens.push(false);
            if scan.after_do.is_none() {
                let after = if i + 2 <= chars.len() {
                    byte + chars[i..i + 2].iter().map(|c| c.len_utf8()).sum::<usize>()
                } else {
                    line.len()
                };
                scan.after_do = Some(line[after..].trim_start());
            }
        } else if before_ok && word_at(b"end") && chars.get(i + 3) != Some(&':') && after_ok(3) {
            scan.closes += 1;
        }
        byte += chars[i].len_utf8();
        prev = Some(chars[i]);
        i += 1;
    }
    scan
}

/// True when a keyword at this position is real code (not atom/field/capture).
fn value_start(prev: Option<char>) -> bool {
    prev.is_none_or(|c| c != ':' && c != '.' && c != '@' && c != '&')
}

fn record_statement(stmt: &str, line_no: usize, scope: &mut Scope) {
    let Some(eq) = find_assignment(stmt) else {
        return;
    };
    let lhs = strip_bitstring_specs(&stmt[..eq]);
    for name in pattern_vars(&lhs) {
        if !scope
            .vars
            .iter()
            .any(|(other, at)| other == &name && *at == line_no)
        {
            scope.vars.push((name, line_no));
        }
    }
}

/// Byte offset of the first top-level `=` that is not `==`, `!=`,
/// `<=`, `>=`, `=>` or `=~`.
fn find_assignment(stmt: &str) -> Option<usize> {
    let chars: Vec<char> = stmt.chars().collect();
    let bytes: Vec<usize> = stmt.char_indices().map(|(byte, _)| byte).collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '=' if depth == 0 => {
                let prev = if i > 0 {
                    chars.get(i - 1).copied()
                } else {
                    None
                };
                let next = chars.get(i + 1).copied();
                let prev_ok = i == 0 || !matches!(prev, Some('=' | '!' | '<' | '>'));
                let next_ok = !matches!(next, Some('=' | '>' | '~'));
                if prev_ok && next_ok {
                    return Some(bytes[i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Bound variables on an assignment left-hand side: lowercase identifiers
/// that are not pinned, attributes, atoms, field accesses, map keys or
/// keywords.
fn pattern_vars(lhs: &str) -> Vec<String> {
    let chars: Vec<char> = lhs.chars().collect();
    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < chars.len() {
        let c = chars[i];
        if (c.is_ascii_lowercase() || c == '_') && (i == 0 || !is_name_char(chars[i - 1])) {
            let mut end = i;
            while end < chars.len() && is_name_char(chars[end]) {
                end += 1;
            }
            let name: String = chars[i..end].iter().collect();
            if keep_var(&chars, i, end, &name) && !out.contains(&name) {
                out.push(name);
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn keep_var(chars: &[char], start: usize, end: usize, name: &str) -> bool {
    if name.starts_with('_') || KEYWORDS.contains(&name) {
        return false;
    }
    if immediate_prev(chars, start).is_some_and(|prev| prev == '.' || prev == ':' || prev == '@') {
        return false;
    }
    if pinned(chars, start) {
        return false;
    }
    // Map/keyword keys (`key:`) never bind; `::` is a bitstring annotation.
    if chars.get(end) == Some(&':') && chars.get(end + 1) != Some(&':') {
        return false;
    }
    true
}

fn immediate_prev(chars: &[char], start: usize) -> Option<char> {
    if start > 0 {
        Some(chars[start - 1])
    } else {
        None
    }
}

fn pinned(chars: &[char], start: usize) -> bool {
    let mut i = start;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i > 0 && chars[i - 1] == '^'
}

/// Drop `::spec` tails inside `<<...>>` segments; size parameters reuse
/// bindings instead of introducing them.
fn strip_bitstring_specs(lhs: &str) -> String {
    let chars: Vec<char> = lhs.chars().collect();
    let mut out = String::new();
    let mut i = 0_usize;
    let mut depth = 0_usize;
    while i < chars.len() {
        if chars[i] == '<' && chars.get(i + 1) == Some(&'<') {
            depth += 1;
            out.push_str("<<");
            i += 2;
        } else if chars[i] == '>' && chars.get(i + 1) == Some(&'>') && depth > 0 {
            depth -= 1;
            out.push_str(">>");
            i += 2;
        } else if chars[i] == ':' && chars.get(i + 1) == Some(&':') && depth > 0 {
            i += 2;
            let mut parens = 0_usize;
            while i < chars.len() {
                match chars[i] {
                    '(' => parens += 1,
                    ')' => parens = parens.saturating_sub(1),
                    ',' | '>' if parens == 0 => break,
                    _ => {}
                }
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '?' || c == '!'
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
    fn single_binding_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def f do\n x = 1\n y = 2\nend\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_rebinding() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("def f do\n x = 1\n x = 2\nend\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn reports_destructured_rebinding() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(
                "defmodule CredoSampleModule do\n  def some_function() do\n    something = \"ABABAB\"\n    {:ok, something} = Base.decode16(something)\n    {a, a} = {2, 2} # this should _not_ trigger it\n  end\nend\n",
            ),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 4);
        assert_eq!(findings[0].column, Some(11));
    }
    #[test]
    fn reports_bang_rebinding_by_default() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(
                "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    a! = 1\n    a! = 2\n  end\nend\n",
            ),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(5));
    }
    #[test]
    fn allows_bang_rebinding_when_opted_in() {
        let mut params = BTreeMap::new();
        params.insert("allow_bang".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy("defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    a! = 1\n    a! = 2\n  end\nend\n"),&params)
        .is_empty());
    }
    #[test]
    fn ignores_end_keyword_key() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("def f do\n x = [end: 1]\n x = 2\nend\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
    }
    #[test]
    fn ignores_fn_keyword_key() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("def f do\n x = [fn: 1]\n x = 2\nend\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
    }
}
