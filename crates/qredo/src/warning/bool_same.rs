use crate::Finding;

/// `EX5002`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    // Raw lines for operand text: masking blanks string contents, which
    // would equate distinct literals (`"a"` vs `"b"`). Delimiter scanning
    // stays on masked text; char counts align (each char masks to one
    // char), so char ranges transfer to raw lines.
    let raw_lines: Vec<&str> = prepared.source().split('\n').collect();
    let redefined = redefined_ops(masked);
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        for hit in line_hits(line) {
            if redefined.contains(&hit.op) {
                continue;
            }
            let level = rank(hit.op);
            let left = operand_text(
                &lines,
                &raw_lines,
                left_span(&lines, idx, hit.base, level),
                left_finish,
            );
            let right = operand_text(
                &lines,
                &raw_lines,
                right_span(&lines, idx, hit.end, level),
                right_finish,
            );
            if !left.is_empty() && left == right {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(char_col(line, hit.base)),
                    format!(
                        "There are identical sub-expressions to the left and to the right of the '{}' operator.",
                        hit.op
                    ),
                    hit.op.to_owned(),
                ));
            }
        }
    }
    findings
}

/// Character span of an operand: line index plus char offsets.
#[derive(Clone, Copy)]
struct Span {
    line: usize,
    start: usize,
    end: usize,
}

/// Operand text for a span, preferring raw literal contents; falls back
/// to masked text when line shapes diverge, then applies `finish`.
fn operand_text(
    lines: &[&str],
    raw_lines: &[&str],
    span: Span,
    finish: impl Fn(&str) -> String,
) -> String {
    let masked_line = lines.get(span.line).copied().unwrap_or("");
    let raw_line = raw_lines.get(span.line).copied().unwrap_or(masked_line);
    let masked: Vec<char> = masked_line.chars().collect();
    let raw: Vec<char> = raw_line.chars().collect();
    let aligned = masked.len() == raw.len();
    let take = |chars: &[char]| {
        chars
            .get(span.start.min(chars.len())..span.end.min(chars.len()))
            .unwrap_or(&[])
            .iter()
            .collect::<String>()
    };
    if aligned {
        // Raw text keeps distinct string literals distinct; strip comments
        // (masked scanning ranges can cover blanked comment regions).
        finish(&strip_comment(&take(&raw)))
    } else {
        finish(&take(&masked))
    }
}

/// Text up to a `#` comment outside string/char literals. Masked ranges
/// may extend over blanked comments, which raw text must not compare.
fn strip_comment(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut idx = 0_usize;
    let mut quote = None;
    while idx < chars.len() {
        let char = chars[idx];
        if let Some(open) = quote {
            out.push(char);
            if char == '\\' && idx + 1 < chars.len() {
                out.push(chars[idx + 1]);
                idx += 1;
            } else if char == open {
                quote = None;
            }
        } else if char == '?' && idx + 1 < chars.len() {
            // `?#` char literal is not a comment.
            out.push(char);
            out.push(chars[idx + 1]);
            idx += 1;
        } else if char == '"' || char == '\'' {
            quote = Some(char);
            out.push(char);
        } else if char == '#' {
            break;
        } else {
            out.push(char);
        }
        idx += 1;
    }
    out
}

/// Left-operand finishing: guard bodies, wrapper keywords, parens.
fn left_finish(operand: &str) -> String {
    strip_prefix_keywords(operand.trim())
}

/// Right-operand finishing: trailing `do` only (prefix behavior kept).
fn right_finish(operand: &str) -> String {
    strip_suffix_do(operand.trim())
}

/// One boolean operator occurrence: its text and byte span.
struct Hit {
    op: &'static str,
    base: usize,
    end: usize,
}

/// All `and`/`or`/`&&`/`||` occurrences (any nesting: each AST node flags).
fn line_hits(line: &str) -> Vec<Hit> {
    let chars: Vec<char> = line.chars().collect();
    let mut hits = Vec::new();
    let mut idx = 0_usize;
    // Byte offset of `chars[idx]`, tracked incrementally so hit spans stay
    // O(1) instead of rescanning the line per occurrence.
    let mut byte = 0_usize;
    while idx < chars.len() {
        match chars[idx] {
            '&' if chars.get(idx + 1) == Some(&'&') => {
                hits.push(Hit {
                    op: "&&",
                    base: byte,
                    end: byte + 2,
                });
                idx += 1;
                byte += 1;
            }
            '|' if chars.get(idx + 1) == Some(&'|') => {
                hits.push(Hit {
                    op: "||",
                    base: byte,
                    end: byte + 2,
                });
                idx += 1;
                byte += 1;
            }
            _ => {
                for op in ["and", "or"] {
                    if word_at(&chars, idx, op.as_bytes()) {
                        hits.push(Hit {
                            op,
                            base: byte,
                            end: byte + op.len(),
                        });
                    }
                }
            }
        }
        byte += chars[idx].len_utf8();
        idx += 1;
    }
    hits
}

