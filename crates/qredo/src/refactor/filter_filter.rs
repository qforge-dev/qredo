use crate::Finding;
use std::collections::BTreeSet;

const MESSAGE: &str = "One `Enum.filter/2` is more efficient than `Enum.filter/2 |> Enum.filter/2`";
const TRIGGER: &str = "|>";

/// `EX4008`: `Enum.filter |> Enum.filter`.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    EnumPair::new("filter", "filter", MESSAGE, TRIGGER).check(prepared.masked())
}

/// Shared `Enum.first |> Enum.second` pipe/nesting scanner (private copy per
/// check; central dedup happens later).
struct EnumPair {
    first: &'static str,
    second: &'static str,
    message: &'static str,
    trigger: &'static str,
    /// `None`: any arity; `Some(1)`: outer call must take exactly one argument.
    outer_arity: Option<usize>,
    /// Outer call must take at least this many arguments.
    outer_min_arity: usize,
    /// Right-hand side call must have empty parentheses (`Enum.count()`).
    rhs_empty: bool,
}

impl EnumPair {
    fn new(
        first: &'static str,
        second: &'static str,
        message: &'static str,
        trigger: &'static str,
    ) -> Self {
        Self {
            first,
            second,
            message,
            trigger,
            outer_arity: None,
            outer_min_arity: 2,
            rhs_empty: false,
        }
    }

    fn check(&self, masked: &str) -> Vec<Finding> {
        let text = masked;
        let table = LineTable::new(text);
        let mut hits: BTreeSet<(usize, Option<usize>)> = BTreeSet::new();
        for pipe in find_pipes(text) {
            if let Some(hit) = self.pipe_hit(text, &table, pipe) {
                hits.insert(hit);
            }
        }
        for call in find_enum_calls(text, self.second) {
            if let Some(hit) = self.nested_hit(text, &table, &call) {
                hits.insert(hit);
            }
        }
        hits.into_iter()
            .map(|(line, column)| Finding::with_trigger(line, column, self.message, self.trigger))
            .collect()
    }

    /// `stage |> Enum.second(...)` where the stage is an `Enum.first` call.
    fn pipe_hit(
        &self,
        text: &str,
        table: &LineTable,
        pipe: usize,
    ) -> Option<(usize, Option<usize>)> {
        let rhs = skip_ws_forward(text, pipe + 2);
        if !is_enum_head(text, rhs, self.second) {
            return None;
        }
        if self.rhs_empty && !has_empty_parens(text, rhs + 5 + self.second.len()) {
            return None;
        }
        let stage = lhs_stage(text, pipe)?;
        if !is_bare_enum_call(&stage, self.first) {
            return None;
        }
        let (line_no, line) = table.line_at(pipe);
        Some((line_no, credo_column(line, self.trigger)))
    }

    /// `Enum.second(Enum.first(...))` or `Enum.second(pipe |> Enum.first(...))`.
    fn nested_hit(
        &self,
        text: &str,
        table: &LineTable,
        call: &EnumCall,
    ) -> Option<(usize, Option<usize>)> {
        let args = split_args(text, call.args_start, call.args_end);
        if let Some(arity) = self.outer_arity {
            if args.len() != arity {
                return None;
            }
        } else if args.len() < self.outer_min_arity {
            return None;
        }
        let first = args.first()?.trim();
        let matched = is_bare_enum_call(first, self.first)
            || last_pipe_stage(first).is_some_and(|stage| is_bare_enum_call(&stage, self.first));
        if !matched {
            return None;
        }
        let (line_no, line) = table.line_at(call.name_start);
        Some((line_no, credo_column(line, self.trigger)))
    }
}

struct EnumCall {
    name_start: usize,
    args_start: usize,
    args_end: usize,
}

/// Byte offset of every `|>` operator.
fn find_pipes(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0_usize;
    while idx + 1 < bytes.len() {
        if bytes[idx] == b'|' && bytes[idx + 1] == b'>' {
            out.push(idx);
            idx += 2;
        } else {
            idx += 1;
        }
    }
    out
}

