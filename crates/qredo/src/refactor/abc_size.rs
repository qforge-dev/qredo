use crate::{Finding, helpers};
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
    // Upstream ignores string interpolation (`traverse_abc({:<<>>, _, _})`
    // drops binaries); blank it after masking (which keeps code visible
    // for other checks).
    let masked = helpers::mask_interpolation(masked);
    if has_ecto_import(&masked) {
        for fun in ECTO_IMPORT {
            if !excluded.contains(&fun.to_owned()) {
                excluded.push(fun.to_owned());
            }
        }
    }
    let pruned = prune_calls(&masked, &excluded);
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
        let (assignments, branches, conditions) = count_abc(&body, def_params(&lines, idx));

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
                    credo_column(&masked_line(&masked, idx), &name),
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
                && !continues_value(masked.as_bytes(), idx)
                && call_like_follows(masked, idx + name.len())
            {
                let paren = skip_ws(masked, idx + name.len());
                if masked.as_bytes().get(paren) == Some(&b'(')
                    && let Some(close) = match_paren(masked, paren)
                {
                    blank_range(&mut out, masked, idx, close + 1);
                    idx = close + 1;
                    continue;
                }
                // Paren-less call (`from x in T, where: ...`): blank the
                // statement through comma/bracket continuations. Upstream
                // prunes the whole call including arguments.
                let stop = bare_call_end(masked, idx).max(idx + 1);
                blank_range(&mut out, masked, idx, stop);
                idx = stop;
                continue;
            }
            idx += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| masked.to_owned())
}

/// Whether the name continues a value (`x.from`, `:from`, `@from`): never
/// an excluded call head.
fn continues_value(bytes: &[u8], idx: usize) -> bool {
    idx > 0 && matches!(bytes[idx - 1], b'.' | b':' | b'@')
}

/// Whether a call follows the name: a word character, quote, sigil or
/// opening bracket (a bare variable of the same spelling is not a call).
fn call_like_follows(masked: &str, mut idx: usize) -> bool {
    let bytes = masked.as_bytes();
    while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
        idx += 1;
    }
    bytes.get(idx).is_some_and(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'_' | b'"' | b'\'' | b'~' | b'(' | b'[' | b'{')
    })
}

/// End of a paren-less excluded call: newlines end it unless brackets are
/// open or the line continues with `,`; `|>` pipes, `;`, stray closers
/// and block `do`/`fn`/`end` (blanking those would corrupt depth tracking)
/// end it too.
fn bare_call_end(text: &str, from: usize) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0_i64;
    let mut idx = from;
    let mut line_start = from;
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return idx;
                }
                depth -= 1;
            }
            b';' if depth == 0 => return idx,
            b'\n' => {
                if depth == 0 && !text.get(line_start..idx).is_some_and(ends_comma) {
                    return idx;
                }
                line_start = idx + 1;
            }
            _ => {}
        }
        if depth == 0 && is_block_word(text, idx) {
            return idx;
        }
        if depth == 0 && text.get(idx..).is_some_and(|rest| rest.starts_with("|>")) {
            return idx;
        }
        idx += 1;
    }
    bytes.len()
}

/// Whether the line continues onto the next one (trailing comma).
fn ends_comma(line: &str) -> bool {
    line.trim_end().ends_with(',')
}

/// Whether a block `do`/`fn`/`end` (never `do:`) starts at `idx`.
fn is_block_word(text: &str, idx: usize) -> bool {
    let bytes = text.as_bytes();
    ["do", "fn", "end"].iter().any(|word| {
        text.get(idx..).is_some_and(|rest| rest.starts_with(word))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + word.len())
            && bytes.get(idx + word.len()) != Some(&b':')
    })
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
    // A head continued below (`...,\n    do: ...`, or a guard `when`
    // opening below) carries an inline body on its continuation line when
    // a depth-zero `do:` precedes any bare block opener: first `do:` wins.
    if (head_continued(line) || guard_continued(lines, from))
        && let Some(body) = continued_do_body(lines, from)
    {
        return body;
    }
    if depth_after(lines, depths, from) == depths[from] {
        // No block opens on the head line: a one-liner body, or a
        // continued head (`...,\n    do:`, guard `when` below) whose body
        // starts after a later `do`.
        if let Some(open) = block_opener(lines, depths, from) {
            let end = span_end(lines, depths, open);
            return lines.get((open + 1)..=end).unwrap_or(&[]).join("\n");
        }
        return inline_body(line);
    }
    let end = span_end(lines, depths, from);
    lines.get((from + 1)..=end).unwrap_or(&[]).join("\n")
}

