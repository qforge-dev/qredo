use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3012`: prefer pipelines over nested calls with `min_pipeline_length`.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let min_len = helpers::param_usize(params, "min_pipeline_length", 2);
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        if skip_line(line) {
            continue;
        }
        for call in remote_calls(line) {
            if is_pipe_target(line, call.head_start) {
                continue;
            }
            let length = 1 + pipe_len(first_arg(&call));
            if length >= min_len {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    credo_column(line, &call.head),
                    "Use a pipeline instead of nested function calls.",
                    call.head,
                ));
            }
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

struct RemoteCall {
    head: String,
    head_start: usize,
    args: String,
}

/// Guards, typespecs and documentation carry no pipeline candidates.
fn skip_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if has_word(line, "when") {
        return true;
    }
    for word in ["defguard ", "defguardp "] {
        if trimmed.starts_with(word) {
            return true;
        }
    }
    if trimmed.starts_with('@') {
        for attr in [
            "@callback",
            "@macrocallback",
            "@opaque",
            "@spec",
            "@type",
            "@typep",
        ] {
            if trimmed.starts_with(attr)
                && trimmed
                    .as_bytes()
                    .get(attr.len())
                    .is_none_or(|b| !is_name_byte(*b))
            {
                return true;
            }
        }
    }
    false
}

/// Remote (`Mod.fun(...)`, `var.fun(...)`) calls on one masked line.
fn remote_calls(line: &str) -> Vec<RemoteCall> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0_usize;
    while idx < bytes.len() {
        if bytes[idx] != b'(' {
            idx += 1;
            continue;
        }
        if let Some((head, head_start)) = call_head(line, idx)
            && let Some(close) = match_paren(line, idx)
        {
            let args = line.get(idx + 1..close).unwrap_or("").to_owned();
            out.push(RemoteCall {
                head,
                head_start,
                args,
            });
            idx += 1;
        } else if let Some((head, head_start)) = call_head(line, idx) {
            out.push(RemoteCall {
                head,
                head_start,
                args: line.get(idx + 1..).unwrap_or("").to_owned(),
            });
            idx += 1;
        } else {
            idx += 1;
        }
    }
    out
}

/// Dotted head (`Alias.name`, `var.name`) ending right before `(` at `open`.
fn call_head(line: &str, open: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    let mut start = open;
    while start > 0 && is_head_byte(bytes[start - 1]) {
        start -= 1;
    }
    let raw = line.get(start..open)?.trim_end().to_owned();
    let trimmed = raw.trim_start().to_owned();
    if !is_remote_head(&trimmed) {
        return None;
    }
    if start > 0 && bytes[start - 1] == b':' {
        return None;
    }
    let offset = raw.len() - trimmed.len();
    Some((trimmed, start + offset))
}

fn is_head_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'?' | b'!' | b' ')
}

fn is_remote_head(head: &str) -> bool {
    let head = head.trim_end();
    let name = head.rsplit('.').next().unwrap_or("");
    if name.is_empty() || !head.contains('.') {
        return false;
    }
    let mut name_chars = name.chars();
    if name_chars
        .next()
        .is_none_or(|c| !(c.is_ascii_lowercase() || c == '_'))
    {
        return false;
    }
    head.split('.').all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
    })
}

/// Whether the call is the direct target of a `|>` on the same line.
fn is_pipe_target(line: &str, head_start: usize) -> bool {
    let before = line.get(..head_start).unwrap_or("").trim_end();
    before.ends_with("|>")
}

/// First top-level argument of the call.
fn first_arg(call: &RemoteCall) -> &str {
    split_args(&call.args).into_iter().next().unwrap_or("")
}

/// Pipeline length contributed by the first argument: another nested call
/// with arguments adds one level plus its own first argument.
fn pipe_len(arg: &str) -> usize {
    let trimmed = strip_parens(arg.trim());
    let Some((head, args)) = split_call(trimmed) else {
        return 0;
    };
    if is_excluded_head(head) || args.trim().is_empty() {
        return 0;
    }
    1 + pipe_len(first_arg_in(&args))
}

fn first_arg_in(args: &str) -> &str {
    split_args(args).into_iter().next().unwrap_or("")
}

/// Heads that cannot start a pipeline (`fn`, operators, sentinels).
fn is_excluded_head(head: &str) -> bool {
    matches!(
        head,
        "fn" | "for" | "with" | "not" | "and" | "or" | "in" | "unquote"
    )
}

/// Leading call (`name(...)` or `Mod.name(...)`) with balanced arguments.
fn split_call(text: &str) -> Option<(&str, String)> {
    let mut head_end = 0_usize;
    for (idx, char) in text.char_indices() {
        if char.is_alphanumeric() || char == '_' || char == '.' || char == '?' || char == '!' {
            head_end = idx + char.len_utf8();
        } else {
            break;
        }
    }
    let head = text.get(..head_end)?;
    if head.is_empty() || head.starts_with('.') || head.ends_with('.') || head.contains("..") {
        return None;
    }
    let open = text[head_end..].find('(')? + head_end;
    if !text
        .get(head_end..open)
        .is_some_and(|gap| gap.trim().is_empty())
    {
        return None;
    }
    let close = match_paren(text, open)?;
    Some((head, text.get(open + 1..close).unwrap_or("").to_owned()))
}

/// Strip fully enclosing parentheses (`(f(x))` -> `f(x)`).
fn strip_parens(text: &str) -> &str {
    let mut current = text;
    while current.starts_with('(') {
        let Some(close) = match_paren(current, 0) else {
            break;
        };
        if close + 1 != current.len() {
            break;
        }
        current = current.get(1..close).unwrap_or("").trim();
    }
    current
}

fn split_args(args: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (idx, byte) in args.bytes().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(args.get(start..idx).unwrap_or(""));
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(args.get(start..).unwrap_or(""));
    parts
}

fn match_paren(line: &str, open: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
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

fn has_word(line: &str, needle: &str) -> bool {
    let bytes = line.as_bytes();
    let mut idx = 0_usize;
    while idx + needle.len() <= bytes.len() {
        if line.get(idx..).is_some_and(|rest| rest.starts_with(needle))
            && word_boundary(bytes, idx)
            && word_boundary(bytes, idx + needle.len())
        {
            return true;
        }
        idx += 1;
    }
    false
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

    #[test]
    fn pipeline_is_clean() {
        let src = "x |> foo() |> bar()\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn reports_deeply_nested() {
        let src = "Foo.bar(Baz.qux(x))\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }

    #[test]
    fn no_arg_inner_call_is_clean() {
        // EX3012.upstream.inner-no-args: `some_list()` takes no arguments.
        let src = "defmodule CredoSampleModule do\n  def some_code do\n    Enum.uniq(some_list())\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn reports_each_nesting_level() {
        // EX3012.upstream.three-nested: outer and middle calls both report.
        let src = "defmodule CredoSampleModule do\n  def some_code do\n    Enum.shuffle(Enum.uniq(Enum.take([1,2,2,3,3], 10)))\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 2);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
        assert_eq!((findings[1].line, findings[1].column), (3, Some(18)));
    }

    #[test]
    fn min_pipeline_length_raises_bar() {
        let src = "Foo.bar(Baz.qux(x))\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
        let mut params = BTreeMap::new();
        params.insert("min_pipeline_length".to_owned(), "3".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
}
