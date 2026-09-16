use crate::Finding;
use std::collections::BTreeMap;

const NEST_OPS: [&str; 7] = ["if", "unless", "case", "cond", "fn", "for", "with"];

/// `EX4021`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let max_nesting: usize = params
        .get("max_nesting")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let masked = prepared.masked();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let scan = Scan::new(masked);
    let mut findings = Vec::new();
    for body in scan.def_bodies() {
        if let Some((depth, line, trigger)) = scan.deepest(body.0, body.1)
            && depth > max_nesting
        {
            findings.push(
                Finding::with_trigger(
                    line + 1,
                    raw_lines.get(line).and_then(|l| trigger_column(l, trigger)),
                    format!(
                        "Function body is nested too deep (max depth is {max_nesting}, was {depth})."
                    ),
                    trigger.to_owned(),
                )
                .with_severity(crate::check_meta::severity_count(depth, max_nesting)),
            );
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Nest(&'static str, usize),
    Other,
}

/// Byte scanner over masked source. Offsets point at ASCII delimiters, so
/// slicing there stays on char boundaries.
struct Scan<'a> {
    bytes: &'a [u8],
    starts: Vec<usize>,
}

impl<'a> Scan<'a> {
    fn new(masked: &'a str) -> Self {
        let bytes = masked.as_bytes();
        let mut starts = vec![0_usize];
        for (idx, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                starts.push(idx + 1);
            }
        }
        Self { bytes, starts }
    }

    fn line_of(&self, pos: usize) -> usize {
        self.starts.partition_point(|s| *s <= pos).saturating_sub(1)
    }

    fn at_word(&self, pos: usize, word: &str) -> bool {
        let end = pos + word.len();
        if self.bytes.len() < end || &self.bytes[pos..end] != word.as_bytes() {
            return false;
        }
        let prev_ok = pos == 0 || !is_name_byte(self.bytes[pos - 1]);
        prev_ok && self.bytes.get(end).is_none_or(|b| !is_name_byte(*b))
    }

    fn plain_prev(&self, idx: usize) -> bool {
        idx == 0 || !matches!(self.bytes[idx - 1], b'.' | b':' | b'@')
    }

    fn nest_op_at(&self, pos: usize) -> Option<&'static str> {
        NEST_OPS
            .into_iter()
            .find(|op| self.at_word(pos, op) && self.plain_prev(pos) && !self.defined_name(pos))
    }

    /// Whether the keyword is defined (`def if ...`) rather than called.
    fn defined_name(&self, pos: usize) -> bool {
        let mut end = pos;
        while end > 0 && self.bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && is_name_byte(self.bytes[start - 1]) {
            start -= 1;
        }
        matches!(
            &self.bytes[start..end],
            b"def" | b"defp" | b"defmacro" | b"defmacrop" | b"defguard" | b"defguardp"
        )
    }

    /// `(body_start, body_end)` spans of `def`/`defp`/`defmacro` blocks.
    /// One-line `def ..., do: ...` bodies hold no blocks.
    fn def_bodies(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut idx = 0_usize;
        while idx < self.bytes.len() {
            let def_len = self.def_at(idx);
            if def_len == 0 {
                idx += 1;
                continue;
            }
            if let Some(body) = self.def_body(idx + def_len) {
                out.push(body);
                idx = body.1 + 1;
            } else {
                idx += def_len;
            }
        }
        out
    }

    fn def_at(&self, pos: usize) -> usize {
        for word in ["defmacro", "defp", "def"] {
            if self.at_word(pos, word) && self.plain_prev(pos) {
                return word.len();
            }
        }
        0
    }

    /// Body span after a `def` head: first bare `do` opens it, `do:` is inline.
    fn def_body(&self, from: usize) -> Option<(usize, usize)> {
        let mut depth = 0_i64;
        let mut idx = from;
        let mut body_start = None;
        while idx < self.bytes.len() {
            if self.at_word(idx, "do") && self.plain_prev(idx) {
                if self.bytes.get(idx + 2) == Some(&b':') {
                    if depth == 0 {
                        return None;
                    }
                    idx += 3;
                    continue;
                }
                depth += 1;
                if depth == 1 && body_start.is_none() {
                    body_start = Some(idx + 2);
                }
                idx += 2;
                continue;
            }
            if self.at_word(idx, "fn") && self.plain_prev(idx) {
                depth += 1;
                idx += 2;
                continue;
            }
            if self.at_word(idx, "end") && self.plain_prev(idx) {
                depth -= 1;
                if depth == 0 {
                    return body_start.map(|start| (start, idx));
                }
                if depth < 0 {
                    return None;
                }
                idx += 3;
                continue;
            }
            idx += 1;
        }
        None
    }

    /// Deepest `(depth, line, trigger)` nesting inside a body span.
    /// Non-nest `do`/`end` pairs stay transparent; `do:` lines only peek.
    fn deepest(&self, from: usize, to: usize) -> Option<(usize, usize, &'static str)> {
        let mut stack: Vec<Frame> = Vec::new();
        let mut best: Option<(usize, usize, &'static str)> = None;
        let mut idx = from;
        while idx < to.min(self.bytes.len()) {
            if let Some(op) = self.nest_op_at(idx) {
                let depth = Self::nest_depth(&stack) + 1;
                let line = self.line_of(idx);
                if best.is_none_or(|b| (depth, line, op) >= (b.0, b.1, b.2)) {
                    best = Some((depth, line, op));
                }
                if !self.inline_only(idx + op.len(), to) {
                    stack.push(Frame::Nest(op, line));
                }
                idx += op.len();
                continue;
            }
            if self.at_word(idx, "do") && self.plain_prev(idx) {
                if self.bytes.get(idx + 2) == Some(&b':') {
                    idx += 3;
                } else {
                    // A `do` on its construct's line opens that frame instead
                    // of stacking a new one (`if x do y end` stays balanced).
                    let line = self.line_of(idx);
                    if !matches!(stack.last(), Some(Frame::Nest(_, l)) if *l == line) {
                        stack.push(Frame::Other);
                    }
                    idx += 2;
                }
                continue;
            }
            if self.at_word(idx, "end") && self.plain_prev(idx) {
                stack.pop();
                idx += 3;
                continue;
            }
            idx += 1;
        }
        best
    }

    fn nest_depth(stack: &[Frame]) -> usize {
        stack
            .iter()
            .filter(|f| matches!(f, Frame::Nest(..)))
            .count()
    }

    /// Whether the construct is inline: `do:` before any bare `do` or `end`
    /// at the same bracket level on this line.
    fn inline_only(&self, from: usize, to: usize) -> bool {
        let end = to.min(self.bytes.len());
        let line_end = self.bytes[from..end]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(end, |off| from + off);
        let mut depth = 0_i64;
        let mut idx = from;
        while idx < line_end {
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            if depth == 0 && self.at_word(idx, "do") && self.plain_prev(idx) {
                return self.bytes.get(idx + 2) == Some(&b':');
            }
            if depth == 0 && self.at_word(idx, "end") && self.plain_prev(idx) {
                return false;
            }
            idx += 1;
        }
        false
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

/// Credo infers the column from the trigger: first occurrence flanked by
/// whitespace, a word boundary, or `(`/`)`/`,`.
fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let first = trigger.chars().next()?;
    let last = trigger.chars().next_back()?;
    let mut search = 0_usize;
    while search <= line.len() {
        let rel = line[search..].find(trigger)?;
        let pos = search + rel;
        let prev_ok = line[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || "() ,".contains(c) || boundary_between(c, first));
        let next_ok = line[pos + trigger.len()..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || "() ,".contains(c) || boundary_between(last, c));
        if prev_ok && next_ok {
            return Some(line[..pos].chars().count() + 1);
        }
        search = pos + trigger.len();
    }
    None
}

fn boundary_between(left: char, right: char) -> bool {
    is_word_char(left) != is_word_char(right)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shallow_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def f do\n if x, do: y\nend\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_deep() {
        let src = "def f do\n if a do\n if b do\n if c do\n d\n end\n end\n end\nend\n";
        assert!(!check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn anonymous_functions_nest() {
        let src = "defmodule M do\n  def f(var1, list) do\n    Enum.reduce(var1, list, fn({_hash, nodes}, list) ->\n      filenames = nodes |> Enum.map(&(&1.filename))\n      Enum.reduce(list, [], fn(item, acc) ->\n        if Enum.member?(filenames, item.filename) do\n          item\n        end\n        acc ++ [item]\n      end)\n    end)\n  end\nend\n";
        let found = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 6);
        assert_eq!(found[0].column, Some(9));
    }
}
