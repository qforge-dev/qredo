use crate::Finding;

const MESSAGE: &str = "One `Enum.map/2` is more efficient than `Enum.map/2 |> Enum.map/2`";
const SECOND: &str = "map";

/// `EX4013`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let scan = Scan::new(masked);
    let mut findings = Vec::new();
    for call in scan.calls(SECOND) {
        let Some(close) = call.close else { continue };
        let args = scan.arg_spans(call.open, close);
        if args.is_empty() {
            continue;
        }
        let nested = args.len() == 2 && scan.call_at(args[0].0, args[0].1, "map").is_some();
        let piped = scan
            .top_pipes(args[0].0, args[0].1)
            .last()
            .is_some_and(|last| scan.call_at(*last + 2, args[0].1, "map").is_some());
        if nested || piped {
            push(&raw_lines, &mut findings, call.line, "|>");
        }
    }
    for pipe in scan.pipes() {
        if scan.right_call(pipe, SECOND).is_none() {
            continue;
        }
        if scan.map_before(pipe).is_none() {
            continue;
        }
        push(&raw_lines, &mut findings, scan.line_of(pipe), "|>");
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

fn push(raw_lines: &[&str], findings: &mut Vec<Finding>, line: usize, trigger: &str) {
    findings.push(Finding::with_trigger(
        line + 1,
        raw_lines.get(line).and_then(|l| trigger_column(l, trigger)),
        MESSAGE,
        trigger.to_owned(),
    ));
}

/// Byte scanner over masked source. All recorded offsets point at ASCII
/// delimiters, so slicing there stays on char boundaries.
struct Scan<'a> {
    bytes: &'a [u8],
    starts: Vec<usize>,
}

struct Call {
    line: usize,
    open: usize,
    close: Option<usize>,
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

    /// `Enum.<fun>(` call sites with their paren spans.
    fn calls(&self, fun: &str) -> Vec<Call> {
        let mut out = Vec::new();
        let mut idx = 0_usize;
        while idx + 4 < self.bytes.len() {
            if self.at_module(idx)
                && self.bytes.get(idx + 4) == Some(&b'.')
                && self.at_word(idx + 5, fun)
            {
                let after = idx + 5 + fun.len();
                if self.next_is(after, b'(') {
                    let open = self.skip_ws(after);
                    out.push(Call {
                        line: self.line_of(idx),
                        open,
                        close: self.match_paren(open),
                    });
                    idx = open + 1;
                    continue;
                }
            }
            idx += 1;
        }
        out
    }

    /// `|>` operators, excluding the `<|>` operator's inner pipes.
    fn pipes(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut idx = 0_usize;
        while idx + 1 < self.bytes.len() {
            if self.bytes[idx] == b'|' && self.bytes[idx + 1] == b'>' {
                if idx == 0 || self.bytes[idx - 1] != b'<' {
                    out.push(idx);
                }
                idx += 2;
            } else {
                idx += 1;
            }
        }
        out
    }

    fn at_word(&self, pos: usize, word: &str) -> bool {
        let end = pos + word.len();
        if self.bytes.len() < end || &self.bytes[pos..end] != word.as_bytes() {
            return false;
        }
        let prev_ok = pos == 0 || !is_name_byte(self.bytes[pos - 1]);
        prev_ok && self.bytes.get(end).is_none_or(|b| !is_name_byte(*b))
    }

    fn next_is(&self, pos: usize, want: u8) -> bool {
        self.skip_ws(pos) < self.bytes.len() && self.bytes[self.skip_ws(pos)] == want
    }

    fn skip_ws(&self, mut pos: usize) -> usize {
        while pos < self.bytes.len() && self.bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        pos
    }

    fn skip_ws_back(&self, mut pos: usize) -> usize {
        while pos > 0 && self.bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        pos
    }

