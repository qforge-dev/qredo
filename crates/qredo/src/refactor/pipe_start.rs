use crate::Finding;
use std::collections::BTreeMap;

const MESSAGE: &str = "Pipe chain should start with a raw value.";

const ALWAYS_VALID_WORDS: &[&str] = &["for", "with", "not", "and", "or", "unquote", "fn"];

/// Operators that make a chain start a raw value rather than a call.
/// (`!`, `=~`, `===` and `!==` are deliberately absent: upstream flags them.)
const VALUE_OPERATORS: &[&str] = &[
    "<-", "|||", "&&&", "<<<", ">>>", "<<~", "~>>", "<~", "<~>", "<|>", "^^^", "~~~", "..//",
    "...", "..", "++", "--", "<>", "&&", "||", "==", "<=", ">=", "+", "-", "*", "/", "<", ">", "|",
    "and", "or", "not", "in", "when",
];

/// Words that end a chain start when met scanning backwards.
/// (`end` is handled separately via its opener kind.)
const BACKWARD_STOP_WORDS: &[&str] = &[
    "do", "else", "fn", "if", "unless", "case", "cond", "with", "for", "try", "quote", "receive",
    "catch", "rescue", "after", "when", "in", "not", "and", "or",
];

/// `EX4023`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let excluded_functions = str_list(params, "excluded_functions");
    let excluded_types = str_list(params, "excluded_argument_types")
        .iter()
        .map(|t| t.strip_prefix(':').unwrap_or(t).to_owned())
        .collect::<Vec<_>>();
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let starts = line_starts(masked);
    // Character and byte indexes built once: every pipe's backward scan
    // shares them instead of rebuilding per pipe.
    let chars: Vec<char> = masked.chars().collect();
    let bytes: Vec<usize> = masked.char_indices().map(|(byte, _)| byte).collect();
    let mut findings = Vec::new();
    for pipe in pipe_positions(masked) {
        if let Chain::First(start) = chain_start(&chars, &bytes, pipe) {
            let segment = masked[start..pipe].trim();
            if start_is_call(segment, &excluded_functions, &excluded_types) {
                let (line_no, line) = line_of(&starts, &lines, pipe);
                findings.push(Finding::with_trigger(
                    line_no,
                    trigger_column(line, "|>"),
                    MESSAGE,
                    "|>".to_owned(),
                ));
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

fn str_list(params: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok())
        .unwrap_or_default()
}

enum Chain {
    First(usize),
    Nested,
}

/// Scan backwards from a pipe for another `|>` (nested chain, skip) or a
/// boundary (first pipe: classify the segment in between). Character and
/// byte indexes come from the caller, built once per file.
fn chain_start(chars: &[char], bytes: &[usize], pipe: usize) -> Chain {
    let mut depth = 0_usize;
    let mut angle = 0_usize;
    let mut i = char_index(bytes, pipe);
    while i > 0 {
        i -= 1;
        match chars[i] {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                if depth > 0 {
                    depth -= 1;
                } else {
                    return Chain::First(after_char(bytes, chars, i));
                }
            }
            _ if depth > 0 => {}
            '>' if i >= 1 && chars[i - 1] == '|' => return Chain::Nested,
            '|' if chars.get(i + 1) == Some(&'>') => return Chain::Nested,
            '>' if i >= 1 && chars[i - 1] == '>' => {
                angle += 1;
                i -= 1;
            }
            '<' if i >= 1 && chars[i - 1] == '<' => {
                angle = angle.saturating_sub(1);
                i -= 1;
            }
            '\n' => {
                if newline_stops(chars, i) {
                    return Chain::First(after_char(bytes, chars, i));
                }
            }
            _ if angle == 0 => {
                if let Some(chain) = word_chain_boundary(chars, bytes, i) {
                    return chain;
                }
            }
            _ => {}
        }
    }
    Chain::First(0)
}

fn after_char(bytes: &[usize], chars: &[char], i: usize) -> usize {
    match (bytes.get(i), chars.get(i)) {
        (Some(byte), Some(c)) => byte + c.len_utf8(),
        _ => 0,
    }
}

fn char_index(bytes: &[usize], pos: usize) -> usize {
    bytes.partition_point(|byte| *byte < pos)
}

/// A word, separator or comparison at char `i` ends the chain start.
fn word_chain_boundary(chars: &[char], bytes: &[usize], i: usize) -> Option<Chain> {
    match chars[i] {
        '=' | ',' | ';' | '>' | '<' => Some(Chain::First(after_char(bytes, chars, i))),
        '!' if chars.get(i + 1) == Some(&'=') => Some(Chain::First(after_char(bytes, chars, i))),
        _ => {
            if word_ending_at(chars, i, b"end") {
                match match_opener(chars, i) {
                    // Raw-value blocks (`for`, `with`, `fn`) end the chain.
                    Opener::RawValue | Opener::None => {
                        Some(Chain::First(after_char(bytes, chars, i)))
                    }
                    Opener::Other => None,
                }
            } else if BACKWARD_STOP_WORDS
                .iter()
                .any(|w| word_ending_at(chars, i, w.as_bytes()))
            {
                Some(Chain::First(after_char(bytes, chars, i)))
            } else {
                None
            }
        }
    }
}

#[derive(PartialEq, Eq)]
enum Opener {
    /// `for`, `with` or `fn`: piping their block result starts a raw value.
    RawValue,
    /// Any other block (`if`, `case`, `quote`, ...): keep scanning.
    Other,
    /// No opener found.
    None,
}

/// Find the `do`/`fn` opening the `end` at char `i`.
fn match_opener(chars: &[char], i: usize) -> Opener {
    let mut depth = 0_usize;
    let mut j = i;
    loop {
        if j == 0 {
            return Opener::None;
        }
        j -= 1;
        if word_ending_at(chars, j, b"end") {
            depth += 1;
        } else if word_ending_at(chars, j, b"fn") {
            if depth == 0 {
                return Opener::RawValue;
            }
            depth -= 1;
        } else if word_ending_at(chars, j, b"do") && chars.get(j + 1) != Some(&':') {
            if depth == 0 {
                return opener_kind(chars, j);
            }
            depth -= 1;
        }
    }
}

/// Classify a block opener by its keyword.
fn opener_kind(chars: &[char], do_at: usize) -> Opener {
    let mut j = do_at;
    while j > 0 {
        j -= 1;
        if word_ending_at(chars, j, b"for") || word_ending_at(chars, j, b"with") {
            return Opener::RawValue;
        }
        for word in ["if", "unless", "case", "cond", "try", "quote", "receive"] {
            if word_ending_at(chars, j, word.as_bytes()) {
                return Opener::Other;
            }
        }
        if word_ending_at(chars, j, b"end") {
            return Opener::Other;
        }
    }
    Opener::Other
}

/// Whether a newline at char `i` ends the previous statement: it does
/// unless either side continues the expression.
fn newline_stops(chars: &[char], i: usize) -> bool {
    if let Some(c) = prev_non_ws(chars, i) {
        if is_continuation_char(c) {
            return false;
        }
        if is_name_char(c) {
            if !prev_word_is(chars, i, b"end") {
                return true;
            }
        } else if !(c == ')' || c == ']' || c == '}' || c == '"' || c == '\'') {
            return true;
        }
    } else {
        return true;
    }
    match next_non_ws(chars, i) {
        None => true,
        Some((c, at)) => {
            if !is_continuation_start(c) {
                return true;
            }
            c == '-' && chars.get(at + 1) == Some(&'>')
        }
    }
}

fn is_continuation_char(c: char) -> bool {
    matches!(
        c,
        '|' | '>' | '<' | '=' | '+' | '*' | '/' | '&' | '.' | '~' | '^' | ',' | '(' | '[' | '{'
    )
}

fn is_continuation_start(c: char) -> bool {
    matches!(
        c,
        '|' | '+' | '-' | '*' | '/' | '>' | '<' | '&' | '.' | '~' | '^'
    )
}

fn prev_non_ws(chars: &[char], i: usize) -> Option<char> {
    let mut j = i;
    while j > 0 {
        j -= 1;
        if !chars[j].is_whitespace() {
            return Some(chars[j]);
        }
    }
    None
}

fn next_non_ws(chars: &[char], i: usize) -> Option<(char, usize)> {
    let mut j = i + 1;
    while j < chars.len() {
        if !chars[j].is_whitespace() {
            return Some((chars[j], j));
        }
        j += 1;
    }
    None
}

fn prev_word_is(chars: &[char], i: usize, word: &[u8]) -> bool {
    let mut j = i;
    while j > 0 && chars[j - 1].is_whitespace() {
        j -= 1;
    }
    j >= word.len()
        && chars[j - word.len()..j]
            .iter()
            .zip(word.iter())
            .all(|(got, want)| *got == *want as char)
        && (j < word.len() + 1 || !is_name_char(chars[j - word.len() - 1]))
}

/// Whether the chain start is a function call with arguments that is not
/// covered by the exclusion parameters.
fn start_is_call(segment: &str, excluded_functions: &[String], excluded_types: &[String]) -> bool {
    let start = strip_outer_parens(segment.trim());
    if start.is_empty() {
        return false;
    }
    // Leading `:` is either a `do:`/`key:` separator (`: value`) or an
    // Erlang call/atom (`:mod.fun(...)` / `:atom`). Atoms are raw values.
    if let Some(stripped) = start.strip_prefix(':') {
        let after = stripped.trim_start();
        if stripped.starts_with([' ', '\t']) {
            if after.is_empty() {
                return false;
            }
            return start_is_call(after, excluded_functions, excluded_types);
        }
        if let Some((callee, args)) = split_paren_call(start) {
            return !excluded(&callee, args, excluded_functions, excluded_types);
        }
        if let Some((callee, first_arg)) = split_space_call(start) {
            return !excluded(&callee, first_arg, excluded_functions, excluded_types);
        }
        return false;
    }
    // `key: value` keyword/map value as chain start: classify the value.
    if let Some(value) = keyword_value(start) {
        return start_is_call(value, excluded_functions, excluded_types);
    }
    let first = start.chars().next().unwrap_or(' ');
    if is_value_prefix(first) {
        return false;
    }
    if starts_with_word(start, ALWAYS_VALID_WORDS) {
        return false;
    }
    if starts_with_bang(start) {
        return true;
    }
    if let Some((callee, args)) = split_paren_call(start) {
        return !excluded(&callee, args, excluded_functions, excluded_types);
    }
    if let Some((callee, first_arg)) = split_space_call(start) {
        return !excluded(&callee, first_arg, excluded_functions, excluded_types);
    }
    if first.is_ascii_uppercase() {
        return false;
    }
    if has_value_operator(start) {
        return false;
    }
    has_flagged_operator(start)
}

fn is_value_prefix(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | '~' | '@' | '%' | '{' | '[' | '&' | ':' | '_' | '?' | '+' | '-'
    ) || c.is_ascii_digit()
}

