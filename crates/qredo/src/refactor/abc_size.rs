use crate::Finding;
use std::collections::BTreeMap;

const ECTO_IMPORT: [&str; 4] = ["where", "from", "select", "join"];

/// `EX4001`: ABC size approximation (assignments/branches/calls).
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let max_raw = params.get("max_size").map_or("30", String::as_str);
    let max_size: f64 = max_raw.parse().unwrap_or(30.0);
    let mut excluded = parse_string_list(params.get("excluded_functions"));
    let masked = prepared.masked();
    if has_ecto_import(masked) {
        for fun in ECTO_IMPORT {
            if !excluded.contains(&fun.to_owned()) {
                excluded.push(fun.to_owned());
            }
        }
    }
    let pruned = prune_calls(masked, &excluded);
    let lines: Vec<&str> = pruned.split('\n').collect();
    let depths = line_depths(&lines);
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !is_def(trimmed) {
            continue;
        }
        let Some(name) = def_name(trimmed) else {
            continue;
        };
        if name == "__using__" {
            continue;
        }
        let body = def_body(&lines, &depths, idx);
        let (assignments, branches, conditions) = count_abc(&body, def_params(trimmed));
        #[allow(
            clippy::cast_precision_loss,
            reason = "ABC counts grow with source length; precision loss only affects gigantic inputs"
        )]
        let size = (saturated_squares(assignments, branches, conditions) as f64)
            .sqrt()
            .round();
        if size > max_size {
            findings.push(
                Finding::with_trigger(
                    idx + 1,
                    credo_column(&masked_line(masked, idx), &name),
                    format!("Function is too complex (ABC size is {size:.0}, max is {max_raw})."),
                    name,
                )
                .with_severity(crate::check_meta::severity(size, max_size)),
            );
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

fn masked_line(masked: &str, idx: usize) -> String {
    masked.split('\n').nth(idx).unwrap_or("").to_owned()
}

fn is_def(trimmed: &str) -> bool {
    ["def ", "defp ", "defmacro "]
        .iter()
        .any(|op| trimmed.starts_with(op))
}

fn def_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.split_whitespace().nth(1)?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Compact-JSON string array (`["where", "join"]`) to names.
fn parse_string_list(raw: Option<&String>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let raw = raw.trim();
    if !(raw.starts_with('[') && raw.ends_with(']')) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let bytes = raw.as_bytes();
    let mut idx = 1_usize;
    while idx < bytes.len() {
        if bytes[idx] == b'"' {
            let mut text = String::new();
            idx += 1;
            while idx < bytes.len() && bytes[idx] != b'"' {
                if bytes[idx] == b'\\' && idx + 1 < bytes.len() {
                    text.push(bytes[idx + 1] as char);
                    idx += 2;
                } else {
                    text.push(bytes[idx] as char);
                    idx += 1;
                }
            }
            idx += 1;
            out.push(text);
        } else {
            idx += 1;
        }
    }
    out
}

fn has_ecto_import(masked: &str) -> bool {
    let mut idx = 0_usize;
    while idx + 6 <= masked.len() {
        if masked
            .get(idx..)
            .is_some_and(|rest| rest.starts_with("import"))
            && word_boundary(masked.as_bytes(), idx)
            && word_boundary(masked.as_bytes(), idx + 6)
        {
            let rest = masked.get(idx + 6..).unwrap_or("").trim_start();
            if rest.starts_with("Ecto.Query")
                && rest.as_bytes().get(10).is_none_or(|b| !is_name_byte(*b))
            {
                return true;
            }
        }
        idx += 1;
    }
    false
}

/// Blank out excluded call spans (name plus balanced arguments, body kept).
fn prune_calls(masked: &str, excluded: &[String]) -> String {
    if excluded.is_empty() {
        return masked.to_owned();
    }
    let mut out = masked.as_bytes().to_vec();
    for name in excluded {
        let mut idx = 0_usize;
        while idx + name.len() <= out.len() {
            if masked.get(idx..).is_some_and(|rest| rest.starts_with(name))
                && word_boundary(masked.as_bytes(), idx)
                && word_boundary(masked.as_bytes(), idx + name.len())
            {
                let paren = skip_ws(masked, idx + name.len());
                if masked.as_bytes().get(paren) == Some(&b'(')
                    && let Some(close) = match_paren(masked, paren)
                {
                    blank_range(&mut out, masked, idx, close + 1);
                    idx = close + 1;
                    continue;
                }
            }
            idx += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| masked.to_owned())
}

fn blank_range(out: &mut [u8], masked: &str, start: usize, end: usize) {
    for idx in start..end.min(out.len()) {
        if masked.as_bytes()[idx] == b'\n' {
            out[idx] = b'\n';
        } else {
            out[idx] = b' ';
        }
    }
}

fn match_paren(text: &str, open: usize) -> Option<usize> {
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

fn skip_ws(text: &str, mut idx: usize) -> usize {
    while text
        .as_bytes()
        .get(idx)
        .is_some_and(u8::is_ascii_whitespace)
    {
        idx += 1;
    }
    idx
}

/// Depth before each line: block `do`/`fn` open, `end` closes.
fn line_depths(lines: &[&str]) -> Vec<usize> {
    let mut depths = Vec::with_capacity(lines.len());
    let mut depth = 0_usize;
    for line in lines {
        depths.push(depth);
        depth += count_word(line, "do") + count_word(line, "fn");
        depth = depth.saturating_sub(count_word(line, "end"));
    }
    depths
}

fn depth_after(lines: &[&str], depths: &[usize], idx: usize) -> usize {
    (depths[idx] + count_word(lines[idx], "do") + count_word(lines[idx], "fn"))
        .saturating_sub(count_word(lines[idx], "end"))
}

/// Body text of the function at `from` (head line excluded, like the AST walk).
fn def_body(lines: &[&str], depths: &[usize], from: usize) -> String {
    let line = lines[from];
    if let Some(pos) = line.find(", do:") {
        return line.get(pos + 5..).unwrap_or("").to_owned();
    }
    if depth_after(lines, depths, from) == depths[from] {
        return inline_body(line);
    }
    let end = span_end(lines, depths, from);
    lines.get((from + 1)..=end).unwrap_or(&[]).join("\n")
}

fn span_end(lines: &[&str], depths: &[usize], from: usize) -> usize {
    let base = depths[from];
    for idx in from + 1..lines.len() {
        if depth_after(lines, depths, idx) == base {
            return idx;
        }
    }
    lines.len() - 1
}

/// `def foo do bar end` on one line: text between `do` and the final `end`.
fn inline_body(line: &str) -> String {
    let Some(start) = bare_do_end(line) else {
        return String::new();
    };
    let Some(end) = line.rfind("end") else {
        return line.get(start..).unwrap_or("").to_owned();
    };
    if end <= start {
        return String::new();
    }
    line.get(start..end).unwrap_or("").to_owned()
}

fn bare_do_end(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut idx = 0_usize;
    while idx + 2 <= bytes.len() {
        if line.get(idx..).is_some_and(|rest| rest.starts_with("do"))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + 2)
            && bytes.get(idx + 2) != Some(&b':')
        {
            return Some(idx + 2);
        }
        idx += 1;
    }
    None
}

/// Simple parameter names from the head line (destructured heads contribute none).
fn def_params(trimmed: &str) -> Vec<String> {
    let Some(open) = trimmed.find('(') else {
        return Vec::new();
    };
    let Some(close) = match_paren(trimmed, open) else {
        return Vec::new();
    };
    split_args(trimmed.get(open + 1..close).unwrap_or(""))
        .into_iter()
        .filter_map(|part| bare_ident(part.trim()))
        .collect()
}

fn split_args(inside: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (idx, byte) in inside.bytes().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(inside.get(start..idx).unwrap_or(""));
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(inside.get(start..).unwrap_or(""));
    parts
}

/// The segment itself is one bare identifier (after one bracket layer).
fn bare_ident(segment: &str) -> Option<String> {
    let stripped = segment
        .trim_start_matches(['(', '[', '{'])
        .trim_end_matches([')', ']', '}'])
        .trim();
    let mut chars = stripped.chars();
    let first = chars.next()?;
    if !first.is_ascii_lowercase() {
        return None;
    }
    if !stripped
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
    {
        return None;
    }
    Some(stripped.to_owned())
}

fn saturated_squares(assignments: usize, branches: usize, conditions: usize) -> usize {
    assignments
        .saturating_mul(assignments)
        .saturating_add(branches.saturating_mul(branches))
        .saturating_add(conditions.saturating_mul(conditions))
}

/// `(assignments, branches, conditions)` over the function body.
fn count_abc(body: &str, params: Vec<String>) -> (usize, usize, usize) {
    let mut scope = params;
    collect_scope(body, &mut scope);
    let ranges = lhs_ranges(body);
    let mut counter = Counter {
        scope: &scope,
        ranges: &ranges,
        assignments: 0,
        branches: 0,
        conditions: 0,
    };
    counter.scan(body);
    (counter.assignments, counter.branches, counter.conditions)
}

/// All assignment target ranges in the body.
fn lhs_ranges(body: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut idx = 0_usize;
    while idx < body.len() {
        if is_plain_assign(body, idx)
            && let Some(range) = lhs_range(body, idx)
        {
            ranges.push(range);
        }
        idx += 1;
    }
    ranges
}

/// Assigned names and `->` head variables enter the variable scope.
fn collect_scope(body: &str, scope: &mut Vec<String>) {
    let bytes = body.as_bytes();
    let mut idx = 0_usize;
    while idx < bytes.len() {
        if is_plain_assign(body, idx) {
            if let Some((start, end)) = lhs_range(body, idx)
                && let Some(name) = bare_ident(body.get(start..end).unwrap_or(""))
                && !scope.contains(&name)
            {
                scope.push(name);
            }
            idx += 1;
        } else if body.get(idx..).is_some_and(|rest| rest.starts_with("->"))
            && word_boundary(bytes, idx + 2)
        {
            for name in head_vars(body_head_before(body, idx)) {
                if !scope.contains(&name) {
                    scope.push(name);
                }
            }
            idx += 2;
        } else {
            idx += 1;
        }
    }
}

fn body_head_before(body: &str, arrow: usize) -> &str {
    let bytes = body.as_bytes();
    let mut idx = arrow;
    let mut depth = 0_usize;
    while idx > 0 {
        idx -= 1;
        match bytes[idx] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' | b',' | b'|' | b'=' => {
                if depth == 0 {
                    return body.get(idx + 1..arrow).unwrap_or("");
                }
                if !matches!(bytes[idx], b'=' | b',') {
                    depth -= 1;
                }
            }
            b'\n' if depth == 0 => return body.get(idx + 1..arrow).unwrap_or(""),
            _ => {}
        }
        if depth == 0 && is_head_stop(body, idx) {
            return body.get(idx + 1..arrow).unwrap_or("");
        }
    }
    body.get(..arrow).unwrap_or("")
}

