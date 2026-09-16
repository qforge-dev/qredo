use crate::{Finding, helpers};
use std::collections::BTreeMap;

const KEYWORDS: &[&str] = &[
    "def",
    "defp",
    "defmodule",
    "do",
    "end",
    "if",
    "unless",
    "case",
    "cond",
    "with",
    "for",
    "fn",
    "in",
    "not",
    "and",
    "or",
    "true",
    "false",
    "nil",
    "when",
    "else",
    "try",
    "rescue",
    "catch",
    "after",
    "quote",
    "unquote",
    "super",
    "receive",
];

/// `EX4028`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut scan = Scan {
        stack: Vec::new(),
        findings: Vec::new(),
        allow_bang: helpers::param_bool(params, "allow_bang", false),
        pending_case: false,
        in_head: false,
    };
    for (idx, line) in lines.iter().enumerate() {
        scan.process_line(line, idx + 1, &lines);
    }
    while let Some(scope) = scan.stack.pop() {
        scope.finish(scan.allow_bang, &lines, &mut scan.findings);
    }
    scan.findings
        .sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    scan.findings
}

struct Scope {
    checked: bool,
    vars: Vec<(String, usize)>,
}

struct Scan {
    stack: Vec<Scope>,
    findings: Vec<Finding>,
    allow_bang: bool,
    /// A `case`/`cond`/`receive`/`try` head is waiting for its `do`: that
    /// `do` opens an unchecked scope (clause bodies never match upstream).
    pending_case: bool,
    /// Inside a multi-line `def`-family head: head matches never count.
    in_head: bool,
}

impl Scan {
    fn process_line(&mut self, line: &str, line_no: usize, lines: &[&str]) {
        let scan = scan_line(line);
        let entering = is_def_start(line.trim_start()) && !head_ending(line, &scan);
        let clearing = self.in_head && !entering && head_ending(line, &scan);
        if entering {
            self.in_head = true;
        } else if clearing {
            self.in_head = false;
        }
        let record = select_record(line, &scan, entering, self.in_head, clearing);
        // `->`-line prefixes belong to the enclosing scope: record them
        // before the line's own openers are pushed.
        let early = !entering && !self.in_head && !clearing && line.contains("->");
        if early {
            self.record_text(record, line_no);
        }
        for event in scan.events {
            match event {
                Event::PushFn => self.stack.push(Scope {
                    checked: false,
                    vars: Vec::new(),
                }),
                Event::PushDo => {
                    let checked = !self.pending_case;
                    self.pending_case = false;
                    self.stack.push(Scope {
                        checked,
                        vars: Vec::new(),
                    });
                }
                Event::MarkCase => self.pending_case = true,
            }
        }
        // `else`/`rescue`/`catch`/`after` extend the keyword list, so the
        // whole block is unchecked upstream: drop what was collected.
        if scan.else_like {
            self.spoil_innermost();
        }
        if !early {
            self.record_text(record, line_no);
        }
        for _ in 0..scan.closes {
            if let Some(scope) = self.stack.pop() {
                scope.finish(self.allow_bang, lines, &mut self.findings);
            }
        }
    }

    /// Record the selected text when the generator-position filter allows.
    fn record_text(&mut self, record: Option<&str>, line_no: usize) {
        if let Some(text) = record
            && record_allowed(text)
        {
            self.record_checked(text, line_no);
        }
    }

    /// The innermost open block loses its collected bindings: its keyword
    /// list grew beyond a single `do`.
    fn spoil_innermost(&mut self) {
        if let Some(top) = self.stack.last_mut() {
            top.checked = false;
            top.vars.clear();
        }
    }

    fn record_checked(&mut self, text: &str, line_no: usize) {
        let Some(top) = self.stack.last_mut() else {
            return;
        };
        if !top.checked {
            return;
        }
        for stmt in text.split(';') {
            record_statement(stmt, line_no, top);
        }
    }
}