fn starts_with_word(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| {
        text.starts_with(w)
            && text[w.len()..]
                .chars()
                .next()
                .is_none_or(|c| !is_name_char(c))
    })
}

fn starts_with_bang(text: &str) -> bool {
    text.starts_with('!') && !text[1..].starts_with('=')
}

/// Split a pure `callee(args)` call: the first depth-zero `(` with nothing
/// but whitespace after its match.
fn split_paren_call(text: &str) -> Option<(String, &str)> {
    let chars: Vec<char> = text.chars().collect();
    let bytes: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => {
                if chars[i] == '(' && depth == 0 {
                    let callee = text[..bytes[i]].trim().to_owned();
                    if !is_callee(&callee) {
                        return None;
                    }
                    let inner = balanced_inner(&text[bytes[i] + 1..])?;
                    if text[bytes[i] + 1 + inner.len() + 1..].trim().is_empty() {
                        return Some((callee, inner));
                    }
                    return None;
                }
                depth += 1;
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_callee(callee: &str) -> bool {
    !callee.is_empty()
        && callee.chars().all(|c| {
            c.is_alphanumeric() || c == '_' || c == '.' || c == ':' || c == '?' || c == '!'
        })
}

/// Split a space-separated `callee arg` call.
fn split_space_call(text: &str) -> Option<(String, &str)> {
    let chars: Vec<char> = text.chars().collect();
    let bytes: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    let mut i = 0_usize;
    while i < chars.len() && is_callee_char(chars[i]) {
        i += 1;
    }
    if i == 0 || i >= chars.len() || (chars[i] != ' ' && chars[i] != '\t') {
        return None;
    }
    let callee = text[..bytes[i]].to_owned();
    let rest = text[bytes[i]..].trim_start();
    if rest.is_empty() || is_excluded_rest_start(rest) {
        return None;
    }
    Some((callee, rest))
}

fn is_callee_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == ':' || c == '?' || c == '!'
}

fn is_excluded_rest_start(rest: &str) -> bool {
    const OPERATOR_STARTS: &[char] = &[
        '<', '>', '=', '|', ',', ';', ')', ']', '}', '+', '-', '*', '/', '&', '~', '^',
    ];
    if rest.starts_with(OPERATOR_STARTS) {
        return true;
    }
    starts_with_word(rest, &["and", "or", "not", "in"])
}

/// Leading `key:` in `key: value` chain starts: the value after the colon.
fn keyword_value(text: &str) -> Option<&str> {
    let mut idx = 0_usize;
    for (byte, c) in text.char_indices() {
        if c.is_alphanumeric() || c == '_' || c == '?' || c == '!' {
            idx = byte + c.len_utf8();
        } else {
            break;
        }
    }
    if idx == 0 {
        return None;
    }
    let rest = text.get(idx..)?;
    if !rest.starts_with(':') || rest[1..].starts_with(':') {
        return None;
    }
    let after = rest[1..].trim_start();
    if after.is_empty() {
        return None;
    }
    Some(after)
}

fn has_value_operator(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => {
                for op in VALUE_OPERATORS {
                    if operator_at(&chars, i, op.as_bytes()) {
                        return true;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Whether the start uses an operator shape upstream flags (`!`, `=~`,
/// `===`, `!==`).
fn has_flagged_operator(text: &str) -> bool {
    text.contains('!') || text.contains("=~") || text.contains("===") || text.contains("!==")
}

fn operator_at(chars: &[char], i: usize, op: &[u8]) -> bool {
    if chars.len() < i + op.len()
        || !(chars[i..i + op.len()]
            .iter()
            .zip(op.iter())
            .all(|(got, want)| *got == *want as char))
    {
        return false;
    }
    if op.iter().all(u8::is_ascii_alphanumeric) {
        let before_ok = i == 0 || !is_name_char(chars[i - 1]);
        let after_ok = chars.get(i + op.len()).is_none_or(|c| !is_name_char(*c));
        before_ok && after_ok
    } else {
        true
    }
}

/// Whether the call is covered by `excluded_functions` or its first
/// argument type is covered by `excluded_argument_types`.
fn excluded(
    callee: &str,
    args: &str,
    excluded_functions: &[String],
    excluded_types: &[String],
) -> bool {
    if args.trim().is_empty() {
        return true;
    }
    let name = callee.trim().to_owned();
    if excluded_functions
        .iter()
        .any(|entry| entry == &name || format!(":{entry}") == name)
    {
        return true;
    }
    let first = first_argument(args);
    argument_types(&first)
        .iter()
        .any(|t| excluded_types.iter().any(|e| e == t))
}

/// The first top-level comma-separated argument.
fn first_argument(args: &str) -> String {
    let chars: Vec<char> = args.chars().collect();
    let bytes: Vec<usize> = args.char_indices().map(|(byte, _)| byte).collect();
    let mut depth = 0_usize;
    let mut angle = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        if chars[i] == '<' && chars.get(i + 1) == Some(&'<') {
            angle += 1;
            i += 2;
            continue;
        }
        if chars[i] == '>' && chars.get(i + 1) == Some(&'>') && angle > 0 {
            angle -= 1;
            i += 2;
            continue;
        }
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 && angle == 0 => return args[..bytes[i]].trim().to_owned(),
            _ => {}
        }
        i += 1;
    }
    args.trim().to_owned()
}

/// Credo-equivalent first-argument types; unknown shapes never match
/// exclusions.
fn argument_types(first: &str) -> Vec<String> {
    let arg = strip_outer_parens(first.trim());
    if arg.is_empty() || has_top_pipe(arg) {
        return Vec::new();
    }
    if let Some(types) = sigil_types(arg) {
        return types;
    }
    if starts_with_word(arg, &["fn"]) {
        return vec!["fn".to_owned()];
    }
    if arg.starts_with("%{") {
        return vec!["map".to_owned()];
    }
    if arg.starts_with('{') {
        return vec!["tuple".to_owned()];
    }
    if arg.starts_with('"') {
        return vec!["binary".to_owned()];
    }
    if arg.starts_with('\'') {
        return vec!["list".to_owned()];
    }
    if arg.starts_with('[') {
        if is_keyword_list(arg) {
            return vec!["keyword".to_owned(), "list".to_owned()];
        }
        return vec!["list".to_owned()];
    }
    if arg.starts_with("<<") {
        return vec!["bitstring".to_owned()];
    }
    if arg == "nil" {
        return vec!["atom".to_owned()];
    }
    if arg == "true" || arg == "false" {
        return vec!["boolean".to_owned()];
    }
    if arg.starts_with(':') {
        return vec!["atom".to_owned()];
    }
    if arg.starts_with('?') {
        return vec!["number".to_owned()];
    }
    number_types(arg)
}

/// Sigil first-argument types (`~r`/`~R` also count as `:regex`).
fn sigil_types(arg: &str) -> Option<Vec<String>> {
    let rest = arg.strip_prefix('~')?;
    let name: String = rest
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    if name.len() == 1 {
        if name == "r" || name == "R" {
            return Some(vec![format!("sigil_{name}"), "regex".to_owned()]);
        }
        return Some(vec![format!("sigil_{name}")]);
    }
    Some(Vec::new())
}

fn number_types(arg: &str) -> Vec<String> {
    if let Some(unsigned) = arg.strip_prefix(['+', '-']) {
        let unsigned = unsigned.trim_start();
        if starts_with_number(unsigned) && !is_range(unsigned) {
            return vec!["number".to_owned()];
        }
        return Vec::new();
    }
    if starts_with_number(arg) && !is_range(arg) {
        return vec!["number".to_owned()];
    }
    Vec::new()
}

fn has_top_pipe(arg: &str) -> bool {
    let chars: Vec<char> = arg.chars().collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '|' if depth == 0 && chars.get(i + 1) == Some(&'>') => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn starts_with_number(arg: &str) -> bool {
    arg.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Whether the argument is a `..` range rather than a plain number.
fn is_range(arg: &str) -> bool {
    let mut chars = arg.chars().peekable();
    while chars.next_if(|c| c.is_ascii_digit() || *c == '_').is_some() {}
    chars.next() == Some('.') && chars.next() == Some('.')
}

fn is_keyword_list(arg: &str) -> bool {
    let inner = arg.strip_prefix('[').unwrap_or(arg);
    let mut chars = inner.trim_start().chars();
    let mut name = String::new();
    for c in chars.by_ref() {
        if c.is_alphanumeric() || c == '_' {
            name.push(c);
        } else {
            break;
        }
    }
    !name.is_empty() && chars.next() == Some(':')
}

/// Remove redundant surrounding parentheses.
fn strip_outer_parens(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim();
        if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
            return trimmed;
        }
        let inner = &trimmed[1..trimmed.len() - 1];
        if balanced(inner) {
            text = inner;
        } else {
            return trimmed;
        }
    }
}

fn balanced(text: &str) -> bool {
    let mut depth = 0_usize;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    depth == 0
}

/// Text inside balanced parens starting just after the opener.
fn balanced_inner(rest: &str) -> Option<&str> {
    let mut depth = 0_usize;
    for (byte, c) in rest.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    if c == ')' {
                        return Some(&rest[..byte]);
                    }
                    return None;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '?' || c == '!'
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether `word` ends at char `i` with identifier boundaries.
/// ASCII words only, compared without allocating.
fn word_ending_at(chars: &[char], i: usize, word: &[u8]) -> bool {
    if word.is_empty() || i + 1 < word.len() {
        return false;
    }
    if !(chars[i + 1 - word.len()..=i]
        .iter()
        .zip(word.iter())
        .all(|(got, want)| *got == *want as char))
    {
        return false;
    }
    if i + 1 == word.len() {
        return true;
    }
    let prev = chars[i - word.len()];
    if is_name_char(prev) || prev == '.' || prev == ':' || prev == '@' {
        return false;
    }
    chars.get(i + 1).is_none_or(|c| !is_name_char(*c))
}

/// Byte offsets of `|>` operators.
fn pipe_positions(masked: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut search = 0_usize;
    while let Some(rel) = masked[search..].find("|>") {
        out.push(search + rel);
        search += rel + "|>".len();
    }
    out
}

fn line_starts(masked: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    for (byte, c) in masked.char_indices() {
        if c == '\n' {
            starts.push(byte + 1);
        }
    }
    starts
}

fn line_of<'a>(starts: &[usize], lines: &[&'a str], pos: usize) -> (usize, &'a str) {
    let line_no = starts.partition_point(|start| *start <= pos).max(1);
    (line_no, lines.get(line_no - 1).copied().unwrap_or(""))
}

fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find(trigger) {
        let pos = search + rel;
        if column_boundary_before(line, pos, trigger) && column_boundary_after(line, pos, trigger) {
            return Some(line[..pos].chars().count() + 1);
        }
        search = pos + 1;
    }
    None
}

fn column_boundary_before(line: &str, pos: usize, trigger: &str) -> bool {
    let first = trigger.chars().next();
    match line[..pos].chars().next_back() {
        None => first.is_some_and(is_word_char),
        Some(c) => {
            c.is_whitespace() || c == '(' || c == ')' || c == ',' || boundary_flip(Some(c), first)
        }
    }
}

fn column_boundary_after(line: &str, pos: usize, trigger: &str) -> bool {
    let last = trigger.chars().next_back();
    match line[pos + trigger.len()..].chars().next() {
        None => last.is_some_and(is_word_char),
        Some(c) => {
            c.is_whitespace() || c == '(' || c == ')' || c == ',' || boundary_flip(last, Some(c))
        }
    }
}

fn boundary_flip(left: Option<char>, right: Option<char>) -> bool {
    match (left, right) {
        (Some(l), Some(r)) => is_word_char(l) != is_word_char(r),
        (Some(l), None) => is_word_char(l),
        (None, Some(r)) => is_word_char(r),
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn value_start_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("x |> foo()\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_call_start() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("String.trim(\"nope\") |> String.upcase\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
        assert_eq!(findings[0].column, Some(21));
        assert_eq!(
            findings[0].message,
            "Pipe chain should start with a raw value."
        );
    }
    #[test]
    fn reports_continuation_line_start() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(
                "String.trim(\"nope\")\n|> String.downcase\n|> String.trim\n",
            ),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, None);
    }
    #[test]
    fn honors_excluded_functions() {
        let mut params = BTreeMap::new();
        params.insert(
            "excluded_functions".to_owned(),
            "[\"String.trim\", \"table\"]".to_owned(),
        );
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("String.trim(\"users\") |> String.upcase\n"),
                &params
            )
            .is_empty()
        );
    }
    #[test]
    fn honors_excluded_argument_types() {
        let mut params = BTreeMap::new();
        params.insert(
            "excluded_argument_types".to_owned(),
            "[\":regex\"]".to_owned(),
        );
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("table(~r/regex/)\n|> DB.run\n"),
                &params
            )
            .is_empty()
        );
    }
    #[test]
    fn erlang_call_start_reports() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(":crypto.hash(:sha256, x) |> Base.encode16()\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
    }
    #[test]
    fn do_colon_call_start_reports() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("def kinds, do: Map.keys(@queries) |> Enum.sort()\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
    }
    #[test]
    fn keyword_value_start_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy(
                    "%{timestamp: DateTime.utc_now() |> DateTime.to_iso8601()}\n"
                ),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn operator_rest_start_is_clean() {
        let src = "def d(groups, measures) do\n  group_cols = Enum.map(groups, &quote_identifier/1)\n  (group_cols ++ Enum.map(measures, &grouped_measure/1)) |> Enum.join(\", \")\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
}