fn is_head_stop(body: &str, idx: usize) -> bool {
    for kw in ["fn", "do", "->"] {
        let Some(head) = body.get(..idx + 1) else {
            continue;
        };
        if !head.ends_with(kw) {
            continue;
        }
        if kw.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && head.len() > kw.len()
            && is_name_byte(head.as_bytes()[head.len() - kw.len() - 1])
        {
            continue;
        }
        return true;
    }
    false
}

fn head_vars(segment: &str) -> Vec<String> {
    let trimmed = segment.trim();
    let inner = strip_paren_layer(trimmed).unwrap_or(trimmed);
    let mut out = Vec::new();
    for part in split_args(inner) {
        if let Some(name) = bare_ident(part.trim()) {
            out.push(name);
        }
    }
    out
}

/// The `fn` argument parentheses enclosing the whole head, if present.
fn strip_paren_layer(segment: &str) -> Option<&str> {
    if !segment.starts_with('(') {
        return None;
    }
    let close = match_paren(segment, 0)?;
    if close + 1 != segment.len() {
        return None;
    }
    segment.get(1..close)
}

/// Assignment target range before `=` at `assign`: a trailing identifier
/// (`x = ...`, `a.b = ...`) or bracket group (`{a, b} = ...`). The target is
/// never traversed, so its variables contribute no branches.
fn lhs_range(body: &str, assign: usize) -> Option<(usize, usize)> {
    let bytes = body.as_bytes();
    let mut end = assign;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }
    if matches!(bytes[end - 1], b')' | b']' | b'}') {
        return match_bracket_back(body, end - 1).map(|open| (open, end));
    }
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    Some((start, end))
}

