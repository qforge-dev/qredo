use crate::Finding;

/// `EX3016`: predicate names must not start with `is_` unless they are
/// guard-safe `defmacro` definitions (which must not end with `?`).
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    let mut impl_signatures: Vec<(String, usize)> = Vec::new();
    let mut pending_impl = false;
    let mut blocks: Vec<bool> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        // Substring gates: every expensive scan below needs one of these
        // ASCII substrings ("def" prefixes all ops incl. "defmodule",
        // "@impl" precedes the attribute word, block tracking needs a
        // do/fn/end word). Lines without them cannot change any state.
        if line.contains("defmodule") && has_word(line, b"defmodule") {
            impl_signatures.clear();
            pending_impl = false;
        }
        if !blocks.iter().any(|quoted| *quoted)
            && line.contains("def")
            && let Some((op, name, arity)) = def_head(line)
        {
            if pending_impl {
                pending_impl = false;
                impl_signatures.push((name.clone(), arity));
            }
            if name != "unquote"
                && name.starts_with("is_")
                && op != "defmacro"
                && !impl_signatures
                    .iter()
                    .any(|known| known.0 == name && known.1 == arity)
            {
                let column = trigger_column(line, &name).unwrap_or(1);
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(column),
                    "Predicate function names should not start with 'is', and should end in a question mark.",
                    name,
                ));
            }
        } else if line.contains("@impl") && is_impl_attribute(line) {
            pending_impl = true;
        }
        if line.contains("do") || line.contains("fn") || line.contains("end") {
            update_blocks(line, &mut blocks);
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// `(op, name, arity)` for `def`/`defp`/`defmacro` heads with plain names.
fn def_head(line: &str) -> Option<(String, String, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        let op = if match_word(&chars, i, b"defmacro") {
            Some("defmacro")
        } else if match_word(&chars, i, b"defp") {
            Some("defp")
        } else if match_word(&chars, i, b"def") {
            Some("def")
        } else {
            None
        };
        if let Some(op) = op {
            let mut start = i + op.len();
            while start < chars.len() && chars[start] == ' ' {
                start += 1;
            }
            let mut end = start;
            while end < chars.len()
                && (chars[end].is_alphanumeric() || matches!(chars[end], '_' | '?' | '!'))
            {
                end += 1;
            }
            if end > start {
                let name: String = chars[start..end].iter().collect();
                if is_head_name(&chars, end) {
                    return Some((op.to_owned(), name.clone(), head_arity(&chars, end)));
                }
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// True when the text after a head name continues as a definition
/// (`(args)`, `,`, `when`, `do` or line end) rather than an operator.
fn is_head_name(chars: &[char], mut i: usize) -> bool {
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    if i >= chars.len() {
        return true;
    }
    if matches!(chars[i], '(' | ',') {
        return true;
    }
    let rest: String = chars[i..].iter().collect();
    rest.starts_with("when ") || rest.starts_with("do") || rest == "when" || rest == "do"
}

fn head_arity(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    if i >= chars.len() || chars[i] != '(' {
        return 0;
    }
    let mut depth = 0_usize;
    let mut commas = 0_usize;
    let mut nonempty = false;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => commas += 1,
            c if depth == 1 && !c.is_whitespace() => nonempty = true,
            _ => {}
        }
        i += 1;
    }
    if nonempty { commas + 1 } else { 0 }
}

/// `@impl` with any value other than `false` marks the next definition.
fn is_impl_attribute(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        if match_word(&chars, i, b"impl") && i > 0 && chars[i - 1] == '@' {
            let mut j = i + 4;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if chars.get(j) == Some(&'(') {
                j += 1;
                while j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
            }
            let rest: String = chars[j..].iter().collect();
            return rest != "false" && !rest.starts_with("false,") && !rest.starts_with("false ");
        }
        i += 1;
    }
    false
}

/// ASCII word match with identifier boundaries; the word must be ASCII so
/// byte length equals char length (no allocation, unlike a char Vec).
fn match_word(chars: &[char], pos: usize, word: &[u8]) -> bool {
    chars.len() >= pos + word.len()
        && chars[pos..pos + word.len()]
            .iter()
            .zip(word.iter())
            .all(|(got, want)| *got == *want as char)
        && (pos == 0 || !(chars[pos - 1].is_alphanumeric() || chars[pos - 1] == '_'))
        && chars
            .get(pos + word.len())
            .is_none_or(|c| !c.is_alphanumeric() && *c != '_' && *c != '?' && *c != '!')
}

fn has_word(line: &str, word: &[u8]) -> bool {
    let chars: Vec<char> = line.chars().collect();
    (0..chars.len()).any(|i| match_word(&chars, i, word))
}

/// Track `quote` vs other `do`/`fn` blocks; `true` frames are `quote`.
fn update_blocks(line: &str, blocks: &mut Vec<bool>) {
    let words = line_words(line);
    let mut bare_dos = 0_usize;
    let mut first_bare_do = None;
    for (idx, (word, follows_colon)) in words.iter().enumerate() {
        if word == "do" && !follows_colon {
            if first_bare_do.is_none() {
                first_bare_do = Some(idx);
            }
            bare_dos += 1;
        }
    }
    let fn_opens = words
        .iter()
        .filter(|(word, _)| word.as_str() == "fn")
        .count();
    let ends = words
        .iter()
        .filter(|(word, _)| word.as_str() == "end")
        .count();
    let has_quote =
        first_bare_do.is_some_and(|do_idx| words[..do_idx].iter().any(|(word, _)| word == "quote"));
    for _ in 0..fn_opens {
        blocks.push(false);
    }
    for n in 0..bare_dos {
        // The first bare `do` after `quote` opens the quoted block.
        blocks.push(has_quote && n == 0);
    }
    for _ in 0..ends {
        blocks.pop();
    }
}

/// `(word, followed_by_colon)` for identifier words in the line.
fn line_words(line: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < chars.len() {
        if chars[i] == '@' || chars[i] == ':' || chars[i] == '.' {
            i = skip_word(&chars, i + 1);
            continue;
        }
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            i = skip_word(&chars, i);
            let word: String = chars[start..i].iter().collect();
            out.push((word, chars.get(i) == Some(&':')));
        } else {
            i += 1;
        }
    }
    out
}

fn skip_word(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
        i += 1;
    }
    i
}

/// First boundary-delimited trigger occurrence in `line` (1-based char col).
fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let pattern = format!(
        r"(\s|\b|\(|\)|,)({})(\s|\b|\(|\)|,)",
        regex::escape(trigger)
    );
    let re = regex::Regex::new(&pattern).ok()?;
    let inner = re.captures(line)?.get(2)?;
    Some(line[..inner.start()].chars().count() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn predicate_with_question_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("def foo?(x), do: x\n")).is_empty());
    }
    #[test]
    fn reports_is_prefix() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("def is_foo(x), do: x\n")).len(),
            1
        );
    }
    #[test]
    fn is_prefixed_question_is_still_flagged() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("def is_valid?(x), do: x\n")).len(),
            1
        );
    }
    #[test]
    fn defmacro_is_prefix_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "defmacro is_user(cookie), do: cookie\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn impl_attributed_def_is_clean() {
        let src = "@impl Foo\ndef is_bar, do: :ok\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn decoy_words_do_not_trigger_scans() {
        // "done", "send", "defend" and "@implementation" carry gated
        // substrings but no keywords; findings must be unaffected.
        let src =
            "done = 1\nsend(self(), :x)\ndefend = 2\n@implementation true\ndef is_bad, do: 1\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 5);
    }
}
