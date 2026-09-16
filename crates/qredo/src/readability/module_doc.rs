use crate::{Finding, Trigger};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// `EX3009`: modules should have a `@moduledoc`.
///
/// Each `defmodule` is judged on its own direct `@moduledoc`: missing docs
/// report `Modules should have a @moduledoc tag.`, while an empty string
/// reports `Use `@moduledoc false` ...`. `defexception` modules are exempt.
/// `ignore_names`/`ignore_modules_using` params override their Credo
/// defaults when present (atoms as `Elixir.` names, binaries as
/// substrings, `{"regex": ...}` as patterns). Ignoring a module skips its
/// whole subtree. The `.exs` filename skip lives at pipeline level:
/// without a filename every module is analyzed.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let facts = prepared.facts();
    let ignore_names = prepare_matchers(params.get("ignore_names"), &default_ignore_names());
    let ignore_using =
        prepare_matchers(params.get("ignore_modules_using"), &default_ignore_using());
    let starts = line_starts(source);
    let mut findings = Vec::new();
    // Pruned subtrees: an ignored module's nested modules are never
    // visited (the walk skips their descent), so findings stop there.
    let mut pruned: Vec<(u32, u32)> = Vec::new();
    for (index, module) in facts.modules.iter().enumerate() {
        if pruned
            .iter()
            .any(|(start, end)| *start <= module.start && module.end <= *end)
        {
            continue;
        }
        let Some((alias_start, alias_end)) = module.alias else {
            continue;
        };
        let Some(raw) = slice(source, alias_start, alias_end) else {
            continue;
        };
        let body: Vec<&crate::facts::BodyStmtFact> = facts
            .module_bodies
            .iter()
            .filter(|body| body.module as usize == index)
            .flat_map(|body| body.stmts.iter())
            .collect();
        if module_ignored(raw, &body, source, &ignore_names, &ignore_using) {
            pruned.push((module.start, module.end));
            continue;
        }
        if let Some(finding) = missing_doc(module, raw, &body, source, &starts) {
            findings.push(finding);
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// True when the module is exempt via name, `use`, or `defexception`.
fn module_ignored(
    name: &str,
    body: &[&crate::facts::BodyStmtFact],
    source: &str,
    ignore_names: &[PreparedMatcher],
    ignore_using: &[PreparedMatcher],
) -> bool {
    if ignore_names.iter().any(|matcher| matcher.matches(name)) {
        return true;
    }
    if has_defexception(body, source) {
        return true;
    }
    body.iter().any(|stmt| {
        let crate::facts::BodyStmtFact::Call {
            head_start,
            head_end,
            arg_alias,
        } = stmt
        else {
            return false;
        };
        if slice(source, *head_start, *head_end) != Some("use") {
            return false;
        }
        arg_alias
            .and_then(|(start, end)| slice(source, start, end))
            .is_some_and(|used| ignore_using.iter().any(|matcher| matcher.matches(used)))
    })
}

/// Missing-doc finding for one module, or `None` when documented.
fn missing_doc(
    module: &crate::facts::ModuleFact,
    raw: &str,
    body: &[&crate::facts::BodyStmtFact],
    source: &str,
    starts: &[usize],
) -> Option<Finding> {
    if has_defexception(body, source) {
        return None;
    }
    let (alias_start, _) = module.alias?;
    let (line, column) = line_col(starts, source, alias_start as usize);
    match moduledoc_value(body, source) {
        Doc::Present => None,
        Doc::Missing => Some(Finding {
            line,
            column: Some(column),
            message: "Modules should have a @moduledoc tag.".to_owned(),
            trigger: Trigger::Text(raw.to_owned()),
            severity: None,
        }),
        Doc::Empty => Some(Finding {
            line,
            column: Some(column),
            message: "Use `@moduledoc false` if a module will not be documented.".to_owned(),
            trigger: Trigger::Text(raw.to_owned()),
            severity: None,
        }),
    }
}

/// Direct `@moduledoc` value of a module body.
fn moduledoc_value(body: &[&crate::facts::BodyStmtFact], source: &str) -> Doc {
    for stmt in body {
        let crate::facts::BodyStmtFact::Attr {
            name_start,
            name_end,
            value,
        } = stmt
        else {
            continue;
        };
        if slice(source, *name_start, *name_end) != Some("moduledoc") {
            continue;
        }
        return match value {
            crate::facts::AttrValue::EmptyString => Doc::Empty,
            crate::facts::AttrValue::Other => Doc::Present,
        };
    }
    Doc::Missing
}

/// True when the module body directly calls `defexception`.
fn has_defexception(body: &[&crate::facts::BodyStmtFact], source: &str) -> bool {
    body.iter().any(|stmt| {
        if let crate::facts::BodyStmtFact::Call {
            head_start,
            head_end,
            ..
        } = stmt
        {
            slice(source, *head_start, *head_end) == Some("defexception")
        } else {
            false
        }
    })
}

/// A name matcher: substring or regex.
#[derive(Clone)]
enum Matcher {
    Contains(String),
    Pattern(String),
}

/// A name matcher with its pattern compiled once.
enum PreparedMatcher {
    Contains(String),
    Regex(regex::Regex),
}

impl PreparedMatcher {
    fn matches(&self, name: &str) -> bool {
        match self {
            PreparedMatcher::Contains(part) => name.contains(part.as_str()),
            PreparedMatcher::Regex(expression) => expression.is_match(name),
        }
    }
}

/// Default `ignore_names` pattern source, shared with its static below.
const DEFAULT_NAME_PATTERN: &str = "(\\.\\w+Controller|\\.Endpoint|\\.\\w+Live(\\.\\w+)?|\\.Repo|\\.Router|\\.\\w+Socket|\\.\\w+View|\\.\\w+HTML|\\.\\w+JSON|\\.Telemetry|\\.Layouts|\\.Mailer)$";

/// Default `ignore_modules_using` pattern source, shared with its static below.
const DEFAULT_USING_PATTERN: &str = "\\.Web$";

/// Default name pattern, compiled once per process.
fn default_name_regex() -> regex::Regex {
    static COMPILED: OnceLock<regex::Regex> = OnceLock::new();
    COMPILED
        .get_or_init(|| {
            regex::Regex::new(DEFAULT_NAME_PATTERN).expect("default ignore_names pattern compiles")
        })
        .clone()
}

/// Default using pattern, compiled once per process.
fn default_using_regex() -> regex::Regex {
    static COMPILED: OnceLock<regex::Regex> = OnceLock::new();
    COMPILED
        .get_or_init(|| {
            regex::Regex::new(DEFAULT_USING_PATTERN).expect("default ignore_using pattern compiles")
        })
        .clone()
}

/// Credo default `ignore_names`: Phoenix-style generated modules.
fn default_ignore_names() -> Vec<Matcher> {
    vec![Matcher::Pattern(DEFAULT_NAME_PATTERN.to_owned())]
}

/// Credo default `ignore_modules_using`.
fn default_ignore_using() -> Vec<Matcher> {
    vec![
        Matcher::Contains("Credo.Check".to_owned()),
        Matcher::Contains("Ecto.Schema".to_owned()),
        Matcher::Contains("Phoenix.LiveView".to_owned()),
        Matcher::Pattern(DEFAULT_USING_PATTERN.to_owned()),
    ]
}

/// Param matchers, or the Credo defaults when the param is absent.
/// Atoms arrive colon-marked (`:Elixir.Foo`, tolerantly bare), binaries match
/// as substrings, `{"regex": ...}` entries match as patterns.
fn matchers(raw: Option<&String>, defaults: &[Matcher]) -> Vec<Matcher> {
    let Some(raw) = raw else {
        return defaults.to_vec();
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            serde_json::Value::String(name) => {
                let bare = name.trim_start_matches(':');
                Some(Matcher::Contains(
                    bare.strip_prefix("Elixir.").unwrap_or(bare).to_owned(),
                ))
            }
            serde_json::Value::Object(_) => item
                .get("regex")
                .and_then(serde_json::Value::as_str)
                .map(|pattern| Matcher::Pattern(pattern.to_owned())),
            _ => None,
        })
        .collect()
}

/// Matchers with patterns compiled once: defaults reuse process-wide
/// statics, custom patterns compile once per call. An invalid custom
/// pattern never matches, mirroring the previous per-match behavior.
fn prepare_matchers(raw: Option<&String>, defaults: &[Matcher]) -> Vec<PreparedMatcher> {
    matchers(raw, defaults)
        .into_iter()
        .filter_map(|matcher| match matcher {
            Matcher::Contains(part) => Some(PreparedMatcher::Contains(part)),
            Matcher::Pattern(pattern) => {
                if pattern == DEFAULT_NAME_PATTERN {
                    Some(PreparedMatcher::Regex(default_name_regex()))
                } else if pattern == DEFAULT_USING_PATTERN {
                    Some(PreparedMatcher::Regex(default_using_regex()))
                } else {
                    regex::Regex::new(&pattern).ok().map(PreparedMatcher::Regex)
                }
            }
        })
        .collect()
}

#[derive(PartialEq, Eq)]
enum Doc {
    Present,
    Missing,
    Empty,
}

/// Byte offsets where each 1-based line starts (`starts[0]` is zero).
fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    starts.extend(source.match_indices('\n').map(|(byte, _)| byte + 1));
    starts
}

