use crate::Finding;

const MESSAGE: &str = "Avoid negated conditions in unless blocks.";

/// `EX4018`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let scan = Scan::new(masked);
    let mut findings = Vec::new();
    for pos in scan.keywords() {
        let Some((neg_pos, trigger)) = scan.negation(pos + "unless".len()) else {
            continue;
        };
        let line = scan.line_of(neg_pos);
        findings.push(Finding::with_trigger(
            line + 1,
            raw_lines.get(line).and_then(|l| trigger_column(l, trigger)),
            MESSAGE,
            trigger.to_owned(),
        ));
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
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

    /// `unless` call sites, skipping attributes, atoms, access, and defs.
    fn keywords(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut idx = 0_usize;
        while idx + "unless".len() <= self.bytes.len() {
            if self.at_word(idx, "unless") && self.is_call(idx) {
                out.push(idx);
                idx += "unless".len();
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

    fn is_call(&self, pos: usize) -> bool {
        if pos > 0 && matches!(self.bytes[pos - 1], b'.' | b':' | b'@') {
            return false;
        }
        !self.defined_name(pos)
    }

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

    /// Leading `!`/`not` of the condition, when it negates the whole
    /// condition. One transparent paren layer is skipped; a doubly wrapped
    /// condition is a block, not a call.
    fn negation(&self, from: usize) -> Option<(usize, &'static str)> {
        let after = self.skip_ws(from);
        let end = self.terminator(after)?;
        let (mut start, mut stop) = self.trim(after, end);
        if stop > start && self.bytes[start] == b'(' && self.match_paren(start) == Some(stop - 1) {
            let inner = self.trim(start + 1, stop - 1);
            start = inner.0;
            stop = inner.1;
        }
        self.negated_whole(start, stop)
    }

    /// Whether `start..stop` is exactly `!operand` / `not operand`.
    fn negated_whole(&self, start: usize, stop: usize) -> Option<(usize, &'static str)> {
        if start >= stop {
            return None;
        }
        if self.bytes[start] == b'!' && self.bytes.get(start + 1).is_none_or(|b| *b != b'=') {
            let op = self.primary(start + 1, stop)?;
            if self.gap_is_ws(op, stop) {
                return Some((start, "!"));
            }
            return None;
        }
        if self.at_word(start, "not") {
            let op = self.primary(start + "not".len(), stop)?;
            if self.gap_is_ws(op, stop) {
                return Some((start, "not"));
            }
        }
        None
    }

    /// End offset of one operand: names, attributes, paren/call/bracket
    /// groups with `.` chains. Anything else ends the operand.
    fn primary(&self, from: usize, stop: usize) -> Option<usize> {
        let mut pos = self.skip_ws(from);
        if self.bytes.get(pos) == Some(&b'@') {
            pos += 1;
        }
        if self.bytes.get(pos) == Some(&b'(') {
            pos = self.match_paren(pos)? + 1;
        } else {
            let name = self.name_end(pos);
            if name == pos {
                return None;
            }
            pos = name;
        }
        loop {
            let next = self.skip_ws(pos);
            if next >= stop {
                return Some(pos);
            }
            if self.bytes[next] == b'.' {
                pos = self.name_end(next + 1);
            } else if self.bytes[next] == b'(' || self.bytes[next] == b'[' {
                pos = self.match_paren(next)? + 1;
            } else {
                return Some(pos);
            }
        }
    }

    fn name_end(&self, mut pos: usize) -> usize {
        while pos < self.bytes.len() && is_name_byte(self.bytes[pos]) {
            pos += 1;
        }
        pos
    }

    /// Whether `op..stop` holds only whitespace (`op` at or past `stop`
    /// never counts).
    fn gap_is_ws(&self, op: usize, stop: usize) -> bool {
        op <= stop
            && op <= self.bytes.len()
            && stop <= self.bytes.len()
            && self.masked()[op..stop].trim().is_empty()
    }

    fn masked(&self) -> &str {
        std::str::from_utf8(self.bytes).unwrap_or("")
    }

    fn trim(&self, start: usize, stop: usize) -> (usize, usize) {
        let from = self.skip_ws(start);
        let mut to = self.skip_ws_back_to(stop, from);
        if to > from && self.bytes[to - 1] == b',' {
            to = self.skip_ws_back_to(to - 1, from);
        }
        (from, to)
    }

    /// First bare `do` / `do:` at depth zero ends the condition, if any.
    fn terminator(&self, from: usize) -> Option<usize> {
        let mut depth = 0_i64;
        let mut idx = from;
        while idx < self.bytes.len() {
            if self.at_word(idx, "do") && self.plain_prev(idx) {
                if self.bytes.get(idx + 2) == Some(&b':') {
                    if depth == 0 {
                        return Some(idx);
                    }
                    idx += 3;
                    continue;
                }
                if depth == 0 {
                    return Some(idx);
                }
                depth += 1;
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
                idx += 3;
                continue;
            }
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            idx += 1;
        }
        None
    }

    fn skip_ws(&self, mut pos: usize) -> usize {
        while pos < self.bytes.len() && self.bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        pos
    }

    fn skip_ws_back_to(&self, mut pos: usize, start: usize) -> usize {
        while pos > start && self.bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        pos
    }

    fn plain_prev(&self, idx: usize) -> bool {
        idx == 0 || !matches!(self.bytes[idx - 1], b'.' | b':' | b'@')
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
        assert!(check_prepared(&crate::batch::Prepared::lazy("unless x, do: y\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("unless !x, do: y\n")).len(),
            1
        );
    }
    #[test]
    fn trigger_is_the_negation() {
        let found = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  def f(p1, p2) do\n    unless !allowed? do\n      something\n    end\n  end\nend\n",
        ));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].column, Some(12));
        assert_eq!(
            match &found[0].trigger {
                crate::Trigger::Text(t) => t.as_str(),
                crate::Trigger::NoTrigger => "no_trigger",
            },
            "!"
        );
    }
    #[test]
    fn comparison_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("unless a != b, do: y\n")).is_empty());
    }
}
