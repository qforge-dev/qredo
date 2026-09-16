use crate::Finding;

/// `EX3022`: avoid a single pipe into a `case`/`if` block.
///
/// Flags `value |> case do` / `step |> if do` chains whose left side holds
/// fewer than two pipes and feeds a plain value, map, list, or one call.
/// Longer chains (`a |> f |> g |> case do`, block piped further) stay clean.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let masked_lines: Vec<&str> = masked.split('\n').collect();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in masked_lines.iter().enumerate() {
        if let Some(block) = block_pipe(line) {
            inspect_chain(&masked_lines, &raw_lines, idx, &block, &mut findings);
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

struct BlockPipe {
    /// Byte offset of the `|>` feeding the block.
    pipe: usize,
    /// `case` or `if`.
    marker: &'static str,
}

/// A same-line `|> case do` / `|> if do` block opener, if present.
fn block_pipe(line: &str) -> Option<BlockPipe> {
    for (pipe, _) in line.match_indices("|>") {
        // `pipe` is a match offset and `"|>"` ASCII: slicing is boundary-safe.
        let after_marker = line[pipe + "|>".len()..].trim_start();
        for marker in ["case", "if"] {
            if let Some(rest) = after_marker.strip_prefix(marker)
                && rest.chars().next().is_none_or(|next| !is_name_char(next))
                && let Some(trailer) = rest.trim_start().strip_prefix("do")
                && trailer
                    .chars()
                    .next()
                    .is_none_or(|next| !is_name_char(next) && next != ':')
                && trailer.trim().is_empty()
            {
                return Some(BlockPipe { pipe, marker });
            }
        }
    }
    None
}

/// Decide whether the chain around line `idx` earns an issue.
fn inspect_chain(
    masked: &[&str],
    raw: &[&str],
    idx: usize,
    block: &BlockPipe,
    findings: &mut Vec<Finding>,
) {
    let back = chain_back(masked, idx, block.pipe);
    let forward = chain_forward(masked, idx);
    if back.pipes + forward > 2 {
        return;
    }
    if left_is_simple(&back.left) {
        let line_no = idx + 1;
        findings.push(Finding::with_trigger(
            line_no,
            derive_column(raw.get(idx).copied().unwrap_or(""), block.marker),
            "Avoid single pipes to a block",
            block.marker,
        ));
    }
}

struct ChainBack {
    pipes: usize,
    /// Source feeding the block pipe (without any pipe operator).
    left: String,
}

/// Pipes on and above the candidate line plus the block's left side.
fn chain_back(masked: &[&str], idx: usize, pipe: usize) -> ChainBack {
    let line = masked[idx];
    let (head, _) = line.split_at(pipe);
    // `pipe` is an ASCII match offset: `split_at` is boundary-safe.
    let mut pipes = line.matches("|>").count();
    if !head.trim().is_empty() {
        // Chain starts on this line; `head` already holds base and middle.
        return ChainBack {
            pipes,
            left: head.trim().to_owned(),
        };
    }
    // Chain started above: absorb pipe-leading lines, then one base line.
    let mut segments: Vec<&str> = Vec::new();
    let mut cursor = idx;
    loop {
        if cursor == 0 {
            return ChainBack {
                pipes,
                left: String::new(),
            };
        }
        cursor -= 1;
        let prev = masked[cursor];
        if prev.trim().is_empty() {
            continue;
        }
        if let Some(segment) = prev.trim_start().strip_prefix("|>") {
            pipes += prev.matches("|>").count();
            segments.push(segment);
            continue;
        }
        pipes += prev.matches("|>").count();
        let base = prev.trim();
        if segments.len() == 1 && !prev.contains("|>") {
            return ChainBack {
                pipes,
                left: format!("{base} |> {}", segments[0].trim()),
            };
        }
        return ChainBack {
            pipes,
            left: base.to_owned(),
        };
    }
}

/// Whether the block's left side is a plain value, map/list, or one call.
fn left_is_simple(left: &str) -> bool {
    let direct = strip_parens(left.trim());
    if direct.is_empty() {
        return false;
    }
    if direct.starts_with('%') || direct.starts_with('[') || is_plain_var(direct) {
        return true;
    }
    // Otherwise the left side must read `base |> call` with a plain base.
    match direct.find("|>") {
        Some(at) => {
            // `at` is an ASCII match offset: slicing is boundary-safe.
            let base = strip_parens(direct[..at].trim());
            let call = direct[at + "|>".len()..].trim();
            (base.starts_with('%') || base.starts_with('[') || is_plain_var(base)) && is_call(call)
        }
        None => false,
    }
}

fn is_plain_var(value: &str) -> bool {
    if value == "true" || value == "false" || value == "nil" {
        return false;
    }
    let mut chars = value.chars();
    // Alias segments start uppercase; plain values do not.
    if chars
        .next()
        .is_none_or(|first| !first.is_lowercase() && first != '_')
    {
        return false;
    }
    chars.all(is_name_char)
}

fn is_call(segment: &str) -> bool {
    let segment = segment.trim();
    if segment.is_empty() {
        return false;
    }
    if segment.starts_with(['%', '[', '{', '"', '\'', ':'])
        || segment
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
    {
        return false;
    }
    segment.contains('(') || segment.contains('.') || segment.contains([' ', '\t'])
}

/// Peel balanced outer parens such as `(arg)`.
fn strip_parens(mut value: &str) -> &str {
    loop {
        let trimmed = value.trim();
        if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
            return trimmed;
        }
        let bytes = trimmed.as_bytes();
        let mut depth = 0_i32;
        let mut balanced = true;
        for (pos, byte) in bytes.iter().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 && pos + 1 < bytes.len() {
                        balanced = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !balanced || depth != 0 {
            return trimmed;
        }
        value = trimmed[1..trimmed.len() - 1].trim();
    }
}

/// Pipes continuing the chain after the block's closing `end`.
fn chain_forward(masked: &[&str], idx: usize) -> usize {
    let Some(end) = block_end(masked, idx) else {
        return 0;
    };
    let mut pipes = masked[end.0][end.1..].matches("|>").count();
    let mut cursor = end.0 + 1;
    while cursor < masked.len() {
        let line = masked[cursor];
        if line.trim().is_empty() {
            break;
        }
        if line.trim_start().starts_with("|>") {
            pipes += line.matches("|>").count();
            cursor += 1;
        } else {
            break;
        }
    }
    pipes
}

/// Line and byte offset just past the `end` closing the block at `idx`.
fn block_end(masked: &[&str], idx: usize) -> Option<(usize, usize)> {
    let mut depth = 0_i32;
    let mut cursor = idx;
    while cursor < masked.len() {
        let line = masked[cursor];
        for (pos, _) in line.match_indices("do") {
            if keyword_at(line, pos, "do") && !line[pos + "do".len()..].starts_with(':') {
                depth += 1;
            }
        }
        for (pos, _) in line.match_indices("fn") {
            if keyword_at(line, pos, "fn") {
                depth += 1;
            }
        }
        for (pos, _) in line.match_indices("end") {
            if keyword_at(line, pos, "end") {
                depth -= 1;
                if depth == 0 {
                    return Some((cursor, pos + "end".len()));
                }
            }
        }
        cursor += 1;
        if cursor > idx + 500 {
            break;
        }
    }
    None
}

fn keyword_at(line: &str, pos: usize, word: &str) -> bool {
    let bytes = line.as_bytes();
    if pos > 0 {
        let prev = bytes[pos - 1];
        // Selectors (`:end`), attributes (`@end`), captures, and field
        // access never open or close blocks.
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

fn is_name_char(next: char) -> bool {
    next.is_alphanumeric() || next == '_' || next == '?' || next == '!'
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
    fn normal_pipe_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x |> foo()\n")).is_empty());
    }
    #[test]
    fn reports_single_to_block() {
        let findings = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule Test do\n  def f(arg) do\n    arg\n    |> case do\n      :this -> :that\n    end\n  end\nend\n",
        ));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 4);
        assert_eq!(findings[0].column, Some(8));
        assert_eq!(findings[0].trigger, crate::Trigger::Text("case".to_owned()));
    }
    #[test]
    fn longer_chain_is_clean() {
        let src = "defmodule Test do\n  def f(arg) do\n    arg\n    |> do_something()\n    |> do_something_else()\n    |> case do\n      :this -> :that\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn piped_block_is_clean() {
        let src = "defmodule Test do\n  def f(arg) do\n    arg\n    |> do_something()\n    |> case do\n      :this -> :that\n    end\n    |> to_string()\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn call_subject_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "foo() |> case do\n  :a -> :b\nend\n"
            ))
            .is_empty()
        );
    }
}
