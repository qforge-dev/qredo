//! Project-level `SpaceInParentheses` consistency (EX1006).
//!
//! Token-level paren spacing votes merge across the project: `(` vs `)`
//! adjacency, empty enums (`%{}`, `()`, `[]`, `{}`) and the `, ]` exemption
//! follow the upstream collector clauses. The majority wins (an explicit
//! `force` param overrides); `allow_empty_enums` switches which locations
//! report when spaces win. Issues carry the token line and column exactly.

use std::collections::BTreeMap;

use super::{ProjectFile, ProjectIssue, majority};
use crate::helpers;

/// Run the check over a file set.
pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_file: Vec<BTreeMap<String, Vec<Loc>>> = Vec::new();
    for file in files {
        let votes = collect(&file.source);
        for (kind, locs) in &votes {
            *counts.entry(kind.clone()).or_insert(0) += locs.len();
        }
        per_file.push(votes);
    }
    if counts.is_empty() {
        return Vec::new();
    }
    let force = normalized_force(params);
    let Some(expected) = majority(&counts, force.as_deref()) else {
        return Vec::new();
    };
    let allow_empty_enums = helpers::param_bool(params, "allow_empty_enums", false);
    let (actual, message) = issue_text(&expected, allow_empty_enums);
    let mut issues = Vec::new();
    for (index, votes) in per_file.iter().enumerate() {
        if let Some(locs) = votes.get(actual) {
            for loc in locs {
                issues.push(ProjectIssue {
                    file: index,
                    line: Some(loc.line),
                    column: Some(loc.column),
                    trigger: loc.trigger.to_owned(),
                    message: message.clone(),
                    severity: None,
                });
            }
        }
    }
    issues
}

/// One reported paren location in token order (already ascending).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Loc {
    trigger: &'static str,
    line: usize,
    column: usize,
}

impl Loc {
    fn at(trigger: &'static str, tok: &Tok) -> Self {
        Self {
            trigger,
            line: tok.start_line,
            column: tok.start_col,
        }
    }
}

/// Collect per-kind locations over the token stream.
fn collect(source: &str) -> BTreeMap<String, Vec<Loc>> {
    let toks = tokenize(source);
    let mut out: BTreeMap<String, Vec<Loc>> = BTreeMap::new();
    for (index, cur) in toks.iter().enumerate() {
        let prev = if index == 0 {
            None
        } else {
            toks.get(index - 1)
        };
        step(prev, cur, toks.get(index + 1), &mut out);
    }
    out
}

/// One upstream collector clause over a `(prev, current, next)` triple.
fn step(prev: Option<&Tok>, cur: &Tok, next: Option<&Tok>, out: &mut BTreeMap<String, Vec<Loc>>) {
    if let Some(left) = prev
        && left.kind == Kind::Map
        && cur.kind == Kind::Open(b'{')
    {
        step_map(left, next, out);
        return;
    }
    match cur.kind {
        Kind::Open(open) => {
            let Some(right) = next else { return };
            if !on_same_line(cur, right) || right.kind == Kind::Eol {
                return;
            }
            if let Kind::Close(_) = right.kind {
                record(
                    out,
                    adjacent(cur, right),
                    true,
                    Loc::at(empty_trigger(open), cur),
                );
            } else {
                record(
                    out,
                    adjacent(cur, right),
                    false,
                    Loc::at(single_trigger(open), cur),
                );
            }
        }
        Kind::Close(close) => step_close(prev, close, cur, out),
        Kind::Map | Kind::Comma | Kind::Eol | Kind::Other => {}
    }
}

/// Closing-paren triples: the `, ]` exemption first, then votes against the
/// previous token unless it opens a pair or ends a line.
fn step_close(prev: Option<&Tok>, close: u8, cur: &Tok, out: &mut BTreeMap<String, Vec<Loc>>) {
    let Some(left) = prev else { return };
    if left.kind == Kind::Comma && close == b']' && on_same_line(left, cur) {
        return;
    }
    if matches!(left.kind, Kind::Open(_)) || left.kind == Kind::Eol {
        return;
    }
    if !on_same_line(left, cur) {
        return;
    }
    record(
        out,
        adjacent(left, cur),
        false,
        Loc::at(single_trigger(close), cur),
    );
}

