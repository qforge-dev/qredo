use crate::Finding;
use std::collections::BTreeMap;

/// `EX3026`: module parts appear in the configured order.
///
/// Tracks first-level `@`/call/`def` parts per `defmodule` (custom macros,
/// literals, and block bodies are not parts) and reports parts ordered
/// before their predecessor, mirroring `Credo.Code.Module.analyze/1`.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let rules = Rules {
        order: read_order(params),
        ignored: read_atoms(params, "ignore"),
        ignored_attrs: read_atoms(params, "ignore_module_attributes"),
    };
    let masked = prepared.masked();
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let mut scan = Scan::default();
    for (idx, masked_line) in masked.split('\n').enumerate() {
        scan.line(masked_line.trim(), masked_line, idx + 1);
    }
    let mut findings = Vec::new();
    for ctx in scan.stack.iter().chain(scan.done.iter()) {
        ctx.report(&rules, &raw_lines, &mut findings);
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

struct Rules {
    order: Vec<String>,
    ignored: Vec<String>,
    ignored_attrs: Vec<String>,
}

#[derive(Default)]
struct Scan {
    stack: Vec<ModuleCtx>,
    done: Vec<ModuleCtx>,
    depth: i32,
    balance: i32,
    prev_comma: bool,
}

impl Scan {
    fn line(&mut self, trimmed: &str, masked_line: &str, line_no: usize) {
        if starts_word(trimmed, "defmodule") {
            self.open_module(trimmed, masked_line, line_no);
            return;
        }
        if !self.continuation()
            && let Some(top) = self.stack.last_mut()
            && top.body_depth == self.depth
        {
            top.observe(trimmed, line_no);
        }
        self.depth += line_depth_delta(masked_line);
        self.balance += bracket_delta(masked_line);
        self.prev_comma = trimmed.ends_with(',');
        while self
            .stack
            .last()
            .is_some_and(|top| top.body_depth > self.depth)
        {
            if let Some(top) = self.stack.pop() {
                self.done.push(top);
            }
        }
    }

    fn continuation(&self) -> bool {
        self.balance > 0 || self.prev_comma
    }

    fn open_module(&mut self, trimmed: &str, masked_line: &str, line_no: usize) {
        let name = module_name(trimmed);
        let full = match self.stack.last() {
            Some(top) => format!("{}.{name}", top.name),
            None => name,
        };
        if let Some(top) = self.stack.last_mut()
            && top.body_depth == self.depth
        {
            top.push("module".to_owned(), line_no);
        }
        self.depth += line_depth_delta(masked_line);
        self.balance += bracket_delta(masked_line);
        self.stack.push(ModuleCtx::new(full, self.depth));
        self.prev_comma = false;
    }
}

fn read_order(params: &BTreeMap<String, String>) -> Vec<String> {
    let fallback = || {
        [
            "shortdoc",
            "moduledoc",
            "behaviour",
            "use",
            "import",
            "alias",
            "require",
        ]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect()
    };
    let Some(raw) = params.get("order") else {
        return fallback();
    };
    let parsed: Vec<String> = serde_json::from_str(raw).unwrap_or_default();
    if parsed.is_empty() {
        return fallback();
    }
    parsed
        .into_iter()
        .map(|name| {
            let plain = name.strip_prefix(':').unwrap_or(&name).to_owned();
            if plain == "callback_fun" {
                "callback_impl".to_owned()
            } else {
                plain
            }
        })
        .collect()
}

fn read_atoms(params: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    let Some(raw) = params.get(key) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .map(|name| name.strip_prefix(':').unwrap_or(&name).to_owned())
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Modifier {
    Private,
    Impl,
}

struct Part {
    kind: String,
    line_no: usize,
}

struct ModuleCtx {
    name: String,
    body_depth: i32,
    parts: Vec<Part>,
    pending: Option<Modifier>,
    last_fun: Option<(String, usize)>,
}

impl ModuleCtx {
    fn new(name: String, body_depth: i32) -> Self {
        Self {
            name,
            body_depth,
            parts: Vec::new(),
            pending: None,
            last_fun: None,
        }
    }

    fn observe(&mut self, trimmed: &str, line_no: usize) {
        if trimmed.starts_with('@') {
            self.observe_attribute(trimmed, line_no);
        } else if let Some(kind) = call_part(trimmed) {
            self.push(kind, line_no);
        } else if starts_word(trimmed, "def")
            || starts_word(trimmed, "defp")
            || starts_word(trimmed, "defmacro")
            || starts_word(trimmed, "defmacrop")
            || starts_word(trimmed, "defguard")
            || starts_word(trimmed, "defguardp")
        {
            self.observe_def(trimmed, line_no);
        }
    }

    fn observe_attribute(&mut self, trimmed: &str, line_no: usize) {
        let ident: String = trimmed[1..]
            .chars()
            .take_while(|next| next.is_ascii_alphanumeric() || *next == '_')
            .collect();
        match ident.as_str() {
            "doc" => self.pending = doc_modifier(&trimmed[1 + ident.len()..], self.pending),
            "impl" => self.pending = impl_modifier(&trimmed[1 + ident.len()..], self.pending),
            "shortdoc" | "moduledoc" | "behaviour" | "type" | "typep" | "opaque" | "callback"
            | "macrocallback" | "optional_callbacks" => self.push(ident, line_no),
            "after_compile" | "before_compile" | "compile" | "deprecated" | "dialyzer"
            | "external_resource" | "file" | "on_definition" | "on_load" | "vsn" | "spec"
            | "enforce_keys" | "typedoc" => {}
            _ => self.push(format!("module_attribute:{ident}"), line_no),
        }
    }

    fn observe_def(&mut self, trimmed: &str, line_no: usize) {
        let Some((op, name, arity)) = def_shape(trimmed) else {
            return;
        };
        // Clauses of one function share a single part, keyed by name/arity
        // exactly like the pinned analyzer (independent of `def`/`defp`).
        if self
            .last_fun
            .as_ref()
            .is_some_and(|last| last.0 == name && last.1 == arity)
        {
            return;
        }
        let kind = fun_kind(&op, self.pending);
        self.last_fun = Some((name, arity));
        self.pending = None;
        self.push(kind, line_no);
    }

    fn push(&mut self, kind: String, line_no: usize) {
        self.parts.push(Part { kind, line_no });
    }

    fn report(&self, rules: &Rules, raw_lines: &[&str], findings: &mut Vec<Finding>) {
        let mut current: Option<&str> = None;
        for part in &self.parts {
            let base = part.kind.split(':').next().unwrap_or(&part.kind);
            if rules.ignored.iter().any(|name| name == base) {
                continue;
            }
            if base == "module_attribute"
                && let Some(attr) = part.kind.split(':').nth(1)
                && rules.ignored_attrs.iter().any(|name| name == attr)
            {
                continue;
            }
            if let Some(prev) = current
                && rank(&rules.order, base) < rank(&rules.order, prev)
            {
                findings.push(Finding::with_trigger(
                    part.line_no,
                    derive_column(
                        raw_lines.get(part.line_no - 1).copied().unwrap_or(""),
                        &self.name,
                    ),
                    format!(
                        "{} must appear before {}",
                        part_label(base),
                        part_label(prev)
                    ),
                    self.name.clone(),
                ));
            }
            current = Some(base);
        }
    }
}

/// Value modifier carried by `@doc <value>`; bare `@doc` keeps the current.
fn doc_modifier(rest: &str, current: Option<Modifier>) -> Option<Modifier> {
    match attr_value(rest) {
        None => current,
        Some(value) if value == "false" => Some(Modifier::Private),
        Some(_) => None,
    }
}

/// Value modifier carried by `@impl <value>`; bare `@impl` keeps the current.
fn impl_modifier(rest: &str, current: Option<Modifier>) -> Option<Modifier> {
    match attr_value(rest) {
        None => current,
        Some(value) if value == "false" => None,
        Some(_) => Some(Modifier::Impl),
    }
}

/// Single attribute value token (`false`, `"doc"`, `(true)`), if any.
fn attr_value(rest: &str) -> Option<String> {
    let mut text = rest.trim_start();
    if text.starts_with('(') && text.ends_with(')') && text.len() > 1 {
        text = text[1..text.len() - 1].trim();
    }
    // `@doc`/`@impl` sit on ASCII prefixes: slicing is boundary-safe.
    let token: String = text
        .chars()
        .take_while(|next| !next.is_whitespace() && *next != ',')
        .collect();
    if token.is_empty() { None } else { Some(token) }
}

fn call_part(trimmed: &str) -> Option<String> {
    for name in ["use", "import", "alias", "require", "defstruct"] {
        if starts_word(trimmed, name) {
            return Some(name.to_owned());
        }
    }
    None
}

/// `(clause, name, arity)` of a `def`-family head on one line.
fn def_shape(trimmed: &str) -> Option<(String, String, usize)> {
    for clause in [
        "defmacro",
        "defmacrop",
        "defguard",
        "defguardp",
        "defp",
        "def",
    ] {
        if !starts_word(trimmed, clause) {
            continue;
        }
        // ASCII keyword: the remainder starts on a char boundary.
        let rest = &trimmed[clause.len()..];
        let name: String = rest
            .trim_start()
            .chars()
            .take_while(|next| {
                next.is_alphanumeric() || *next == '_' || *next == '?' || *next == '!'
            })
            .collect();
        if name.is_empty() {
            return None;
        }
        let after = rest.trim_start()[name.len()..].trim_start();
        if !after.starts_with('(') {
            return Some((clause.to_owned(), name, 0));
        }
        return Some((clause.to_owned(), name, arg_arity(after)));
    }
    None
}

/// Argument count of a `(...)` head remainder (commas at depth 0).
fn arg_arity(after: &str) -> usize {
    let bytes = after.as_bytes();
    let mut depth = 0_i32;
    let mut commas = 0_usize;
    let mut empty = true;
    let mut i = 0_usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return if empty { 0 } else { commas + 1 };
                }
            }
            b',' if depth == 1 => commas += 1,
            b' ' | b'\t' => {}
            _ => {
                if depth == 1 {
                    empty = false;
                }
            }
        }
        i += 1;
    }
    usize::MAX
}

