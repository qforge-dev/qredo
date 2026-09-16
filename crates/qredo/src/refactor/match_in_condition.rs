use crate::Finding;
use std::collections::BTreeMap;

/// `EX4016`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let allow_tagged = params
        .get("allow_tagged_tuples")
        .is_some_and(|v| v == "true");
    let allow_ops = params.get("allow_operators").is_some_and(|v| v == "true");
    let masked = prepared.masked();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let scan = Scan::new(masked);
    let mut findings = Vec::new();
    for (kw_pos, keyword) in scan.keywords() {
        let Some(cond) = scan.condition(kw_pos, keyword.len()) else {
            continue;
        };
        let matches = scan.matches(cond.start, cond.end);
        let base_count = matches.iter().filter(|m| m.depth == cond.base).count();
        for m in &matches {
            let lhs_start = scan.lhs_start(m, &cond);
            let leading_ws = masked[cond.start..lhs_start].trim().is_empty();
            let direct = m.depth == cond.base && leading_ws && base_count == 1;
            let lhs = classify(&masked[lhs_start..m.pos]);
            let issue = if direct && lhs == Lhs::Simple {
                scan.rhs_has_ops(m.pos + 1, cond.trimmed_end) && !allow_ops
            } else if lhs == Lhs::Tagged {
                !allow_tagged
            } else {
                true
            };
            if issue {
                findings.push(Finding::with_trigger(
                    scan.line_of(m.pos) + 1,
                    raw_lines
                        .get(scan.line_of(m.pos))
                        .and_then(|l| trigger_column(l, "=")),
                    format!("Avoid matches in `{keyword}` conditions."),
                    "=".to_owned(),
                ));
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Lhs {
    Simple,
    Tagged,
    Complex,
}

fn classify(raw: &str) -> Lhs {
    let text = raw.trim();
    if is_var(text) {
        return Lhs::Simple;
    }
    if is_tagged_tuple(text) {
        return Lhs::Tagged;
    }
    Lhs::Complex
}

fn is_var(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_lowercase() => {}
        _ => return false,
    }
    let mut rest: Vec<char> = chars.collect();
    if matches!(rest.last(), Some('?' | '!')) {
        rest.pop();
    }
    !rest.is_empty() && rest.iter().all(|c| c.is_alphanumeric() || *c == '_')
}

fn is_tagged_tuple(text: &str) -> bool {
    let inner = text.strip_prefix('{').and_then(|t| t.strip_suffix('}'));
    let Some(inner) = inner else { return false };
    if inner
        .bytes()
        .any(|b| matches!(b, b'(' | b')' | b'[' | b']' | b'{' | b'}'))
    {
        return false;
    }
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 2 {
        return false;
    }
    is_atom(parts[0].trim()) && is_var(parts[1].trim())
}

fn is_atom(text: &str) -> bool {
    let Some(name) = text.strip_prefix(':') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

struct Cond {
    start: usize,
    end: usize,
    trimmed_end: usize,
    base: usize,
}

struct Match {
    pos: usize,
    depth: usize,
}

/// Byte scanner over masked source. Recorded offsets point at ASCII
/// delimiters or char starts, so slicing there stays on char boundaries.
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

    /// `if`/`unless` call sites, skipping bindings, atoms, access, and defs.
    fn keywords(&self) -> Vec<(usize, &'static str)> {
        let mut out = Vec::new();
        for word in ["if", "unless"] {
            let mut idx = 0_usize;
            while idx + word.len() <= self.bytes.len() {
                if self.text_at(idx, word) && self.at_word(idx, word) && self.is_call(idx) {
                    out.push((idx, word));
                    idx += word.len();
                } else {
                    idx += 1;
                }
            }
        }
        out.sort_unstable();
        out
    }

    fn text_at(&self, pos: usize, word: &str) -> bool {
        self.bytes.len() >= pos + word.len()
            && &self.bytes[pos..pos + word.len()] == word.as_bytes()
    }

    fn at_word(&self, pos: usize, word: &str) -> bool {
        let end = pos + word.len();
        if !self.text_at(pos, word) || self.bytes.len() < end {
            return false;
        }
        let prev_ok = pos == 0 || !is_name_byte(self.bytes[pos - 1]);
        prev_ok && self.bytes.get(end).is_none_or(|b| !is_name_byte(*b))
    }

    /// Whether the keyword acts as a call: not a binding (`if in ...`),
    /// key (`if:`), attribute, access, or definition.
    fn is_call(&self, pos: usize) -> bool {
        let word_len = if self.text_at(pos, "unless") { 6 } else { 2 };
        if pos > 0 && matches!(self.bytes[pos - 1], b'.' | b':' | b'@') {
            return false;
        }
        let after = self.skip_ws(pos + word_len);
        if self.bytes.get(after) == Some(&b':') {
            return false;
        }
        if self.at_word(after, "in") {
            return false;
        }
        if self.bytes.get(after) == Some(&b'<') && self.bytes.get(after + 1) == Some(&b'-') {
            return false;
        }
        !self.defined_name(pos)
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

    /// Condition span of an `if`/`unless` keyword. A paren group directly
    /// followed by the `do` block is the call itself (single layer); deeper
    /// full wraps nest. Otherwise the paren groups and we scan plainly.
    fn condition(&self, kw_pos: usize, kw_len: usize) -> Option<Cond> {
        let after = self.skip_ws(kw_pos + kw_len);
        if self.bytes.get(after) == Some(&b'(')
            && let Some(cond) = self.call_condition(after)
        {
            return Some(cond);
        }
        // Keyword-argument calls (`if(cond, do: ..., else: ...)`) scope to
        // the first argument; anything else parenthesized keeps the plain
        // scan so nesting (`if (a = b) && c do`) still reads as nested.
        if self.bytes.get(after) == Some(&b'(')
            && let Some(close) = self.match_paren(after)
            && self.has_keyword_args(after, close)
            && let Some(cond) = self.arg_condition(after)
        {
            return Some(cond);
        }
        let end = self.terminator(after)?;
        Some(self.finish(after, end, 0))
    }

    /// `if(...)` call style: the group must be followed by `do`/`do:`.
    fn call_condition(&self, open: usize) -> Option<Cond> {
        let close = self.match_paren(open)?;
        let (start, end, base) = self.unwrap_layers(open + 1, close);
        let mut next = self.skip_ws(close + 1);
        if self.bytes.get(next) == Some(&b',') {
            next = self.skip_ws(next + 1);
        }
        if !self.at_word(next, "do") {
            return None;
        }
        Some(self.finish(start, end, base))
    }

    /// True when the paren group holds keyword arguments (`if(cond, do:
    /// ..., else: ...)`): a top-level comma followed by `do:`/`else:`.
    fn has_keyword_args(&self, open: usize, close: usize) -> bool {
        let mut depth = 0_i64;
        let mut idx = open + 1;
        while idx < close {
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    let mut next = self.skip_ws(idx + 1);
                    while self.bytes.get(next) == Some(&b',') {
                        next = self.skip_ws(next + 1);
                    }
                    if self.at_word(next, "do") || self.at_word(next, "else") {
                        return true;
                    }
                }
                b'~' => {
                    idx = self.skip_sigil(idx).unwrap_or(idx + 1);
                    continue;
                }
                _ => {}
            }
            idx += 1;
        }
        false
    }

    /// `if(cond, do: ..., else: ...)` keyword style: the condition is the
    /// first argument (up to the top-level comma). Without this the plain
    /// terminator scan starts inside the paren group, skips the nested
    /// `do:` and runs away to a later `do`, swallowing assignments.
    fn arg_condition(&self, open: usize) -> Option<Cond> {
        let close = self.match_paren(open)?;
        let mut depth = 0_i64;
        let mut idx = open + 1;
        let mut end = close;
        while idx < close {
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    end = idx;
                    break;
                }
                b'~' => {
                    idx = self.skip_sigil(idx).unwrap_or(idx + 1);
                    continue;
                }
                _ => {}
            }
            idx += 1;
        }
        let (start, end, base) = self.unwrap_layers(open + 1, end);
        Some(self.finish(start, end, base))
    }

    fn finish(&self, start: usize, end: usize, base: usize) -> Cond {
        let trimmed = self.skip_ws(start);
        let mut trimmed_end = self.skip_ws_back(end);
        if trimmed_end > trimmed && self.bytes[trimmed_end - 1] == b',' {
            trimmed_end = self.skip_ws_back(trimmed_end - 1);
        }
        Cond {
            start,
            end,
            trimmed_end,
            base,
        }
    }

    /// Strip fully wrapped paren layers, counting each as one nesting level.
    fn unwrap_layers(&self, mut start: usize, mut end: usize) -> (usize, usize, usize) {
        let mut extra = 0_usize;
        loop {
            let inner_start = self.skip_ws_to(start, end);
            let inner_end = self.skip_ws_back_to(end, start);
            if inner_start < inner_end
                && self.bytes[inner_start] == b'('
                && self.match_paren(inner_start) == Some(inner_end - 1)
            {
                start = inner_start + 1;
                end = inner_end - 1;
                extra += 1;
            } else {
                return (start, end, extra);
            }
        }
    }

    /// First bare `do` / `do:` at depth zero ends the condition.
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
                b'~' => {
                    idx = self.skip_sigil(idx).unwrap_or(idx + 1);
                    continue;
                }
                _ => {}
            }
            idx += 1;
        }
        None
    }

    /// Match (`=`) operators inside the condition with their depths.
    fn matches(&self, from: usize, to: usize) -> Vec<Match> {
        let mut out = Vec::new();
        let mut depth = 0_usize;
        let mut idx = from;
        while idx < to {
            if self.opens_block(idx) {
                depth += 1;
                idx += 2;
                continue;
            }
            if self.at_word(idx, "end") && self.plain_prev(idx) {
                depth = depth.saturating_sub(1);
                idx += 3;
                continue;
            }
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b'~' => {
                    idx = self.skip_sigil(idx).unwrap_or(idx + 1).min(to);
                    continue;
                }
                b'=' => {
                    if self.is_match(idx) {
                        out.push(Match { pos: idx, depth });
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        out
    }

    /// Whether `=` is a match rather than `==`/`=>`/`=~`/comparison.
    fn is_match(&self, idx: usize) -> bool {
        let prev_ok = idx == 0 || !matches!(self.bytes[idx - 1], b'=' | b'!' | b'<' | b'>');
        let next = self.bytes.get(idx + 1);
        prev_ok && next.is_none_or(|b| !matches!(b, b'=' | b'>' | b'~'))
    }

    /// Backward scan for the match's left-hand side start (byte offset).
    fn lhs_start(&self, m: &Match, cond: &Cond) -> usize {
        let region: Vec<(usize, char)> = self.masked_str()[cond.start..m.pos]
            .char_indices()
            .map(|(o, c)| (cond.start + o, c))
            .collect();
        let mut depth = m.depth;
        let mut start = cond.start;
        let mut idx = region.len();
        while idx > 0 {
            if depth == m.depth && Self::keyword_ends_at(&region, idx) {
                start = region[idx].0;
                break;
            }
            idx -= 1;
            let (off, c) = region[idx];
            match c {
                ')' | ']' | '}' => depth += 1,
                '(' | '[' | '{' => {
                    if depth == m.depth {
                        start = off + 1;
                        break;
                    }
                    depth = depth.saturating_sub(1);
                }
                ',' | '=' | '+' | '-' | '*' | '/' | '<' | '>' | '&' | '|' | '^' | '~'
                    if depth == m.depth =>
                {
                    start = off + 1;
                    break;
                }
                _ => {}
            }
        }
        start
    }

    /// Whether `and`/`or`/`not`/`in` ends at region index `idx`.
    fn keyword_ends_at(region: &[(usize, char)], idx: usize) -> bool {
        for word in ["and", "or", "not", "in"] {
            let chars: Vec<char> = word.chars().collect();
            if idx >= chars.len()
                && region[idx - chars.len()..idx]
                    .iter()
                    .map(|(_, c)| *c)
                    .eq(chars.iter().copied())
            {
                let prev_ok = idx == chars.len() || !is_word_char(region[idx - chars.len() - 1].1);
                let next_ok = idx == region.len() || !is_word_char(region[idx].1);
                if prev_ok && next_ok {
                    return true;
                }
            }
        }
        false
    }

    /// Whether the right-hand side uses operators (any depth, sigils skipped).
    fn rhs_has_ops(&self, from: usize, to: usize) -> bool {
        let mut idx = from;
        while idx < to {
            if self.bytes[idx] == b'~'
                && self.bytes.get(idx + 1).is_some_and(u8::is_ascii_alphabetic)
            {
                idx = self.skip_sigil(idx).unwrap_or(idx + 1).min(to);
                continue;
            }
            if self.three_op(idx, to) || self.two_op(idx, to) || self.one_op(idx) {
                return true;
            }
            idx += 1;
        }
        false
    }

    fn three_op(&self, idx: usize, to: usize) -> bool {
        idx + 3 <= to
            && matches!(
                &self.bytes[idx..idx + 3],
                b"&&&"
                    | b"|||"
                    | b"<<<"
                    | b">>>"
                    | b"<<~"
                    | b"~>>"
                    | b"<~>"
                    | b"<|>"
                    | b"^^^"
                    | b"~~~"
                    | b"+++"
                    | b"---"
            )
    }

    fn two_op(&self, idx: usize, to: usize) -> bool {
        idx + 2 <= to
            && matches!(
                &self.bytes[idx..idx + 2],
                b"&&" | b"||" | b"++" | b"--" | b".." | b"<>" | b"=~" | b"|>" | b"**"
            )
    }

    fn one_op(&self, idx: usize) -> bool {
        match self.bytes[idx] {
            b'*' | b'+' => true,
            b'!' => self.bytes.get(idx + 1).is_none_or(|b| *b != b'='),
            b'-' => self.bytes.get(idx + 1).is_none_or(|b| *b != b'>'),
            b'/' => {
                let prev_ok = idx == 0
                    || !(self.bytes[idx - 1].is_ascii_alphanumeric()
                        || matches!(self.bytes[idx - 1], b'_' | b'?' | b'!' | b'/'));
                prev_ok && self.bytes.get(idx + 1).is_none_or(|b| *b != b'/')
            }
            _ => false,
        }
    }

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

    fn skip_ws(&self, mut pos: usize) -> usize {
        while pos < self.bytes.len() && self.bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        pos
    }

    fn skip_ws_to(&self, mut pos: usize, end: usize) -> usize {
        while pos < end && self.bytes[pos].is_ascii_whitespace() {
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

    fn skip_ws_back_to(&self, mut pos: usize, start: usize) -> usize {
        while pos > start && self.bytes[pos - 1].is_ascii_whitespace() {
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

    fn masked_str(&self) -> &str {
        std::str::from_utf8(self.bytes).unwrap_or("")
    }

    /// Byte offset past a `~name...` sigil starting at `idx`, if well-formed.
    fn skip_sigil(&self, idx: usize) -> Option<usize> {
        let mut name_end = idx + 1;
        if !self
            .bytes
            .get(name_end)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            return None;
        }
        while self
            .bytes
            .get(name_end)
            .is_some_and(u8::is_ascii_alphanumeric)
        {
            name_end += 1;
        }
        let open = *self.bytes.get(name_end)?;
        let close = match open {
            b'/' | b'|' | b'"' | b'\'' => open,
            b'(' => b')',
            b'[' => b']',
            b'{' => b'}',
            b'<' => b'>',
            _ => return None,
        };
        let nested = open != close;
        let mut depth = 0_usize;
        let mut pos = name_end + 1;
        while pos < self.bytes.len() {
            if self.bytes[pos] == b'\\' && pos + 1 < self.bytes.len() {
                pos += 2;
                continue;
            }
            if nested && self.bytes[pos] == open {
                depth += 1;
            } else if self.bytes[pos] == close {
                if depth > 0 {
                    depth -= 1;
                } else {
                    return Some(pos + 1);
                }
            }
            pos += 1;
        }
        None
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("if x == 1, do: y\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("if x = foo(), do: y\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn bracket_access_is_simple_assignment() {
        let src = "defmodule M do\n  def f(p1) do\n    if token = cookies[@remember_me_cookie] do\n      token\n    end\n    if token = cookies[:remember_me] do\n      token\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn paren_call_style_scopes_to_first_argument() {
        // Native reference: `if(` conditions end at the top-level comma;
        // the span must not run away to a later `do` and swallow
        // assignments (checkpoints.ex false positives).
        let clean = "defmodule M do\n  def f(reason) do\n    {:error, if(reason == :not_found, do: :not_found, else: :invalid)}\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(clean), &BTreeMap::new()).is_empty());
        let matched = "defmodule M do\n  def f(x) do\n    if({:ok, y} = foo(x), do: y, else: :e)\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(matched), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
    }
    #[test]
    fn allow_tagged_tuples_suppresses_tagged_match() {
        let src = "defmodule M do\n  def f(x) do\n    if {:ok, contents} = foo(x) do\n      contents\n    end\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
        let params: BTreeMap<String, String> =
            [("allow_tagged_tuples".to_owned(), "true".to_owned())]
                .into_iter()
                .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
    #[test]
    fn allow_operators_suppresses_operator_rhs() {
        let src = "defmodule M do\n  def f(x) do\n    if contents = foo(x) + bar(x) do\n      contents\n    end\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
        let params: BTreeMap<String, String> = [("allow_operators".to_owned(), "true".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
}