/// `Enum.name` calls with balanced argument spans.
fn find_enum_calls(text: &str, name: &str) -> Vec<EnumCall> {
    let mut out = Vec::new();
    let prefix = format!("Enum.{name}");
    let mut idx = 0_usize;
    while idx + prefix.len() <= text.len() {
        if text
            .get(idx..)
            .is_some_and(|rest| rest.starts_with(&prefix))
            && head_start_ok(text, idx)
            && head_end_ok(text, idx + prefix.len())
        {
            let paren = skip_ws_forward(text, idx + prefix.len());
            if text.as_bytes().get(paren) == Some(&b'(')
                && let Some(close) = match_paren_forward(text, paren)
            {
                out.push(EnumCall {
                    name_start: idx,
                    args_start: paren + 1,
                    args_end: close,
                });
                idx = close + 1;
                continue;
            }
        }
        idx += 1;
    }
    out
}

fn head_start_ok(text: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = text.as_bytes()[idx - 1];
    !(prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.' || prev == b':')
}

fn head_end_ok(text: &str, idx: usize) -> bool {
    match text.as_bytes().get(idx) {
        None => true,
        Some(byte) => {
            !(byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'?' || *byte == b'!')
        }
    }
}

/// The piped expression left of `|` at `pipe` (same nesting level).
/// Newlines are tentative (multiline calls may span them); block keywords,
/// assignment, commas and brackets end the stage.
fn lhs_stage(text: &str, pipe: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut idx = pipe;
    let mut depth = 0_usize;
    while idx > 0 {
        idx -= 1;
        match bytes[idx] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' => {
                if depth == 0 {
                    return text
                        .get(idx + 1..pipe)
                        .map(str::trim)
                        .map(ToOwned::to_owned);
                }
                depth -= 1;
            }
            b'>' if depth == 0 && idx > 0 && bytes[idx - 1] == b'|' => {
                return text
                    .get(idx + 1..pipe)
                    .map(str::trim)
                    .map(ToOwned::to_owned);
            }
            b'=' | b',' if depth == 0 => {
                return text
                    .get(idx + 1..pipe)
                    .map(str::trim)
                    .map(ToOwned::to_owned);
            }
            b'\n' if depth == 0 => {
                let candidate = text.get(idx + 1..pipe).map_or("", str::trim);
                if !candidate.is_empty() {
                    return Some(candidate.to_owned());
                }
            }
            _ => {}
        }
        if depth == 0 && is_block_stop(text, idx) {
            return text
                .get(idx + 1..pipe)
                .map(str::trim)
                .map(ToOwned::to_owned);
        }
    }
    text.get(..pipe).map(str::trim).map(ToOwned::to_owned)
}

/// Block keywords plus `->` and `<-` end a stage when scanning backwards.
fn is_block_stop(text: &str, idx: usize) -> bool {
    for kw in [
        "do", "end", "else", "rescue", "catch", "after", "fn", "->", "<-",
    ] {
        if text.get(..idx + 1).is_some_and(|head| {
            head.ends_with(kw)
                && head.len() > kw.len()
                && !is_name_byte(head.as_bytes()[head.len() - kw.len() - 1])
        }) && text
            .as_bytes()
            .get(idx + 1)
            .is_none_or(|b| !is_name_byte(*b))
        {
            // `do:` keyword arguments do not open blocks.
            if kw == "do" && text.as_bytes().get(idx + 1) == Some(&b':') {
                continue;
            }
            return true;
        }
    }
    false
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

/// Stage text is exactly one `Enum.name` call (bare or with parentheses).
fn is_bare_enum_call(stage: &str, name: &str) -> bool {
    let prefix = format!("Enum.{name}");
    if !stage.starts_with(&prefix) {
        return false;
    }
    let rest = stage[prefix.len()..].trim_start();
    if rest.is_empty() {
        return true;
    }
    if rest.starts_with('(') {
        let mut depth = 0_usize;
        let mut end = None;
        for (idx, byte) in rest.bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(idx + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            return false;
        };
        return rest.get(end..).unwrap_or("").trim_start().is_empty();
    }
    false
}

/// `Enum.name` (with optional whitespace) at `idx`, not part of a longer name.
fn is_enum_head(text: &str, idx: usize, name: &str) -> bool {
    let prefix = format!("Enum.{name}");
    text.get(idx..)
        .is_some_and(|rest| rest.starts_with(&prefix))
        && head_start_ok(text, idx)
        && head_end_ok(text, idx + prefix.len())
}

/// Empty `()` (allowing whitespace) or a bare call at `idx`.
fn has_empty_parens(text: &str, idx: usize) -> bool {
    let open = skip_ws_forward(text, idx);
    match text.as_bytes().get(open) {
        Some(b'(') => {
            let close = skip_ws_forward(text, open + 1);
            text.as_bytes().get(close) == Some(&b')')
        }
        _ => is_stage_end(text, open),
    }
}

fn is_stage_end(text: &str, idx: usize) -> bool {
    match text.as_bytes().get(idx) {
        None | Some(b'\n' | b',' | b')' | b']' | b'}') => true,
        Some(b'|') => text.as_bytes().get(idx + 1) == Some(&b'>'),
        _ => false,
    }
}

/// Text after the last same-level `|>` in `expr`, if any.
fn last_pipe_stage(expr: &str) -> Option<String> {
    let bytes = expr.as_bytes();
    let mut depth = 0_usize;
    let mut idx = bytes.len();
    while idx > 0 {
        idx -= 1;
        match bytes[idx] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' => depth = depth.saturating_sub(1),
            b'>' if depth == 0 && idx > 0 && bytes[idx - 1] == b'|' => {
                return expr.get(idx + 1..).map(str::trim).map(ToOwned::to_owned);
            }
            _ => {}
        }
    }
    None
}