fn fun_kind(op: &str, pending: Option<Modifier>) -> String {
    let kind = match op {
        "def" if pending == Some(Modifier::Impl) => "callback_impl",
        "def" if pending == Some(Modifier::Private) => "private_fun",
        "defp" => "private_fun",
        "defmacro" if pending.is_none() => "public_macro",
        "defmacro" if pending == Some(Modifier::Impl) => "callback_impl",
        "defmacro" | "defmacrop" => "private_macro",
        "defguard" if pending.is_none() => "public_guard",
        "defguard" | "defguardp" => "private_guard",
        _ => "public_fun",
    };
    kind.to_owned()
}

fn part_label(kind: &str) -> &str {
    match kind {
        "module_attribute" => "module attribute",
        "public_guard" => "public guard",
        "public_macro" => "public macro",
        "public_fun" => "public function",
        "private_fun" => "private function",
        "private_guard" => "private guard",
        "callback_impl" => "callback implementation",
        _ => kind,
    }
}

fn rank(order: &[String], kind: &str) -> usize {
    order
        .iter()
        .position(|name| name == kind)
        .unwrap_or(order.len())
}

fn module_name(trimmed: &str) -> String {
    // ASCII keyword: the remainder starts on a char boundary.
    let rest = trimmed["defmodule".len()..].trim_start();
    let name: String = rest
        .chars()
        .take_while(|next| next.is_alphanumeric() || *next == '_' || *next == '.')
        .collect();
    if name.is_empty() {
        "Unknown".to_owned()
    } else {
        name
    }
}