impl Scope {
    fn finish(self, allow_bang: bool, lines: &[&str], findings: &mut Vec<Finding>) {
        let mut done: Vec<&str> = Vec::new();
        for (name, line_no) in self.vars.iter().rev() {
            if done.contains(&name.as_str()) {
                continue;
            }
            done.push(name);
            if allow_bang && name.ends_with('!') {
                continue;
            }
            if self.vars.iter().filter(|(other, _)| other == name).count() >= 2 {
                let line = lines.get(line_no - 1).copied().unwrap_or("");
                findings.push(Finding::with_trigger(
                    *line_no,
                    trigger_column(line, name),
                    format!("Variable \"{name}\" was declared more than once."),
                    name.clone(),
                ));
            }
        }
    }
}

/// Block-structure events of one line, in order: `fn`/`do` openers and
/// `case`-family heads arming the next `do` as unchecked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Event {
    PushFn,
    PushDo,
    MarkCase,
}

/// Clause heads whose `do` opens an unchecked scope.
const CASE_WORDS: [&str; 4] = ["case", "cond", "receive", "try"];

/// Keywords extending a block past a single `do` (unchecked upstream).
const ELSE_WORDS: [&str; 4] = ["else", "rescue", "catch", "after"];

/// `def`-family heads that can bind across lines.
const DEF_WORDS: [&str; 6] = [
    "def",
    "defp",
    "defmacro",
    "defmacrop",
    "defguard",
    "defguardp",
];

/// Block structure of one line, computed in a single left-to-right pass:
/// opener events in order, `end` closers, whether an `else`-family keyword
/// extends the block, and text after the first block `do` for same-line
/// recording.
struct LineScan<'line> {
    events: Vec<Event>,
    closes: usize,
    after_do: Option<&'line str>,
    else_like: bool,
}

impl LineScan<'_> {
    /// Whether the line opens any checked or unchecked scope.
    fn has_push(&self) -> bool {
        self.events
            .iter()
            .any(|event| matches!(event, Event::PushFn | Event::PushDo))
    }

    /// Record a `case`-family head or `else`-family keyword at this position.
    fn visit_keyword(
        &mut self,
        word_at: &dyn Fn(&[u8]) -> bool,
        colon_free: &dyn Fn(usize) -> bool,
        after_ok: &dyn Fn(usize) -> bool,
    ) {
        if let Some(len) = match_len(word_at, &CASE_WORDS)
            && colon_free(len)
            && after_ok(len)
        {
            self.events.push(Event::MarkCase);
        } else if let Some(len) = match_len(word_at, &ELSE_WORDS)
            && colon_free(len)
            && after_ok(len)
        {
            self.else_like = true;
        }
    }
}

/// Single scan replacing `openers`/`count_ends`/`after_block_do` (which each
/// rebuilt char and byte vectors per line with backward scans per position).
/// Word boundaries mirror `is_word_at`: before must avoid name chars plus
/// `./:/@`, after must avoid name chars; `do`/`end` reject a `:` suffix.
fn scan_line(line: &str) -> LineScan<'_> {
    let chars: Vec<char> = line.chars().collect();
    let mut scan = LineScan {
        events: Vec::new(),
        closes: 0,
        after_do: None,
        else_like: false,
    };
    let mut byte = 0_usize;
    let mut prev: Option<char> = None;
    let mut i = 0_usize;
    while i < chars.len() {
        let word_at = |word: &[u8]| {
            chars.len() >= i + word.len()
                && chars[i..i + word.len()]
                    .iter()
                    .zip(word.iter())
                    .all(|(got, want)| *got == *want as char)
        };
        let before_ok = prev.is_none_or(|c| !is_name_char(c) && c != '.' && c != ':' && c != '@');
        let after_ok = |len: usize| chars.get(i + len).is_none_or(|c| !is_name_char(*c));
        let colon_free = |len: usize| chars.get(i + len) != Some(&':');
        if before_ok && word_at(b"fn") && colon_free(2) && after_ok(2) {
            scan.events.push(Event::PushFn);
        } else if before_ok && word_at(b"do") && value_start(prev) && colon_free(2) && after_ok(2) {
            scan.events.push(Event::PushDo);
            if scan.after_do.is_none() {
                let after = if i + 2 <= chars.len() {
                    byte + chars[i..i + 2].iter().map(|c| c.len_utf8()).sum::<usize>()
                } else {
                    line.len()
                };
                scan.after_do = Some(line[after..].trim_start());
            }
        } else if before_ok && word_at(b"end") && colon_free(3) && after_ok(3) {
            scan.closes += 1;
        } else if before_ok {
            scan.visit_keyword(&word_at, &colon_free, &after_ok);
        }
        byte += chars[i].len_utf8();
        prev = Some(chars[i]);
        i += 1;
    }
    scan
}