/// Precedence rank: `&&`/`||` bind tighter than `and`/`or`.
fn rank(op: &str) -> usize {
    if op == "and" || op == "or" { 1 } else { 2 }
}

/// Byte index of the `idx`-th char (ASCII operators keep this boundary-safe).
fn byte_of(chars: &[char], idx: usize) -> usize {
    chars[..idx.min(chars.len())]
        .iter()
        .map(|c| c.len_utf8())
        .sum()
}

/// True for a whole-word operator at `idx`. ASCII operators only.
fn word_at(chars: &[char], idx: usize, op: &[u8]) -> bool {
    chars.len() >= idx + op.len()
        && chars[idx..idx + op.len()]
            .iter()
            .zip(op.iter())
            .all(|(got, want)| *got == *want as char)
        && (idx == 0 || !is_name(chars[idx - 1]))
        && !chars.get(idx + op.len()).is_some_and(|c| is_name(*c))
}

fn is_name(char: char) -> bool {
    char.is_alphanumeric() || char == '_' || char == '?' || char == '!'
}

/// Left operand span: char offsets since the last depth-0 delimiter.
/// Tighter boolean operators belong to the operand; looser ones (and
/// arrows) delimit it, mirroring left-associative nesting
/// (`x && x && x` flags only the inner).
fn left_span(lines: &[&str], line_idx: usize, base: usize, level: usize) -> Span {
    let line = lines.get(line_idx).copied().unwrap_or("");
    let chars: Vec<char> = line[..base.min(line.len())].chars().collect();
    let end = chars.len();
    let mut depth = 0_usize;
    let mut idx = end;
    while idx > 0 {
        idx -= 1;
        match chars[idx] {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                if depth == 0 {
                    idx += 1;
                    break;
                }
                depth -= 1;
            }
            ',' | ';' if depth == 0 => {
                idx += 1;
                break;
            }
            '=' if depth == 0 && is_bare_equals(&chars, idx) => {
                idx += 1;
                break;
            }
            '>' if depth == 0 && idx > 0 && chars[idx - 1] == '-' => {
                // `->` clause arrow.
                idx += 1;
                break;
            }
            '-' if depth == 0 && idx > 0 && chars[idx - 1] == '<' => {
                // `<-` clause arrow.
                idx += 1;
                break;
            }
            _ if depth == 0 && bool_rank_at(&chars, idx).is_some_and(|rank| rank < level) => {
                idx += bool_len_at(&chars, idx);
                break;
            }
            _ => {}
        }
    }
    Span {
        line: line_idx,
        start: idx,
        end,
    }
}

/// Next non-blank line after `idx`, if any.
fn continuation_line(lines: &[&str], idx: usize) -> Option<usize> {
    lines
        .get(idx + 1..)
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .find(|(_, next)| !next.trim().is_empty())
        .map(|(offset, _)| idx + 1 + offset)
}

/// Rank of the boolean operator starting at `idx`, if any.
fn bool_rank_at(chars: &[char], idx: usize) -> Option<usize> {
    if chars.get(idx) == Some(&'&') && chars.get(idx + 1) == Some(&'&') {
        return Some(2);
    }
    if chars.get(idx) == Some(&'|') && chars.get(idx + 1) == Some(&'|') {
        return Some(2);
    }
    for op in [b"and".as_slice(), b"or".as_slice()] {
        if word_at(chars, idx, op) {
            return Some(1);
        }
    }
    None
}

/// Character length of the boolean operator starting at `idx`.
fn bool_len_at(chars: &[char], idx: usize) -> usize {
    if chars.get(idx) == Some(&'&') && chars.get(idx + 1) == Some(&'&') {
        return 2;
    }
    if chars.get(idx) == Some(&'|') && chars.get(idx + 1) == Some(&'|') {
        return 2;
    }
    for op in [b"and".as_slice(), b"or".as_slice()] {
        if word_at(chars, idx, op) {
            return op.len();
        }
    }
    1
}

/// True for a standalone `=` (not `==`, `!=`, `<=`, `>=`, `=>`, `~=`).
fn is_bare_equals(chars: &[char], idx: usize) -> bool {
    const NEIGHBORS: [char; 7] = ['=', '<', '>', '!', '~', '+', '-'];
    if idx > 0 && NEIGHBORS.contains(&chars[idx - 1]) {
        return false;
    }
    !chars.get(idx + 1).is_some_and(|c| NEIGHBORS.contains(c))
}