/// `%{` triples: empty `%{}` votes once, other maps test `%{}` vs next token.
fn step_map(left: &Tok, next: Option<&Tok>, out: &mut BTreeMap<String, Vec<Loc>>) {
    match next {
        Some(right) if right.kind == Kind::Close(b'}') => {
            record(out, adjacent(left, right), true, Loc::at("%{}", left));
        }
        Some(right) => {
            record(out, adjacent(left, right), false, Loc::at("%{", left));
        }
        None => {
            record(out, false, false, Loc::at("%{", left));
        }
    }
}

/// Record one vote: adjacent pairs vote `without_space`, others `with_space`.
/// Non-empty adjacent pairs additionally vote the `allow_empty_enums` kind.
fn record(out: &mut BTreeMap<String, Vec<Loc>>, is_adjacent: bool, empty_enum: bool, loc: Loc) {
    if is_adjacent {
        push(out, "without_space", loc);
        if !empty_enum {
            push(out, "without_space_allow_empty_enums", loc);
        }
    } else {
        push(out, "with_space", loc);
    }
}

/// Push one location into its kind bucket.
fn push(out: &mut BTreeMap<String, Vec<Loc>>, key: &str, loc: Loc) {
    out.entry(key.to_owned()).or_default().push(loc);
}

/// Same-line guard: the first token ends where the second starts its line.
fn on_same_line(left: &Tok, right: &Tok) -> bool {
    left.end_line == right.start_line
}

/// No-space guard: same line with the first token ending at the second start.
fn adjacent(left: &Tok, right: &Tok) -> bool {
    on_same_line(left, right) && left.end_col == right.start_col
}

/// Trigger for an empty `()`/`[]`/`{}` pair by its opener.
fn empty_trigger(open: u8) -> &'static str {
    match open {
        b'(' => "()",
        b'[' => "[]",
        _ => "{}",
    }
}

/// Trigger for a single paren token.
fn single_trigger(paren: u8) -> &'static str {
    match paren {
        b'(' => "(",
        b')' => ")",
        b'[' => "[",
        b']' => "]",
        b'{' => "{",
        _ => "}",
    }
}

/// Explicit `force` param wins; the gate strips `:atom` colons already.
fn normalized_force(params: &BTreeMap<String, String>) -> Option<String> {
    let bare = helpers::param_str(params, "force", "").trim_start_matches(':');
    if bare == "with_space" || bare == "without_space" || bare == "without_space_allow_empty_enums"
    {
        Some(bare.to_owned())
    } else {
        None
    }
}

/// Reporting kind and message for the winning style.
fn issue_text(expected: &str, allow_empty_enums: bool) -> (&'static str, String) {
    if expected == "with_space" {
        let actual = if allow_empty_enums {
            "without_space_allow_empty_enums"
        } else {
            "without_space"
        };
        (
            actual,
            "There is whitespace around parentheses/brackets most of the time, but here there is not."
                .to_owned(),
        )
    } else {
        (
            "with_space",
            "There is no whitespace around parentheses/brackets most of the time, but here there is."
                .to_owned(),
        )
    }
}

/// Flat token with 1-based char columns (end-exclusive), mirroring the
/// upstream tokenizer positions used by the `is_same_line`/`no_space_between`
/// guards. Strings, charlists, heredocs, sigils and char literals are single
/// opaque `Other` tokens, so their contents never vote.
#[derive(Debug, Clone, Copy)]
struct Tok {
    kind: Kind,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
}

/// Token kinds relevant to the collector clauses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open(u8),
    Close(u8),
    Map,
    Comma,
    Eol,
    Other,
}