/// Length of the first of `words` starting at this position, if any.
fn match_len(word_at: &dyn Fn(&[u8]) -> bool, words: &[&str]) -> Option<usize> {
    words
        .iter()
        .find(|word| word_at(word.as_bytes()))
        .map(|word| word.len())
}

/// True when a keyword at this position is real code (not atom/field/capture).
fn value_start(prev: Option<char>) -> bool {
    prev.is_none_or(|c| c != ':' && c != '.' && c != '@' && c != '&')
}

/// Recordable text of one line: head lines contribute nothing, a clearing
/// line contributes past its `do`, `->` lines contribute an outer-assignment
/// prefix, opener lines past their `do`, and plain lines wholly.
fn select_record<'line>(
    line: &'line str,
    scan: &LineScan<'line>,
    entering: bool,
    in_head: bool,
    clearing: bool,
) -> Option<&'line str> {
    if entering || (in_head && !clearing) {
        None
    } else if clearing {
        scan.after_do.or_else(|| after_first_do_colon(line))
    } else if line.contains("->") {
        arrow_prefix_text(line)
    } else if scan.has_push() {
        scan.after_do
    } else {
        Some(line)
    }
}

/// Whether the trimmed line starts a `def`-family head.
fn is_def_start(trimmed: &str) -> bool {
    DEF_WORDS.iter().any(|word| {
        trimmed.starts_with(word)
            && trimmed[word.len()..]
                .starts_with(|c: char| !c.is_alphanumeric() && c != '_' && c != '?' && c != '!')
    })
}

/// Whether the line ends a `def` head: it opens a block `do` or carries a
/// `do:` value.
fn head_ending(line: &str, scan: &LineScan<'_>) -> bool {
    scan.events
        .iter()
        .any(|event| matches!(event, Event::PushDo))
        || find_code_word(line, "do", true, true).is_some()
}

/// Text after the first `do:` value on the line, if any.
fn after_first_do_colon(line: &str) -> Option<&str> {
    find_code_word(line, "do", true, true).map(|pos| line[pos + "do:".len()..].trim_start())
}

/// Recordable prefix of a `->` line: text before the first arrow, and only
/// when its first top-level `=` precedes any `fn`/`do` (a genuine outer
/// assignment, not a clause or `fn` head match).
fn arrow_prefix_text(line: &str) -> Option<&str> {
    let arrow = line.find("->")?;
    let prefix = &line[..arrow];
    let eq = find_assignment(prefix)?;
    let anchor = first_fn_or_do(prefix).unwrap_or(usize::MAX);
    (eq < anchor).then_some(prefix)
}

/// Whether a recordable text is a genuine assignment: reject generator
/// territory where the first top-level `=` follows a top-level `<-`
/// without an intervening block `do` (the `=` lives in generator or
/// `do:`-value position, unchecked upstream).
fn record_allowed(text: &str) -> bool {
    let Some(eq) = find_assignment(text) else {
        return true;
    };
    let Some(lt) = first_lt_arrow(text) else {
        return true;
    };
    if lt > eq {
        return true;
    }
    first_bare_do(&text[lt..eq]).is_some()
}

/// Byte offset of the first depth-zero `<-`, if any.
fn first_lt_arrow(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0_usize;
    let mut idx = 0_usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'<' if depth == 0 && bytes.get(idx + 1) == Some(&b'-') => return Some(idx),
            _ => {}
        }
        idx += 1;
    }
    None
}

/// Byte offset of the first bare `fn`/`do` word in code position, if any.
fn first_fn_or_do(text: &str) -> Option<usize> {
    find_code_word(text, "fn", false, false)
        .into_iter()
        .chain(find_code_word(text, "do", true, false))
        .min()
}

/// Byte offset of the first bare `do` word in code position, if any.
fn first_bare_do(text: &str) -> Option<usize> {
    find_code_word(text, "do", true, false)
}

