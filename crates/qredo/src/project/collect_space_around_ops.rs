//! Project-level `SpaceAroundOperators` consistency (EX1005).
//!
//! Per-file operator spacing votes merge across the project: an operator with
//! a space on either side votes `with_space`, an operator adjacent on either
//! side (outside the usual no-space shapes) votes `without_space`. The
//! majority wins and files holding the other style get one issue per
//! surviving location after the `ignore` list and the number, capture,
//! binary-size, typespec and call-argument filters.
//!
//! Oracle-verified token notes: `<<` truncates the rest of the stream (the
//! upstream binary drop never matches the tuple `>>`); a symbolic operator
//! followed by `/` on its line reads as a capture name (`&+/2`, `1 + / 2`);
//! `+/-` after an identifier followed by space, tab or `(` reads as a call
//! argument (`assert -24`) while a glued variable does not (`b+c`); `@spec`
//! and `@type` lines never vote.

use std::collections::BTreeMap;

use super::{ProjectFile, ProjectIssue, majority};
use crate::helpers;

/// Run the check over a file set.
pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let (counts, per_file) = tally(files);
    if counts.is_empty() {
        return Vec::new();
    }
    let force = helpers::param_str(params, "force", "");
    let force = if force.is_empty() { None } else { Some(force) };
    let Some(expected) = majority(&counts, force) else {
        return Vec::new();
    };
    let message = issue_message(&expected);
    let ignored = ignored_triggers(params);
    let mut issues = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let unexpected = per_file[index]
            .iter()
            .any(|vote| votes_against(vote, &expected));
        if !unexpected {
            continue;
        }
        let lines: Vec<&str> = file.source.split('\n').collect();
        for vote in &per_file[index] {
            if !votes_against(vote, &expected) || ignored.iter().any(|item| item == &vote.trigger) {
                continue;
            }
            let line = lines.get(vote.line.wrapping_sub(1)).unwrap_or(&"");
            if filtered(line, vote) {
                continue;
            }
            issues.push(ProjectIssue {
                file: index,
                line: Some(vote.line),
                column: Some(vote.col),
                trigger: vote.trigger.clone(),
                message: message.to_owned(),
                severity: None,
            });
        }
    }
    issues
}

/// Per-file operator verdicts plus merged across-file counts.
fn tally(files: &[ProjectFile]) -> (BTreeMap<String, usize>, Vec<Vec<OpVote>>) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_file: Vec<Vec<OpVote>> = Vec::new();
    for file in files {
        let votes = collect(&file.source);
        for vote in &votes {
            if vote.with_v {
                *counts.entry("with_space".to_owned()).or_insert(0) += 1;
            }
            if vote.without_v {
                *counts.entry("without_space".to_owned()).or_insert(0) += 1;
            }
        }
        per_file.push(votes);
    }
    (counts, per_file)
}

/// One examined operator with its spacing verdicts.
struct OpVote {
    line: usize,
    col: usize,
    trigger: String,
    with_v: bool,
    without_v: bool,
    callish_prev: bool,
}

/// Per-file operator verdicts in document order.
fn collect(source: &str) -> Vec<OpVote> {
    let toks = tokenize(source);
    let lines: Vec<&str> = source.split('\n').collect();
    let mut votes = Vec::new();
    let mut pos = 0_usize;
    loop {
        pos = skip_window(&toks, pos);
        if pos + 3 > toks.len() {
            break;
        }
        let (prev, current, next) = (&toks[pos], &toks[pos + 1], &toks[pos + 2]);
        if current.kind == Kind::Op {
            let with_v = spaced(prev, current) || spaced(current, next);
            let without_v = (!usually_before(prev, current) && adjacent(prev, current))
                || (!usually_after(prev, current, next)
                    && adjacent(current, next)
                    && next.kind != Kind::Eol);
            votes.push(OpVote {
                line: current.line,
                col: current.col,
                trigger: current.text.clone(),
                with_v,
                without_v,
                callish_prev: followed_by_call_char(&lines, prev),
            });
        }
        pos += 1;
    }
    votes
}

/// Whether the vote contradicts the project-wide `expected` style.
fn votes_against(vote: &OpVote, expected: &str) -> bool {
    if expected == "without_space" {
        vote.with_v
    } else {
        vote.without_v
    }
}

/// Message for the winning style.
fn issue_message(expected: &str) -> &'static str {
    if expected == "without_space" {
        "There are no spaces around operators most of the time, but here there are."
    } else {
        "There are spaces around operators most of the time, but not here."
    }
}