/// Char-granular cursor keeping byte offsets on UTF-8 boundaries.
struct Cursor<'a> {
    src: &'a str,
    pos: usize,
    line: usize,
    col: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_next(&self) -> Option<char> {
        let mut chars = self.src[self.pos..].chars();
        chars.next()?;
        chars.next()
    }

    fn starts_with(&self, pat: &str) -> bool {
        self.src[self.pos..].starts_with(pat)
    }

    fn prev_is_name(&self) -> bool {
        self.pos.checked_sub(1).is_some_and(|before| {
            matches!(
                self.src.as_bytes()[before],
                b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'?' | b'!'
            )
        })
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.src[self.pos..].chars().next()?;
        self.pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn token(&self, kind: Kind, line: usize, col: usize) -> Tok {
        Tok {
            kind,
            start_line: line,
            start_col: col,
            end_line: self.line,
            end_col: self.col,
        }
    }
}

/// Lex the source into the flat token stream.
fn tokenize(source: &str) -> Vec<Tok> {
    let mut cur = Cursor {
        src: source,
        pos: 0,
        line: 1,
        col: 1,
    };
    let mut toks = Vec::new();
    while cur.peek().is_some() {
        step_token(&mut cur, &mut toks);
    }
    toks
}

/// Lex one token (or skip whitespace/comments), always making progress.
fn step_token(cur: &mut Cursor<'_>, toks: &mut Vec<Tok>) {
    match cur.peek() {
        Some('\n') => {
            let (line, col) = (cur.line, cur.col);
            cur.bump();
            toks.push(cur.token(Kind::Eol, line, col));
        }
        Some('#') => {
            while matches!(cur.peek(), Some(c) if c != '\n') {
                cur.bump();
            }
        }
        Some('"') => toks.push(span(cur, |c| skip_quoted(c, '"'))),
        Some('\'') => toks.push(span(cur, |c| skip_quoted(c, '\''))),
        _ if cur.starts_with("\"\"\"") || cur.starts_with("'''") => {
            toks.push(span(cur, skip_heredoc));
        }
        _ => step_simple(cur, toks),
    }
}

/// Lex tokens after strings, comments and heredocs are handled.
fn step_simple(cur: &mut Cursor<'_>, toks: &mut Vec<Tok>) {
    match cur.peek() {
        Some('~') => {
            let (line, col) = (cur.line, cur.col);
            if skip_sigil(cur) {
                toks.push(cur.token(Kind::Other, line, col));
            } else {
                cur.bump();
                toks.push(cur.token(Kind::Other, line, col));
            }
        }
        Some('?') => {
            let (line, col) = (cur.line, cur.col);
            if is_char_literal(cur) {
                cur.bump();
                if cur.peek() == Some('\\') {
                    cur.bump();
                }
                cur.bump();
            } else {
                cur.bump();
            }
            toks.push(cur.token(Kind::Other, line, col));
        }
        Some('%') if cur.peek_next() == Some('{') => {
            // The `%{}` token spans both chars (like upstream) while `{`
            // still lexes separately next iteration; only `%` is consumed.
            let (line, col) = (cur.line, cur.col);
            cur.bump();
            toks.push(Tok {
                kind: Kind::Map,
                start_line: line,
                start_col: col,
                end_line: line,
                end_col: col + 2,
            });
        }
        // A lone `%` (struct names like `%Foo{`) is plain code.
        Some('%') => toks.push(single(cur, Kind::Other)),
        Some('(') => toks.push(single(cur, Kind::Open(b'('))),
        Some('{') => toks.push(single(cur, Kind::Open(b'{'))),
        Some('[') => toks.push(single(cur, Kind::Open(b'['))),
        Some(')') => toks.push(single(cur, Kind::Close(b')'))),
        Some('}') => toks.push(single(cur, Kind::Close(b'}'))),
        Some(']') => toks.push(single(cur, Kind::Close(b']'))),
        Some(',') => toks.push(single(cur, Kind::Comma)),
        Some(c) if c.is_whitespace() => {
            cur.bump();
        }
        Some(_) => toks.push(other_run(cur)),
        None => {}
    }
}

/// Lex one single-char token.
fn single(cur: &mut Cursor<'_>, kind: Kind) -> Tok {
    let (line, col) = (cur.line, cur.col);
    cur.bump();
    cur.token(kind, line, col)
}