/// Inline body after a continued head's depth-zero `do:` (`...,\n do:`),
/// if the head never opens a block.
fn continued_do_body(lines: &[&str], from: usize) -> Option<String> {
    let mut depth = 0_i64;
    for line in lines.iter().skip(from) {
        let bytes = line.as_bytes();
        let mut idx = 0_usize;
        while idx < bytes.len() {
            match bytes[idx] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            if depth == 0
                && line[idx..].starts_with("do")
                && line[idx + 2..].starts_with(':')
                && word_boundary(bytes, idx)
                && word_boundary(bytes, idx + 2)
            {
                return line.get(idx + 3..).map(ToOwned::to_owned);
            }
            // A bare block opener wins over any later inline body.
            if depth == 0
                && line[idx..].starts_with("do")
                && !line[idx + 2..].starts_with(':')
                && word_boundary(bytes, idx)
                && word_boundary(bytes, idx + 2)
            {
                return None;
            }
            idx += 1;
        }
        // Stop at dedent: the head is over without a body opener.
        if depth < 0 {
            return None;
        }
    }
    None
}

/// First later line opening a continued head's block, if the head at
/// `from` keeps going below its first line: unbalanced brackets, a
/// trailing comma/operator, or a guard `when` opening below.
fn block_opener(lines: &[&str], depths: &[usize], from: usize) -> Option<usize> {
    if !head_continued(lines[from]) && !guard_continued(lines, from) {
        return None;
    }
    let mut idx = from + 1;
    while idx < lines.len() {
        if depth_after(lines, depths, idx) > depths[idx] {
            return Some(idx);
        }
        if depths[idx] < depths[from] {
            return None;
        }
        idx += 1;
    }
    None
}

/// Whether the head at `from` continues with a guard `when` clause on a
/// following line (`...)\n    when ... do`): only the first non-blank
/// line below decides.
fn guard_continued(lines: &[&str], from: usize) -> bool {
    let Some(next) = lines
        .iter()
        .skip(from + 1)
        .map(|line| line.trim_start())
        .find(|trimmed| !trimmed.is_empty())
    else {
        return false;
    };
    next.starts_with("when")
        && next["when".len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '?' && c != '!')
}
fn head_continued(line: &str) -> bool {
    let mut depth = 0_i64;
    for byte in line.bytes() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
    }
    if depth > 0 {
        return true;
    }
    let trimmed = line.trim_end();
    if trimmed.ends_with(',') {
        return true;
    }
    if trimmed.as_bytes().last().is_some_and(|byte| {
        matches!(
            byte,
            b'+' | b'-'
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
        )
    }) {
        return true;
    }
    trimmed
        .split_whitespace()
        .last()
        .is_some_and(|word| matches!(word, "and" | "or" | "not" | "in"))
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

/// Parameter names from the (possibly multi-line) head at `from`.
/// Guarded heads scope nothing upstream (`when` hides every parameter);
/// destructured heads contribute none.
fn def_params(lines: &[&str], from: usize) -> Vec<String> {
    let head = head_text(lines, from);
    // Guarded heads scope nothing upstream (`get_parameters` sees the
    // `when` wrapper, not the head tuple). The guard may sit on the def
    // line itself (past `head_text`'s balanced close) or open below it.
    if has_guard(&head) || has_guard(lines.get(from).unwrap_or(&"")) || guard_continued(lines, from)
    {
        return Vec::new();
    }
    let Some(open) = head.find('(') else {
        return Vec::new();
    };
    let Some(close) = match_paren(&head, open) else {
        return Vec::new();
    };
    split_args(head.get(open + 1..close).unwrap_or(""))
        .into_iter()
        .filter_map(|part| bare_ident(part.trim()))
        .collect()
}

/// Head lines at `from`, joined while brackets stay open.
fn head_text(lines: &[&str], from: usize) -> String {
    let mut text = lines[from].to_owned();
    let mut depth = bracket_depth(&text);
    let mut idx = from;
    while depth > 0 && idx + 1 < lines.len() && idx - from < 32 {
        idx += 1;
        text.push('\n');
        text.push_str(lines[idx]);
        depth += bracket_depth(lines[idx]);
    }
    text
}

