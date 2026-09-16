use crate::Finding;

/// `EX3032`: avoid custom tagged tuples as `with` placeholders.
///
/// Flags `<-` clauses where both sides are 2-tuples sharing one atom tag,
/// e.g. `{:resource, x} <- {:resource, fetch()}`.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let masked_lines: Vec<&str> = masked.split('\n').collect();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let mut findings = Vec::new();
    let mut line_no = 0_usize;
    while line_no < masked_lines.len() {
        if let Some(keyword_end) = with_start(masked_lines[line_no]) {
            let region = collect_region(&masked_lines, line_no, keyword_end);
            examine_region(&region, &raw_lines, &mut findings);
            line_no = region.last_line + 1;
        } else {
            line_no += 1;
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// Byte offset just past a `with` keyword opening a statement, if present.
fn with_start(line: &str) -> Option<usize> {
    for (pos, _) in line.match_indices("with") {
        if keyword_at(line, pos, "with") {
            return Some(pos + "with".len());
        }
    }
    None
}

struct Region {
    first_line: usize,
    last_line: usize,
    flat: String,
    line_starts: Vec<usize>,
    keyword_end: usize,
}

/// Lines from a `with` opener through its body opener (or statement end).
fn collect_region(lines: &[&str], first: usize, keyword_end: usize) -> Region {
    let mut flat = String::new();
    let mut line_starts = Vec::new();
    let mut last = first;
    let mut idx = first;
    while idx < lines.len() {
        line_starts.push(flat.len());
        flat.push_str(lines[idx]);
        last = idx;
        let start = line_starts[line_starts.len() - 1];
        if body_opens_here(&flat, start) || !statement_continues(lines[idx]) {
            break;
        }
        flat.push('\n');
        idx += 1;
    }
    Region {
        first_line: first,
        last_line: last,
        flat,
        line_starts,
        keyword_end,
    }
}

/// Whether one masked statement line may continue onto the next line: a
/// trailing comma or unbalanced openers.
fn statement_continues(line: &str) -> bool {
    line.trim_end().ends_with(',') || bracket_depth(line) > 0
}

/// Net `(`/`[`/`{` depth change of one masked line.
fn bracket_depth(line: &str) -> i32 {
    let mut depth = 0_i32;
    for byte in line.bytes() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Whether a depth-0 body opener (`do:` or word `do`) starts at `line_off`.
fn body_opens_here(flat: &str, line_off: usize) -> bool {
    let bytes = flat.as_bytes();
    let mut depth = relative_depth(flat, line_off);
    let mut i = line_off;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && flat[i..].starts_with("do") && keyword_at(flat, i, "do") {
            return true;
        }
        i += 1;
    }
    false
}

/// Bracket depth just before `off` relative to the region start.
fn relative_depth(flat: &str, off: usize) -> i32 {
    let mut depth = 0_i32;
    for byte in &flat.as_bytes()[..off] {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Flag every depth-0 `<-` with matching tuple tags before the body opener.
fn examine_region(region: &Region, raw_lines: &[&str], out: &mut Vec<Finding>) {
    let flat = &region.flat;
    let opener = first_opener(region);
    let mut search = 0_usize;
    while let Some(found) = flat[search..].find("<-") {
        let pos = search + found;
        if opener.is_some_and(|limit| pos >= limit) {
            break;
        }
        if relative_depth(flat, pos) == 0
            && let Some(tag) = clause_tag(region, pos)
        {
            let line_no = region.first_line + line_index(region, pos) + 1;
            let raw = raw_lines.get(line_no - 1).copied().unwrap_or("");
            out.push(Finding::with_trigger(
                line_no,
                derive_column(raw, &tag),
                format!("Avoid using tagged tuples as placeholders in `with` (found: `{tag}`)."),
                tag,
            ));
        }
        search = pos + 2;
    }
}

/// Offset of the body's `do` opener, if one was collected.
fn first_opener(region: &Region) -> Option<usize> {
    let bytes = region.flat.as_bytes();
    let mut depth = 0_i32;
    let mut i = 0_usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && region.flat[i..].starts_with("do") && keyword_at(&region.flat, i, "do") {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Shared atom tag when both `<-` sides are 2-tuples opening with it.
fn clause_tag(region: &Region, arrow: usize) -> Option<String> {
    let flat = &region.flat;
    let start = clause_start(region, arrow);
    let end = clause_end(region, arrow);
    let left = tuple_tag(flat[start..arrow].trim())?;
    let right = tuple_tag(flat[arrow + "<-".len()..end].trim())?;
    if left == right { Some(left) } else { None }
}

/// Start of the clause holding `arrow`: previous depth-0 comma or region head.
fn clause_start(region: &Region, arrow: usize) -> usize {
    let flat = &region.flat;
    let head = line_head_end(region);
    let mut depth = relative_depth(flat, arrow);
    let mut i = arrow;
    while i > head {
        i -= 1;
        match flat.as_bytes()[i] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' => depth -= 1,
            b',' if depth == 0 => return i + 1,
            _ => {}
        }
    }
    head
}

/// End of the clause holding `arrow`: next depth-0 comma, opener, or tail.
fn clause_end(region: &Region, arrow: usize) -> usize {
    let flat = &region.flat;
    let stop = first_opener(region).unwrap_or(flat.len());
    let mut depth = 0_i32;
    let mut i = arrow + "<-".len();
    while i < stop {
        match flat.as_bytes()[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    stop
}

/// Offset where clause text may start: past the `with` keyword on its line.
fn line_head_end(region: &Region) -> usize {
    if region.line_starts.is_empty() {
        return 0;
    }
    region.line_starts[0] + region.keyword_end
}

/// Index into `line_starts` holding `pos` (offsets are ASCII `\n`-joined).
fn line_index(region: &Region, pos: usize) -> usize {
    let mut idx = 0_usize;
    for (step, _) in region.line_starts.iter().enumerate() {
        if region.line_starts[step] <= pos {
            idx = step;
        } else {
            break;
        }
    }
    idx
}

/// Leading `:atom` of a `{...}` 2-tuple literal, if exactly two elements.
fn tuple_tag(text: &str) -> Option<String> {
    let inner = text.strip_prefix('{')?;
    let mut chars = inner.trim_start().strip_prefix(':')?.chars();
    let mut tag = String::from(":");
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }
    tag.push(first);
    for next in chars.by_ref() {
        if next.is_ascii_alphanumeric() || next == '_' || next == '?' || next == '!' {
            tag.push(next);
        } else {
            break;
        }
    }
    let after = inner.trim_start()[tag.len()..].trim_start();
    if !after.starts_with(',') {
        return None;
    }
    // Both tuple positions exist exactly when one top-level comma separates
    // the tag from a single closing element.
    if top_commas(text)? != 1 {
        return None;
    }
    Some(tag)
}

/// Depth-0 commas inside the tuple starting at offset 0 of `text`.
fn top_commas(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0_i32;
    let mut commas = 0_usize;
    let mut i = 0_usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(commas);
                }
            }
            b',' if depth == 1 => commas += 1,
            _ => {}
        }
        i += 1;
    }
    None
}

fn keyword_at(line: &str, pos: usize, word: &str) -> bool {
    let bytes = line.as_bytes();
    if pos > 0 {
        let prev = bytes[pos - 1];
        // Selectors (`:with`), attributes, captures, and field access never
        // open a `with` statement or a `do` body.
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
    next.is_alphanumeric() || next == '_'
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
    fn ok_tag_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "with {:ok, x} <- foo(), do: x\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_matching_tags() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "with {:custom, x} <- {:custom, foo()}, do: x\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn same_tag_on_both_sides_reports_without_column() {
        let src = "defmodule Test do\n  def run(u, r) do\n    with {:resource, {:ok, res}} <- {:resource, Resource.fetch(u)},\n         {:authz, :ok} <- {:authz, Resource.authorize(res, u)} do\n      res\n    end\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].line, 3);
        assert_eq!(findings[0].column, None);
        assert_eq!(
            findings[0].trigger,
            crate::Trigger::Text(":resource".to_owned())
        );
        assert_eq!(
            findings[1].trigger,
            crate::Trigger::Text(":authz".to_owned())
        );
    }
    #[test]
    fn mismatched_tags_are_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "with {:a, x} <- {:b, foo()}, do: x\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn three_tuple_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "with {:a, x, y} <- {:a, foo()}, do: x\n"
            ))
            .is_empty()
        );
    }
}
