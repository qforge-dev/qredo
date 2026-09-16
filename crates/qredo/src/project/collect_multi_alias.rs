//! Project-level `MultiAliasImportRequireUse` consistency (EX1003).
//!
//! Directives (`alias`, `import`, `require`, `use`) inside a `defmodule`
//! classify per module: `Alias.Foo.{Bar}` (and the same shape under the other
//! directives) is multi, while a repeated `Alias.Foo.Bar`-style single with
//! the same `{directive, base}` pair is a multi candidate. Single-segment and
//! option-bearing (`alias Foo.Bar, as: Baz`) forms never vote, and `def`/`defp`
//! bodies are pruned exactly like the upstream traversal. Stats merge across
//! modules and files; the majority wins (ties toward the smallest key) and
//! locations depend on it: an expected `:multi` flags the last line of every
//! repeated-single group, an expected `:single` flags every multi line.
//! Neither the check nor its inventory entry declares parameters, so `params`
//! is ignored. Findings always carry an empty trigger; columns come from the
//! same trigger-search backfill as every other check.

use std::collections::BTreeMap;

use super::{ProjectFile, ProjectIssue, majority};

/// Run the check over a file set.
pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let facts: Vec<crate::facts::Facts> = files
        .iter()
        .map(|file| {
            crate::ts_parser::parse(&file.source).map_or_else(crate::facts::Facts::empty, |tree| {
                crate::facts::extract(&tree, &file.source)
            })
        })
        .collect();
    let refs: Vec<&crate::facts::Facts> = facts.iter().collect();
    run_with_facts(files, &refs, params)
}

/// Run the check over shared single-walk facts: the pipeline shares one
/// parse and one walk per file instead of walking trees per file.
pub(crate) fn run_with_facts(
    files: &[ProjectFile],
    facts: &[&crate::facts::Facts],
    _params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_file = Vec::with_capacity(files.len());
    for (position, file) in files.iter().enumerate() {
        let modules = match facts.get(position) {
            Some(facts) => collect_from_facts(&file.source, facts),
            None => BTreeMap::new(),
        };
        let stats = file_stats(&modules);
        for (key, count) in &stats {
            *counts.entry(key.clone()).or_insert(0) += count;
        }
        per_file.push((stats, modules));
    }
    if counts.is_empty() {
        return Vec::new();
    }
    let Some(expected) = majority(&counts, None) else {
        return Vec::new();
    };
    let mut issues = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let (stats, modules) = &per_file[index];
        let flagged = stats.keys().any(|key| key != &expected);
        if !flagged {
            continue;
        }
        for line in locations(modules, &expected) {
            issues.push(issue_for(index, &file.source, line, &expected));
        }
    }
    issues
}

/// One directive occurrence: `base` is `None` for multi syntax.
struct Entry {
    directive: String,
    base: Option<String>,
    line: usize,
}

/// Merged per-module stats for one file: multi counts every multi line,
/// single counts every repeated `{directive, base}` group once.
fn file_stats(modules: &BTreeMap<String, Vec<Entry>>) -> BTreeMap<String, usize> {
    let mut stats = BTreeMap::new();
    for entries in modules.values() {
        let multi = entries.iter().filter(|entry| entry.base.is_none()).count();
        let single = repeated_single_lines(entries).len();
        if multi > 0 {
            *stats.entry("multi".to_owned()).or_insert(0) += multi;
        }
        if single > 0 {
            *stats.entry("single".to_owned()).or_insert(0) += single;
        }
    }
    stats
}

/// Last line of every repeated `{directive, base}` single group: entries are
/// visited in source order, so the maximum line is the newest occurrence,
/// matching the upstream prepend-then-head behavior.
fn repeated_single_lines(entries: &[Entry]) -> Vec<usize> {
    let mut groups: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    for entry in entries {
        let Some(base) = &entry.base else {
            continue;
        };
        let slot = groups
            .entry((entry.directive.clone(), base.clone()))
            .or_insert((0, 0));
        slot.0 += 1;
        slot.1 = slot.1.max(entry.line);
    }
    groups
        .into_values()
        .filter(|(count, _)| *count > 1)
        .map(|(_, line)| line)
        .collect()
}

/// Lines not matching the winner: repeated singles under `:multi`, every
/// multi line under `:single`. Modules iterate in name order; the gate sorts
/// findings, so only the multiset matters.
fn locations(modules: &BTreeMap<String, Vec<Entry>>, expected: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    for entries in modules.values() {
        if expected == "multi" {
            lines.extend(repeated_single_lines(entries));
        } else {
            lines.extend(
                entries
                    .iter()
                    .filter(|entry| entry.base.is_none())
                    .map(|entry| entry.line),
            );
        }
    }
    lines
}