fn bracket_depth(line: &str) -> i64 {
    let mut depth = 0_i64;
    for byte in line.bytes() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Whether the head carries a `when` guard clause.
fn has_guard(head: &str) -> bool {
    let bytes = head.as_bytes();
    let mut idx = 0_usize;
    while idx + 4 <= bytes.len() {
        if head.get(idx..).is_some_and(|rest| rest.starts_with("when"))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + 4)
        {
            return true;
        }
        idx += 1;
    }
    false
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

/// The segment itself is one bare identifier (after transparent parens).
/// Brackets and braces are NOT transparent upstream (`{v0} = ...` binds
/// nothing observable; only `(v0) = ...` scopes `v0`), so they disqualify.
fn bare_ident(segment: &str) -> Option<String> {
    let stripped = segment.trim_start_matches('(').trim_end_matches(')').trim();
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
    let body = blank_bitstrings(body);
    // Bindings carry their byte offset: like the native top-down
    // accumulator, a use counts unless a binding at or before it scopes
    // the name. Head params bind at 0 (the native initial scope).
    let mut scope: Vec<(String, usize)> = params.into_iter().map(|name| (name, 0)).collect();
    collect_scope(&body, &mut scope);
    let ranges = lhs_ranges(&body);
    let mut counter = Counter {
        scope: &scope,
        ranges: &ranges,
        assignments: 0,
        branches: 0,
        conditions: 0,
    };
    counter.scan(&body);
    (counter.assignments, counter.branches, counter.conditions)
}

/// Blank balanced `<<...>>` bitstring spans: upstream prunes them before
/// traversal. A `<<` continuing an expression (`a << b`) is a bit-shift
/// operator and is left alone.
fn blank_bitstrings(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = body.as_bytes().to_vec();
    let mut idx = 0_usize;
    while idx < bytes.len() {
        if bytes[idx] == b'<'
            && bytes.get(idx + 1) == Some(&b'<')
            && !continues_expression(bytes, idx)
        {
            let mut depth = 0_usize;
            let mut close = idx;
            while close + 1 < bytes.len() {
                if bytes[close] == b'<' && bytes.get(close + 1) == Some(&b'<') {
                    depth += 1;
                    close += 2;
                } else if bytes[close] == b'>' && bytes.get(close + 1) == Some(&b'>') {
                    depth -= 1;
                    close += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    close += 1;
                }
            }
            if depth == 0 {
                blank_span(&mut out, idx, close);
                idx = close;
                continue;
            }
        }
        idx += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| body.to_owned())
}

/// Whether `<<` at `idx` continues an expression (bit-shift): the previous
/// non-blank byte on the same line can end a value.
fn continues_expression(bytes: &[u8], idx: usize) -> bool {
    let mut back = idx;
    while back > 0 {
        back -= 1;
        if bytes[back] == b'\n' {
            return false;
        }
        if !bytes[back].is_ascii_whitespace() {
            return bytes[back].is_ascii_alphanumeric()
                || matches!(
                    bytes[back],
                    b'_' | b'?' | b'!' | b')' | b']' | b'}' | b'"' | b'\''
                );
        }
    }
    false
}

/// Spaces (newlines kept) over `[start, end)`.
fn blank_span(out: &mut [u8], start: usize, end: usize) {
    let len = out.len();
    for slot in &mut out[start..end.min(len)] {
        if *slot != b'\n' {
            *slot = b' ';
        }
    }
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

/// Whether `name` is bound at or before byte `at`.
fn bound_at(scope: &[(String, usize)], name: &str, at: usize) -> bool {
    scope
        .iter()
        .any(|(known, bound)| known == name && *bound <= at)
}

/// Assigned names and `->` head variables enter the variable scope.
/// `=` inside a `->` head never scopes (its whole head is discarded
/// upstream), so those are skipped via their head spans.
fn collect_scope(body: &str, scope: &mut Vec<(String, usize)>) {
    let heads = arrow_head_spans(body);
    let bytes = body.as_bytes();
    let mut idx = 0_usize;
    while idx < bytes.len() {
        if is_plain_assign(body, idx) {
            if !heads.iter().any(|(start, end)| *start <= idx && idx < *end)
                && let Some((start, end)) = lhs_range(body, idx)
                && let Some(name) = bare_ident(body.get(start..end).unwrap_or(""))
                && !scope.iter().any(|(known, _)| known == &name)
            {
                scope.push((name, idx));
            }
            idx += 1;
        } else if body.get(idx..).is_some_and(|rest| rest.starts_with("->"))
            && word_boundary(bytes, idx + 2)
        {
            let start = head_start_idx(body, idx);
            // Bind at the head start (not the arrow): native visits the
            // `->` node — adding its head vars — before descending into
            // the head patterns themselves.
            for name in head_vars(body.get(start..idx).unwrap_or("")) {
                if !scope.iter().any(|(known, _)| known == &name) {
                    scope.push((name, start));
                }
            }
            idx += 2;
        } else {
            idx += 1;
        }
    }
}

/// `[head_start, arrow)` spans of every `->` in the body.
fn arrow_head_spans(body: &str) -> Vec<(usize, usize)> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0_usize;
    while idx + 2 <= bytes.len() {
        if body.get(idx..).is_some_and(|rest| rest.starts_with("->"))
            && word_boundary(bytes, idx + 2)
        {
            out.push((head_start_idx(body, idx), idx));
            idx += 2;
        } else {
            idx += 1;
        }
    }
    out
}

/// Start of the `->` head at `arrow`: back over balanced brackets and
/// commas to an opening bracket, head keyword, `;` or newline. Commas
/// never stop the scan: every `fn` parameter scopes.
fn head_start_idx(body: &str, arrow: usize) -> usize {
    let bytes = body.as_bytes();
    let mut idx = arrow;
    let mut depth = 0_usize;
    while idx > 0 {
        idx -= 1;
        match bytes[idx] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' => {
                if depth == 0 {
                    return idx + 1;
                }
                depth -= 1;
            }
            b'\n' | b';' if depth == 0 => return idx + 1,
            _ => {}
        }
        if depth == 0 && is_head_stop(body, idx) {
            return idx + 1;
        }
    }
    0
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
    // A match head (`=` at depth zero) scopes nothing upstream: the
    // whole head node maps to nil.
    if has_top_assign(segment) {
        return Vec::new();
    }
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