    fn match_paren(&self, open: usize) -> Option<usize> {
        let mut depth = 0_i64;
        let mut idx = open;
        while idx < self.bytes.len() {
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(idx);
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        None
    }

    fn match_open(&self, close: usize) -> Option<usize> {
        let mut depth = 0_i64;
        let mut idx = close + 1;
        while idx > 0 {
            idx -= 1;
            match self.bytes[idx] {
                b')' | b']' | b'}' => depth += 1,
                b'(' | b'[' | b'{' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(idx);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Top-level argument spans of a call, tracking brackets and `do`/`fn`
    /// blocks so commas inside nested constructs do not split.
    fn arg_spans(&self, open: usize, close: usize) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let mut depth = 0_i64;
        let mut start = self.skip_ws(open + 1);
        let mut idx = start;
        while idx < close {
            if self.opens_block(idx) {
                depth += 1;
                idx += 2;
                continue;
            }
            if self.at_word(idx, "end") && self.plain_prev(idx) {
                depth -= 1;
                idx += 3;
                continue;
            }
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    spans.push((start, idx));
                    start = self.skip_ws(idx + 1);
                    idx = start;
                    continue;
                }
                _ => {}
            }
            idx += 1;
        }
        if start < close {
            spans.push((start, close));
        }
        spans
    }

    /// Top-level `|>` offsets inside a span, skipping `<|>` and `fn` bodies.
    fn top_pipes(&self, from: usize, to: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut depth = 0_i64;
        let mut idx = from;
        while idx + 1 < to.min(self.bytes.len()) {
            if self.opens_block(idx) {
                depth += 1;
                idx += 2;
                continue;
            }
            if self.at_word(idx, "end") && self.plain_prev(idx) {
                depth -= 1;
                idx += 3;
                continue;
            }
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'|' if depth == 0
                    && self.bytes[idx + 1] == b'>'
                    && (idx == 0 || self.bytes[idx - 1] != b'<') =>
                {
                    out.push(idx);
                    idx += 2;
                    continue;
                }
                _ => {}
            }
            idx += 1;
        }
        out
    }

    /// `Enum.<fun>(` call starting at a span edge (after whitespace).
    fn call_at(&self, from: usize, to: usize, fun: &str) -> Option<usize> {
        let pos = self.skip_ws(from);
        if pos + 4 >= self.bytes.len() || pos >= to {
            return None;
        }
        if !self.at_module(pos) || self.bytes.get(pos + 4) != Some(&b'.') {
            return None;
        }
        if !self.at_word(pos + 5, fun) {
            return None;
        }
        let after = self.skip_ws(pos + 5 + fun.len());
        if after < self.bytes.len() && self.bytes[after] == b'(' {
            Some(after)
        } else {
            None
        }
    }

    /// `fn` or bare `do` (not the `do:` inline keyword, atoms, or access).
    fn opens_block(&self, idx: usize) -> bool {
        if self.at_word(idx, "fn") && self.plain_prev(idx) {
            return true;
        }
        if !self.at_word(idx, "do") || !self.plain_prev(idx) {
            return false;
        }
        self.bytes.get(idx + 2).is_none_or(|b| *b != b':')
    }

    fn plain_prev(&self, idx: usize) -> bool {
        idx == 0 || !matches!(self.bytes[idx - 1], b'.' | b':' | b'@')
    }

    /// `Enum` alias opener: word boundary plus no `.`/`:`/`@` qualifier.
    fn at_module(&self, pos: usize) -> bool {
        if !self.at_word(pos, "Enum") {
            return false;
        }
        pos == 0 || !matches!(self.bytes[pos - 1], b'.' | b':' | b'@')
    }

    /// `Enum.<fun>(` call directly right of a pipe; returns `Enum` start and
    /// the paren open.
    fn right_call(&self, pipe: usize, fun: &str) -> Option<(usize, usize)> {
        let pos = self.skip_ws(pipe + 2);
        if pos + 4 >= self.bytes.len() {
            return None;
        }
        if !self.at_module(pos) || self.bytes.get(pos + 4) != Some(&b'.') {
            return None;
        }
        if !self.at_word(pos + 5, fun) {
            return None;
        }
        let after = self.skip_ws(pos + 5 + fun.len());
        if after < self.bytes.len() && self.bytes[after] == b'(' {
            Some((pos, after))
        } else {
            None
        }
    }

    /// `Enum.map(` call directly left of a pipe; returns `Enum` start.
    fn map_before(&self, pipe: usize) -> Option<usize> {
        let end = self.skip_ws_back(pipe);
        if end == 0 || self.bytes[end - 1] != b')' {
            return None;
        }
        let open = self.match_open(end - 1)?;
        let before = self.skip_ws_back(open);
        if before < 3 || &self.bytes[before - 3..before] != b"map" {
            return None;
        }
        if before < 4 || self.bytes[before - 4] != b'.' {
            return None;
        }
        if before < 8 || &self.bytes[before - 8..before - 4] != b"Enum" {
            return None;
        }
        let enum_start = before - 8;
        if enum_start > 0
            && (is_name_byte(self.bytes[enum_start - 1])
                || matches!(self.bytes[enum_start - 1], b'.' | b':' | b'@'))
        {
            return None;
        }
        Some(enum_start)
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
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("Enum.map(x, & &1)\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "x |> Enum.map(& &1) |> Enum.map(& &2)\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn message_has_no_trailing_period() {
        let src = "defmodule M do\n  def f(p1) do\n    [:a, :b, :c]\n    |> Enum.map(&inspect/1)\n    |> Enum.map(&String.upcase/1)\n  end\nend\n";
        let found = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "One `Enum.map/2` is more efficient than `Enum.map/2 |> Enum.map/2`"
        );
        assert_eq!(found[0].column, Some(5));
    }
}