/// Lex a maximal run of plain code chars as one opaque token.
fn other_run(cur: &mut Cursor<'_>) -> Tok {
    let (line, col) = (cur.line, cur.col);
    while let Some(c) = cur.peek() {
        if c == '\n'
            || c.is_whitespace()
            || matches!(
                c,
                '(' | ')' | '{' | '}' | '[' | ']' | ',' | '"' | '\'' | '#' | '~' | '?' | '%'
            )
        {
            break;
        }
        cur.bump();
    }
    cur.token(Kind::Other, line, col)
}

/// Wrap a skip over literal contents in one opaque token.
fn span(cur: &mut Cursor<'_>, skip: impl FnOnce(&mut Cursor<'_>)) -> Tok {
    let (line, col) = (cur.line, cur.col);
    skip(cur);
    cur.token(Kind::Other, line, col)
}

/// A `?x` char literal starts where an expression can start: the next char is
/// present and blank-free, and the previous byte is not a name byte (so a
/// trailing `?` name suffix does not open a literal).
fn is_char_literal(cur: &Cursor<'_>) -> bool {
    matches!(cur.peek_next(), Some(c) if c != '\n' && c != ' ') && !cur.prev_is_name()
}

/// Skip a `"..."`/`'...'` literal with escapes and `#{...}` interpolation.
fn skip_quoted(cur: &mut Cursor<'_>, quote: char) {
    cur.bump();
    while let Some(c) = cur.peek() {
        if c == '\\' {
            cur.bump();
            cur.bump();
            continue;
        }
        if c == '#' && cur.peek_next() == Some('{') {
            cur.bump();
            cur.bump();
            skip_interp(cur);
            continue;
        }
        cur.bump();
        if c == quote {
            return;
        }
    }
}

/// Skip an interpolation body after `#{`, tracking brace nesting and
/// embedded literals so the true closing `}` ends the scan.
fn skip_interp(cur: &mut Cursor<'_>) {
    let mut depth = 1_usize;
    while cur.peek().is_some() && depth > 0 {
        if skip_embedded(cur) {
            continue;
        }
        match cur.peek() {
            Some('\\') => {
                cur.bump();
                cur.bump();
            }
            Some('#') if cur.peek_next() == Some('{') => {
                cur.bump();
                cur.bump();
                depth += 1;
            }
            Some('#') => {
                while matches!(cur.peek(), Some(c) if c != '\n') {
                    cur.bump();
                }
            }
            Some('{') => {
                cur.bump();
                depth += 1;
            }
            Some('}') => {
                cur.bump();
                depth -= 1;
            }
            _ => {
                cur.bump();
            }
        }
    }
}

/// Skip one embedded literal inside interpolation; false when the cursor is
/// not on a literal opener.
fn skip_embedded(cur: &mut Cursor<'_>) -> bool {
    match cur.peek() {
        Some('"') => {
            skip_quoted(cur, '"');
            true
        }
        Some('\'') => {
            skip_quoted(cur, '\'');
            true
        }
        Some('~') => skip_sigil(cur),
        Some('?') if is_char_literal(cur) => {
            cur.bump();
            if cur.peek() == Some('\\') {
                cur.bump();
            }
            cur.bump();
            true
        }
        _ if cur.starts_with("\"\"\"") || cur.starts_with("'''") => {
            skip_heredoc(cur);
            true
        }
        _ => false,
    }
}

/// Skip a `"""..."""`/`'''...'''` heredoc starting at its opener.
fn skip_heredoc(cur: &mut Cursor<'_>) {
    let triple = if cur.starts_with("\"\"\"") {
        "\"\"\""
    } else {
        "'''"
    };
    let escapes = triple == "\"\"\"";
    cur.bump();
    cur.bump();
    cur.bump();
    while cur.peek().is_some() {
        if escapes && cur.peek() == Some('\\') {
            cur.bump();
            cur.bump();
            continue;
        }
        if cur.starts_with(triple) {
            cur.bump();
            cur.bump();
            cur.bump();
            return;
        }
        cur.bump();
    }
}