/// Whether the segment holds `=` outside brackets.
fn has_top_assign(segment: &str) -> bool {
    let mut depth = 0_usize;
    for byte in segment.bytes() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'=' if depth == 0 => return true,
            _ => {}
        }
    }
    false
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
        return match_bracket_back(body, end - 1)
            .map(|open| (struct_prefix_start(body, open).unwrap_or(open), end));
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

/// Start of a `%Alias` prefix immediately before the `{` at `open`, if
/// any: `%E{...}` patterns belong to the match LHS whole, mirroring
/// upstream (which never visits `=` left-hand sides).
fn struct_prefix_start(body: &str, open: usize) -> Option<usize> {
    if body.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let bytes = body.as_bytes();
    let mut start = open;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == open || bytes.get(start.wrapping_sub(1)) != Some(&b'%') {
        return None;
    }
    Some(start - 1)
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
    scope: &'a [(String, usize)],
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
}

/// Longest compound operator (`=>` pairs are plain tuples upstream:
/// consumed, never counted; `->` stays a branch) at a rest slice.
fn match_compound_op(rest: &str) -> Option<&'static str> {
    [
        "|>", "->", "<-", "=>", "==", "!=", "=~", "<=", ">=", "&&", "||", "<>", "++", "**", "//",
        "..",
    ]
    .into_iter()
    .find(|op| rest.starts_with(op))
}