/// Byte offset of the first `word` occurrence in code position: before
/// avoids name chars plus `./:@` (plus `&` for statement keywords), after
/// avoids name chars and requires (`need_colon`) or forbids a `:` suffix.
fn find_code_word(text: &str, word: &str, stmt: bool, need_colon: bool) -> Option<usize> {
    let bytes = text.as_bytes();
    let target = word.as_bytes();
    let mut idx = 0_usize;
    while idx + target.len() <= bytes.len() {
        if &bytes[idx..idx + target.len()] == target {
            let before_ok = bytes[..idx].last().is_none_or(|prev| {
                !(prev.is_ascii_alphanumeric()
                    || matches!(prev, b'_' | b'?' | b'!' | b'.' | b':' | b'@')
                    || stmt && matches!(prev, b':' | b'.' | b'@' | b'&'))
            });
            let after_ok = match bytes.get(idx + target.len()).copied() {
                None => !need_colon,
                Some(next) => {
                    !(next.is_ascii_alphanumeric() || matches!(next, b'_' | b'?' | b'!'))
                        && (next == b':') == need_colon
                }
            };
            if before_ok && after_ok {
                return Some(idx);
            }
        }
        idx += 1;
    }
    None
}

fn record_statement(stmt: &str, line_no: usize, scope: &mut Scope) {
    let Some(eq) = find_assignment(stmt) else {
        return;
    };
    let lhs = strip_bitstring_specs(&stmt[..eq]);
    for name in pattern_vars(&lhs) {
        if !scope
            .vars
            .iter()
            .any(|(other, at)| other == &name && *at == line_no)
        {
            scope.vars.push((name, line_no));
        }
    }
}