/// Skip a `~name<open>...<close>` sigil with escapes, bracket nesting and
/// modifiers; false when `~` does not open a sigil.
fn skip_sigil(cur: &mut Cursor<'_>) -> bool {
    let mut name_end = cur.pos + 1;
    if !matches!(
        cur.src[name_end..].chars().next(),
        Some(c) if c.is_ascii_alphabetic()
    ) {
        return false;
    }
    while let Some(c) = cur.src[name_end..].chars().next() {
        if c.is_ascii_alphanumeric() {
            name_end += c.len_utf8();
        } else {
            break;
        }
    }
    if cur.src[name_end..].starts_with("\"\"\"") || cur.src[name_end..].starts_with("'''") {
        let triple = if cur.src[name_end..].starts_with("\"\"\"") {
            "\"\"\""
        } else {
            "'''"
        };
        while cur.pos < name_end + 3 {
            cur.bump();
        }
        skip_heredoc_body(cur, triple);
        skip_modifiers(cur);
        return true;
    }
    let Some(open) = cur.src[name_end..].chars().next() else {
        return false;
    };
    let close = match open {
        '/' | '|' | '"' | '\'' => open,
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => return false,
    };
    while cur.pos < name_end + open.len_utf8() {
        cur.bump();
    }
    skip_sigil_body(cur, open, close);
    skip_modifiers(cur);
    true
}

/// Skip a triple-quoted sigil body after its opener.
fn skip_heredoc_body(cur: &mut Cursor<'_>, triple: &str) {
    while cur.peek().is_some() {
        if cur.peek() == Some('\\') {
            cur.bump();
            cur.bump();
            continue;
        }
        if cur.starts_with(triple) {
            cur.bump();
            cur.bump();
            cur.bump();
            return;
        }
        cur.bump();
    }
}

/// Skip a bracketed sigil body after its opener.
fn skip_sigil_body(cur: &mut Cursor<'_>, open: char, close: char) {
    let nested = open != close;
    let mut depth = 0_usize;
    while let Some(c) = cur.peek() {
        if c == '\\' {
            cur.bump();
            cur.bump();
            continue;
        }
        if nested && c == open {
            depth += 1;
            cur.bump();
            continue;
        }
        if c == close {
            if nested && depth > 0 {
                depth -= 1;
                cur.bump();
                continue;
            }
            cur.bump();
            return;
        }
        cur.bump();
    }
}