/// Drop leading call keywords (`assert x` compares `x`), matching AST operands.
/// Only wrappers that take the whole condition qualify; operators like `not`
/// stay (`not x` differs from `x`).
fn strip_prefix_keywords(operand: &str) -> String {
    let mut rest = operand.trim().to_owned();
    // A top-level `when` starts a guard: `def f() when x` compares `x`.
    rest = guard_body(&rest);
    loop {
        let mut hit = false;
        for keyword in ["assert", "refute", "if", "unless", "case"] {
            if let Some(tail) = rest.strip_prefix(keyword)
                && let Some(next) = tail.chars().next()
                && (next.is_whitespace() || next == '(')
            {
                rest = tail.trim_start_matches([' ', '\t']).trim().to_owned();
                hit = true;
                break;
            }
        }
        if !hit {
            return unwrap_parens(&rest);
        }
    }
}

/// Text after the last top-level ` when ` (guard bodies compare alone).
fn guard_body(operand: &str) -> String {
    let chars: Vec<char> = operand.chars().collect();
    let mut depth = 0_usize;
    let mut cut = None;
    let mut idx = 0_usize;
    while idx < chars.len() {
        match chars[idx] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && word_at(&chars, idx, b"when") => {
                cut = Some(byte_of(&chars, idx + "when".len()));
            }
            _ => {}
        }
        idx += 1;
    }
    match cut {
        Some(end) => operand[end..].trim().to_owned(),
        None => operand.to_owned(),
    }
}

/// Remove balanced outer parentheses: `(x)` compares as `x`.
fn unwrap_parens(operand: &str) -> String {
    let mut rest = operand.trim();
    while rest.starts_with('(') && rest.ends_with(')') {
        let mut depth = 0_usize;
        let mut balanced = true;
        for (idx, char) in rest.char_indices() {
            match char {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && idx + 1 < rest.len() {
                        balanced = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !balanced || depth != 0 {
            break;
        }
        rest = rest[1..rest.len() - 1].trim();
    }
    rest.to_owned()
}

/// Right operand span: char offsets up to the next depth-0 delimiter,
/// continuing on the next non-blank line when the rest of the line is
/// empty. Operators of the same or looser precedence delimit it (left
/// associativity); tighter ones belong to the operand.
fn right_span(lines: &[&str], idx: usize, end: usize, level: usize) -> Span {
    let line = lines.get(idx).copied().unwrap_or("");
    let tail = if end <= line.len() { &line[end..] } else { "" };
    let mut scanned = tail.to_owned();
    let mut scanned_line = idx;
    // Char offset where the scanned text starts within its line.
    let mut scanned_start = line[..end.min(line.len())].chars().count();
    if scanned.trim().is_empty() {
        match continuation_line(lines, idx) {
            Some(next) => {
                scanned.clear();
                scanned.push_str(lines[next]);
                scanned_line = next;
                scanned_start = 0;
            }
            None => {
                return Span {
                    line: idx,
                    start: scanned_start,
                    end: scanned_start,
                };
            }
        }
    }
    let chars: Vec<char> = scanned.chars().collect();
    let mut depth = 0_usize;
    let mut len = 0_usize;
    while len < chars.len() && !right_delimiter(&chars, len, depth, level) {
        match chars[len] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        len += 1;
    }
    Span {
        line: scanned_line,
        start: scanned_start,
        end: scanned_start + len,
    }
}

/// True when the right-operand scan ends before `at`.
fn right_delimiter(chars: &[char], at: usize, depth: usize, level: usize) -> bool {
    if depth != 0 {
        return false;
    }
    match chars[at] {
        ')' | ']' | '}' | ',' | ';' => true,
        '-' if chars.get(at + 1) == Some(&'>') => true,
        _ if bool_rank_at(chars, at).is_some_and(|rank| rank <= level) => true,
        _ if word_at(chars, at, b"do") => true,
        _ if word_at(chars, at, b"end") => true,
        _ if word_at(chars, at, b"else") => true,
        _ if word_at(chars, at, b"when") => true,
        _ => false,
    }
}

/// Drop a trailing `do` keyword from the operand.
fn strip_suffix_do(operand: &str) -> String {
    if let Some(head) = operand.strip_suffix("do")
        && let Some(prev) = head.chars().next_back()
        && (prev.is_whitespace() || prev == '(')
    {
        return head.trim_end().to_owned();
    }
    operand.to_owned()
}

/// Operators redefined in this file (`import Kernel, except:` + `def`).
/// Files without the import cannot redefine anything.
fn redefined_ops(masked: &str) -> Vec<&'static str> {
    if !masked.contains("import Kernel, except:") {
        return Vec::new();
    }
    ["&&", "||", "and", "or"]
        .into_iter()
        .filter(|op| is_redefined(masked, op))
        .collect()
}

fn is_redefined(masked: &str, op: &str) -> bool {
    let mut imported = false;
    let mut defined = false;
    for line in masked.split('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("import Kernel, except:") && line.contains(op) {
            imported = true;
        }
        if is_def_directive(trimmed) && op_at_depth_zero(line, op) {
            defined = true;
        }
    }
    imported && defined
}

/// True for `def`/`defp`/`defmacro`/`defguard` directives (not `default`).
fn is_def_directive(trimmed: &str) -> bool {
    ["def ", "defp ", "defmacro ", "defguard ", "def("]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
        || matches!(trimmed, "def" | "defp" | "defmacro" | "defguard")
}

/// Operator token at bracket depth zero (a redefined operator is defined
/// outside parentheses, unlike `def foo(a && b)`).
fn op_at_depth_zero(line: &str, op: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let mut depth = 0_usize;
    let mut idx = 0_usize;
    while idx < chars.len() {
        match chars[idx] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && op_here(&chars, idx, op.as_bytes()) => return true,
            _ => {}
        }
        idx += 1;
    }
    false
}