/// 1-based `(line, column)` with the column counted in characters.
fn line_col(starts: &[usize], source: &str, byte: usize) -> (usize, usize) {
    if byte > source.len() {
        return (1, 1);
    }
    let line = starts.partition_point(|start| *start <= byte);
    let column = source
        .get(starts[line - 1]..byte)
        .unwrap_or("")
        .chars()
        .count()
        + 1;
    (line, column)
}

/// Source slice for fact spans; `None` on invalid boundaries.
fn slice(source: &str, start: u32, end: u32) -> Option<&str> {
    source.get(start as usize..end as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Single-source entry: the same lazy parse the old `check` built.
    fn check(src: &str, params: &BTreeMap<String, String>) -> Vec<Finding> {
        check_prepared(&crate::batch::Prepared::lazy(src), params)
    }

    #[test]
    fn documented_module_is_clean() {
        let src = "defmodule M do\n  @moduledoc \"Docs\"\nend\n";
        assert!(check(src, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn reports_missing_moduledoc() {
        let src = "defmodule M do\nend\n";
        assert_eq!(check(src, &BTreeMap::new()).len(), 1);
    }

    #[test]
    fn reports_parent_missing_moduledoc_when_child_has_one() {
        let src = "defmodule Foo do\n  # distinctly no moduledoc here\n  defmodule Bar do\n    @moduledoc false\n  end\nend\n";
        let mut params = BTreeMap::new();
        params.insert("ignore_names".to_owned(), "[]".to_owned());
        let findings = check(src, &params);
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (1, Some(11)));
    }

    #[test]
    fn reports_empty_moduledoc_with_hint_message() {
        let src = "defmodule CredoSampleModule do\n  @moduledoc \"\"\nend\n";
        let findings = check(src, &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Use `@moduledoc false` if a module will not be documented."
        );
    }

    #[test]
    fn skips_exception_modules() {
        let src = "defmodule CredoSampleModule do\n  defexception message: \"Bad luck\"\nend\n";
        assert!(check(src, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn respects_ignore_names() {
        let src = "defmodule CredoSampleModule do\n  def some_fun, do: :ok\nend\n";
        let mut params = BTreeMap::new();
        params.insert(
            "ignore_names".to_owned(),
            r#"["Elixir.CredoSampleModule"]"#.to_owned(),
        );
        assert!(check(src, &params).is_empty());
    }
    #[test]
    fn respects_ignore_modules_using() {
        let src = "defmodule CredoSampleModule do\n  use ExUnit.Case\nend\n";
        assert_eq!(check(src, &BTreeMap::new()).len(), 1);
        let mut params = BTreeMap::new();
        params.insert(
            "ignore_modules_using".to_owned(),
            r#"["ExUnit.Case"]"#.to_owned(),
        );
        assert!(check(src, &params).is_empty());
    }
    #[test]
    fn default_ignores_phoenix_controller() {
        let src = "defmodule MyApp.UserController do\nend\n";
        assert!(check(src, &BTreeMap::new()).is_empty());
    }
    #[test]
    fn invalid_custom_pattern_never_matches() {
        let src = "defmodule M do\nend\n";
        let mut params = BTreeMap::new();
        params.insert("ignore_names".to_owned(), r#"[{"regex": "(["}]"#.to_owned());
        assert_eq!(check(src, &params).len(), 1);
    }
    #[test]
    fn ignored_parent_prunes_nested_modules() {
        // An ignored module's subtree is never descended into: the nested
        // module reports nothing even though it has no moduledoc either.
        let src =
            "defmodule MyApp.SomePhoenixController do\n  defmodule SubModule do\n  end\nend\n";
        assert!(check(src, &BTreeMap::new()).is_empty());
    }
    #[test]
    fn nested_trigger_uses_short_name() {
        let src =
            "defmodule MyApp.SomePhoenixController do\n  defmodule SubModule do\n  end\nend\n";
        let mut params = BTreeMap::new();
        params.insert("ignore_names".to_owned(), "[]".to_owned());
        let findings = check(src, &params);
        assert_eq!(findings.len(), 2);
        assert_eq!(
            findings[1].trigger,
            crate::Trigger::Text("SubModule".to_owned())
        );
    }
}