/// Skip trailing sigil modifiers.
fn skip_modifiers(cur: &mut Cursor<'_>) {
    while matches!(cur.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_') {
        cur.bump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../../compatibility/cases/EX1006.json");

    #[derive(serde::Deserialize)]
    struct Entry {
        id: String,
        source: String,
        #[serde(default)]
        group: Option<String>,
        #[serde(default)]
        params: BTreeMap<String, serde_json::Value>,
        #[serde(default)]
        findings: Vec<ExpectedFinding>,
        #[serde(default)]
        excluded_reason: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct ExpectedFinding {
        line: Option<usize>,
        column: Option<usize>,
        trigger: String,
        message: String,
    }

    fn param_string(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Bool(flag) => flag.to_string(),
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::String(text) => text.strip_prefix(':').unwrap_or(text).to_owned(),
            array_or_object => serde_json::to_string(array_or_object).unwrap_or_default(),
        }
    }

    fn finding_key(
        line: Option<usize>,
        column: Option<usize>,
        trigger: &str,
        message: &str,
    ) -> String {
        format!(
            "{}|{}|{trigger}|{message}",
            line.unwrap_or(0),
            column.unwrap_or(0)
        )
    }

    fn source_of(id: &str) -> String {
        let parsed: Vec<Entry> = serde_json::from_str(CASES).expect("valid corpus JSON");
        parsed
            .iter()
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("missing corpus entry {id}"))
            .source
            .clone()
    }

    fn counts_of(source: &str) -> BTreeMap<String, usize> {
        collect(source)
            .into_iter()
            .map(|(kind, locs)| (kind, locs.len()))
            .collect()
    }

    #[test]
    fn oracle_pinned_snippet_counts() {
        // Verified against the upstream collector before porting.
        let cases: [(&str, &[(&str, usize)]); 6] = [
            (
                "%{a: 1}",
                &[("without_space", 2), ("without_space_allow_empty_enums", 2)],
            ),
            ("%{ a: 1 }", &[("with_space", 2)]),
            ("%Foo{}", &[("without_space", 1)]),
            (
                "%Foo{a: 1}",
                &[("without_space", 2), ("without_space_allow_empty_enums", 2)],
            ),
            ("f( )", &[("with_space", 1)]),
            (
                "[foo: 1, bar: 2, ]",
                &[("without_space", 1), ("without_space_allow_empty_enums", 1)],
            ),
        ];
        for (source, expected) in cases {
            let expected: BTreeMap<String, usize> = expected
                .iter()
                .map(|&(kind, count)| (kind.to_owned(), count))
                .collect();
            assert_eq!(counts_of(source), expected, "counts for {source:?}");
        }
    }

    #[test]
    fn oracle_pinned_corpus_counts() {
        // Guards the refute entries against vacuous passes from under-counting.
        let cases = [
            (
                "EX1006.upstream.no-spaces-ok",
                [
                    ("without_space", 33),
                    ("without_space_allow_empty_enums", 31),
                ],
            ),
            (
                "EX1006.upstream.integration-no-spaces",
                [
                    ("without_space", 10),
                    ("without_space_allow_empty_enums", 10),
                ],
            ),
        ];
        for (id, expected) in cases {
            let expected: BTreeMap<String, usize> = expected
                .into_iter()
                .map(|(kind, count)| (kind.to_owned(), count))
                .collect();
            assert_eq!(counts_of(&source_of(id)), expected, "counts for {id}");
        }
    }

    #[test]
    fn corpus_projects_match_upstream_findings() {
        let failures = check_corpus();
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    fn check_corpus() -> Vec<String> {
        let parsed: Vec<Entry> = serde_json::from_str(CASES).expect("valid corpus JSON");
        let mut subgroups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
        for (index, entry) in parsed.iter().enumerate() {
            if entry.excluded_reason.is_some() {
                continue;
            }
            let group = entry.group.clone().unwrap_or_else(|| entry.id.clone());
            let params = serde_json::to_string(&entry.params).unwrap_or_default();
            subgroups.entry((group, params)).or_default().push(index);
        }
        assert!(!subgroups.is_empty(), "corpus must gate something");
        let mut failures = Vec::new();
        for indexes in subgroups.values() {
            failures.extend(check_subgroup(&parsed, indexes));
        }
        failures
    }

    fn check_subgroup(parsed: &[Entry], indexes: &[usize]) -> Vec<String> {
        let first = &parsed[indexes[0]];
        let params: BTreeMap<String, String> = first
            .params
            .iter()
            .map(|(key, value)| (key.clone(), param_string(value)))
            .collect();
        let files: Vec<ProjectFile> = indexes
            .iter()
            .map(|index| ProjectFile {
                filename: parsed[*index].id.clone(),
                source: parsed[*index].source.clone(),
            })
            .collect();
        let issues = run(&files, &params);
        let mut failures = Vec::new();
        for (position, index) in indexes.iter().enumerate() {
            compare_entry(parsed, &issues, position, *index, &mut failures);
        }
        failures
    }

    fn compare_entry(
        parsed: &[Entry],
        issues: &[ProjectIssue],
        position: usize,
        index: usize,
        failures: &mut Vec<String>,
    ) {
        let mut actual: Vec<String> = issues
            .iter()
            .filter(|issue| issue.file == position)
            .map(|issue| finding_key(issue.line, issue.column, &issue.trigger, &issue.message))
            .collect();
        let mut expected: Vec<String> = parsed[index]
            .findings
            .iter()
            .map(|finding| {
                finding_key(
                    finding.line,
                    finding.column,
                    &finding.trigger,
                    &finding.message,
                )
            })
            .collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            failures.push(format!(
                "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
                parsed[index].id
            ));
        }
    }
}
