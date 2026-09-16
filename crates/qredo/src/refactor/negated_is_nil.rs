use crate::Finding;

const MESSAGE: &str = "Avoid negated `is_nil/1` in guard clauses.";

/// `EX4020`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let scan = Scan::new(masked);
    let mut findings = Vec::new();
    for when_pos in scan.guards() {
        let end = scan.guard_end(when_pos + "when".len());
        for (neg_pos, trigger) in scan.negations(when_pos + "when".len(), end) {
            let line = scan.line_of(neg_pos);
            findings.push(Finding::with_trigger(
                line + 1,
                raw_lines.get(line).and_then(|l| trigger_column(l, trigger)),
                MESSAGE,
                trigger.to_owned(),
            ));
        }
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

    /// `when` keywords starting guard clauses.
    fn guards(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut idx = 0_usize;
        while idx + "when".len() <= self.bytes.len() {
            if self.at_word(idx, "when") && self.plain_prev(idx) {
                out.push(idx);
                idx += "when".len();
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

    /// End of the guard: `do` / `do:` / `->` at bracket depth zero.
    fn guard_end(&self, from: usize) -> usize {
        let mut depth = 0_i64;
        let mut idx = from;
        while idx < self.bytes.len() {
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'-' if depth == 0 && self.bytes.get(idx + 1) == Some(&b'>') => {
                    return idx;
                }
                _ => {}
            }
            if depth == 0 && self.at_word(idx, "do") && self.plain_prev(idx) {
                return idx;
            }
            idx += 1;
        }
        self.bytes.len()
    }

    /// `!is_nil(` / `not is_nil(` inside the guard span.
    fn negations(&self, from: usize, to: usize) -> Vec<(usize, &'static str)> {
        let mut out = Vec::new();
        let mut idx = from;
        while idx < to {
            if self.bytes[idx] == b'!'
                && self.bytes.get(idx + 1).is_none_or(|b| *b != b'=')
                && let Some(open) = self.guarded_call(idx + 1, to)
            {
                out.push((idx, "!"));
                idx = self.match_paren(open).map_or(open + 1, |c| c + 1);
                continue;
            }
            if self.at_word(idx, "not")
                && let Some(open) = self.guarded_call(idx + "not".len(), to)
            {
                out.push((idx, "not"));
                idx = self.match_paren(open).map_or(open + 1, |c| c + 1);
                continue;
            }
            idx += 1;
        }
        out
    }

    /// `is_nil(` call starting at `from` (after whitespace and transparent
    /// parens), if any.
    fn guarded_call(&self, from: usize, to: usize) -> Option<usize> {
        let mut pos = from;
        while pos < to && self.bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        while pos < to && self.bytes[pos] == b'(' {
            pos += 1;
            while pos < to && self.bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
        }
        if pos + "is_nil".len() <= self.bytes.len()
            && &self.bytes[pos..pos + "is_nil".len()] == b"is_nil"
            && self
                .bytes
                .get(pos + "is_nil".len())
                .is_none_or(|b| !is_name_byte(*b))
        {
            let mut open = pos + "is_nil".len();
            while open < to && self.bytes[open].is_ascii_whitespace() {
                open += 1;
            }
            if open < self.bytes.len() && self.bytes[open] == b'(' {
                return Some(open);
            }
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
        assert!(check_prepared(&crate::batch::Prepared::lazy("is_nil(x)\n")).is_empty());
    }
    #[test]
    fn reports() {
        let found = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  def f(x) when !is_nil(x), do: x\nend\n",
        ));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].column, Some(17));
    }
    #[test]
    fn body_negation_is_clean() {
        let src = "defmodule M do\n  def f(%{parameter1: parameter2, id: id}) when is_binary(parameter2) do\n    something = not is_nil(parameter2)\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
}