/// Operator token at `idx` (word ops need boundaries).
fn op_here(chars: &[char], idx: usize, op: &[u8]) -> bool {
    if op == b"and" || op == b"or" {
        return word_at(chars, idx, op);
    }
    if op.len() != 2 {
        return false;
    }
    if (op == b"&&" && chars.get(idx) != Some(&'&'))
        || (op == b"||" && chars.get(idx) != Some(&'|'))
    {
        return false;
    }
    chars.len() >= idx + 2
        && chars[idx] == op[0] as char
        && chars.get(idx + 1) == Some(&(op[1] as char))
}

/// 1-based column for a byte index at an ASCII token.
fn char_col(line: &str, base: usize) -> usize {
    line[..base].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x && y\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("x && x\n")).len(),
            1
        );
    }
    #[test]
    fn distinct_string_literals_are_distinct_operands() {
        // Native reference: `body =~ "a" and body =~ "b"` is clean (0),
        // `a || a` reports (1); masking blanks literal contents, so the
        // comparison must use raw literal text.
        let clean = "defmodule M do\n  def f(body) do\n    body =~ \"new pool replica\" and body =~ \"event: log.error\"\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(clean)).is_empty());
        let or_clean = "defmodule M do\n  def f(conn) do\n    Map.get(conn, \"entries\") || Map.get(conn, \"metrics\") || []\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(or_clean)).is_empty());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "defmodule M do\n  def f(a) do\n    a || a\n  end\nend\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_and_word_with_exact_finding() {
        let src = "defmodule CredoSampleModule do\n  use ExUnit.Case\n\n  def some_fun do\n    x and x\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src)),
            vec![Finding::with_trigger(
                5,
                Some(7),
                "There are identical sub-expressions to the left and to the right of the 'and' operator.",
                "and",
            )]
        );
    }
    #[test]
    fn ignores_redefined_operators() {
        let src = "defmodule CredoSampleModule do\n  use ExUnit.Case\n\n  import Kernel, except: [&&: 2]\n\n  def x && x, do: true\n  def _ && _, do: false\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_multiline_operands() {
        let src = "defmodule CredoSampleModule do\n  use ExUnit.Case\n\n  def some_fun do\n    x and x\n    x or x\n    x && x\n    x || x\n    x &&\n      x # on different lines\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 5);
        assert_eq!(findings[4].line, 9);
        assert_eq!(findings[4].column, Some(7));
    }
    #[test]
    fn chained_operators_flag_inner_only() {
        // Left-associative: `(x && x) && x` flags the inner pair once.
        let findings = check_prepared(&crate::batch::Prepared::lazy("x && x && x\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(3));
    }
    #[test]
    fn call_wrapped_operands_flag() {
        let findings = check_prepared(&crate::batch::Prepared::lazy("g(x && x)\n"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(5));
    }
    #[test]
    fn distinct_columns_on_one_line() {
        let findings = check_prepared(&crate::batch::Prepared::lazy("x && x; y || y\n"));
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].column, Some(3));
        assert_eq!(findings[1].column, Some(11));
    }
}