/// Operators ignored for issues; the gate encodes lists as compact JSON.
fn ignored_triggers(params: &BTreeMap<String, String>) -> Vec<String> {
    let Some(raw) = params.get("ignore") else {
        return vec!["|".to_owned()];
    };
    match serde_json::from_str::<Vec<String>>(raw) {
        Ok(items) => items
            .iter()
            .map(|item| item.strip_prefix(':').unwrap_or(item).to_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Token kinds mirroring the shapes `SpaceHelper` matches on.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Kind {
    Op,
    Ident,
    ParenIdent,
    Number,
    Caret,
    Atom,
    Str,
    Alias,
    Keyword,
    At,
    Capture,
    CaptureInt,
    Dot,
    DotCall,
    Comma,
    Colon,
    Assoc,
    Type,
    Arrow,
    Range,
    Ellipsis,
    BinOpen,
    BinClose,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    MapOpen,
    Percent,
    Semi,
    Eol,
    Other,
}

/// One scanned token with 1-based line/column starts and an end column.
#[derive(Clone, Debug)]
struct Tok {
    kind: Kind,
    text: String,
    line: usize,
    col: usize,
    end: usize,
}

/// Scan cursor over the source with line/column tracking.
struct Cursor<'a> {
    src: &'a str,
    pos: usize,
    line: usize,
    col: usize,
}

impl Cursor<'_> {
    /// Next character without consuming it.
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    /// Character `offset` ahead without consuming it.
    fn peek_at(&self, offset: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(offset)
    }

    /// Last consumed character, if any.
    fn prev_char(&self) -> Option<char> {
        self.src[..self.pos].chars().next_back()
    }

    /// Consume one character, tracking newlines.
    fn bump(&mut self, current: char) {
        self.pos += current.len_utf8();
        if current == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
    }

    /// Source text consumed since `start`.
    fn text(&self, start: usize) -> String {
        self.src[start..self.pos].to_owned()
    }

    /// Build a token from a scanned span.
    fn tok(&self, kind: Kind, start: usize, line: usize, col: usize) -> Tok {
        Tok {
            kind,
            text: self.text(start),
            line,
            col,
            end: self.col,
        }
    }
}

/// Tokenize the source into the operator-relevant token stream.
fn tokenize(source: &str) -> Vec<Tok> {
    let mut cursor = Cursor {
        src: source,
        pos: 0,
        line: 1,
        col: 1,
    };
    let mut toks = Vec::new();
    while cursor.pos < cursor.src.len() {
        toks.extend(next_toks(&mut cursor));
    }
    demote_non_operators(&mut toks);
    toks
}

/// Demote symbolic operators native lexes as non-operators: an operator
/// directly glued to `:` is an atom (`:*`, `:==`, `:->` read as
/// `{:atom, …}`), and one glued to `.` is a remote call name
/// (`Kernel.||` reads as `{:paren_identifier, …}`); neither satisfies
/// native `operator?/1` (`space_helper.ex`). `::` lexes as `Type` and
/// `..` as `Range`, so they never trigger this.
fn demote_non_operators(toks: &mut [Tok]) {
    for index in 0..toks.len() {
        if toks[index].kind != Kind::Op {
            continue;
        }
        let (previous_kind, adjacent) = match index.checked_sub(1).and_then(|at| toks.get(at)) {
            Some(previous) => (
                previous.kind.clone(),
                previous.line == toks[index].line && previous.end == toks[index].col,
            ),
            None => continue,
        };
        if !adjacent {
            continue;
        }
        if previous_kind == Kind::Colon {
            toks[index].kind = Kind::Atom;
        } else if previous_kind == Kind::Dot {
            toks[index].kind = Kind::ParenIdent;
        }
    }
}

/// Scan the tokens starting at the cursor (gaps yield none).
fn next_toks(cursor: &mut Cursor) -> Vec<Tok> {
    let start = cursor.pos;
    let (line, col) = (cursor.line, cursor.col);
    let Some(current) = cursor.peek() else {
        return Vec::new();
    };
    match current {
        '\n' => {
            cursor.bump(current);
            vec![Tok {
                kind: Kind::Eol,
                text: String::new(),
                line,
                col,
                end: col + 3,
            }]
        }
        '#' => {
            skip_comment(cursor);
            Vec::new()
        }
        '"' | '\'' => vec![scan_quoted(cursor, start, line, col, current)],
        '~' => scan_sigil_or_symbol(cursor, start, line, col),
        '?' => vec![scan_char_or_other(cursor, start, line, col)],
        current if current.is_ascii_digit() => vec![scan_number(cursor, start, line, col)],
        current if current.is_ascii_lowercase() || current == '_' || current.is_alphabetic() => {
            vec![scan_word(cursor, start, line, col)]
        }
        current if current.is_ascii_uppercase() => vec![scan_alias(cursor, start, line, col)],
        ':' => vec![scan_colon(cursor, start, line, col)],
        '.' => vec![scan_dot(cursor, start, line, col)],
        _ => next_simple_tok(cursor, start, line, col, current),
    }
}

/// Single-shape punctuation tokens; falls back to symbolic scanning.
fn next_simple_tok(
    cursor: &mut Cursor,
    start: usize,
    line: usize,
    col: usize,
    current: char,
) -> Vec<Tok> {
    match current {
        '(' | ')' | '[' | ']' | '{' | '}' => {
            cursor.bump(current);
            let kind = match current {
                '(' => Kind::LParen,
                ')' => Kind::RParen,
                '[' => Kind::LBracket,
                ']' => Kind::RBracket,
                '{' => Kind::LBrace,
                _ => Kind::RBrace,
            };
            vec![cursor.tok(kind, start, line, col)]
        }
        '%' => scan_percent(cursor, start, line, col),
        ',' => {
            cursor.bump(current);
            vec![cursor.tok(Kind::Comma, start, line, col)]
        }
        ';' => {
            cursor.bump(current);
            vec![cursor.tok(Kind::Semi, start, line, col)]
        }
        '@' => {
            cursor.bump(current);
            vec![cursor.tok(Kind::At, start, line, col)]
        }
        '&' => vec![scan_capture(cursor, start, line, col)],
        current if current.is_whitespace() => {
            cursor.bump(current);
            Vec::new()
        }
        _ => vec![scan_symbolic(cursor, start, line, col)],
    }
}

/// Skip a `#` comment, leaving the newline for the EOL token.
fn skip_comment(cursor: &mut Cursor) {
    while let Some(current) = cursor.peek() {
        if current == '\n' {
            break;
        }
        cursor.bump(current);
    }
}

/// Scan a string, charlist or heredoc as one opaque token.
fn scan_quoted(cursor: &mut Cursor, start: usize, line: usize, col: usize, quote: char) -> Tok {
    cursor.bump(quote);
    let triple = cursor.peek() == Some(quote) && cursor.peek_at(1) == Some(quote);
    if triple {
        if let Some(second) = cursor.peek() {
            cursor.bump(second);
        }
        if let Some(third) = cursor.peek() {
            cursor.bump(third);
        }
    }
    loop {
        let Some(current) = cursor.peek() else {
            break;
        };
        if current == '\\' {
            cursor.bump(current);
            if let Some(escaped) = cursor.peek() {
                cursor.bump(escaped);
            }
            continue;
        }
        if current != quote {
            cursor.bump(current);
            continue;
        }
        if !triple {
            cursor.bump(current);
            break;
        }
        if cursor.peek_at(1) == Some(quote) && cursor.peek_at(2) == Some(quote) {
            cursor.bump(current);
            if let Some(second) = cursor.peek() {
                cursor.bump(second);
            }
            if let Some(third) = cursor.peek() {
                cursor.bump(third);
            }
            break;
        }
        cursor.bump(current);
    }
    cursor.tok(Kind::Str, start, line, col)
}

/// Scan a sigil, or fall back to symbolic operators (`~>`, `~`).
fn scan_sigil_or_symbol(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Vec<Tok> {
    let mut name_end = start + 1;
    while cursor.src[name_end..]
        .chars()
        .next()
        .is_some_and(|current| current.is_ascii_alphabetic())
    {
        name_end += 1;
    }
    if name_end == start + 1 {
        return vec![scan_symbolic(cursor, start, line, col)];
    }
    let opener = cursor.src[name_end..].chars().next();
    if !opener.is_some_and(sigil_opener) {
        if let Some(mark) = cursor.peek() {
            cursor.bump(mark);
        }
        return vec![cursor.tok(Kind::Other, start, line, col)];
    }
    cursor.pos = name_end;
    cursor.col += name_end - start - 1;
    vec![scan_sigil(cursor, start, line, col)]
}

/// Sigil opening delimiters.
fn sigil_opener(current: char) -> bool {
    matches!(current, '(' | '[' | '{' | '<' | '|' | '/' | '"' | '\'')
}

/// Matching sigil closer for an opener.
fn sigil_closer(opener: char) -> char {
    match opener {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => opener,
    }
}

/// Scan a sigil body through its closer and modifiers as one token.
fn scan_sigil(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    let Some(opener) = cursor.peek() else {
        return cursor.tok(Kind::Str, start, line, col);
    };
    let (triple, closer, nested) = consume_sigil_opener(cursor, opener);
    scan_sigil_body(cursor, opener, triple, closer, nested);
    while cursor
        .peek()
        .is_some_and(|current| current.is_ascii_alphabetic())
    {
        if let Some(modifier) = cursor.peek() {
            cursor.bump(modifier);
        }
    }
    cursor.tok(Kind::Str, start, line, col)
}

/// Consume the sigil opener; returns (triple-quoted, closer, nested).
fn consume_sigil_opener(cursor: &mut Cursor, opener: char) -> (bool, char, bool) {
    let triple = (opener == '"' || opener == '\'')
        && cursor.peek_at(1) == Some(opener)
        && cursor.peek_at(2) == Some(opener);
    for _ in 0..usize::from(triple) * 3 + usize::from(!triple) {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
    }
    let closer = sigil_closer(opener);
    let nested = closer != opener && !triple;
    (triple, closer, nested)
}

/// Consume a sigil body up to its closer.
fn scan_sigil_body(cursor: &mut Cursor, opener: char, triple: bool, closer: char, nested: bool) {
    if triple {
        scan_triple_body(cursor, opener);
        return;
    }
    scan_delimited_body(cursor, opener, closer, nested);
}

/// Consume a triple-quoted sigil body up to the closing triple.
fn scan_triple_body(cursor: &mut Cursor, opener: char) {
    loop {
        let Some(current) = cursor.peek() else {
            break;
        };
        if current == '\\' {
            cursor.bump(current);
            if let Some(escaped) = cursor.peek() {
                cursor.bump(escaped);
            }
            continue;
        }
        if current == opener
            && cursor.peek_at(1) == Some(opener)
            && cursor.peek_at(2) == Some(opener)
        {
            for _ in 0..3 {
                if let Some(mark) = cursor.peek() {
                    cursor.bump(mark);
                }
            }
            break;
        }
        cursor.bump(current);
    }
}

/// Consume a delimited sigil body; brackets nest.
fn scan_delimited_body(cursor: &mut Cursor, opener: char, closer: char, nested: bool) {
    let mut depth = 0_usize;
    loop {
        let Some(current) = cursor.peek() else {
            break;
        };
        if current == '\\' {
            cursor.bump(current);
            if let Some(escaped) = cursor.peek() {
                cursor.bump(escaped);
            }
            continue;
        }
        if nested && current == opener {
            depth += 1;
            cursor.bump(current);
            continue;
        }
        if current == closer {
            if nested && depth > 0 {
                depth -= 1;
                cursor.bump(current);
                continue;
            }
            cursor.bump(current);
            break;
        }
        cursor.bump(current);
    }
}

/// Scan a `?x` char literal, or a bare `?` outside strings.
fn scan_char_or_other(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    let literal = !matches!(cursor.prev_char(), Some(previous) if is_name_char(previous))
        && matches!(cursor.peek_at(1), Some(next) if !next.is_whitespace());
    if !literal {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
        return cursor.tok(Kind::Other, start, line, col);
    }
    if let Some(mark) = cursor.peek() {
        cursor.bump(mark);
    }
    if let Some(current) = cursor.peek() {
        cursor.bump(current);
        if current == '\\'
            && let Some(escaped) = cursor.peek()
        {
            cursor.bump(escaped);
        }
    }
    cursor.tok(Kind::Atom, start, line, col)
}

/// Scan an integer or float literal as one token.
fn scan_number(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    if cursor.peek() == Some('0')
        && matches!(cursor.peek_at(1), Some('x' | 'X' | 'o' | 'O' | 'b' | 'B'))
    {
        return scan_based_number(cursor, start, line, col);
    }
    scan_number_digits(cursor);
    scan_frac_part(cursor);
    scan_exp_part(cursor);
    cursor.tok(Kind::Number, start, line, col)
}

/// `0x`/`0o`/`0b` prefixed number literal.
fn scan_based_number(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    for _ in 0..2 {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
    }
    while cursor
        .peek()
        .is_some_and(|current| current.is_ascii_hexdigit() || current == '_')
    {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
    }
    cursor.tok(Kind::Number, start, line, col)
}

/// Decimal digit/underscore run.
fn scan_number_digits(cursor: &mut Cursor) {
    while cursor
        .peek()
        .is_some_and(|current| current.is_ascii_digit() || current == '_')
    {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
    }
}

/// Fractional `.digits` part of a float literal.
fn scan_frac_part(cursor: &mut Cursor) {
    if cursor.peek() == Some('.') && cursor.peek_at(1).is_some_and(|next| next.is_ascii_digit()) {
        if let Some(dot) = cursor.peek() {
            cursor.bump(dot);
        }
        scan_number_digits(cursor);
    }
}

/// `e`/`E` exponent part of a float literal.
fn scan_exp_part(cursor: &mut Cursor) {
    if matches!(cursor.peek(), Some('e' | 'E')) && exponent_ahead(cursor) {
        if let Some(mark) = cursor.peek() {
            cursor.bump(mark);
        }
        if matches!(cursor.peek(), Some('+' | '-'))
            && let Some(sign) = cursor.peek()
        {
            cursor.bump(sign);
        }
        scan_number_digits(cursor);
    }
}

/// Whether an `e`/`E` starts a float exponent at the cursor.
fn exponent_ahead(cursor: &Cursor) -> bool {
    match cursor.peek_at(1) {
        Some(next) if next.is_ascii_digit() => true,
        Some(sign) if sign == '+' || sign == '-' => {
            cursor.peek_at(2).is_some_and(|next| next.is_ascii_digit())
        }
        _ => false,
    }
}

/// Scan a lowercase word, keyword or operator word.
fn scan_word(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    while cursor
        .peek()
        .is_some_and(|current| current.is_alphanumeric() || matches!(current, '_' | '?' | '!'))
    {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
    }
    let text = cursor.text(start);
    if text == "and" || text == "or" {
        let kind = if slash_ahead(cursor.src, cursor.pos) {
            Kind::Ident
        } else {
            Kind::Op
        };
        return cursor.tok(kind, start, line, col);
    }
    if matches!(
        text.as_str(),
        "in" | "not" | "true" | "false" | "nil" | "do" | "end" | "fn" | "when"
    ) {
        return cursor.tok(Kind::Keyword, start, line, col);
    }
    if cursor.peek() == Some(':') && cursor.peek_at(1) != Some(':') {
        if let Some(mark) = cursor.peek() {
            cursor.bump(mark);
        }
        return cursor.tok(Kind::Keyword, start, line, col);
    }
    if cursor.peek() == Some('(')
        && text.starts_with(|current: char| current.is_lowercase() || current == '_')
    {
        return cursor.tok(Kind::ParenIdent, start, line, col);
    }
    if text.starts_with(|current: char| current.is_uppercase()) {
        cursor.tok(Kind::Alias, start, line, col)
    } else {
        cursor.tok(Kind::Ident, start, line, col)
    }
}

/// Whether the next same-line token starts with `/`, making an operator read
/// as a capture name (`&+/2`, and even `1 + / 2` upstream).
fn slash_ahead(src: &str, mut pos: usize) -> bool {
    let bytes = src.as_bytes();
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    bytes.get(pos) == Some(&b'/')
}

/// Scan an uppercase alias, merging a keyword colon (`Alias:`).
fn scan_alias(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    while cursor
        .peek()
        .is_some_and(|current| current.is_alphanumeric() || current == '_')
    {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
    }
    if cursor.peek() == Some(':') && cursor.peek_at(1) != Some(':') {
        if let Some(mark) = cursor.peek() {
            cursor.bump(mark);
        }
        return cursor.tok(Kind::Keyword, start, line, col);
    }
    cursor.tok(Kind::Alias, start, line, col)
}

/// Scan `:`, `::`, or a quoted/unquoted atom.
fn scan_colon(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    if cursor.peek_at(1) == Some(':') {
        for _ in 0..2 {
            if let Some(current) = cursor.peek() {
                cursor.bump(current);
            }
        }
        return cursor.tok(Kind::Type, start, line, col);
    }
    let quoted = matches!(cursor.peek_at(1), Some('"' | '\''));
    let wordy = cursor
        .peek_at(1)
        .is_some_and(|next| next.is_alphabetic() || next == '_');
    if !quoted && !wordy {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
        return cursor.tok(Kind::Colon, start, line, col);
    }
    if let Some(mark) = cursor.peek() {
        cursor.bump(mark);
    }
    if quoted {
        if let Some(quote) = cursor.peek() {
            scan_quoted(cursor, start, line, col, quote);
        }
        return cursor.tok(Kind::Atom, start, line, col);
    }
    while cursor.peek().is_some_and(is_name_char) {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
    }
    cursor.tok(Kind::Atom, start, line, col)
}

/// Scan `.`, `..`, `...`, or `.(`.
fn scan_dot(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    if cursor.peek_at(1) == Some('.') && cursor.peek_at(2) == Some('.') {
        for _ in 0..3 {
            if let Some(current) = cursor.peek() {
                cursor.bump(current);
            }
        }
        return cursor.tok(Kind::Ellipsis, start, line, col);
    }
    if cursor.peek_at(1) == Some('.') {
        for _ in 0..2 {
            if let Some(current) = cursor.peek() {
                cursor.bump(current);
            }
        }
        return cursor.tok(Kind::Range, start, line, col);
    }
    if let Some(dot) = cursor.peek() {
        cursor.bump(dot);
    }
    if cursor.peek() == Some('(') {
        cursor.tok(Kind::DotCall, start, line, col)
    } else {
        cursor.tok(Kind::Dot, start, line, col)
    }
}

/// Scan `%`, emitting the two tokens `%{` becomes upstream.
fn scan_percent(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Vec<Tok> {
    if let Some(mark) = cursor.peek() {
        cursor.bump(mark);
    }
    if cursor.peek() != Some('{') {
        return vec![cursor.tok(Kind::Percent, start, line, col)];
    }
    let first = cursor.tok(Kind::MapOpen, start, line, col);
    if let Some(brace) = cursor.peek() {
        cursor.bump(brace);
    }
    let second = cursor.tok(Kind::LBrace, start, line, col);
    vec![first, second]
}

/// Scan `&`: a capture, a `&1` capture integer, or `&&` operators.
fn scan_capture(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    if cursor.peek_at(1) == Some('&') {
        return scan_symbolic(cursor, start, line, col);
    }
    if let Some(mark) = cursor.peek() {
        cursor.bump(mark);
    }
    if cursor.peek().is_some_and(|next| next.is_ascii_digit()) {
        return cursor.tok(Kind::CaptureInt, start, line, col);
    }
    cursor.tok(Kind::Capture, start, line, col)
}

/// Scan one symbolic operator with longest-match priority.
fn scan_symbolic(cursor: &mut Cursor, start: usize, line: usize, col: usize) -> Tok {
    let rest = &cursor.src[cursor.pos..];
    let (kind, text) = symbolic_token(rest);
    for _ in 0..text.chars().count() {
        if let Some(current) = cursor.peek() {
            cursor.bump(current);
        }
    }
    if kind == Kind::Op && slash_ahead(cursor.src, cursor.pos) {
        return cursor.tok(Kind::Ident, start, line, col);
    }
    cursor.tok(kind, start, line, col)
}

/// Longest-match symbolic token for the operator table.
fn symbolic_token(rest: &str) -> (Kind, String) {
    let head3: String = rest.chars().take(3).collect();
    let head2: String = rest.chars().take(2).collect();
    let head1: String = rest.chars().take(1).collect();
    match head3.as_str() {
        "===" | "!==" | "&&&" | "|||" => (Kind::Op, head3),
        "<<<" | ">>>" | "<~>" | "<|>" | "~>>" => (Kind::Arrow, head3),
        _ => match head2.as_str() {
            // `**` is a power-op token, which native `operator?/1` rejects:
            // never an operator occurrence.
            "**" => (Kind::Other, head2),
            "=>" => (Kind::Assoc, head2),
            "|>" | "~>" | "<~" | "|~>" => (Kind::Arrow, head2),
            "::" => (Kind::Type, head2),
            "<<" => (Kind::BinOpen, head2),
            ">>" => (Kind::BinClose, head2),
            "//" | "==" | "!=" | "=~" | "<=" | ">=" | "++" | "--" | "<>" | "&&" | "||" | "->"
            | "<-" | "\\\\" => (Kind::Op, head2),
            _ => match head1.as_str() {
                "+" | "-" | "*" | "/" | "<" | ">" | "=" | "|" => (Kind::Op, head1),
                "^" => (Kind::Caret, head1),
                _ => (Kind::Other, head1),
            },
        },
    }
}

/// Character continuing an Elixir name.
fn is_name_char(current: char) -> bool {
    current.is_ascii_alphanumeric() || matches!(current, '_' | '?' | '!')
}

/// Advance past specs, types, captures and binary patterns like upstream.
fn skip_window(toks: &[Tok], pos: usize) -> usize {
    let Some(first) = toks.get(pos) else {
        return pos;
    };
    if first.kind == Kind::At {
        let spec_or_type = toks.get(pos + 1).is_some_and(|next| {
            next.kind == Kind::Ident && (next.text == "spec" || next.text == "type")
        });
        if spec_or_type {
            let mut end = pos;
            while end < toks.len() && toks[end].line == first.line {
                end += 1;
            }
            return end;
        }
        return pos;
    }
    if first.kind == Kind::Capture {
        return drop_fun_capture(toks, pos + 1);
    }
    if first.kind == Kind::Ident && first.text == "&" {
        let slash = toks
            .get(pos + 1)
            .is_some_and(|next| next.kind == Kind::Ident && next.text == "/");
        if slash {
            return drop_fun_capture(toks, pos + 2);
        }
        return pos;
    }
    if first.kind == Kind::BinOpen {
        return toks.len();
    }
    pos
}

/// Drop capture contents (`&Module.fun/2`), stopping at arity integers.
fn drop_fun_capture(toks: &[Tok], mut pos: usize) -> usize {
    while pos < toks.len() && droppable(&toks[pos]) {
        pos += 1;
    }
    pos
}

/// Tokens skipped inside a function capture.
fn droppable(token: &Tok) -> bool {
    match token.kind {
        Kind::Atom
        | Kind::Alias
        | Kind::Ident
        | Kind::At
        | Kind::Dot
        | Kind::LParen
        | Kind::RParen => true,
        Kind::ParenIdent => token.text == "unquote",
        Kind::Op => token.text == "/",
        _ => false,
    }
}

/// Space between two tokens on the same line.
fn spaced(left: &Tok, right: &Tok) -> bool {
    left.line == right.line && left.end < right.col
}

/// Two tokens on the same line with nothing between them.
fn adjacent(left: &Tok, right: &Tok) -> bool {
    left.line == right.line && left.end == right.col
}

/// Usually no space before the operator (`-` after anything but a name).
fn usually_before(prev: &Tok, current: &Tok) -> bool {
    if current.text == "-" {
        return !matches!(prev.kind, Kind::Ident | Kind::Number);
    }
    if current.text == "//" {
        return true;
    }
    false
}

/// Usually no space after the operator (negative numbers, steps, `//`).
fn usually_after(prev: &Tok, current: &Tok, next: &Tok) -> bool {
    if current.text != "-" {
        return current.text == "//";
    }
    if matches!(prev.kind, Kind::LParen | Kind::LBrace)
        && matches!(next.kind, Kind::Ident | Kind::Number)
    {
        return true;
    }
    matches!(prev.text.as_str(), "^" | "=" | "..")
}

/// Whether the previous identifier reads as a call name for `+`/`-` filtering.
/// Upstream correlates the token to the AST; a call name is one followed by
/// an argument or an opening paren (`assert -24`, `ExUnit.assert -12`), while
/// a plain variable is followed by its operator (`b+c`).
fn followed_by_call_char(lines: &[&str], prev: &Tok) -> bool {
    if prev.kind != Kind::Ident {
        return false;
    }
    let Some(line) = lines.get(prev.line.wrapping_sub(1)) else {
        return false;
    };
    matches!(
        line.chars().nth(prev.end.wrapping_sub(1)),
        Some(' ' | '\t' | '(')
    )
}

/// Issue-stage filters shared with the check module.
fn filtered(line: &str, vote: &OpVote) -> bool {
    let (prefix, suffix) = split_line(line, vote.col);
    match vote.trigger.as_str() {
        "+" | "-" => {
            number_with_sign(&prefix)
                || number_in_range(&suffix)
                || (vote.trigger == "-" && minus_in_binary_size(&prefix, &suffix))
                || vote.callish_prev
        }
        "->" => arrow_in_typespec(&prefix),
        "/" => number_in_function_capture(&prefix),
        "*" => typespec_binary_unit(line),
        _ => false,
    }
}

/// Text before the trigger and from just past it, by character.
fn split_line(line: &str, col: usize) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    let cut = col.saturating_sub(1).min(chars.len());
    let after = col.min(chars.len());
    (
        chars[..cut].iter().collect(),
        chars[after..].iter().collect(),
    )
}

/// Signed numbers (`@min -1`, `|| -1`, leading spaces).
fn number_with_sign(prefix: &str) -> bool {
    let Some(expression) =
        regex::Regex::new(r"(\A\s+|@[a-zA-Z0-9_]+\.?|[|\\{\[(,:><=+\-*/])\s*$").ok()
    else {
        return false;
    };
    expression.is_match(prefix)
}

/// Upper range bound (`-999..-1` keeps its sign glued).
fn number_in_range(suffix: &str) -> bool {
    let Some(expression) = regex::Regex::new(r"^\d+\.\.").ok() else {
        return false;
    };
    expression.is_match(suffix)
}

/// Capture arity (`&json_library().encode!/1` keeps its slash glued).
fn number_in_function_capture(prefix: &str) -> bool {
    let Some(expression) = regex::Regex::new(r"[.&][a-z0-9_]+[!]?$").ok() else {
        return false;
    };
    expression.is_match(prefix)
}

/// Minus inside a binary size (`<<size(valsize)-binary>>` keeps it glued).
fn minus_in_binary_size(prefix: &str, suffix: &str) -> bool {
    let typed_after =
        regex::Regex::new(r"^\s*(integer|native|signed|unsigned|binary|size|little|float)").ok();
    let typed_before =
        regex::Regex::new(r"(integer|native|signed|unsigned|binary|size|little|float)\s*$").ok();
    let mut heuristics = 0_usize;
    heuristics += usize::from(prefix.contains("<<"));
    heuristics += usize::from(suffix.contains(">>"));
    heuristics += usize::from(prefix.contains("::"));
    heuristics += usize::from(typed_after.is_some_and(|it| it.is_match(suffix)));
    heuristics += usize::from(typed_before.is_some_and(|it| it.is_match(prefix)));
    heuristics >= 2
}

/// Arrow closing a typespec paren (`(-> Config.t())` keeps it glued).
fn arrow_in_typespec(prefix: &str) -> bool {
    let Some(expression) = regex::Regex::new(r"\(\s*$").ok() else {
        return false;
    };
    expression.is_match(prefix)
}

/// Asterisk inside a binary typespec (`_::_*8` keeps it glued).
fn typespec_binary_unit(line: &str) -> bool {
    let Some(expression) = regex::Regex::new(r"_::_*").ok() else {
        return false;
    };
    expression.is_match(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../../compatibility/cases/EX1005.json");

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

    #[test]
    fn corpus_projects_match_upstream_findings() {
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
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// One subgroup project run; failure descriptions for mismatches.
    fn check_subgroup(parsed: &[Entry], indexes: &[usize]) -> Vec<String> {
        let mut failures = Vec::new();
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
        for (position, index) in indexes.iter().enumerate() {
            let mut actual: Vec<String> = issues
                .iter()
                .filter(|issue| issue.file == position)
                .map(|issue| finding_key(issue.line, issue.column, &issue.trigger, &issue.message))
                .collect();
            let mut expected: Vec<String> = parsed[*index]
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
                    parsed[*index].id
                ));
            }
        }
        failures
    }

    #[test]
    fn oracle_pinned_group_votes() {
        // Guards the refute entries against vacuous passes from under-counting.
        // Each pair is the oracle `(with_space, without_space)` vote total.
        let cases: [(&str, (usize, usize)); 7] = [
            ("EX1005.upstream.mixed-1.file1", (2, 3)),
            ("EX1005.upstream.mixed-1.file2", (2, 1)),
            ("EX1005.upstream.result-5", (1, 1)),
            ("EX1005.upstream.binary-typespec", (0, 0)),
            ("EX1005.upstream.spaces-everywhere-step.file5", (2, 1)),
            ("EX1005.upstream.spaces-two-files.file2", (6, 3)),
            ("EX1005.upstream.no-spaces-everywhere.file2", (0, 2)),
        ];
        for (id, (with_space, without_space)) in cases {
            let (mut with_count, mut without_count) = (0_usize, 0_usize);
            for vote in collect(&source_of(id)) {
                with_count += usize::from(vote.with_v);
                without_count += usize::from(vote.without_v);
            }
            assert_eq!(
                (with_count, without_count),
                (with_space, without_space),
                "votes for {id}"
            );
        }
    }

    #[test]
    fn multibyte_text_hides_operators_without_hanging() {
        let source = "λ = \"μ+μ\"\ny = 1 + 2\n";
        let votes = collect(source);
        assert_eq!(votes.len(), 3);
        assert!(votes.iter().all(|vote| vote.with_v && !vote.without_v));
    }

    #[test]
    fn operator_atoms_cast_no_votes() {
        // Native lexes `:*`, `:==`, `:->` as `{:atom, …}` — never operators.
        // (Other operators in the source may still vote; only the atom
        // forms must not.)
        for (source, atom_ops) in [
            ("match :*, \"/notes\"\n", vec!["*"]),
            (
                "@conditions [:==, :!=, :===, :!==, :in]\n",
                vec!["==", "!=", "===", "!=="],
            ),
            ("{:->, _, _} = child\n", vec!["->"]),
        ] {
            let votes = collect(source);
            let triggers: Vec<&str> = votes.iter().map(|vote| vote.trigger.as_str()).collect();
            for atom_op in &atom_ops {
                assert!(
                    !triggers.contains(atom_op),
                    "atom vote for {source:?}: {triggers:?}"
                );
            }
        }
    }

    #[test]
    #[test]
    fn power_operator_casts_no_votes() {
        // `**` lexes as one power-op token, which native `operator?/1`
        // rejects: never an operator occurrence.
        let votes = collect("x = 2 ** (n - 1)\n");
        assert!(
            votes.iter().all(|vote| vote.trigger != "*"),
            "power votes: {:?}",
            votes.iter().map(|vote| &vote.trigger).collect::<Vec<_>>()
        );
    }

    fn remote_operator_calls_cast_no_votes() {
        // Native lexes `Kernel.||` as `{:paren_identifier, …}` — never an
        // operator.
        for (source, call_op) in [
            ("query = uri.query |> Kernel.||(\"\")\n", "||"),
            ("x = Kernel.<>(a, b)\n", "<>"),
        ] {
            let votes = collect(source);
            let triggers: Vec<&str> = votes.iter().map(|vote| vote.trigger.as_str()).collect();
            assert!(
                !triggers.contains(&call_op),
                "call-name vote for {source:?}: {triggers:?}"
            );
        }
    }

    #[test]
    fn tie_breaks_toward_with_space_and_blames_without_space_file() {
        // Equal votes (2 with_space vs 2 without_space): the tie resolves
        // to the smallest key ("with_space"), so the tight-operators file
        // is blamed.
        let files = vec![
            ProjectFile {
                filename: "a.ex".to_owned(),
                source: "x = 1 + 2\n".to_owned(),
            },
            ProjectFile {
                filename: "b.ex".to_owned(),
                source: "x=1+2\n".to_owned(),
            },
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().all(|issue| issue.file == 1));
    }
}