fn starts_word(trimmed: &str, word: &str) -> bool {
    trimmed
        .strip_prefix(word)
        .is_some_and(|rest| rest.chars().next().is_none_or(|next| !is_name_char(next)))
}

fn is_name_char(next: char) -> bool {
    next.is_alphanumeric() || next == '_' || next == '?' || next == '!'
}

/// Net `do`/`fn` minus `end` keywords on one masked line.
fn line_depth_delta(line: &str) -> i32 {
    count_keyword(line, "do", true) + count_keyword(line, "fn", false)
        - count_keyword(line, "end", false)
}

fn count_keyword(line: &str, word: &str, skip_colon_after: bool) -> i32 {
    let bytes = line.as_bytes();
    let mut count = 0_i32;
    // `match_indices` yields char boundaries; `word` is ASCII.
    for (pos, _) in line.match_indices(word) {
        if keyword_boundary_before(bytes, pos)
            && keyword_boundary_after(line, pos + word.len())
            && !(skip_colon_after && bytes.get(pos + word.len()) == Some(&b':'))
        {
            count += 1;
        }
    }
    count
}

fn keyword_boundary_before(bytes: &[u8], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let prev = bytes[pos - 1];
    !prev.is_ascii_alphanumeric()
        && prev != b'_'
        && prev != b'?'
        && prev != b'!'
        && prev != b':'
        && prev != b'@'
        && prev != b'.'
        && prev != b'&'
}