/// Split a parenthesized argument list at top-level commas.
fn split_args(text: &str, start: usize, end: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = text.as_bytes();
    let mut depth = 0_usize;
    let mut part_start = start;
    let mut idx = start;
    while idx < end.min(bytes.len()) {
        match bytes[idx] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(text.get(part_start..idx).unwrap_or(""));
                part_start = idx + 1;
            }
            _ => {}
        }
        idx += 1;
    }
    parts.push(text.get(part_start..end.min(bytes.len())).unwrap_or(""));
    parts
}

fn match_paren_forward(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0_usize;
    let mut idx = open;
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' => depth += 1,
            b')' => {
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

fn skip_ws_forward(text: &str, mut idx: usize) -> usize {
    let bytes = text.as_bytes();
    while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
        idx += 1;
    }
    idx
}

/// Line starts plus offset-to-line lookup for masked text.
struct LineTable {
    starts: Vec<usize>,
    text: String,
}

impl LineTable {
    fn new(text: &str) -> Self {
        let mut starts = vec![0_usize];
        for (idx, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(idx + 1);
            }
        }
        Self {
            starts,
            text: text.to_owned(),
        }
    }

    fn line_at(&self, offset: usize) -> (usize, &str) {
        let line_no = self.starts.partition_point(|start| *start <= offset);
        let start = self.starts[line_no - 1];
        let end = self.starts.get(line_no).copied().unwrap_or(self.text.len());
        (line_no, self.text.get(start..end).unwrap_or(""))
    }
}

/// Credo `SourceFile.column/3`: 1-based column of `trigger` when surrounded by
/// whitespace, parens, commas or word boundaries; `None` otherwise.
fn credo_column(line: &str, trigger: &str) -> Option<usize> {
    if trigger.is_empty() {
        return None;
    }
    let lchars: Vec<char> = line.chars().collect();
    let tchars: Vec<char> = trigger.chars().collect();
    if lchars.len() < tchars.len() {
        return None;
    }
    for idx in 0..=lchars.len() - tchars.len() {
        if lchars[idx..idx + tchars.len()] != tchars[..] {
            continue;
        }
        let before_ok = if idx == 0 {
            is_word(tchars[0])
        } else {
            before_ok(lchars[idx - 1], tchars[0])
        };
        let after = idx + tchars.len();
        let after_ok = if after == lchars.len() {
            is_word(tchars[tchars.len() - 1])
        } else {
            after_ok(tchars[tchars.len() - 1], lchars[after])
        };
        if before_ok && after_ok {
            return Some(idx + 1);
        }
    }
    None
}

fn before_ok(prev: char, first: char) -> bool {
    prev.is_whitespace()
        || prev == '('
        || prev == ')'
        || prev == ','
        || is_word(prev) != is_word(first)
}

fn after_ok(last: char, next: char) -> bool {
    next.is_whitespace()
        || next == '('
        || next == ')'
        || next == ','
        || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_filter_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("Enum.filter(x, & &1)\n")).is_empty());
    }
    #[test]
    fn reports_double_filter() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "x |> Enum.filter(& &1) |> Enum.filter(& &2)\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_pipe_chain_like_upstream() {
        // EX4008.upstream.violation: trigger is `|>`, column points at it.
        let src = "defmodule Credo.Sample.Module do\n  def some_function(p1, p2, p3, p4, p5) do\n    [\"a\", \"b\", \"c\"]\n    |> Enum.filter(&String.contains?(&1, \"x\"))\n    |> Enum.filter(&String.contains?(&1, \"a\"))\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (5, Some(5)));
    }
}
