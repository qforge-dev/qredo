use crate::Finding;

/// `EX3030`: `alias Foo.{Bar}` without need for braces.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, pair) in source.split('\n').zip(masked.split('\n')).enumerate() {
        let (raw, masked_line) = pair;
        if !masked_line.contains("alias") || !masked_line.contains('{') {
            continue;
        }
        for child in single_expansions(masked_line) {
            findings.push(Finding::with_trigger(
                idx + 1,
                derive_column(raw, &child.name),
                format!(
                    "Unnecessary alias expansion for {}, consider removing braces.",
                    child.name
                ),
                child.name,
            ));
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

struct Expansion {
    name: String,
}

/// Single-segment `.{Child}` expansions closed on the same masked line.
///
/// Mirrors the pinned walk: each `alias` reports only its outermost (last)
/// single expansion, so `App.{Module2}.{Module3}` flags `Module3` alone.
fn single_expansions(line: &str) -> Vec<Expansion> {
    let mut out = Vec::new();
    let mut starts: Vec<usize> = line
        .match_indices("alias")
        .filter(|(pos, _)| keyword_at(line, *pos, "alias") && alias_call_at(line, *pos))
        .map(|(pos, _)| pos)
        .collect();
    starts.push(line.len());
    for pair in starts.windows(2) {
        let (start, stop) = (pair[0], pair[1]);
        // An `alias` statement ends at `;`, the next `alias`, or the line end.
        // All are ASCII, hence char boundaries.
        let end = line[start..stop].find(';').map_or(stop, |off| start + off);
        let span = &line[start..end];
        if let Some(name) = last_single_child(span) {
            out.push(Expansion { name });
        }
    }
    out
}

/// Last lone `.{Child}` segment in one alias statement span.
fn last_single_child(span: &str) -> Option<String> {
    let bytes = span.as_bytes();
    let mut found = None;
    let mut i = 0_usize;
    while i + 1 < bytes.len() {
        // `.` and `{` are ASCII, so these byte positions are char boundaries.
        if bytes[i] == b'.' && bytes[i + 1] == b'{' {
            let inner_start = i + 2;
            if let Some(relative_end) = span[inner_start..].find('}') {
                let inner = &span[inner_start..inner_start + relative_end];
                if let Some(name) = single_child(inner) {
                    found = Some(name);
                }
                i = inner_start + relative_end + 1;
                continue;
            }
            break;
        }
        i += 1;
    }
    found
}

/// A lone alias segment such as `Module3` (no commas, dots, or whitespace).
fn single_child(inner: &str) -> Option<String> {
    let name = inner.trim().to_owned();
    if name.is_empty() || !is_alias_segment(&name) {
        return None;
    }
    Some(name)
}

fn is_alias_segment(name: &str) -> bool {
    let mut chars = name.chars();
    if chars.next().is_none_or(|first| !first.is_ascii_uppercase()) {
        return false;
    }
    chars.all(|next| next.is_ascii_alphanumeric() || next == '_')
}

fn keyword_at(line: &str, pos: usize, word: &str) -> bool {
    let bytes = line.as_bytes();
    if pos > 0 {
        let prev = bytes[pos - 1];
        // Atoms (`:alias`), attributes, captures, and field access are not
        // alias calls.
        if prev.is_ascii_alphanumeric()
            || prev == b'_'
            || prev == b'?'
            || prev == b'!'
            || prev == b':'
            || prev == b'@'
            || prev == b'.'
            || prev == b'&'
        {
            return false;
        }
    }
    // `pos` comes from `match_indices` and `word` is ASCII.
    line[pos + word.len()..]
        .chars()
        .next()
        .is_none_or(|next| !is_name_char(next))
}

/// An `alias` call opens an uppercase target, `__MODULE__`, or parens.
/// A bare `alias` variable (`alias = 1`) never carries an expansion.
fn alias_call_at(line: &str, pos: usize) -> bool {
    // `pos` comes from `match_indices` and `"alias"` is ASCII.
    line[pos + "alias".len()..]
        .chars()
        .find(|next| !next.is_whitespace())
        .is_some_and(|next| next.is_ascii_uppercase() || next == '_' || next == '(')
}

fn is_name_char(next: char) -> bool {
    next.is_alphanumeric() || next == '_'
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
    fn multi_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("alias Foo.{Bar, Baz}\n")).is_empty());
    }
    #[test]
    fn reports_single_expansion() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("alias Foo.{Bar}\n")).len(),
            1
        );
    }
    #[test]
    fn reports_child_trigger_and_column() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  alias App.Module2.{Module3}\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(22));
        assert_eq!(
            findings[0].trigger,
            crate::Trigger::Text("Module3".to_owned())
        );
    }
    #[test]
    fn reports_double_expansion_child() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "  alias App.{Module2}.{Module3}\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(24));
        assert_eq!(
            findings[0].trigger,
            crate::Trigger::Text("Module3".to_owned())
        );
    }
    #[test]
    fn call_alias_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "alias alias!(MyAliasedModule)\n"
            ))
            .is_empty()
        );
    }
}
