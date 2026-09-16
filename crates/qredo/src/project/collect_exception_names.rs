//! Project-level `ExceptionNames` consistency (EX1001).
//!
//! Every `defmodule` whose body directly calls `defexception` votes once for
//! the prefix and once for the suffix of its last name segment, split on
//! `PascalCase` boundaries (`Credo.Code.Name.split_pascal_case/1`). Votes merge
//! across the project; the majority over the combined key space wins (ties
//! toward the smallest key) and exceptions losing on that dimension get one
//! issue at their `defmodule` line. A winning count of one suppresses every
//! issue: the check passes `supress_issues_for_single_match? = true`
//! upstream. Neither the check nor its inventory entry declares parameters,
//! so `params` is ignored. Column backfill mirrors
//! `Credo.SourceFile.column/3`: the first trigger occurrence flanked by
//! whitespace, word boundaries or parens/commas, 1-based.

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
/// parse and one walk per file instead of matching a query per file.
pub(crate) fn run_with_facts(
    files: &[ProjectFile],
    facts: &[&crate::facts::Facts],
    _params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    let (counts, per_file) = collect_all(files, facts);
    if counts.is_empty() {
        return Vec::new();
    }
    let Some(expected) = majority(&counts, None) else {
        return Vec::new();
    };
    // The check passes `supress_issues_for_single_match? = true`: a winning
    // count of one means every vote is unique, so nothing is reported.
    if counts.get(&expected).is_none_or(|count| *count <= 1) {
        return Vec::new();
    }
    let mut issues = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let flagged = per_file[index]
            .iter()
            .flat_map(exception_keys)
            .any(|key| key != expected);
        if !flagged {
            continue;
        }
        for exception in &per_file[index] {
            if loses(exception, &expected) {
                issues.push(issue_for(index, &file.source, exception, &expected));
            }
        }
    }
    issues
}

/// Per-file exception inventories with merged vote counts, in file
/// order. Fact scans are cheap slice work, so collection stays
/// sequential; merging stays sequential too.
fn collect_all(
    files: &[ProjectFile],
    facts: &[&crate::facts::Facts],
) -> (BTreeMap<String, usize>, Vec<Vec<Exception>>) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_file: Vec<Vec<Exception>> = (0..files.len()).map(|_| Vec::new()).collect();
    for (position, file) in files.iter().enumerate() {
        let found = match facts.get(position) {
            Some(facts) => collect_from_facts(&file.source, facts),
            None => Vec::new(),
        };
        for exception in &found {
            *counts.entry(prefix_key(&exception.prefix)).or_insert(0) += 1;
            *counts.entry(suffix_key(&exception.suffix)).or_insert(0) += 1;
        }
        per_file[position] = found;
    }
    (counts, per_file)
}