impl Counter<'_> {
    /// Multi-character operators and punctuation; advances `idx` past a match.
    fn scan_operator(&mut self, body: &str, idx: &mut usize) -> bool {
        let bytes = body.as_bytes();
        let rest = body.get(*idx..).unwrap_or("");
        if let Some(op) = match_compound_op(rest) {
            if !matches!(op, "|>" | "==" | "=>") {
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
        // `a["k"]` is `Access.get`: every other `[` opens a literal.
        if byte == b'[' && is_index_target(body, *idx) {
            self.branches += 1;
            *idx += 1;
            return true;
        }
        if matches!(byte, b'%' | b'^' | b'#') {
            // Named structs (`%Foo{}`) are calls upstream; `%{}` is not.
            // Inside `=` patterns neither counts (upstream never visits
            // match left-hand sides).
            if byte == b'%'
                && bytes
                    .get(*idx + 1)
                    .is_some_and(|next| next.is_ascii_uppercase() || *next == b'_')
                && !self.in_lhs_range(*idx)
            {
                self.branches += 1;
            }
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
    /// A lone `|` inside a `=` pattern is invisible upstream (match
    /// left-hand sides are never visited); `||` cannot occur there.
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
            b'|' if self.in_lhs_range(*idx) => {}
            b'&' | b'|' | b'+' | b'-' | b'*' | b'/' | b'<' | b'>' | b'!' => {
                self.branches += 1;
            }
            _ => return false,
        }
        *idx += 1;
        true
    }

    /// Dots mirror upstream dot heads: every `.` counts once, except a
    /// bare-variable receiver (`a.b`) and alias-path middles (`A.B.c`
    /// counts once for the whole path). Call chains (`a.b.c`) count every
    /// dot past the first.
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
        let continued = start > 0 && bytes[start - 1] == b'.';
        // Dotted captures (`& &1.k`): the counter is no bare variable.
        if !receiver.is_empty()
            && receiver.bytes().all(|byte| byte.is_ascii_digit())
            && capture_before(bytes, start)
        {
            self.branches += 1;
            return true;
        }
        if !receiver
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        {
            // Bare-variable receiver: silent at path start, the counted
            // outer head of a call chain (`a.b.c`) afterwards.
            if continued {
                self.branches += 1;
            }
            return true;
        }
        // Alias-path middle (`A.B.c`): already decided at path start.
        if continued {
            return true;
        }
        if alias_path_call(bytes, idx) {
            self.branches += 1;
        }
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
        // Inside a numeric literal (`0x0FFF`): integers count nothing.
        if *idx > word.len() && bytes[*idx - word.len() - 1].is_ascii_digit() {
            return true;
        }
        if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            return true;
        }
        if word.starts_with('_') {
            return true;
        }
        if prev_is_dot_or_colon(bytes, *idx - word.len()) {
            return true;
        }
        if start_is_module_attr(bytes, *idx - word.len()) {
            return true;
        }
        // Keyword keys (`case: :lower`) and atom literals are tuple
        // elements upstream: never branches, not even reserved words.
        if follows_colon(body, end) {
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
            || bound_at(self.scope, word, start))
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

/// Whether the alias path starting at the dot `idx` continues into a
/// counted call head: swallow every uppercase `.Segment`; a lowercase
/// `.fun` continuing the path (or directly) counts.
fn alias_path_call(bytes: &[u8], idx: usize) -> bool {
    let mut tail = idx;
    loop {
        let mut seg = tail + 1;
        while seg < bytes.len() && (bytes[seg].is_ascii_alphanumeric() || bytes[seg] == b'_') {
            seg += 1;
        }
        if seg == tail + 1 {
            return false;
        }
        if !bytes[tail + 1].is_ascii_uppercase() {
            return true;
        }
        if bytes.get(seg) == Some(&b'.') && bytes.get(seg + 1).is_some_and(u8::is_ascii_uppercase) {
            tail = seg;
            continue;
        }
        // Path ends here: count only a lowercase call continuation.
        return bytes.get(seg) == Some(&b'.')
            && bytes
                .get(seg + 1)
                .is_some_and(|b| b.is_ascii_lowercase() || *b == b'_');
    }
}

/// Whether the word ending at `end` is a keyword key (`key:` but not `::`).
fn follows_colon(body: &str, end: usize) -> bool {
    let bytes = body.as_bytes();
    bytes.get(end) == Some(&b':') && bytes.get(end + 1) != Some(&b':')
}

/// Whether `[` at `idx` indexes a value (`Access.get` upstream): the
/// previous value-ending byte, with keyword-led lines (`do [..]`) ruled
/// out. Anything else opens a list literal.
fn is_index_target(body: &str, idx: usize) -> bool {
    let bytes = body.as_bytes();
    let mut back = idx;
    // Same-line blanks only: `x\n[y]` is two expressions upstream, never
    // `Access.get` (which the AST only forms on one line).
    while back > 0 && matches!(bytes[back - 1], b' ' | b'\t' | b'\r') {
        back -= 1;
    }
    if back == 0 {
        return false;
    }
    let prev = bytes[back - 1];
    if prev.is_ascii_alphanumeric() || matches!(prev, b'_' | b'?' | b'!') {
        let mut start = back - 1;
        while start > 0 && is_ident_byte(bytes[start - 1]) {
            start -= 1;
        }
        // The whole preceding word (`start..back`): dropping its last
        // byte turned every reserved word (`do`, `end`, `if`, ...) into a
        // non-reserved prefix, miscounting pattern brackets as access.
        let word = body.get(start..back).unwrap_or("");
        return !is_reserved(word);
    }
    matches!(prev, b')' | b']' | b'}' | b'"' | b'\'')
}

/// Whether the digit receiver at `start` is an `&N` capture counter: a
/// lone `&` (never `&&`) stands before it across blanks.
fn capture_before(bytes: &[u8], start: usize) -> bool {
    let mut back = start;
    while back > 0 && matches!(bytes[back - 1], b' ' | b'\t') {
        back -= 1;
    }
    back > 0 && bytes[back - 1] == b'&' && (back < 2 || bytes[back - 2] != b'&')
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
    fn bracketed_destructuring_binds_nothing() {
        // Upstream `var_name/1` only scopes bare `{name, _, nil}` (parens
        // transparent): `{v0} = ...` leaves `v0` unknown, so later uses
        // count as branches.
        let src = "def f() do\n  {v0} = pin(v0)\n  v0\nend\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "2".to_owned())]
            .into_iter()
            .collect();
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params).len(),
            1
        );
        // ...while `(v0) = ...` scopes normally and stays clean.
        let src = "def f() do\n  (v0) = pin(v0)\n  v0\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn match_pattern_structs_are_invisible() {
        // Upstream descends only into `=` right-hand sides: a struct on
        // the pattern side adds no branch (native size 2 here).
        let src = "def f(a) do\n  %E{} = x\n  x\nend\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "2".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn match_pattern_cons_is_invisible() {
        // Same for `|` inside `=` patterns (native size 3 here).
        let src = "def f(a) do\n  [h | t] = foo(a)\n  {h, t}\nend\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "3".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn use_before_binding_counts() {
        // Scoping is positional like the native accumulator: a use before
        // its `=`-binding counts (native sizes 5 and 6 here).
        for (src, max, size) in [
            (
                "def f(a) do\n  with {:ok, m} <- g(a) do\n    m = h(m)\n    m\n  end\nend\n",
                "4",
                "ABC size is 5",
            ),
            (
                "def f(a) do\n  with {:ok, m} <- g(a) do\n    x = h(m)\n    x\n  end\nend\n",
                "5",
                "ABC size is 6",
            ),
        ] {
            let params: BTreeMap<String, String> = [("max_size".to_owned(), max.to_owned())]
                .into_iter()
                .collect();
            let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
            assert_eq!(findings.len(), 1, "{src:?}");
            assert!(findings[0].message.contains(size), "{src:?}");
        }
    }

    #[test]
    fn list_pattern_bracket_is_not_index_access() {
        // `[]` after a reserved word (`do`) is a pattern, not `Access.get`
        // (native sizes 3 and 5 here).
        for (src, max) in [
            (
                "def f(c) do\n  case g(c) do\n    [] -> 1\n  end\nend\n",
                "3",
            ),
            (
                "def f(c) do\n  case g(c) do\n    [build] -> build\n  end\nend\n",
                "5",
            ),
        ] {
            let params: BTreeMap<String, String> = [("max_size".to_owned(), max.to_owned())]
                .into_iter()
                .collect();
            assert!(
                check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty(),
                "{src:?}"
            );
        }
    }

    #[test]
    fn bracket_after_newline_is_not_index_access() {
        // `1\n[]` across a newline is two expressions, never `Access.get`
        // (native size 5 here): index lookup stays on its line.
        let src = "def f(c) do\n  case g(c) do\n    [build] -> 1\n    [] -> 2\n  end\nend\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "5".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }

    #[test]
    fn guard_continued_head_counts_body() {
        // Guard `when` opening below the head line still scopes the body.
        let src = "def invoke(a, b) when is_binary(a) and b != \"\" do\n    x = Req.post(a)\n    x\n  end\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "0".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn guard_inline_body_does_not_swallow_next_clause() {
        // A guard-continued head with an inline `do:` body ends there: the
        // following clause is a separate function, not its body.
        let src = "defp validate_source(_d, checkpoint, asset)\n     when not is_nil(checkpoint) and not is_nil(asset),\n     do: {:error, :ambiguous_source}\n\ndefp validate_source(definition, checkpoint, nil) do\n  x = Models.get_version(definition)\n  x\nend\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "0".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 5);
    }

    #[test]
    fn single_line_guard_scopes_nothing() {
        // The guard may sit past `head_text`'s balanced close; params
        // still stay unscoped, so later uses count.
        let src = "def f(v) when is_binary(v) do\n    Models.get_version(v)\n  end\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "1".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("ABC size is 2"));
    }

    #[test]
    fn multiline_guard_scopes_nothing() {
        // A `when` opening below the head line still wraps the head
        // upstream: params stay unscoped, so later uses count.
        let src = "def f(v)\n    when is_binary(v) do\n    Models.get_version(v)\n  end\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "1".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("ABC size is 2"));
    }

    #[test]
    fn continued_head_inline_body_counts() {
        // `,\n    do:` carries the body on the continuation line.
        let src =
            "defp fetch_cursor(raw, collection, view),\n    do: decode(raw, collection, view)\n";
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "0".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("ABC size is 1"));
    }

    #[test]
    fn interpolation_calls_do_not_count() {
        // Upstream ignores string interpolation (`:<<>>` binaries): calls
        // inside `"#{...}"` add no branches.
        let src = "def f(a, b) do\n  x = \"#{encode(a)}=#{encode(b)}\"\n  y = g(x) |> h()\n  {x, y}\nend\n";
        // Without interpolation blanking the two `encode/1` calls push
        // size from 3 to 6; with it the function stays at 3.
        for max in ["3", "4", "5"] {
            let params: BTreeMap<String, String> = [("max_size".to_owned(), max.to_owned())]
                .into_iter()
                .collect();
            assert!(
                check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty(),
                "max_size {max}"
            );
        }
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

    /// Six assignment-heavy pad lines shared by the parity probes below.
    fn pads() -> String {
        use std::fmt::Write as _;
        let mut pads = String::new();
        for i in 1..=3 {
            let _ = writeln!(
                pads,
                "    t{i} = a + b + c + foo({i}) + bar({i}) + baz({i})"
            );
        }
        for i in 1..=3 {
            let _ = writeln!(pads, "    u{i} = a + b + c + foo({i}) + bar({i})");
        }
        pads
    }

    fn size_of(src: &str) -> String {
        let params: BTreeMap<String, String> = [("max_size".to_owned(), "0".to_owned())]
            .into_iter()
            .collect();
        let found = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(found.len(), 1);
        found[0].message.clone()
    }

    #[test]
    fn fat_arrow_pairs_are_not_branches() {
        // ABC-O4: `=>` pairs are plain tuples upstream.
        let src = format!(
            "defmodule M do\n  def f(a, b, c) do\n{}    m1 = %{{\"k1\" => a, \"k2\" => b}}\n    m2 = %{{\"k3\" => c, \"k4\" => a}}\n  end\nend\n",
            pads()
        );
        assert_eq!(
            size_of(&src),
            "Function is too complex (ABC size is 43, max is 0)."
        );
    }
    #[test]
    fn unparenthesized_ecto_calls_are_pruned() {
        // ABC-O1: excluded calls prune the whole call without parens too.
        let src = "defmodule M do\n  import Ecto.Query\n  def f(x) do\n    q1 = from u in User, where: u.id == x, select: u\n    q2 = from u in User, where: u.id == x, select: u\n    q3 = from u in User, where: u.id == x, select: u\n  end\nend\n";
        assert_eq!(
            size_of(src),
            "Function is too complex (ABC size is 3, max is 0)."
        );
    }
    #[test]
    fn multi_arg_fn_heads_scope_every_param() {
        // ABC-O5: `fn event, acc ->` scopes both parameters.
        let src = "defmodule M do\n  def f(l) do\n    r1 = Enum.reduce(l, 0, fn event, acc -> event + acc end)\n    r2 = Enum.reduce(l, 0, fn event, acc -> event + acc end)\n    r3 = Enum.reduce(l, 0, fn event, acc -> event + acc end)\n  end\nend\n";
        assert_eq!(
            size_of(src),
            "Function is too complex (ABC size is 9, max is 0)."
        );
    }
    #[test]
    fn bitstrings_kwargs_and_hex_are_plain() {
        // ABC-O2/O3/O6: `<<>>` pruned, reserved kwargs and hex plain.
        let src = format!(
            "defmodule M do\n  def f(a, b, c) do\n{}    <<first::binary-size(2), rest::binary>> = a\n    x = Base.encode16(b, case: :lower)\n    y = Base.encode16(c, case: :lower)\n    d = 0x0FFF\n    e = 0x5000\n  end\nend\n",
            pads()
        );
        assert_eq!(
            size_of(&src),
            "Function is too complex (ABC size is 45, max is 0)."
        );
    }
    #[test]
    fn bare_alias_paths_are_not_calls() {
        // ABC-O7: `Foo.Bar` without a call is an alias, not a branch.
        let src = format!(
            "defmodule M do\n  def f(a, b, c) do\n{}    bar(Foo.Bar)\n    bar(Baz.Qux)\n    bar(Quux.Corge)\n    bar(Grault.Garply)\n  end\nend\n",
            pads()
        );
        assert_eq!(
            size_of(&src),
            "Function is too complex (ABC size is 46, max is 0)."
        );
    }
    #[test]
    fn multiline_def_head_keeps_its_body() {
        // ABC-U7: a continued head must not empty the body.
        let src = "defmodule M do\n  def f(\n    a,\n    b\n  ) when is_list(a) do\n    t1 = a + b + c + foo(1) + bar(1) + baz(1)\n    t2 = a + b + c + foo(2) + bar(2) + baz(2)\n    t3 = a + b + c + foo(3) + bar(3) + baz(3)\n    t4 = a + b + c + foo(4) + bar(4) + baz(4)\n    t5 = a + b + c + foo(5) + bar(5) + baz(5)\n    t6 = a + b + c + foo(6) + bar(6) + baz(6)\n  end\nend\n";
        assert_eq!(
            size_of(src),
            "Function is too complex (ABC size is 66, max is 0)."
        );
    }
    #[test]
    fn guard_head_params_stay_unscoped() {
        // ABC-U1: `when` heads scope nothing upstream.
        let src = format!(
            "defmodule M do\n  def f(attrs, x) when is_map(attrs) do\n    a = attrs\n    b = attrs\n    c = attrs\n    d = attrs\n{}  end\nend\n",
            pads()
        );
        assert_eq!(
            size_of(&src),
            "Function is too complex (ABC size is 47, max is 0)."
        );
    }
    #[test]
    fn bracket_access_is_a_call() {
        // ABC-U2: `a["k"]` is `Access.get`.
        let src = format!(
            "defmodule M do\n  def f(a, b, c) do\n{}    v1 = a[\"k1\"]\n    v2 = a[\"k2\"]\n    v3 = a[\"k3\"]\n    v4 = a[\"k4\"]\n  end\nend\n",
            pads()
        );
        assert_eq!(
            size_of(&src),
            "Function is too complex (ABC size is 47, max is 0)."
        );
    }
    #[test]
    fn capture_receivers_are_calls() {
        // ABC-U3: `&1.k` is a dotted capture, not a bare variable.
        let src = format!(
            "defmodule M do\n  def f(a, b, c) do\n{}    r = Enum.map(a, & &1.k)\n    s = Enum.map(b, & &1.k)\n    t = Enum.map(c, & &1.k)\n    u = Enum.map(a, & &1.j)\n  end\nend\n",
            pads()
        );
        assert_eq!(
            size_of(&src),
            "Function is too complex (ABC size is 59, max is 0)."
        );
    }
    #[test]
    fn arrow_head_matches_stay_unscoped() {
        // ABC-U4: `=` inside `->` heads scopes nothing upstream.
        let src = format!(
            "defmodule M do\n  def f(l) do\n{}    r = Enum.map(l, fn {{:ok, x}} = y -> y end)\n    s = Enum.map(l, fn {{:ok, x}} = y -> y end)\n  end\nend\n",
            pads()
        );
        assert_eq!(
            size_of(&src),
            "Function is too complex (ABC size is 69, max is 0)."
        );
    }
    #[test]
    fn named_structs_are_calls() {
        // ABC-U5: `%Foo{}` is a call upstream.
        let src = format!(
            "defmodule M do\n  def f(a, b, c) do\n{}    s1 = %T{{k: a}}\n    s2 = %T{{k: b}}\n    s3 = %T{{k: c}}\n    s4 = %T{{k: a}}\n  end\nend\n",
            pads()
        );
        assert_eq!(
            size_of(&src),
            "Function is too complex (ABC size is 47, max is 0)."
        );
    }
}