/// Byte offset of the first top-level `=` that is not `==`, `!=`,
/// `<=`, `>=`, `=>` or `=~`.
fn find_assignment(stmt: &str) -> Option<usize> {
    let chars: Vec<char> = stmt.chars().collect();
    let bytes: Vec<usize> = stmt.char_indices().map(|(byte, _)| byte).collect();
    let mut depth = 0_usize;
    let mut i = 0_usize;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '=' if depth == 0 => {
                let prev = if i > 0 {
                    chars.get(i - 1).copied()
                } else {
                    None
                };
                let next = chars.get(i + 1).copied();
                let prev_ok = i == 0 || !matches!(prev, Some('=' | '!' | '<' | '>'));
                let next_ok = !matches!(next, Some('=' | '>' | '~'));
                if prev_ok && next_ok {
                    return Some(bytes[i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Bound variables on an assignment left-hand side: lowercase identifiers
/// that are not pinned, attributes, atoms, field accesses, map keys or
/// keywords.
fn pattern_vars(lhs: &str) -> Vec<String> {
    let chars: Vec<char> = lhs.chars().collect();
    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < chars.len() {
        let c = chars[i];
        if (c.is_ascii_lowercase() || c == '_') && (i == 0 || !is_name_char(chars[i - 1])) {
            let mut end = i;
            while end < chars.len() && is_name_char(chars[end]) {
                end += 1;
            }
            let name: String = chars[i..end].iter().collect();
            if keep_var(&chars, i, end, &name) && !out.contains(&name) {
                out.push(name);
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn keep_var(chars: &[char], start: usize, end: usize, name: &str) -> bool {
    if name.starts_with('_') || KEYWORDS.contains(&name) {
        return false;
    }
    if immediate_prev(chars, start).is_some_and(|prev| prev == '.' || prev == ':' || prev == '@') {
        return false;
    }
    if pinned(chars, start) {
        return false;
    }
    // Map/keyword keys (`key:`) never bind; `::` is a bitstring annotation.
    if chars.get(end) == Some(&':') && chars.get(end + 1) != Some(&':') {
        return false;
    }
    true
}

fn immediate_prev(chars: &[char], start: usize) -> Option<char> {
    if start > 0 {
        Some(chars[start - 1])
    } else {
        None
    }
}

fn pinned(chars: &[char], start: usize) -> bool {
    let mut i = start;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i > 0 && chars[i - 1] == '^'
}

/// Drop `::spec` tails inside `<<...>>` segments; size parameters reuse
/// bindings instead of introducing them.
fn strip_bitstring_specs(lhs: &str) -> String {
    let chars: Vec<char> = lhs.chars().collect();
    let mut out = String::new();
    let mut i = 0_usize;
    let mut depth = 0_usize;
    while i < chars.len() {
        if chars[i] == '<' && chars.get(i + 1) == Some(&'<') {
            depth += 1;
            out.push_str("<<");
            i += 2;
        } else if chars[i] == '>' && chars.get(i + 1) == Some(&'>') && depth > 0 {
            depth -= 1;
            out.push_str(">>");
            i += 2;
        } else if chars[i] == ':' && chars.get(i + 1) == Some(&':') && depth > 0 {
            i += 2;
            let mut parens = 0_usize;
            while i < chars.len() {
                match chars[i] {
                    '(' => parens += 1,
                    ')' => parens = parens.saturating_sub(1),
                    ',' | '>' if parens == 0 => break,
                    _ => {}
                }
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '?' || c == '!'
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
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
    #[test]
    fn single_binding_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def f do\n x = 1\n y = 2\nend\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_rebinding() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("def f do\n x = 1\n x = 2\nend\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn reports_destructured_rebinding() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(
                "defmodule CredoSampleModule do\n  def some_function() do\n    something = \"ABABAB\"\n    {:ok, something} = Base.decode16(something)\n    {a, a} = {2, 2} # this should _not_ trigger it\n  end\nend\n",
            ),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 4);
        assert_eq!(findings[0].column, Some(11));
    }
    #[test]
    fn reports_bang_rebinding_by_default() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(
                "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    a! = 1\n    a! = 2\n  end\nend\n",
            ),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(5));
    }
    #[test]
    fn allows_bang_rebinding_when_opted_in() {
        let mut params = BTreeMap::new();
        params.insert("allow_bang".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy("defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    a! = 1\n    a! = 2\n  end\nend\n"),&params)
        .is_empty());
    }
    #[test]
    fn ignores_end_keyword_key() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("def f do\n x = [end: 1]\n x = 2\nend\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
    }
    #[test]
    fn ignores_fn_keyword_key() {
        let findings = check_prepared(
            &crate::batch::Prepared::lazy("def f do\n x = [fn: 1]\n x = 2\nend\n"),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
    }
    #[test]
    fn records_assignment_before_fn_arrow() {
        // RB-MISS: the `->` line still holds a top-level `=` before `fn`.
        let findings = check_prepared(
            &crate::batch::Prepared::lazy(
                "def f(x, l) do\n y = x + 1\n y = Enum.map(l, fn z -> z end)\nend\n",
            ),
            &BTreeMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
    }
    #[test]
    fn case_branches_do_not_merge() {
        // RB-FP-branch: case clause bodies are unchecked upstream.
        let src = "def f(conn, x) do\n case x do\n {:ok, state} ->\n conn = state\n conn\n {:error, _} ->\n conn = :err\n conn\n end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn with_else_is_unchecked() {
        // RB-FP-withelse: `[do:, else:]` never matches upstream.
        let src = "def f(x) do\n with {:ok, y} <- x do\n view = y\n view\n else\n view = nil\n view\n end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn for_generator_vars_do_not_count() {
        // RB-FP-for: the `=` lives in the `do:` value after `<-`.
        let src = "def f(topics) do\n for topic <- topics, do: :ok = publish(topic)\n for topic <- topics, do: :ok = publish(topic)\n :done\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn rescue_spoils_the_block() {
        // RB-FP-rescue: multi-key blocks never match upstream.
        let src = "def f(x) do\n headers = x\n headers = call(headers)\nrescue\n _ -> :err\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn multiline_fn_heads_do_not_leak() {
        // RB-FP-head: head matches never count upstream.
        let src = "defmodule M do\n def f(\n {:ok, x} = y\n ) do\n :ok\n end\n def g(\n {:ok, x} = z\n ) do\n :ok\n end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn single_line_fn_head_match_does_not_count() {
        // Head `=` plus one body `=` is a single upstream occurrence.
        let src = "def f({:ok, x} = y) do\n {:ok, x} = z\n :done\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
}
