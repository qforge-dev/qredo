use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX2001`: fully-qualified calls that could use an existing alias.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let config = Config::from(params);
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let scoped = module_scoped_lines(&lines);
    let aliases = collect_aliases(&lines, &scoped);
    let deps = collect_deps(&lines, &scoped);
    // Lastname indexes (with full paths for the inequality checks below).
    let alias_pairs: Vec<(&str, &str)> = aliases
        .iter()
        .map(|alias| (last_of(alias), alias.as_str()))
        .collect();
    let dep_pairs: Vec<(&str, &str)> = deps
        .iter()
        .filter(|dep| dep.bytes().any(|byte| byte == b'.'))
        .map(|dep| (last_of(dep), dep.as_str()))
        .collect();
    let mut hits: Vec<(usize, usize, String)> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if !scoped[idx] || !flaggable_line(line) {
            continue;
        }
        for found in find_modules(line) {
            if config.should_flag(&found.path, &alias_pairs, &dep_pairs)
                && is_reference(line, &found, config.if_referenced)
            {
                hits.push((idx + 1, found.column, found.path));
            }
        }
    }
    apply_call_threshold(hits, config.if_called_more_often_than)
}

/// Lines inside a `defmodule` body. Upstream only walks `defmodule` ASTs, so
/// top-level script code and `defimpl` bodies never flag.
fn module_scoped_lines(lines: &[&str]) -> Vec<bool> {
    let mut scoped = Vec::with_capacity(lines.len());
    let mut depth = 0_usize;
    let mut stack: Vec<usize> = Vec::new();
    for line in lines {
        scoped.push(!stack.is_empty());
        let chars: Vec<char> = line.chars().collect();
        let mut idx = 0_usize;
        while idx < chars.len() {
            if word_here(&chars, idx, b"defmodule") && directive_start(&chars, idx) {
                stack.push(depth);
            } else if block_opens(&chars, idx) {
                depth += 1;
            } else if word_here(&chars, idx, b"end") && value_start(&chars, idx) {
                depth = depth.saturating_sub(1);
                while stack.last().is_some_and(|base| *base >= depth) {
                    stack.pop();
                }
            }
            idx += 1;
        }
    }
    scoped
}

/// True for a whole word at `idx` (identifier boundaries both sides).
/// The word must be ASCII so byte length equals char length.
fn word_here(chars: &[char], idx: usize, word: &[u8]) -> bool {
    chars.len() >= idx + word.len()
        && chars[idx..idx + word.len()]
            .iter()
            .zip(word.iter())
            .all(|(got, want)| *got == *want as char)
        && (idx == 0 || !is_scope_char(chars[idx - 1]))
        && !chars
            .get(idx + word.len())
            .is_some_and(|c| is_scope_char(*c))
}

fn is_scope_char(char: char) -> bool {
    char.is_alphanumeric() || char == '_' || char == '?' || char == '!'
}

/// True when `do` or `fn` at `idx` opens a block.
fn block_opens(chars: &[char], idx: usize) -> bool {
    (word_here(chars, idx, b"do") && do_opens(chars, idx))
        || (word_here(chars, idx, b"fn") && value_start(chars, idx))
}

/// True when a keyword at `idx` is real code (not an atom, field or capture).
fn value_start(chars: &[char], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = chars[idx - 1];
    !(prev == ':' || prev == '.' || prev == '@' || prev == '&')
}

/// True when `defmodule` at `idx` opens a module (same code-position rule).
fn directive_start(chars: &[char], idx: usize) -> bool {
    value_start(chars, idx)
}

/// True when `do` at `idx` opens a block (`do:` keywords take no `end`).
fn do_opens(chars: &[char], idx: usize) -> bool {
    if !value_start(chars, idx) {
        return false;
    }
    chars.get(idx + "do".len()) != Some(&':')
}

/// One dotted module path on a line.
struct FoundModule {
    /// Byte index where the path starts.
    base: usize,
    /// Byte index just past the path.
    end: usize,
    path: String,
    column: usize,
}

/// Check configuration with Credo defaults.
struct Config {
    excluded_namespaces: Vec<String>,
    excluded_lastnames: Vec<String>,
    if_nested_deeper_than: usize,
    if_called_more_often_than: usize,
    if_referenced: bool,
    only: Option<Vec<regex::Regex>>,
}

const DEFAULT_EXCLUDED_NAMESPACES: &[&str] = &[
    "File",
    "IO",
    "Inspect",
    "Kernel",
    "Macro",
    "Supervisor",
    "Task",
    "Version",
];

const DEFAULT_EXCLUDED_LASTNAMES: &[&str] = &[
    "Access",
    "Agent",
    "Application",
    "Atom",
    "Base",
    "Behaviour",
    "Bitwise",
    "Code",
    "Date",
    "DateTime",
    "Dict",
    "Enum",
    "Exception",
    "File",
    "Float",
    "GenEvent",
    "GenServer",
    "HashDict",
    "HashSet",
    "Integer",
    "IO",
    "Kernel",
    "Keyword",
    "List",
    "Macro",
    "Map",
    "MapSet",
    "Module",
    "NaiveDateTime",
    "Node",
    "OptionParser",
    "Path",
    "Port",
    "Process",
    "Protocol",
    "Range",
    "Record",
    "Regex",
    "Registry",
    "Set",
    "Stream",
    "String",
    "StringIO",
    "Supervisor",
    "System",
    "Task",
    "Time",
    "Tuple",
    "URI",
    "Version",
];

impl Config {
    fn from(params: &BTreeMap<String, String>) -> Self {
        Self {
            excluded_namespaces: string_list(
                params.get("excluded_namespaces"),
                DEFAULT_EXCLUDED_NAMESPACES,
            ),
            excluded_lastnames: string_list(
                params.get("excluded_lastnames"),
                DEFAULT_EXCLUDED_LASTNAMES,
            ),
            if_nested_deeper_than: helpers::param_usize(params, "if_nested_deeper_than", 0),
            if_called_more_often_than: helpers::param_usize(params, "if_called_more_often_than", 0),
            if_referenced: helpers::param_bool(params, "if_referenced", false),
            only: parse_only(params.get("only")),
        }
    }

    /// Module-level filters: nesting, exclusions and `only` regexes.
    fn should_flag(
        &self,
        path: &str,
        alias_pairs: &[(&str, &str)],
        dep_pairs: &[(&str, &str)],
    ) -> bool {
        // `parts.len() <= threshold` without collecting: dots + 1 <= threshold.
        if path.bytes().filter(|byte| *byte == b'.').count() < self.if_nested_deeper_than {
            return false;
        }
        let first = path.split('.').next().unwrap_or("");
        let last = last_of(path);
        if self.excluded_namespaces.iter().any(|ns| ns == first) {
            return false;
        }
        if self.excluded_lastnames.iter().any(|name| name == last) {
            return false;
        }
        if let Some(only) = &self.only
            && !only.iter().any(|regex| regex.is_match(path))
        {
            return false;
        }
        if alias_pairs
            .iter()
            .any(|(known_last, known_full)| *known_last == last && *known_full != path)
        {
            return false;
        }
        !dep_pairs
            .iter()
            .any(|(known_last, known_full)| *known_last == last && *known_full != path)
    }
}

fn last_of(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

/// Parse a JSON string list param, falling back to defaults.
fn string_list(raw: Option<&String>, default: &[&str]) -> Vec<String> {
    if let Some(text) = raw
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
        && let Some(items) = value.as_array()
    {
        return items
            .iter()
            .filter_map(|item| item.as_str().map(ToOwned::to_owned))
            .collect();
    }
    default.iter().map(|item| (*item).to_owned()).collect()
}

/// Parse the `only` regex (or regex list) param. An empty list means no
/// filter, matching upstream (`only: []` excludes nothing).
fn parse_only(raw: Option<&String>) -> Option<Vec<regex::Regex>> {
    let text = raw?;
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut out = Vec::new();
    collect_patterns(&value, &mut out);
    if out.is_empty() { None } else { Some(out) }
}

fn collect_patterns(value: &serde_json::Value, out: &mut Vec<regex::Regex>) {
    match value {
        serde_json::Value::String(pattern) => {
            if let Ok(compiled) = regex::Regex::new(pattern) {
                out.push(compiled);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(pattern)) = map.get("regex")
                && let Ok(compiled) = regex::Regex::new(pattern)
            {
                out.push(compiled);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_patterns(item, out);
            }
        }
        _ => {}
    }
}

/// Aliases declared via `alias` directives (multi-aliases expanded).
fn collect_aliases(lines: &[&str], scoped: &[bool]) -> Vec<String> {
    let mut aliases = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if !scoped[idx] {
            continue;
        }
        let trimmed = line.trim_start();
        if !is_directive(trimmed, "alias") {
            continue;
        }
        let rest = &trimmed["alias".len()..];
        if rest.contains("unquote") || rest.contains("__MODULE__") {
            continue;
        }
        if let Some(open) = rest.find('{')
            && let Some(close) = rest.find('}')
            && open < close
        {
            let prefix = rest[..open].trim().trim_end_matches('.');
            for part in rest[open + 1..close].split(',') {
                let part = part.trim();
                if !part.is_empty() {
                    aliases.push(format!("{prefix}.{part}"));
                }
            }
            continue;
        }
        let name = rest
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(',');
        if !name.is_empty() && is_alias_name(name) {
            aliases.push(name.to_owned());
        }
    }
    aliases
}

/// True for a plain alias target (`Foo` or `Foo.Bar`, not calls or assigns).
fn is_alias_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && name.contains(|c: char| c.is_ascii_alphabetic())
}

/// All multi-part module paths referenced outside `alias`/`defmodule` lines.
fn collect_deps(lines: &[&str], scoped: &[bool]) -> Vec<String> {
    let mut deps = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if !scoped[idx] {
            continue;
        }
        let trimmed = line.trim_start();
        if is_directive(trimmed, "alias") || is_directive(trimmed, "defmodule") {
            continue;
        }
        for found in find_modules(line) {
            if found.path.split('.').count() > 1 {
                deps.push(found.path);
            }
        }
    }
    deps
}

/// True for a directive keyword (`alias`, `use`, ...) at a word boundary.
fn is_directive(trimmed: &str, keyword: &str) -> bool {
    match trimmed.strip_prefix(keyword) {
        Some(rest) => !rest
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!'),
        None => false,
    }
}

/// Lines where a usage may flag (directives and attributes never flag).
fn flaggable_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('@') {
        return false;
    }
    !(is_directive(trimmed, "alias")
        || is_directive(trimmed, "use")
        || is_directive(trimmed, "import")
        || is_directive(trimmed, "require")
        || is_directive(trimmed, "defmodule"))
}

/// Dotted `Alias.Path` occurrences with columns.
fn find_modules(line: &str) -> Vec<FoundModule> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut idx = 0_usize;
    // Byte offset of `chars[idx]`, tracked incrementally so module slicing
    // stays O(1) amortized instead of rescanning the line per occurrence.
    let mut byte = 0_usize;
    while idx < chars.len() {
        if is_segment_start(&chars, idx)
            && let Some(end) = match_path(&chars, idx)
        {
            let path: String = chars[idx..end].iter().collect();
            let end_byte = byte + chars[idx..end].iter().map(|c| c.len_utf8()).sum::<usize>();
            found.push(FoundModule {
                base: byte,
                end: end_byte,
                path,
                column: idx + 1,
            });
            byte = end_byte;
            idx = end;
            continue;
        }
        byte += chars[idx].len_utf8();
        idx += 1;
    }
    found
}

/// True for an uppercase segment start outside identifiers and field access.
fn is_segment_start(chars: &[char], idx: usize) -> bool {
    if !chars[idx].is_ascii_uppercase() {
        return false;
    }
    if idx == 0 {
        return true;
    }
    let prev = chars[idx - 1];
    // `%` passes through so struct modules (`%Foo.Bar{}`) still land in the
    // dependency set; they never flag since `{` is not a call.
    !(prev.is_alphanumeric() || prev == '_' || prev == '.' || prev == ':')
}

/// Match `Seg(.Seg)*` at `idx`; returns the end index when multi-part.
fn match_path(chars: &[char], idx: usize) -> Option<usize> {
    let mut end = match_segment(chars, idx)?;
    let mut multi = false;
    while chars.get(end) == Some(&'.') && chars.get(end + 1).is_some_and(char::is_ascii_uppercase) {
        multi = true;
        end = match_segment(chars, end + 1)?;
    }
    if multi { Some(end) } else { None }
}

/// Match one `PascalCase` segment; returns the index just past it.
fn match_segment(chars: &[char], idx: usize) -> Option<usize> {
    if !chars.get(idx).is_some_and(char::is_ascii_uppercase) {
        return None;
    }
    let mut end = idx + 1;
    while chars
        .get(end)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
    {
        end += 1;
    }
    Some(end)
}

/// True for a remote call (`Mod.fun`) or, with `if_referenced`, a bare
/// single-argument module reference (`fun(Mod)`).
fn is_reference(line: &str, found: &FoundModule, if_referenced: bool) -> bool {
    if follows_dot_call(line, found) {
        return true;
    }
    if_referenced && (is_sole_paren_arg(line, found) || is_sole_bare_arg(line, found))
}

/// True when the path is a call receiver (`Mod.fun`, not `Mod.unquote`).
fn follows_dot_call(line: &str, found: &FoundModule) -> bool {
    let tail = &line[found.end..];
    if !tail.starts_with('.') {
        return false;
    }
    let name: String = tail[1..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
        .collect();
    // `Mod.unquote(...)` is explicitly ignored upstream.
    !name.is_empty() && name != "unquote"
}

/// True for `fun(Mod)`: sole parenthesized argument of a call.
fn is_sole_paren_arg(line: &str, found: &FoundModule) -> bool {
    let before: Vec<char> = line[..found.base].chars().collect();
    let mut idx = before.len();
    while idx > 0 && before[idx - 1].is_whitespace() {
        idx -= 1;
    }
    if idx == 0 || before[idx - 1] != '(' {
        return false;
    }
    let mut head = idx - 1;
    while head > 0 && before[head - 1].is_whitespace() {
        head -= 1;
    }
    if head == 0 || !is_call_char(before[head - 1]) {
        return false;
    }
    let after: Vec<char> = line[found.end..].chars().collect();
    let mut tail = 0_usize;
    while tail < after.len() && after[tail].is_whitespace() {
        tail += 1;
    }
    if tail >= after.len() || after[tail] != ')' {
        return false;
    }
    tail += 1;
    while tail < after.len() && after[tail].is_whitespace() {
        tail += 1;
    }
    // A following comma means more arguments (`if (Mod), do: ...` is clean).
    tail >= after.len() || after[tail] != ','
}

fn is_call_char(char: char) -> bool {
    char.is_alphanumeric() || char == '_' || char == '?' || char == '!'
}

/// True for `fun Mod`: sole unparenthesized argument (not after keywords).
fn is_sole_bare_arg(line: &str, found: &FoundModule) -> bool {
    let before: Vec<char> = line[..found.base].chars().collect();
    let mut idx = before.len();
    while idx > 0 && before[idx - 1].is_whitespace() {
        idx -= 1;
    }
    let mut end = idx;
    while end > 0 && is_call_char(before[end - 1]) {
        end -= 1;
    }
    if end == idx {
        return false;
    }
    let word: String = before[end..idx].iter().collect();
    if is_keyword(&word) {
        return false;
    }
    let after: Vec<char> = line[found.end..].chars().collect();
    let mut tail = 0_usize;
    while tail < after.len() && after[tail].is_whitespace() {
        tail += 1;
    }
    tail >= after.len() || [')', ']', '}', ';', '|', '#'].contains(&after[tail])
}

/// Keywords that never take a flaggable bare module argument.
fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "alias"
            | "use"
            | "import"
            | "require"
            | "defmodule"
            | "def"
            | "defp"
            | "defmacro"
            | "defguard"
            | "do"
            | "end"
            | "else"
            | "fn"
            | "in"
            | "not"
            | "and"
            | "or"
            | "when"
            | "with"
            | "if"
            | "unless"
            | "case"
            | "cond"
            | "try"
            | "rescue"
            | "catch"
            | "after"
            | "quote"
            | "unquote"
            | "super"
            | "receive"
            | "for"
            | "true"
            | "false"
            | "nil"
    )
}

/// Keep only modules called more often than the threshold, sorted by position.
fn apply_call_threshold(
    mut hits: Vec<(usize, usize, String)>,
    more_often_than: usize,
) -> Vec<Finding> {
    hits.sort();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, _, path) in &hits {
        *counts.entry(path.clone()).or_default() += 1;
    }
    hits.into_iter()
        .filter(|(_, _, path)| counts.get(path).unwrap_or(&0) > &more_often_than)
        .map(|(line, column, path)| {
            Finding::with_trigger(
                line,
                Some(column),
                "Nested modules could be aliased at the top of the invoking module.",
                path,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    #[test]
    fn no_alias_no_issue() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy("Foo.Bar.baz()\n"), &params()).is_empty()
        );
    }
    #[test]
    fn reports_nested_use() {
        let src = "defmodule T do\n  alias Foo.Qux\n  Foo.Qux.baz()\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params()).len(),
            1
        );
    }
    #[test]
    fn reports_nested_call() {
        let src = "defmodule CredoSampleModule do\n  def fun1 do\n    ExUnit.Case.something\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params()),
            vec![Finding::with_trigger(
                3,
                Some(5),
                "Nested modules could be aliased at the top of the invoking module.",
                "ExUnit.Case",
            )]
        );
    }
    #[test]
    fn stdlib_lastnames_are_clean() {
        let src = "defmodule CredoSampleModule do\n  alias ExUnit.Case\n\n  def fun1 do\n    Case.something\n\n    fun_call().Api.Case\n\n    {:error, reason} = __MODULE__.Sup.start_link(fn() -> :foo end)\n\n    [:faint, filename]\n    |> IO.ANSI.format\n    |> Credo.Foo.Code.run\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params()).is_empty());
    }
    #[test]
    fn excluded_lastname_param_is_clean() {
        let src =
            "defmodule Test do\n  def just_an_example do\n    Credo.Foo.Bar.call\n  end\nend\n";
        let mut given = params();
        given.insert("excluded_lastnames".to_owned(), "[\"Bar\"]".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &given).is_empty());
    }
    #[test]
    fn excluded_namespace_param_is_clean() {
        let src = "defmodule Test do\n  def just_an_example do\n    Foo.Qux.baz()\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params()).len(),
            1
        );
        let mut given = params();
        given.insert("excluded_namespaces".to_owned(), "[\"Foo\"]".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &given).is_empty());
    }
    #[test]
    fn nested_threshold_param_is_clean() {
        let src = "defmodule Test do\n  def just_an_example do\n    Foo.Qux.baz()\n  end\nend\n";
        let mut given = params();
        given.insert("if_nested_deeper_than".to_owned(), "2".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &given).is_empty());
    }
    #[test]
    fn call_threshold_param_is_clean() {
        let src = "defmodule Test do\n  def just_an_example do\n    Foo.Qux.baz()\n  end\nend\n";
        let mut given = params();
        given.insert("if_called_more_often_than".to_owned(), "1".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &given).is_empty());
    }
    #[test]
    fn referenced_param_reports_bare_module() {
        let src = "defmodule Test do\n  def just_an_example do\n    foo(Foo.Qux)\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params()).is_empty());
        let mut given = params();
        given.insert("if_referenced".to_owned(), "true".to_owned());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &given).len(),
            1
        );
    }
    #[test]
    fn only_param_filters_modules() {
        let src = "defmodule Test do\n  def just_an_example do\n    Foo.Qux.baz()\n  end\nend\n";
        let mut matching = params();
        matching.insert("only".to_owned(), "\"Foo\"".to_owned());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &matching).len(),
            1
        );
        let mut missing = params();
        missing.insert("only".to_owned(), "\"Bar\"".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &missing).is_empty());
    }
    #[test]
    fn conflicting_alias_is_clean() {
        let src = "defmodule Test do\n  alias Exzmq.Socket\n  alias Exzmq.Tcp\n\n  def just_an_example do\n    Socket.test1\n    Tcp.Socket.test2\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params()).is_empty());
    }
    #[test]
    fn top_level_usage_is_clean() {
        // Upstream only walks `defmodule` bodies.
        let src = "alias Foo.Qux\nFoo.Qux.baz()\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params()).is_empty());
    }
    #[test]
    fn struct_modules_count_as_dependencies() {
        // `%Foo.Bar{}` shares its lastname with `Qux.Bar`, blocking the alias.
        let src = "defmodule T do\n  def f do\n    %Foo.Bar{}\n    Qux.Bar.baz()\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params()).is_empty());
    }
    #[test]
    fn reports_module_call_inside_interpolation() {
        // AU-A: `#{Ecto.UUID.generate()}` is code, not string content.
        let src = "defmodule T do\n  def f do\n    \"#{Ecto.UUID.generate()}\"\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params()),
            vec![Finding::with_trigger(
                3,
                Some(8),
                "Nested modules could be aliased at the top of the invoking module.",
                "Ecto.UUID",
            )]
        );
    }
    #[test]
    fn quote_cocktail_does_not_hide_later_usage() {
        // AU-B: quotes nested in interpolation must not desync the masker
        // and hide the module calls on the following lines.
        let src = "defmodule T do\n  def f(path) do\n    x = 'p#{String.replace(path, \"'\", \"''\")}'\n    Labqoat.Config.get()\n    Labqoat.DuckDB.open()\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src), &params());
        assert_eq!(
            out.iter().map(|f| f.line).collect::<Vec<_>>(),
            vec![4, 5],
            "findings were: {out:?}"
        );
    }
    #[test]
    fn multibyte_prefix_keeps_columns_and_slices() {
        // Byte offsets must track char columns past multibyte content.
        let src = "defmodule T do\n  # héllo wörld\n  Foo.Qux.baz()\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params()),
            vec![Finding::with_trigger(
                3,
                Some(3),
                "Nested modules could be aliased at the top of the invoking module.",
                "Foo.Qux",
            )]
        );
    }
}
