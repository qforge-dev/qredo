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
        NEST_OPS.into_iter().find(|op| {
            self.at_word(pos, op)
                && self.plain_prev(pos)
                && !self.defined_name(pos)
                && self.bytes.get(pos + op.len()) != Some(&b':')
        })
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
    /// Hollow constructs (zero-arg-call-only subtrees, mirroring upstream
    /// nil-collapse) record and push nothing.
    fn deepest(&self, from: usize, to: usize) -> Option<(usize, usize, &'static str)> {
        let mut stack: Vec<Frame> = Vec::new();
        let mut best: Option<(usize, usize, &'static str)> = None;
        let mut idx = from;
        while idx < to.min(self.bytes.len()) {
            // Tuple/map/list literals are opaque to nesting (native never
            // descends into them, even nested in call args); call parens
            // stay transparent. `#{` interpolation is code, not a literal.
            if (self.bytes[idx] == b'[' || self.bytes[idx] == b'{')
                && self.bytes.get(idx.wrapping_sub(1)) != Some(&b'#')
                && let Some(after) = self.literal_end(idx, to)
            {
                idx = after;
                continue;
            }
            if let Some(op) = self.nest_op_at(idx) {
                let inline = self.inline_only(idx + op.len(), to);
                if self.subtree_visible(idx + op.len(), to, inline) {
                    let depth = Self::nest_depth(&stack) + 1;
                    let line = self.line_of(idx);
                    if best.is_none_or(|b| (depth, line, op) >= (b.0, b.1, b.2)) {
                        best = Some((depth, line, op));
                    }
                    if !inline {
                        stack.push(Frame::Nest(op, line));
                    }
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

    /// End offset just past the balanced literal (`[...]`, `{...}`,
    /// `%{...}`) starting at `open`, or `None` when unclosed in range.
    fn literal_end(&self, open: usize, to: usize) -> Option<usize> {
        let mut depth = 0_usize;
        let mut idx = open;
        let end = to.min(self.bytes.len());
        while idx < end {
            match self.bytes[idx] {
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(idx + 1);
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        None
    }

    fn nest_depth(stack: &[Frame]) -> usize {
        stack
            .iter()
            .filter(|f| matches!(f, Frame::Nest(..)))
            .count()
    }

    /// Whether the construct is inline: its `do:` (on this line, inside its
    /// own brackets, or on a continued line) arrives before any bare `do`.
    /// A bare `end` or closing bracket ends the statement without deciding
    /// against inline; a bare `do` at depth zero always means a block.
    fn inline_only(&self, from: usize, to: usize) -> bool {
        let end = to.min(self.bytes.len());
        let mut depth = 0_i64;
        let mut idx = from;
        let mut line_start = from;
        let mut seen_do_colon = false;
        while idx < end {
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    if depth == 0 {
                        return seen_do_colon;
                    }
                    depth -= 1;
                }
                b'\n' if depth == 0 => {
                    if seen_do_colon {
                        return true;
                    }
                    if !line_continues(&self.bytes[line_start..idx]) {
                        return false;
                    }
                    line_start = idx + 1;
                }
                _ => {}
            }
            if depth == 0 && self.at_word(idx, "do") && self.plain_prev(idx) {
                if self.bytes.get(idx + 2) == Some(&b':') {
                    seen_do_colon = true;
                    idx += 3;
                    continue;
                }
                return false;
            }
            if depth == 0 && self.at_word(idx, "end") && self.plain_prev(idx) {
                return seen_do_colon;
            }
            if self.at_word(idx, "do")
                && self.plain_prev(idx)
                && self.bytes.get(idx + 2) == Some(&b':')
            {
                seen_do_colon = true;
                idx += 3;
                continue;
            }
            idx += 1;
        }
        seen_do_colon
    }

    /// Whether the construct rooted after `from` holds any solid leaf
    /// (literal, atom, attribute, bare variable/alias read, or solid
    /// pair); childless-only subtrees collapse upstream and stay quiet.
    /// Unbounded spans keep current behavior (visible).
    fn subtree_visible(&self, from: usize, to: usize, inline: bool) -> bool {
        let span = if inline {
            self.stmt_end(from, to)
        } else {
            self.block_end(from, to)
        };
        span.is_none_or(|end| span_solid(self.bytes, from, end))
    }

    /// End of an inline construct's statement: newline at bracket depth
    /// zero without continuation, `;`, or a bracket below start depth.
    fn stmt_end(&self, from: usize, to: usize) -> Option<usize> {
        let end = to.min(self.bytes.len());
        let mut depth = 0_i64;
        let mut idx = from;
        let mut line_start = from;
        while idx < end {
            match self.bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    if depth == 0 {
                        return Some(idx);
                    }
                    depth -= 1;
                }
                b';' if depth == 0 => return Some(idx),
                b'\n' if depth == 0 => {
                    if line_continues(&self.bytes[line_start..idx]) {
                        line_start = idx + 1;
                    } else {
                        return Some(idx);
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        None
    }

    /// End position of the block opened after `from` (its first bare
    /// `do`/`fn` closed by the matching `end`), or `None` when unbalanced.
    fn block_end(&self, from: usize, to: usize) -> Option<usize> {
        let end = to.min(self.bytes.len());
        let mut depth = 0_i64;
        let mut idx = from;
        while idx < end {
            if self.at_word(idx, "do") && self.plain_prev(idx) {
                if self.bytes.get(idx + 2) == Some(&b':') {
                    idx += 3;
                    continue;
                }
                depth += 1;
                idx += 2;
                continue;
            }
            if self.at_word(idx, "fn") && self.plain_prev(idx) && self.colon_free(idx + 2) {
                depth += 1;
                idx += 2;
                continue;
            }
            if self.at_word(idx, "end") && self.plain_prev(idx) && self.colon_free(idx + 3) {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
                idx += 3;
                continue;
            }
            idx += 1;
        }
        None
    }

    /// Whether the byte after a keyword is not `:` (rules out `do:`/`end:`
    /// style keyword options at the given offset).
    fn colon_free(&self, pos: usize) -> bool {
        self.bytes.get(pos) != Some(&b':')
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

/// `word:` keys whose values upstream still visits inside call parens
/// (`foo(do: x)` sees `x`); any other call-paren `word:` tail is dropped
/// with its keyword list.
const BLOCK_KEYS: [&[u8]; 4] = [b"do", b"else", b"rescue", b"after"];

/// Words that never form variable leaves (control words and call heads by
/// construction); `true`/`false`/`nil` are atoms and stay solid.
const SOLID_SKIP_WORDS: [&[u8]; 29] = [
    b"do",
    b"end",
    b"else",
    b"fn",
    b"if",
    b"unless",
    b"case",
    b"cond",
    b"with",
    b"for",
    b"try",
    b"rescue",
    b"catch",
    b"after",
    b"when",
    b"and",
    b"or",
    b"in",
    b"not",
    b"quote",
    b"unquote",
    b"receive",
    b"def",
    b"defp",
    b"defmacro",
    b"defmacrop",
    b"defguard",
    b"defguardp",
    b"defmodule",
];

/// Solidity context of one bracket layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SolidCtx {
    /// Ordinary code: literals, atoms, bare reads and pairs are solid.
    Solid,
    /// Call arguments: `[...]` spans and non-block `word:` tails are hollow.
    CallArgs,
    /// Call-arg list span or keyword tail: only brackets are tracked.
    Hollow,
    /// Non-block `word:` tail inside call parens: pops its call layer too.
    HollowTail,
}

/// Whether the span holds any solid leaf, mirroring upstream `find_depth`
/// collapse: zero-arg calls, empty literals and transparent syntax alone
/// stay hollow. `->` pattern/guard heads never contribute and are skipped.
fn span_solid(bytes: &[u8], from: usize, end: usize) -> bool {
    let heads = head_spans(bytes, from, end);
    let mut head_idx = 0_usize;
    let mut stack = vec![SolidCtx::Solid];
    let mut idx = from;
    while idx < end {
        while head_idx < heads.len() && idx > heads[head_idx].0 {
            head_idx += 1;
        }
        if head_idx < heads.len() && idx == heads[head_idx].0 {
            idx = heads[head_idx].1;
            head_idx += 1;
            continue;
        }
        let hollow = matches!(stack.last(), Some(SolidCtx::Hollow | SolidCtx::HollowTail));
        match bytes[idx] {
            b'(' | b'[' | b'{' => {
                push_ctx(&mut stack, bytes, idx, hollow);
                idx += 1;
            }
            b')' | b']' | b'}' => {
                if pop_ctx(&mut stack) {
                    return true;
                }
                idx += 1;
            }
            _ => match token_step(bytes, idx, end, &mut stack, hollow) {
                TokenStep::Solid => return true,
                TokenStep::Skip(next) => idx = next,
                TokenStep::Plain => idx += 1,
            },
        }
    }
    false
}

/// Push the context opened by `(`/`[`/`{` at `idx`.
fn push_ctx(stack: &mut Vec<SolidCtx>, bytes: &[u8], idx: usize, hollow: bool) {
    let ctx = match bytes[idx] {
        b'(' if !hollow => {
            if is_call_open(bytes, idx) {
                SolidCtx::CallArgs
            } else {
                SolidCtx::Solid
            }
        }
        b'[' if !(hollow
            || stack.last() == Some(&SolidCtx::CallArgs)
                && matches!(prev_non_ws(bytes, idx), Some(b'(' | b',' | b'='))) =>
        {
            SolidCtx::Solid
        }
        b'{' if !hollow => SolidCtx::Solid,
        _ => SolidCtx::Hollow,
    };
    stack.push(ctx);
}

/// Pop one context; a keyword tail pops its call layer too. True when the
/// stack ran dry (unbalanced input stays visible).
fn pop_ctx(stack: &mut Vec<SolidCtx>) -> bool {
    if stack.pop() == Some(SolidCtx::HollowTail) {
        stack.pop();
    }
    stack.is_empty()
}

/// One token's solidity verdict.
enum TokenStep {
    Solid,
    Skip(usize),
    Plain,
}

/// Solidity of the token at `idx`: literals, atoms and bare reads report;
/// hollow spans only advance.
fn token_step(
    bytes: &[u8],
    idx: usize,
    end: usize,
    stack: &mut Vec<SolidCtx>,
    hollow: bool,
) -> TokenStep {
    match bytes[idx] {
        b'"' | b'\'' => {
            if hollow {
                TokenStep::Skip(skip_quoted(bytes, idx, end))
            } else {
                TokenStep::Solid
            }
        }
        b'~' if bytes.get(idx + 1).is_some_and(u8::is_ascii_alphabetic) => {
            if hollow {
                TokenStep::Skip(idx + 1)
            } else {
                TokenStep::Solid
            }
        }
        b'@' if bytes.get(idx + 1).is_some_and(|b| is_name_start(*b)) => {
            if hollow {
                TokenStep::Skip(idx + 1)
            } else {
                TokenStep::Solid
            }
        }
        b':' if is_atom_at(bytes, idx, end) => {
            if hollow {
                TokenStep::Skip(idx + 1)
            } else {
                TokenStep::Solid
            }
        }
        b'0'..=b'9' if idx == 0 || !is_name_byte(bytes[idx - 1]) => {
            if hollow || is_capture_dot(bytes, idx, end) {
                TokenStep::Skip(idx + 1)
            } else {
                TokenStep::Solid
            }
        }
        _ if is_word_start(bytes, idx) => {
            if !hollow && word_is_solid(bytes, idx, end, stack) {
                TokenStep::Solid
            } else {
                TokenStep::Skip(word_end(bytes, idx, end))
            }
        }
        _ => TokenStep::Plain,
    }
}

/// Whether the word at `idx` is a solid leaf in its context: atoms yes;
/// keywords, remote/call heads and block keys no; a bare read yes; a
/// non-block `word:` inside call parens opens a hollow tail (pairs there
/// are dropped upstream) while elsewhere it is a solid pair leaf.
fn word_is_solid(bytes: &[u8], idx: usize, end: usize, stack: &mut Vec<SolidCtx>) -> bool {
    let stop = word_end(bytes, idx, end);
    let word = bytes.get(idx..stop).unwrap_or(&[]);
    if word == b"true" || word == b"false" || word == b"nil" {
        return true;
    }
    if SOLID_SKIP_WORDS.contains(&word) {
        return false;
    }
    // Uppercase alias paths are solid unless a call/path continues.
    if word[0].is_ascii_uppercase() {
        return !matches!(next_non_ws(bytes, stop, end), Some(b'(' | b'.'));
    }
    let next = next_non_ws(bytes, stop, end);
    if next == Some(b'(') {
        return false;
    }
    // Dot receivers live inside the call head upstream, never as leaves.
    if next == Some(b'.') {
        return false;
    }
    if next == Some(b':') && bytes.get(stop + 1) != Some(&b':') {
        if stack.last() == Some(&SolidCtx::CallArgs) && !BLOCK_KEYS.contains(&word) {
            stack.push(SolidCtx::HollowTail);
            return false;
        }
        return !BLOCK_KEYS.contains(&word);
    }
    true
}

/// Whether a word starts at `idx` (alphabetic name outside any name,
/// remote head, atom or attribute position).
fn is_word_start(bytes: &[u8], idx: usize) -> bool {
    if !bytes[idx].is_ascii_alphabetic() && bytes[idx] != b'_' {
        return false;
    }
    if idx == 0 {
        return true;
    }
    !is_name_byte(bytes[idx - 1]) && !matches!(bytes[idx - 1], b'.' | b':' | b'@')
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

/// Whether `(` opens call arguments rather than grouping: preceded by a
/// non-keyword name or a closing value.
fn is_call_open(bytes: &[u8], idx: usize) -> bool {
    let Some(prev) = prev_non_ws(bytes, idx) else {
        return false;
    };
    if matches!(prev, b')' | b']' | b'}' | b'"' | b'\'') {
        return true;
    }
    if !is_name_byte(prev) {
        return false;
    }
    let mut start = idx;
    while start > 0 && is_name_byte(bytes[start - 1]) {
        start -= 1;
    }
    !SOLID_SKIP_WORDS.contains(&bytes.get(start..idx).unwrap_or(&[]))
}

/// Whether `:` starts an atom literal: a name follows and no name byte
/// precedes (the second colon of `::` is allowed, keyword keys are not).
fn is_atom_at(bytes: &[u8], idx: usize, end: usize) -> bool {
    let _ = end;
    bytes
        .get(idx + 1)
        .is_some_and(|b| is_name_start(*b) && (idx == 0 || !is_name_byte(bytes[idx - 1])))
}

/// Whether the digit run at `idx` is an `&N.` capture counter (hollow
/// upstream inside dotted captures) rather than a number literal.
fn is_capture_dot(bytes: &[u8], idx: usize, end: usize) -> bool {
    let mut stop = idx;
    while stop < end && bytes[stop].is_ascii_digit() {
        stop += 1;
    }
    if bytes.get(stop) != Some(&b'.') {
        return false;
    }
    let mut back = idx;
    while back > 0 && matches!(bytes[back - 1], b' ' | b'\t') {
        back -= 1;
    }
    back > 0 && bytes[back - 1] == b'&' && (back < 2 || bytes[back - 2] != b'&')
}

/// End offset of the word starting at `idx`.
fn word_end(bytes: &[u8], idx: usize, end: usize) -> usize {
    let mut stop = idx;
    while stop < end && is_name_byte(bytes[stop]) {
        stop += 1;
    }
    stop
}

/// Previous non-blank byte before `idx`, if any.
fn prev_non_ws(bytes: &[u8], idx: usize) -> Option<u8> {
    let mut back = idx;
    while back > 0 {
        back -= 1;
        if !matches!(bytes[back], b' ' | b'\t' | b'\n' | b'\r') {
            return Some(bytes[back]);
        }
    }
    None
}

/// Next non-blank byte at or after `idx` within `end`, if any.
fn next_non_ws(bytes: &[u8], idx: usize, end: usize) -> Option<u8> {
    let mut fwd = idx;
    while fwd < end {
        if !matches!(bytes[fwd], b' ' | b'\t' | b'\n' | b'\r') {
            return Some(bytes[fwd]);
        }
        fwd += 1;
    }
    None
}

/// Offset just past the quoted literal opened at `idx`.
fn skip_quoted(bytes: &[u8], idx: usize, end: usize) -> usize {
    let quote = bytes[idx];
    let mut next = idx + 1;
    while next < end && bytes[next] != quote {
        next += 1;
    }
    (next + 1).min(end)
}

/// Whether a source line continues the statement: a trailing comma or
/// operator means the construct's `do:`/`do` may live on a later line.
fn line_continues(line: &[u8]) -> bool {
    let mut stop = line.len();
    while stop > 0 && matches!(line[stop - 1], b' ' | b'\t' | b'\r') {
        stop -= 1;
    }
    if stop == 0 {
        return false;
    }
    if matches!(
        line[stop - 1],
        b',' | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'|'
            | b'<'
            | b'>'
            | b'='
            | b'!'
            | b'~'
            | b'&'
            | b'^'
            | b'%'
            | b':'
            | b'.'
    ) {
        return true;
    }
    let mut start = stop;
    while start > 0 && is_name_byte(line[start - 1]) {
        start -= 1;
    }
    matches!(&line[start..stop], b"and" | b"or" | b"not" | b"in")
}

/// `[head_start, arrow+2)` spans of every `->` in the range: patterns and
/// guards never contribute leaves upstream, so solidity skips them.
fn head_spans(bytes: &[u8], from: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut idx = from;
    while idx + 2 <= end && idx + 2 <= bytes.len() {
        if bytes[idx] == b'-' && bytes[idx + 1] == b'>' && (idx == 0 || bytes[idx - 1] != b'-') {
            out.push((head_start(bytes, from, idx), idx + 2));
            idx += 2;
        } else {
            idx += 1;
        }
    }
    out
}

/// Start of the `->` head at `arrow`: back over balanced brackets to an
/// opening bracket, head keyword, `;` or newline.
fn head_start(bytes: &[u8], from: usize, arrow: usize) -> usize {
    let mut depth = 0_i64;
    let mut idx = arrow;
    while idx > from {
        idx -= 1;
        match bytes[idx] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' => {
                if depth == 0 {
                    return idx + 1;
                }
                depth -= 1;
            }
            b';' | b'\n' if depth == 0 => return idx + 1,
            _ => {}
        }
        if depth == 0
            && let Some(len) = head_keyword(bytes, idx + 1, arrow)
        {
            return idx + 1 + len;
        }
    }
    from
}

/// Length of the head keyword (`fn`, `do`, `rescue`, `else`, `catch`, `->`)
/// starting at `pos`, if bounded and fully before `arrow`.
fn head_keyword(bytes: &[u8], pos: usize, arrow: usize) -> Option<usize> {
    for word in ["fn", "do", "rescue", "else", "catch"] {
        if bytes.len() >= pos + word.len()
            && &bytes[pos..pos + word.len()] == word.as_bytes()
            && (pos == 0 || !is_name_byte(bytes[pos - 1]))
            && bytes
                .get(pos + word.len())
                .is_none_or(|b| !is_name_byte(*b) && *b != b':')
        {
            return Some(word.len());
        }
    }
    if pos + 2 <= arrow
        && bytes.len() >= pos + 2
        && &bytes[pos..pos + 2] == b"->"
        && (pos == 0 || bytes[pos - 1] != b'-')
    {
        return Some(2);
    }
    None
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
    #[test]
    fn max_nesting_param_is_honored() {
        let src = "def f do\n if a do\n if b do\n if c do\n d\n end\n end\n end\nend\n";
        assert!(!check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
        let params: BTreeMap<String, String> = [("max_nesting".to_owned(), "5".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
    #[test]
    fn multiline_inline_do_is_not_a_block() {
        // N-A: `do:` on a later line is still inline; it must not push a
        // phantom block frame.
        let src = "defmodule Probe do\n  def f(a, b) do\n    x = if a and\n      b, do: 1\n    if a do\n      if b do\n        x\n      end\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn paren_condition_inline_counts_when_deep() {
        // `w = if(c, do: 1)` is inline but still nests: two block levels
        // plus the inline `if` exceed the max natively.
        let src = "defmodule Probe do\n  def f(a, b) do\n    if a do\n      if b do\n        w = if(c, do: 1)\n        w\n      end\n    end\n  end\nend\n";
        let found = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 5);
    }
    #[test]
    fn case_keyword_option_is_not_an_opener() {
        // N-B: `case:` in `foo(case: 1)` is a keyword option, not `case`.
        let src = "defmodule Probe do\n  def f(a, b) do\n    if a do\n      w = foo(case: 1)\n      if b do\n        w\n      end\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn literal_wrapped_calls_do_not_nest() {
        // Native never descends into `{...}`, `[...]` or `%{...}`
        // literals (even nested in call args); `(...)` stays transparent.
        for source in [
            "def f(x) do\n  case g(x) do\n    nil ->\n      case h(x) do\n        {:error, r} ->\n          {:error, if(r == :bad, do: :a, else: :b)}\n      end\n  end\nend\n",
            "def f(x) do\n  case g(x) do\n    nil ->\n      case h(x) do\n        r ->\n          [if(r == :bad, do: :a, else: :b)]\n      end\n  end\nend\n",
            "def f(x) do\n  case g(x) do\n    nil ->\n      case h(x) do\n        r ->\n          %{x: if(r == :bad, do: :a, else: :b)}\n      end\n  end\nend\n",
        ] {
            assert!(
                check_prepared(&crate::batch::Prepared::lazy(source), &BTreeMap::new()).is_empty(),
                "{source:?}"
            );
        }
        let direct = "def f(x) do\n  case g(x) do\n    nil ->\n      case h(x) do\n        r ->\n          foo(if(r == :bad, do: :a, else: :b))\n      end\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(direct), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn dot_only_subtree_is_invisible() {
        // N-C: every leaf path of the `if` bottoms out in zero-arg calls,
        // so upstream collapses it and stays clean.
        let src = "defmodule Probe do\n  def f(a) do\n    Enum.map(a, fn x ->\n      with {:ok, y} <- foo(x) do\n        if y.state != x.state, do: sync(y.id)\n      end\n    end)\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn bare_var_body_stays_visible() {
        // Control: `go(y)` has a bare-variable leaf, so the `if` counts.
        let src = "defmodule Probe do\n  def f(a) do\n    Enum.map(a, fn x ->\n      with {:ok, y} <- foo(x) do\n        if y.state != x.state, do: go(y)\n      end\n    end)\n  end\nend\n";
        let found = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 5);
    }
}
