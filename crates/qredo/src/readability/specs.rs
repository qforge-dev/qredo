use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3025`: public functions should have `@spec`.
///
/// Each `@spec name/arity` only covers same-arity definitions below it;
/// a non-`false` `@impl` covers every definition below it. Definitions
/// inside `quote` blocks are not checked.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let include_defp = helpers::param_bool(params, "include_defp", false);
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    let mut specs: Vec<(String, usize)> = Vec::new();
    let mut impl_covers = false;
    let mut blocks: Vec<bool> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        // Substring gates: each scan below needs one of these ASCII
        // substrings ("@spec" precedes the attribute word, "def" prefixes
        // all definition ops, block tracking needs a do/fn/end word).
        if line.contains("@spec")
            && let Some((name, arity)) = parse_spec(line)
        {
            specs.push((name, arity));
        }
        if line.contains("impl") && is_impl_cover(line) {
            impl_covers = true;
        }
        if !blocks.iter().any(|quoted| *quoted)
            && line.contains("def")
            && let Some((op, name, arity)) = def_head(line)
        {
            let private = op == "defp";
            if !private || include_defp {
                let covered = impl_covers
                    || specs
                        .iter()
                        .any(|known| known.0 == name && known.1 == arity);
                if !covered {
                    let column = trigger_column(line, &name).unwrap_or(1);
                    findings.push(Finding::with_trigger(
                        idx + 1,
                        Some(column),
                        "Functions should have a @spec type specification.",
                        name,
                    ));
                }
            }
        }
        if line.contains("do") || line.contains("fn") || line.contains("end") {
            update_blocks(line, &mut blocks);
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// `(name, arity)` from a `@spec name(args) :: ...` line.
fn parse_spec(line: &str) -> Option<(String, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        if match_word(&chars, i, b"spec") && i > 0 && chars[i - 1] == '@' {
            let mut start = i + 4;
            while start < chars.len() && chars[start] == ' ' {
                start += 1;
            }
            let mut end = start;
            while end < chars.len()
                && (chars[end].is_alphanumeric() || matches!(chars[end], '_' | '?' | '!'))
            {
                end += 1;
            }
            if end == start {
                return None;
            }
            let name: String = chars[start..end].iter().collect();
            return Some((name, spec_arity(&chars, end)));
        }
        i += 1;
    }
    None
}

/// Arity from the parenthesised type arguments, or zero without parens.
fn spec_arity(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    if i >= chars.len() || chars[i] != '(' {
        return 0;
    }
    count_args(chars, i)
}

/// Count top-level comma-separated items in the parens at `open`.
fn count_args(chars: &[char], open: usize) -> usize {
    let mut depth = 0_usize;
    let mut commas = 0_usize;
    let mut nonempty = false;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && chars[i] == ')' {
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

/// `@impl` with a non-`false` value, or a bare `impl(...)` call, covers below.
fn is_impl_cover(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        if match_word(&chars, i, b"impl") {
            let is_attr = i > 0 && chars[i - 1] == '@';
            let after = i + 4;
            let bare_call =
                chars.get(after) == Some(&'(') && chars[..i].iter().all(|c| c.is_whitespace());
            if is_attr || bare_call {
                let mut j = after;
                if chars.get(j) == Some(&'(') {
                    j += 1;
                }
                while j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
                let rest = &chars[j..];
                if rest != ['f', 'a', 'l', 's', 'e']
                    && !rest.starts_with(&['f', 'a', 'l', 's', 'e', ' '])
                    && !rest.starts_with(&['f', 'a', 'l', 's', 'e', ','])
                {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// `(op, name-or-unquote, arity)` for `def`/`defp` heads with plain names.
fn def_head(line: &str) -> Option<(String, String, usize)> {
    const UNQUOTE: &[u8] = b"unquote";
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        let op = if match_word(&chars, i, b"defp") {
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
            if starts_with_ascii(&chars, start, UNQUOTE) {
                let after = start + 7;
                if chars.get(after) == Some(&'(')
                    && let Some(trigger) = balanced_call(&chars, start)
                {
                    return Some((op.to_owned(), trigger, 0));
                }
                i = start + 1;
                continue;
            }
            let mut end = start;
            while end < chars.len()
                && (chars[end].is_alphanumeric() || matches!(chars[end], '_' | '?' | '!'))
            {
                end += 1;
            }
            if end > start && is_head_name(&chars, end) {
                let name: String = chars[start..end].iter().collect();
                return Some((op.to_owned(), name, def_arity(&chars, end)));
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// Balanced `unquote(...)` text as the trigger for generated definitions.
fn balanced_call(chars: &[char], start: usize) -> Option<String> {
    let mut depth = 0_usize;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(chars[start..=i].iter().collect());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// True when the text after a head name continues as a definition.
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

fn def_arity(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    if i >= chars.len() || chars[i] != '(' {
        return 0;
    }
    count_args(chars, i)
}

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

/// ASCII-literal prefix match without allocating (all callers pass ASCII).
fn starts_with_ascii(chars: &[char], pos: usize, prefix: &[u8]) -> bool {
    chars.len() >= pos + prefix.len()
        && chars[pos..pos + prefix.len()]
            .iter()
            .zip(prefix.iter())
            .all(|(got, want)| *got == *want as char)
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
    let has_quote = first_bare_do.is_some_and(|do_idx| {
        words[..do_idx]
            .iter()
            .any(|(word, _)| word.as_str() == "quote")
    });
    for _ in 0..fn_opens {
        blocks.push(false);
    }
    for n in 0..bare_dos {
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
    fn spec_is_clean() {
        let src = "@spec foo(integer) :: integer\ndef foo(x), do: x\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn reports_missing_spec() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo(x), do: x\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn decoy_substrings_do_not_trigger_scans() {
        // "specified", "implement", "defeat" and "done" carry gated
        // substrings but no attributes, definitions or blocks.
        let src = "specified = 1\nimplement = 2\ndefeat = 3\ndone = 4\ndef foo(x), do: x\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 5);
    }
    #[test]
    fn impl_true_needs_no_spec() {
        let src = "defmodule M do\n  @impl true\n  def foo(a), do: a\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn arity_mismatch_is_reported() {
        let src = "@spec foo(integer) :: integer\ndef foo(a, b), do: a\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].trigger, crate::Trigger::Text("foo".to_owned()));
    }
    #[test]
    fn trigger_column_boundary_cases() {
        assert_eq!(trigger_column("def foo(x), do: x", "foo"), Some(5));
        assert_eq!(trigger_column("foo", "foo"), Some(1));
        assert_eq!(trigger_column("(foo, bar)", "foo"), Some(2));
        assert_eq!(trigger_column("x = foobar", "foo"), None);
        assert_eq!(trigger_column("x = barfoo", "foo"), None);
        assert_eq!(
            trigger_column("def caf\u{e9}(x), do: x", "caf\u{e9}"),
            Some(5)
        );
        assert_eq!(trigger_column("a+b", "a+b"), Some(1));
        assert_eq!(trigger_column("", "foo"), None);
    }
}