fn match_bracket_back(body: &str, close: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let (open, shut) = match bytes[close] {
        b')' => (b'(', b')'),
        b']' => (b'[', b']'),
        b'}' => (b'{', b'}'),
        _ => return None,
    };
    let mut depth = 0_usize;
    let mut idx = close + 1;
    while idx > 0 {
        idx -= 1;
        if bytes[idx] == shut {
            depth += 1;
        } else if bytes[idx] == open {
            depth -= 1;
            if depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'?' | b'!')
}

/// Plain `=` (not `==`, `!=`, `<=`, `>=`, `=>`, `=~`).
fn is_plain_assign(body: &str, idx: usize) -> bool {
    let bytes = body.as_bytes();
    if bytes.get(idx) != Some(&b'=') {
        return false;
    }
    if idx > 0 && matches!(bytes[idx - 1], b'=' | b'!' | b'<' | b'>' | b'~') {
        return false;
    }
    if matches!(bytes.get(idx + 1), Some(b'=' | b'>' | b'~')) {
        return false;
    }
    true
}

struct Counter<'a> {
    scope: &'a [String],
    ranges: &'a [(usize, usize)],
    assignments: usize,
    branches: usize,
    conditions: usize,
}

impl Counter<'_> {
    fn scan(&mut self, body: &str) {
        let bytes = body.as_bytes();
        let mut idx = 0_usize;
        while idx < bytes.len() {
            if self.scan_operator(body, &mut idx) {
                continue;
            }
            if self.scan_word(body, &mut idx) {
                continue;
            }
            idx += 1;
        }
    }

    /// Multi-character operators and punctuation; advances `idx` past a match.
    fn scan_operator(&mut self, body: &str, idx: &mut usize) -> bool {
        let bytes = body.as_bytes();
        let rest = body.get(*idx..).unwrap_or("");
        let mut matched: Option<&str> = None;
        for op in [
            "|>", "->", "<-", "=>", "==", "!=", "=~", "<=", ">=", "&&", "||", "<>", "++", "**",
            "//", "..",
        ] {
            if rest.starts_with(op) {
                matched = Some(op);
                break;
            }
        }
        if let Some(op) = matched {
            if !matches!(op, "|>" | "==") {
                self.branches += 1;
            }
            *idx += op.len();
            return true;
        }
        let byte = bytes[*idx];
        if byte == b'.' {
            if self.scan_dot(body, *idx) {
                *idx += 1;
                return true;
            }
            return false;
        }
        if matches!(byte, b'%' | b'^' | b'#') {
            *idx += 1;
            return true;
        }
        if matches!(
            byte,
            b'=' | b'@' | b'&' | b'|' | b'+' | b'-' | b'*' | b'/' | b'<' | b'>' | b'!'
        ) {
            return self.scan_sign(body, idx);
        }
        false
    }

    /// Single-character operator assignments, attributes and branches.
    fn scan_sign(&mut self, body: &str, idx: &mut usize) -> bool {
        match body.as_bytes()[*idx] {
            b'=' if is_plain_assign(body, *idx) => {
                self.assignments += 1;
            }
            b'@' if body
                .as_bytes()
                .get(*idx + 1)
                .is_some_and(|byte| is_name_byte(*byte)) =>
            {
                self.assignments += 1;
            }
            b'&' | b'|' | b'+' | b'-' | b'*' | b'/' | b'<' | b'>' | b'!' => {
                self.branches += 1;
            }
            _ => return false,
        }
        *idx += 1;
        true
    }

    /// Dot calls count unless the receiver is a bare variable, an alias path
    /// segment, or a float point. Returns whether the dot was consumed.
    fn scan_dot(&mut self, body: &str, idx: usize) -> bool {
        let bytes = body.as_bytes();
        if bytes.get(idx + 1).is_some_and(u8::is_ascii_digit)
            && idx > 0
            && bytes[idx - 1].is_ascii_digit()
        {
            return true;
        }
        if !bytes
            .get(idx + 1)
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            return false;
        }
        let mut start = idx;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            start -= 1;
        }
        if start == idx {
            // Call-result or literal receiver (`foo().bar`).
            self.branches += 1;
            return true;
        }
        let receiver = body.get(start..idx).unwrap_or("");
        if receiver
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        {
            // Alias path (`Foo.Bar`): no branch.
            if start > 0 && bytes[start - 1] == b'.' {
                return true;
            }
            self.branches += 1;
            return true;
        }
        // Bare-variable receiver (`user.name`): no branch (the variable
        // itself still counts when visited).
        true
    }

    /// Keywords, calls and variables; advances `idx` past a match.
    fn scan_word(&mut self, body: &str, idx: &mut usize) -> bool {
        let bytes = body.as_bytes();
        let byte = bytes[*idx];
        if !(byte.is_ascii_alphabetic() || byte == b'_') {
            return false;
        }
        let mut end = *idx;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'?' | b'!'))
        {
            end += 1;
        }
        let word = body.get(*idx..end).unwrap_or("");
        *idx = end;
        if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            return true;
        }
        if word.starts_with('_') {
            return true;
        }
        if is_reserved(word) {
            if matches!(word, "if" | "or") {
                self.conditions += 1;
            } else if matches!(
                word,
                "unless" | "for" | "try" | "case" | "cond" | "with" | "and" | "in" | "not" | "when"
            ) {
                self.branches += 1;
            }
            return true;
        }
        if prev_is_dot_or_colon(bytes, *idx - word.len()) {
            return true;
        }
        if start_is_module_attr(bytes, *idx - word.len()) {
            return true;
        }
        self.scan_call_tail(body, word, end);
        true
    }

    /// Local calls count; atom keys, assignment targets and known variables do not.
    fn scan_call_tail(&mut self, body: &str, word: &str, end: usize) {
        let start = end - word.len();
        if body
            .get(end..)
            .is_some_and(|rest| rest.trim_start().starts_with('('))
        {
            if word != "unquote" {
                self.branches += 1;
            }
        } else if !(body.get(end..).is_some_and(|rest| rest.starts_with(':'))
            || self.in_lhs_range(start)
            || self.scope.contains(&word.to_owned()))
        {
            self.branches += 1;
        }
    }

    /// Whether a word start sits inside an assignment target.
    fn in_lhs_range(&self, start: usize) -> bool {
        self.ranges
            .iter()
            .any(|(from, to)| *from <= start && start < *to)
    }
}