fn keyword_boundary_after(line: &str, end: usize) -> bool {
    line[end..]
        .chars()
        .next()
        .is_none_or(|next| !is_name_char(next))
}

/// Net bracket depth change of one masked line.
fn bracket_delta(line: &str) -> i32 {
    let mut delta = 0_i32;
    for byte in line.bytes() {
        match byte {
            b'(' | b'[' | b'{' => delta += 1,
            b')' | b']' | b'}' => delta -= 1,
            _ => {}
        }
    }
    delta
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
    fn ordered_is_clean() {
        let src = "defmodule M do\n  @moduledoc \"x\"\n  use Foo\n  alias Bar\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
    #[test]
    fn reports_misordered() {
        let src = "defmodule M do\n  alias Bar\n  use Foo\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn shortdoc_before_moduledoc() {
        let src = "defmodule CredoSampleModule do\n  @moduledoc \"some doc\"\n  @shortdoc \"shortdoc\"\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
        assert_eq!(findings[0].column, None);
        assert_eq!(findings[0].message, "shortdoc must appear before moduledoc");
        assert_eq!(
            findings[0].trigger,
            crate::Trigger::Text("CredoSampleModule".to_owned())
        );
    }
    #[test]
    fn custom_order_guards() {
        let mut params = BTreeMap::new();
        params.insert(
            "order".to_owned(),
            "[\"moduledoc\",\"public_guard\",\"private_guard\"]".to_owned(),
        );
        let src = "defmodule CredoSampleModule do\n  @moduledoc \"\"\n\n  defguardp is_foo(term) when term == :foo\n\n  defguard is_bar(term) when term == :bar\n\n  defguard is_baz(term) when not is_foo(term) and term == :baz\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 6);
        assert_eq!(
            findings[0].message,
            "public guard must appear before private guard"
        );
    }
    #[test]
    fn callback_impl_groups() {
        let mut params = BTreeMap::new();
        params.insert(
            "order".to_owned(),
            "[\"public_fun\",\"callback_impl\"]".to_owned(),
        );
        let src = "defmodule CredoSampleModule do\n  @impl true\n  def foo\n\n  def baz, do: :ok\n\n  @impl true\n  defmacro bar\n\n  def qux, do: :ok\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].line, 5);
        assert_eq!(findings[1].line, 10);
    }
}
