use crate::Finding;

/// `EX3021`: consecutive `alias`/`require` calls stay grouped per module.
///
/// Mirrors the pinned walk over each `defmodule` body: an `alias` (or
/// `require`) separated from its group by another group reports an issue.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let mut findings = Vec::new();
    let mut stack: Vec<ModuleSeq> = Vec::new();
    let mut depth = 0_i32;
    let mut balance = 0_i32;
    let mut prev_comma = false;
    for (idx, masked_line) in masked.split('\n').enumerate() {
        let line_no = idx + 1;
        let trimmed = masked_line.trim();
        if is_defmodule_open(trimmed) {
            if let Some(top) = stack.last_mut()
                && top.body_depth == depth
            {
                top.observe(Head::Other);
            }
            depth += line_depth_delta(masked_line);
            balance += bracket_delta(masked_line);
            stack.push(ModuleSeq::new(depth));
            prev_comma = false;
            continue;
        }
        let continuation = balance > 0 || prev_comma;
        if !continuation
            && let Some(top) = stack.last_mut()
            && top.body_depth == depth
        {
            observe_statement(top, trimmed, line_no, &raw_lines, &mut findings);
        }
        depth += line_depth_delta(masked_line);
        balance += bracket_delta(masked_line);
        prev_comma = trimmed.ends_with(',');
        while stack.last().is_some_and(|top| top.body_depth > depth) {
            stack.pop();
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Head {
    Alias,
    Require,
    Other,
}

struct ModuleSeq {
    body_depth: i32,
    previous: Vec<Head>,
}

impl ModuleSeq {
    fn new(body_depth: i32) -> Self {
        Self {
            body_depth,
            previous: Vec::new(),
        }
    }

    fn observe(&mut self, head: Head) {
        if self.previous.last() == Some(&head) {
            return;
        }
        self.previous.push(head);
    }
}

/// Classify one body-depth statement line into the call sequence.
fn observe_statement(
    seq: &mut ModuleSeq,
    trimmed: &str,
    line_no: usize,
    raw_lines: &[&str],
    findings: &mut Vec<Finding>,
) {
    let Some(head) = statement_head(trimmed) else {
        return;
    };
    if seq.previous.last() == Some(&head) {
        return;
    }
    if (head == Head::Alias || head == Head::Require) && seq.previous.contains(&head) {
        let (name, message) = match head {
            Head::Alias => (
                "alias",
                "`alias` calls should be consecutive within a module.",
            ),
            Head::Require => (
                "require",
                "`require` calls should be consecutive within a module.",
            ),
            Head::Other => ("", ""),
        };
        let raw = raw_lines.get(line_no - 1).copied().unwrap_or("");
        findings.push(Finding::with_trigger(
            line_no,
            derive_column(raw, name),
            message,
            name,
        ));
        return;
    }
    seq.previous.push(head);
}

/// Head of a module-body statement, or `None` for lines that are not calls
/// (blanks, continuations residues, literals, block closers).
fn statement_head(trimmed: &str) -> Option<Head> {
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("|>") || trimmed.starts_with('.') {
        return None;
    }
    if starts_word(trimmed, "end")
        || starts_word(trimmed, "else")
        || starts_word(trimmed, "rescue")
        || starts_word(trimmed, "catch")
        || starts_word(trimmed, "after")
    {
        return None;
    }
    if starts_word(trimmed, "alias") && call_target_follows(trimmed, "alias") {
        return Some(Head::Alias);
    }
    if starts_word(trimmed, "require") && call_target_follows(trimmed, "require") {
        return Some(Head::Require);
    }
    if is_literal(trimmed) {
        return None;
    }
    Some(Head::Other)
}

/// Lines holding plain values rather than calls: strings, charlists,
/// numbers, atoms, collections, `true`/`false`/`nil`, bare aliases.
fn is_literal(trimmed: &str) -> bool {
    if trimmed == "true" || trimmed == "false" || trimmed == "nil" {
        return true;
    }
    if let Some(first) = trimmed.chars().next() {
        if first == '"'
            || first == '\''
            || first == ':'
            || first == '%'
            || first == '['
            || first == '{'
        {
            return true;
        }
        if first.is_ascii_digit() {
            return true;
        }
        if first.is_ascii_uppercase() && is_bare_alias_path(trimmed) {
            return true;
        }
    }
    false
}

fn is_bare_alias_path(trimmed: &str) -> bool {
    let mut chars = trimmed.chars();
    if chars.next().is_none_or(|first| !first.is_ascii_uppercase()) {
        return false;
    }
    chars.all(|next| next.is_ascii_alphanumeric() || next == '_' || next == '.')
}

fn starts_word(trimmed: &str, word: &str) -> bool {
    trimmed
        .strip_prefix(word)
        .is_some_and(|rest| rest.chars().next().is_none_or(|next| !is_name_char(next)))
}

/// An `alias`/`require` call opens an uppercase target, `__MODULE__`, or
/// parens. A bare variable (`alias = 1`) is any other call.
fn call_target_follows(trimmed: &str, word: &str) -> bool {
    // `starts_word` matched an ASCII keyword: slicing is boundary-safe.
    trimmed[word.len()..]
        .chars()
        .find(|next| !next.is_whitespace())
        .is_some_and(|next| next.is_ascii_uppercase() || next == '_' || next == '(')
}

fn is_defmodule_open(trimmed: &str) -> bool {
    starts_word(trimmed, "defmodule")
}

/// Net `do`/`fn` minus `end` keywords on one masked line.
fn line_depth_delta(line: &str) -> i32 {
    count_keyword(line, "do", true) + count_keyword(line, "fn", false)
        - count_keyword(line, "end", false)
}

/// Occurrences of a whole-word keyword, honouring selector/atom prefixes.
/// For `do`, a trailing `:` (the `do:` option) does not open a block.
fn count_keyword(line: &str, word: &str, skip_colon_after: bool) -> i32 {
    let bytes = line.as_bytes();
    let mut count = 0_i32;
    // `match_indices` yields char boundaries; `word` is ASCII.
    for (pos, _) in line.match_indices(word) {
        if keyword_boundary_before(bytes, pos)
            && keyword_boundary_after(line, pos + word.len())
            && !(skip_colon_after && bytes.get(pos + word.len()) == Some(&b':'))
        {
            count += 1;
        }
    }
    count
}

fn keyword_boundary_before(bytes: &[u8], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let prev = bytes[pos - 1];
    !prev.is_ascii_alphanumeric()
        && prev != b'_'
        && prev != b'?'
        && prev != b'!'
        && prev != b':'
        && prev != b'@'
        && prev != b'.'
        && prev != b'&'
}

fn keyword_boundary_after(line: &str, end: usize) -> bool {
    line[end..]
        .chars()
        .next()
        .is_none_or(|next| !is_name_char(next))
}

fn is_name_char(next: char) -> bool {
    next.is_alphanumeric() || next == '_' || next == '?' || next == '!'
}

/// Net bracket depth change of one masked line.
fn bracket_delta(line: &str) -> i32 {
    let mut delta = 0_i32;
    for byte in line.bytes() {
        match byte {
            b'(' | b'[' | b'{' => delta += 1,
            b')' | b']' | b'}' => delta -= 1,
            _ => {}
        }
    }
    delta
}

/// Mirror of `Credo.SourceFile.column/3`: first trigger occurrence flanked by
/// whitespace, parens, commas, or word boundaries (byte-based, 1-based).
fn derive_column(line: &str, trigger: &str) -> Option<usize> {
    if trigger.is_empty() {
        return None;
    }
    let bytes = line.as_bytes();
    let first = trigger.as_bytes()[0];
    let last = trigger.as_bytes()[trigger.len() - 1];
    for (pos, _) in line.match_indices(trigger) {
        if boundary_before(bytes, pos, first) && boundary_after(bytes, pos + trigger.len(), last) {
            return Some(pos + 1);
        }
    }
    None
}

fn boundary_before(bytes: &[u8], pos: usize, first: u8) -> bool {
    if pos == 0 {
        return is_word_byte(first);
    }
    let prev = bytes[pos - 1];
    is_delim_byte(prev) || (is_word_byte(prev) != is_word_byte(first))
}

fn boundary_after(bytes: &[u8], end: usize, last: u8) -> bool {
    if end >= bytes.len() {
        return is_word_byte(last);
    }
    let next = bytes[end];
    is_delim_byte(next) || (is_word_byte(last) != is_word_byte(next))
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_delim_byte(byte: u8) -> bool {
    byte.is_ascii_whitespace() || byte == b'(' || byte == b')' || byte == b','
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separated_is_clean() {
        let src = "alias Foo\n\nrequire Bar\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_adjacent_mix() {
        let src = "defmodule Test do\n  alias App\n\n  alias App.{\n    Module1,\n    Module2\n  }\n\n  require App.Module5\n\n  alias App.Module3\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 11);
        assert_eq!(findings[0].column, Some(3));
        assert_eq!(
            findings[0].message,
            "`alias` calls should be consecutive within a module."
        );
    }
    #[test]
    fn def_named_require_is_clean() {
        let src = "defmodule Test do\n  alias Foo\n  require Foo\n\n  defp require do\n    :foo\n  end\n\n  defp alias do\n    :foo\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn consecutive_aliases_are_clean() {
        let src = "defmodule Test do\n  alias App.Module1\n  alias App.Module2\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn quoted_alias_body_is_ignored() {
        let src = "defmodule Test do\n  require Foo\n  alias App.Module1\n\n  defmacro __using__ do\n    quote do\n      alias App.Module2\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
}