fn prev_is_dot_or_colon(bytes: &[u8], start: usize) -> bool {
    start > 0 && matches!(bytes[start - 1], b'.' | b':')
}

fn start_is_module_attr(bytes: &[u8], start: usize) -> bool {
    start > 0 && bytes[start - 1] == b'@'
}

fn is_reserved(word: &str) -> bool {
    matches!(
        word,
        "do" | "end"
            | "else"
            | "fn"
            | "if"
            | "unless"
            | "case"
            | "cond"
            | "with"
            | "for"
            | "try"
            | "and"
            | "or"
            | "in"
            | "not"
            | "when"
            | "true"
            | "false"
            | "nil"
            | "after"
            | "rescue"
            | "catch"
            | "unquote"
    )
}

fn count_word(line: &str, needle: &str) -> usize {
    let bytes = line.as_bytes();
    let mut count = 0_usize;
    let mut idx = 0_usize;
    while idx + needle.len() <= bytes.len() {
        if line.get(idx..).is_some_and(|rest| rest.starts_with(needle))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + needle.len())
        {
            if needle != "do" || bytes.get(idx + 2) != Some(&b':') {
                count += 1;
            }
            idx += needle.len();
        } else {
            idx += 1;
        }
    }
    count
}

fn word_boundary(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 || idx >= bytes.len() {
        return true;
    }
    !is_name_byte(bytes[idx]) || !is_name_byte(bytes[idx - 1])
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
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
    use std::fmt::Write as _;
    #[test]
    fn simple_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo(x), do: x\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_complex() {
        let mut src = String::from("def foo(a, b, c) do\n");
        for i in 0..40 {
            let _ = writeln!(src, "  x{i} = a + b + c + foo({i}) + bar({i}) + baz({i})");
        }
        src.push_str("end\n");
        assert!(!check_prepared(&crate::batch::Prepared::lazy(&src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn reports_exact_size_like_upstream() {
        // EX4001.upstream.violation: ABC size is 5, column points at the name.
        let src = "def some_function do\n  if true == true or false == 2 do\n    my_options = MyHash.create\n  end\n  my_options\n  |> Enum.each(fn(key, value) ->\n    IO.puts key\n    IO.puts value\n  end)\nend\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "3".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (1, Some(5)));
        assert_eq!(
            findings[0].message,
            "Function is too complex (ABC size is 5, max is 3)."
        );
    }
    #[test]
    fn excluded_functions_are_ignored() {
        let src = "def foo do\n  foo(1)\n  foo(2)\n  foo(3)\n  foo(4)\n  foo(5)\nend\n";
        let max: BTreeMap<String, String> = [("max_size".to_owned(), "3".to_owned())]
            .into_iter()
            .collect();
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &max).len(),
            1
        );
        let mut params = max;
        params.insert("excluded_functions".to_owned(), "[\"foo\"]".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
}