/// One issue with an empty trigger and a backfilled column.
fn issue_for(file: usize, source: &str, line: usize, expected: &str) -> ProjectIssue {
    let line_text = source.split('\n').nth(line.saturating_sub(1)).unwrap_or("");
    ProjectIssue {
        file,
        line: Some(line),
        column: trigger_column(line_text, ""),
        trigger: String::new(),
        message: message_for(expected).to_owned(),
        severity: None,
    }
}

/// Message for the winning style, mirroring the check module.
fn message_for(expected: &str) -> &'static str {
    if expected == "multi" {
        "Most of the time you are using the multi-alias/require/import/use syntax, but here you are using multiple single directives"
    } else {
        "Most of the time you are using the multiple single line alias/require/import/use directives but here you are using the multi-alias/require/import/use syntax"
    }
}

/// A module or directive event in walk order for module threading.
enum Event<'a> {
    Module(&'a crate::facts::ModuleFact),
    Directive(&'a crate::facts::AliasDirectiveFact),
}

/// Per-module directive entries in source order. A repeated `defmodule` name
/// resets its entries (`Map.put` upstream); `def`/`defp` bodies are pruned.
/// `facts` reuses the prepare-phase single walk; the error gate applies.
fn collect_from_facts(source: &str, facts: &crate::facts::Facts) -> BTreeMap<String, Vec<Entry>> {
    if facts.has_error {
        return BTreeMap::new();
    }
    let mut modules: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for (_, event) in ordered_events(source, facts) {
        match event {
            Event::Module(module) => {
                let Some((alias_start, alias_end)) = module.alias else {
                    continue;
                };
                let Some(name) = source.get(alias_start as usize..alias_end as usize) else {
                    continue;
                };
                modules.insert(name.to_owned(), Vec::new());
                current = Some(name.to_owned());
            }
            Event::Directive(directive) => {
                let Some(head) =
                    source.get(directive.head_start as usize..directive.head_end as usize)
                else {
                    continue;
                };
                if !matches!(head, "alias" | "import" | "require" | "use") {
                    continue;
                }
                let Some(active) = current.clone() else {
                    continue;
                };
                if let Some(entry) = classify(directive, source, head)
                    && let Some(entries) = modules.get_mut(&active)
                {
                    entries.push(entry);
                }
            }
        }
    }
    modules
}

/// Modules and directives merged in walk order outside pruned `def`/`defp`
/// bodies; the current module is never popped, so later directives keep
/// belonging to it.
fn ordered_events<'a>(source: &'a str, facts: &'a crate::facts::Facts) -> Vec<(u32, Event<'a>)> {
    // `def`/`defp` (not `defmacro`) bodies are pruned like the walk.
    let pruned: Vec<(u32, u32)> = facts
        .calls
        .iter()
        .filter(|call| {
            if let Some(crate::facts::HeadFact::Plain { start, end }) = &call.head {
                source.get(*start as usize..*end as usize) == Some("def")
                    || source.get(*start as usize..*end as usize) == Some("defp")
            } else {
                false
            }
        })
        .map(|call| (call.start, call.end))
        .collect();
    let pruned = |start: u32, end: u32| pruned.iter().any(|(ps, pe)| *ps <= start && end <= *pe);
    let mut events: Vec<(u32, Event)> = Vec::new();
    for module in &facts.modules {
        if pruned(module.start, module.end) {
            continue;
        }
        events.push((module.start, Event::Module(module)));
    }
    for directive in &facts.alias_directives {
        if pruned(directive.start, directive.end) {
            continue;
        }
        events.push((directive.start, Event::Directive(directive)));
    }
    events.sort_by_key(|(start, _)| *start);
    events
}

/// Classify one directive: a lone dotted `alias` argument is single with
/// its base (all but the last segment); a lone `Base.{Parts}` dot is multi.
/// Anything else (single segments, options, `__MODULE__` heads) never votes.
fn classify(
    directive: &crate::facts::AliasDirectiveFact,
    source: &str,
    head: &str,
) -> Option<Entry> {
    if directive.arg_kinds.len() != 1 {
        return None;
    }
    let line = directive.line as usize;
    if directive.arg_kinds[0] == crate::facts::NodeKind::Alias {
        let (start, end) = directive.single?;
        let text = source.get(start as usize..end as usize)?;
        let (base, _) = text.rsplit_once('.')?;
        return Some(Entry {
            directive: head.to_owned(),
            base: Some(base.to_owned()),
            line,
        });
    }
    if directive.dot_kids == [crate::facts::NodeKind::Alias, crate::facts::NodeKind::Tuple] {
        return Some(Entry {
            directive: head.to_owned(),
            base: None,
            line,
        });
    }
    None
}