/// Exceptions of one source from shared facts: every `defmodule` with an
/// alias head whose `do`-block body directly calls `defexception`. The
/// error gate applies like the old query path.
fn collect_from_facts(source: &str, facts: &crate::facts::Facts) -> Vec<Exception> {
    if facts.has_error {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (index, module) in facts.modules.iter().enumerate() {
        let Some((alias_start, alias_end)) = module.alias else {
            continue;
        };
        let Some(name) = source.get(alias_start as usize..alias_end as usize) else {
            continue;
        };
        let direct = facts
            .module_bodies
            .iter()
            .filter(|body| body.from_block && body.module as usize == index)
            .flat_map(|body| body.stmts.iter())
            .any(|stmt| {
                if let crate::facts::BodyStmtFact::Call {
                    head_start,
                    head_end,
                    ..
                } = stmt
                {
                    source.get(*head_start as usize..*head_end as usize) == Some("defexception")
                } else {
                    false
                }
            });
        if !direct {
            continue;
        }
        let short = name.rsplit('.').next().unwrap_or(name);
        let (prefix, suffix) = prefix_suffix(short);
        out.push(Exception {
            name: name.to_owned(),
            prefix,
            suffix,
            line: module.line as usize,
        });
    }
    out
}

/// One exception module: its written name, `PascalCase` parts and defmodule line.
struct Exception {
    name: String,
    prefix: String,
    suffix: String,
    line: usize,
}

/// Vote keys of one exception across the combined key space.
fn exception_keys(exception: &Exception) -> [String; 2] {
    [prefix_key(&exception.prefix), suffix_key(&exception.suffix)]
}

/// True when the exception loses on the winning dimension.
fn loses(exception: &Exception, expected: &str) -> bool {
    let (is_prefix, want) = expected_part(expected);
    if is_prefix {
        exception.prefix != want
    } else {
        exception.suffix != want
    }
}

/// One issue at the exception's `defmodule` line with backfilled column.
fn issue_for(file: usize, source: &str, exception: &Exception, expected: &str) -> ProjectIssue {
    let (is_prefix, want) = expected_part(expected);
    let line_text = source
        .split('\n')
        .nth(exception.line.saturating_sub(1))
        .unwrap_or("");
    ProjectIssue {
        file,
        line: Some(exception.line),
        column: trigger_column(line_text, &exception.name),
        trigger: exception.name.clone(),
        message: message_for(is_prefix, want, &exception.name),
        severity: None,
    }
}

/// Split a merged winner back into its dimension and expected part.
fn expected_part(expected: &str) -> (bool, &str) {
    if let Some(want) = expected.strip_prefix("prefix:") {
        (true, want)
    } else {
        (false, expected.strip_prefix("suffix:").unwrap_or(expected))
    }
}

/// Message for the winning dimension, mirroring the check module.
fn message_for(is_prefix: bool, want: &str, trigger: &str) -> String {
    if is_prefix {
        format!(
            "Exception modules should be named consistently. It seems your strategy is to prefix them with `{want}`, but `{trigger}` does not follow that convention."
        )
    } else {
        format!(
            "Exception modules should be named consistently. It seems your strategy is to have `{want}` as a suffix, but `{trigger}` does not follow that convention."
        )
    }
}

/// Prefix vote key; every prefix sorts before every suffix, as upstream.
fn prefix_key(prefix: &str) -> String {
    format!("prefix:{prefix}")
}

/// Suffix vote key.
fn suffix_key(suffix: &str) -> String {
    format!("suffix:{suffix}")
}

/// Prefix and suffix of a name's last segment on `PascalCase` boundaries:
/// a space before every ASCII uppercase letter, then first/last words
/// (mirrors `String.replace(~r/([A-Z])/, " \\1") |> String.split()`).
fn prefix_suffix(last: &str) -> (String, String) {
    let mut spaced = String::with_capacity(last.len() + 2);
    for ch in last.chars() {
        if ch.is_ascii_uppercase() {
            spaced.push(' ');
        }
        spaced.push(ch);
    }
    let mut parts = spaced.split_whitespace();
    let first = parts.next().unwrap_or(last);
    let last_part = parts.last().unwrap_or(first);
    (first.to_owned(), last_part.to_owned())
}

/// Column of `trigger` in `line`, mirroring `Credo.SourceFile.column/3`:
/// the first occurrence flanked by whitespace, word boundaries or
/// parens/commas, 1-based. Byte-based like the Elixir original; the match
/// start of a literal is always a character boundary.
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
    fn consistent_suffix_project_is_clean() {
        let files = vec![
            file(
                "defmodule Credo.Sample do\n  defmodule UriParserError do\n    defexception [:message]\n  end\nend\n",
            ),
            file("defmodule SomeOtherError do\n  defexception [:message]\nend\n"),
        ];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn unique_names_are_suppressed() {
        let files = vec![
            file(
                "defmodule Credo.Sample do\n  defmodule SomeError do\n    defexception [:message]\n  end\nend\n",
            ),
            file("defmodule UndefinedResponse do\n  defexception [:message]\nend\n"),
        ];
        assert!(run(&files, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn suffix_minority_reports_defmodule_line() {
        let files = vec![
            file(
                "defmodule Credo.Sample do\n  defmodule SomeException do\n    defexception [:message]\n  end\n  defmodule UndefinedResponse do    # <--- does not have the suffix \"Exception\"\n    defexception [:message]\n  end\nend\n",
            ),
            file("defmodule InputValidationException do\n  defexception [:message]\nend\n"),
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 0);
        assert_eq!(issues[0].line, Some(5));
        assert_eq!(issues[0].column, Some(13));
        assert_eq!(issues[0].trigger, "UndefinedResponse");
        assert_eq!(
            issues[0].message,
            "Exception modules should be named consistently. It seems your strategy is to have `Exception` as a suffix, but `UndefinedResponse` does not follow that convention."
        );
    }

    #[test]
    fn shared_facts_match_fresh_parses() {
        // The pipeline shares prepare-phase facts instead of re-matching;
        // both paths must report identically.
        let sources = [
            "defmodule Credo.Sample do\n  defmodule SomeException do\n    defexception [:message]\n  end\n  defmodule UndefinedResponse do\n    defexception [:message]\n  end\nend\n",
            "defmodule InputValidationException do\n  defexception [:message]\nend\n",
        ];
        let files: Vec<ProjectFile> = sources
            .iter()
            .map(|source| ProjectFile {
                filename: "case.ex".to_owned(),
                source: (*source).to_owned(),
            })
            .collect();
        let expected = run(&files, &BTreeMap::new());
        assert_eq!(expected.len(), 1);
        let prepared: Vec<crate::batch::Prepared<'_>> = sources
            .iter()
            .map(|source| crate::batch::Prepared::eager(source))
            .collect();
        let facts: Vec<&crate::facts::Facts> =
            prepared.iter().map(crate::batch::Prepared::facts).collect();
        assert_eq!(run_with_facts(&files, &facts, &BTreeMap::new()), expected);
    }

    #[test]
    fn minority_reports_are_deterministic_across_runs() {
        // Collection is sequential; repeated runs must agree
        // exactly (file order, issue order, counts).
        let files = vec![
            file(
                "defmodule Credo.Sample do\n  defmodule SomeException do\n    defexception [:message]\n  end\n  defmodule UndefinedResponse do\n    defexception [:message]\n  end\nend\n",
            ),
            file("defmodule InputValidationException do\n  defexception [:message]\nend\n"),
            file("defmodule OtherException do\n  defexception [:message]\nend\n"),
        ];
        let first = run(&files, &BTreeMap::new());
        assert_eq!(first.len(), 1);
        for _ in 0..4 {
            assert_eq!(run(&files, &BTreeMap::new()), first);
        }
    }

    #[test]
    fn prefix_minority_reports_first_line() {
        let files = vec![
            file(
                "defmodule Credo.Sample do\n  defmodule InvalidDataRequest do\n    defexception [:message]\n  end\nend\n",
            ),
            file("defmodule InvalidReponseFromServer do\n  defexception [:message]\nend\n"),
            file(
                "defmodule UndefinedDataFormat do    # <--- does not have the prefix \"Invalid\"\n  defexception [:message]\nend\n",
            ),
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 2);
        assert_eq!(issues[0].line, Some(1));
        assert_eq!(issues[0].column, Some(11));
        assert_eq!(issues[0].trigger, "UndefinedDataFormat");
        assert_eq!(
            issues[0].message,
            "Exception modules should be named consistently. It seems your strategy is to prefix them with `Invalid`, but `UndefinedDataFormat` does not follow that convention."
        );
    }

    #[test]
    fn tie_breaks_toward_smallest_prefix_and_blames_other_file() {
        // Key space is combined `prefix:<first>` + `suffix:<last>` with every
        // `prefix:*` sorting before every `suffix:*`. Two `Alpha*` votes tie
        // two `Beta*` votes at 2 (all suffixes stay at 1), so the smallest
        // key `prefix:Alpha` wins and the non-smallest side (file 1) is blamed.
        let files = vec![
            file(
                "defmodule AlphaOne do\n  defexception [:message]\nend\ndefmodule AlphaTwo do\n  defexception [:message]\nend\n",
            ),
            file(
                "defmodule BetaThree do\n  defexception [:message]\nend\ndefmodule BetaFour do\n  defexception [:message]\nend\n",
            ),
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 2);
        for issue in &issues {
            assert_eq!(issue.file, 1);
            assert!(
                issue.message.contains("prefix them with `Alpha`"),
                "unexpected message: {}",
                issue.message
            );
        }
        let mut triggers: Vec<&str> = issues.iter().map(|issue| issue.trigger.as_str()).collect();
        triggers.sort_unstable();
        assert_eq!(triggers, vec!["BetaFour", "BetaThree"]);
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
    fn pascal_case_splits_first_and_last_words() {
        assert_eq!(
            prefix_suffix("SampleError"),
            ("Sample".to_owned(), "Error".to_owned())
        );
        assert_eq!(
            prefix_suffix("InvalidReponseFromServer"),
            ("Invalid".to_owned(), "Server".to_owned())
        );
        assert_eq!(prefix_suffix("A"), ("A".to_owned(), "A".to_owned()));
    }

    #[test]
    fn trigger_column_matches_source_file_backfill() {
        assert_eq!(
            trigger_column("  defmodule UndefinedResponse do", "UndefinedResponse"),
            Some(13)
        );
        assert_eq!(trigger_column("defmodule M do", "Other"), None);
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
            "/compatibility/cases/EX1001.json"
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