/// Column of `trigger` in `line`, mirroring `Credo.SourceFile.column/3`.
/// The empty trigger matches at the first word/paren/comma boundary, which
/// is column 2 on indented directive lines and column 1 otherwise.
fn trigger_column(line: &str, trigger: &str) -> Option<usize> {
    let pattern = format!(
        r"(\s|\b|\(|\)|,)({})(\s|\b|\(|\)|,)",
        regex::escape(trigger)
    );
    let regex = regex::Regex::new(&pattern).ok()?;
    regex
        .captures(line)
        .and_then(|captures| captures.get(2))
        .map(|hit| hit.start() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(source: &str) -> ProjectFile {
        ProjectFile {
            filename: "case.ex".to_owned(),
            source: source.to_owned(),
        }
    }

    #[test]
    fn consistent_single_project_is_clean() {
        let files = vec![file(
            "defmodule Credo.Sample2 do\n  alias Foo.Bar\n  alias Foo.Quux\n  require Foo.Bar\nend\n",
        )];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn def_bodies_are_skipped() {
        let files = vec![
            file("defmodule A.B.Foo do\n  alias A.{B, C}\nend\n"),
            file(
                "defmodule SomeModule do\n  @moduledoc false\n\n  alias A.B.Foo\n\n  case Application.compile_env(:app, SomeModule)[:use_bar] do\n    \"true\" ->\n      defp a do\n        alias A.B.Bar\n        Bar.f()\n      end\n\n    _ ->\n      defp a, do: nil\n  end\nend\n",
            ),
        ];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn mixed_styles_flag_repeated_singles() {
        let files = vec![
            file(
                "defmodule Credo.Sample2 do\n  alias Foo.Bar\n  alias Foo.Quux\n  require Foo.Bar\nend\n",
            ),
            file(
                "defmodule Credo.Sample3 do\n  alias Foo.{Bar, Quux}\n  alias Bar.{Baz, Bang}\n  alias Foo.Bar\n  require Foo.Quux\nend\n",
            ),
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 0);
        assert_eq!(issues[0].line, Some(3));
        assert_eq!(issues[0].column, Some(2));
        assert_eq!(issues[0].trigger, "");
        assert_eq!(
            issues[0].message,
            "Most of the time you are using the multi-alias/require/import/use syntax, but here you are using multiple single directives"
        );
    }

    #[test]
    fn shared_facts_match_fresh_parses() {
        // The pipeline shares prepare-phase facts instead of re-walking;
        // both paths must report identically.
        let sources = [
            "defmodule Credo.Sample2 do\n  alias Foo.Bar\n  alias Foo.Quux\n  require Foo.Bar\nend\n",
            "defmodule Credo.Sample3 do\n  alias Foo.{Bar, Quux}\n  alias Bar.{Baz, Bang}\n  alias Foo.Bar\n  require Foo.Quux\nend\n",
        ];
        let files: Vec<ProjectFile> = sources.iter().map(|source| file(source)).collect();
        let expected = run(&files, &BTreeMap::new());
        assert!(!expected.is_empty());
        let prepared: Vec<crate::batch::Prepared<'_>> = sources
            .iter()
            .map(|source| crate::batch::Prepared::eager(source))
            .collect();
        let facts: Vec<&crate::facts::Facts> =
            prepared.iter().map(crate::batch::Prepared::facts).collect();
        assert_eq!(run_with_facts(&files, &facts, &BTreeMap::new()), expected);
    }

    #[test]
    fn corpus_groups_match_native() {
        let mismatches = corpus_mismatches();
        assert!(
            mismatches.is_empty(),
            "corpus mismatches:\n{}",
            mismatches.join("\n")
        );
    }

    #[test]
    fn tie_breaks_toward_multi_and_flags_last_single_line() {
        // Oracle-verified: `{multi: 1, single: 1}` wins `:multi` and the
        // repeated group reports its newest line (4), not its first.
        let files = vec![file(
            "defmodule M do\n  alias Foo.{A, B}\n  alias Bar.Baz\n  alias Bar.Qux\nend\n",
        )];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, Some(4));
        assert_eq!(issues[0].column, Some(2));
    }

    #[test]
    fn tie_break_two_files_toward_multi_blames_single_file() {
        // Key space is `multi`/`single`; smallest key `multi` wins a 1-1 tie,
        // so the file holding the non-smallest `single` vote (file 1) is blamed.
        let files = vec![
            file("defmodule M do\n  alias Foo.{A, B}\nend\n"),
            file("defmodule N do\n  alias Bar.Baz\n  alias Bar.Qux\nend\n"),
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 1);
        assert_eq!(issues[0].line, Some(3));
        assert_eq!(issues[0].column, Some(2));
        assert_eq!(
            issues[0].message,
            "Most of the time you are using the multi-alias/require/import/use syntax, but here you are using multiple single directives"
        );
    }

    #[test]
    fn single_segment_and_option_forms_never_vote() {
        let files = vec![file(
            "defmodule M do\n  import Assertions\n  use Foo\n  alias Bar.Baz, as: Qux\nend\n",
        )];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[derive(serde::Deserialize)]
    struct CorpusEntry {
        id: String,
        group: Option<String>,
        params: BTreeMap<String, serde_json::Value>,
        source: String,
        findings: Vec<CorpusFinding>,
        excluded_reason: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct CorpusFinding {
        line: Option<usize>,
        column: Option<usize>,
        message: String,
        trigger: String,
    }

    fn corpus_value_text(value: &serde_json::Value) -> Option<String> {
        if let Some(text) = value.as_str() {
            return Some(text.to_owned());
        }
        if let Some(number) = value.as_i64() {
            return Some(number.to_string());
        }
        if let Some(number) = value.as_u64() {
            return Some(number.to_string());
        }
        value.as_bool().map(|flag| flag.to_string())
    }

    fn corpus_params(entry: &CorpusEntry) -> BTreeMap<String, String> {
        entry
            .params
            .iter()
            .filter_map(|(key, value)| corpus_value_text(value).map(|text| (key.clone(), text)))
            .collect()
    }

    fn finding_key(finding: &CorpusFinding) -> (Option<usize>, Option<usize>, String, String) {
        (
            finding.line,
            finding.column,
            finding.trigger.clone(),
            finding.message.clone(),
        )
    }

    fn issue_key(issue: &ProjectIssue) -> (Option<usize>, Option<usize>, String, String) {
        (
            issue.line,
            issue.column,
            issue.trigger.clone(),
            issue.message.clone(),
        )
    }

    fn group_keys(entries: &[CorpusEntry]) -> Vec<String> {
        let mut order = Vec::new();
        for entry in entries {
            if entry.excluded_reason.is_some() {
                continue;
            }
            let key = entry.group.clone().unwrap_or_else(|| entry.id.clone());
            if !order.contains(&key) {
                order.push(key);
            }
        }
        order
    }

    fn check_group(entries: &[CorpusEntry], key: &str, mismatches: &mut Vec<String>) {
        let group: Vec<(usize, &CorpusEntry)> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.excluded_reason.is_none()
                    && entry.group.clone().unwrap_or_else(|| entry.id.clone()) == key
            })
            .collect();
        let files: Vec<ProjectFile> = group.iter().map(|(_, entry)| file(&entry.source)).collect();
        let params = corpus_params(group[0].1);
        for (_, entry) in &group {
            assert!(
                corpus_params(entry) == params,
                "non-uniform params in group {key}"
            );
        }
        let issues = run(&files, &params);
        for (position, (_, entry)) in group.iter().enumerate() {
            let mut got: Vec<_> = issues
                .iter()
                .filter(|issue| issue.file == position)
                .map(issue_key)
                .collect();
            got.sort();
            let mut want: Vec<_> = entry.findings.iter().map(finding_key).collect();
            want.sort();
            if got != want {
                mismatches.push(format!("{} :: got {got:?} want {want:?}", entry.id));
            }
        }
    }

    fn corpus_mismatches() -> Vec<String> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/compatibility/cases/EX1003.json"
        );
        let raw = std::fs::read_to_string(path).expect("corpus file loads");
        let entries: Vec<CorpusEntry> = serde_json::from_str(&raw).expect("corpus parses");
        let mut mismatches = Vec::new();
        for key in group_keys(&entries) {
            check_group(&entries, &key, &mut mismatches);
        }
        mismatches
    }
}
