//! Project-level `ParameterPatternMatching` consistency (EX1004).
//!
//! Per-file `def`/`defp` parameter votes merge across the project: `%{...} =
//! var` (before) against `var = %{...}` (after) per top-level `=` parameter.
//! The majority wins (an explicit `force` param overrides) and files holding
//! the other style get one issue per non-matching parameter. Columns are
//! backfilled from the trigger with the same boundary-sensitive search Credo
//! applies when an issue carries a trigger but no column.

use std::collections::BTreeMap;

use super::{ProjectFile, ProjectIssue, majority};
use crate::helpers;

/// Run the check over a file set.
pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_file: Vec<Vec<ParamMatch>> = Vec::new();
    for file in files {
        let found = collect(&file.source);
        for found_match in &found {
            *counts.entry(found_match.kind.clone()).or_insert(0) += 1;
        }
        per_file.push(found);
    }
    if counts.is_empty() {
        return Vec::new();
    }
    let force = helpers::param_str(params, "force", "");
    let force = if force.is_empty() { None } else { Some(force) };
    let Some(expected) = majority(&counts, force) else {
        return Vec::new();
    };
    let message = issue_message(&expected);
    let mut issues = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let unexpected = per_file[index].iter().any(|found| found.kind != expected);
        if !unexpected {
            continue;
        }
        let lines: Vec<&str> = file.source.split('\n').collect();
        for found in &per_file[index] {
            if found.kind == expected {
                continue;
            }
            let line = lines.get(found.line.wrapping_sub(1)).unwrap_or(&"");
            issues.push(ProjectIssue {
                file: index,
                line: Some(found.line),
                column: backfilled_column(line, &found.name),
                trigger: found.name.clone(),
                message: message.clone(),
                severity: None,
            });
        }
    }
    issues
}

/// One top-level `=` parameter: its style, variable line and capture name.
struct ParamMatch {
    kind: String,
    line: usize,
    name: String,
}

/// Per-file matches in document order over the masked source.
fn collect(source: &str) -> Vec<ParamMatch> {
    let masked = helpers::mask_strings_comments(source);
    let bytes = masked.as_bytes();
    let starts = line_starts(&masked);
    let mut found = Vec::new();
    let mut i = 0_usize;
    while i < bytes.len() {
        let Some(after_keyword) = def_keyword(bytes, i) else {
            i += 1;
            continue;
        };
        let Some((inner, _)) = head_parens(bytes, after_keyword) else {
            i += 1;
            continue;
        };
        for (kind, name_offset, name_len) in split_params(bytes, inner) {
            let name: String = masked[name_offset..name_offset + name_len].to_owned();
            found.push(ParamMatch {
                kind,
                line: line_of(&starts, name_offset),
                name,
            });
        }
        i = inner.1 + 1;
    }
    found
}

/// Message for the winning style; the actual style is its inverse.
fn issue_message(expected: &str) -> String {
    if expected == "before" {
        "File has the variable name after the pattern while most of the files have the variable name before the pattern when naming parameter pattern matches"
            .to_owned()
    } else {
        "File has the variable name before the pattern while most of the files have the variable name after the pattern when naming parameter pattern matches"
            .to_owned()
    }
}

/// Column backfill mirroring `Credo.SourceFile.column/3`: the first trigger
/// occurrence in the boundary-sensitive search, byte offset plus one.
fn backfilled_column(line: &str, trigger: &str) -> Option<usize> {
    let pattern = format!(
        r"(\s|\b|\(|\)|,)({})(\s|\b|\(|\)|,)",
        regex::escape(trigger)
    );
    let expression = regex::Regex::new(&pattern).ok()?;
    expression.captures(line)?.get(2).map(|hit| hit.start() + 1)
}

/// Byte offsets starting each `\n`-split line.
fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    for (offset, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(offset + 1);
        }
    }
    starts
}

/// 1-based line containing `offset` via the start table.
fn line_of(starts: &[usize], offset: usize) -> usize {
    match starts.binary_search(&offset) {
        Ok(line) => line + 1,
        Err(next) => next,
    }
}

/// Offset just past `def`/`defp` at `i`, requiring keyword boundaries.
/// `defmacro` and friends never match: the keyword must end at whitespace.
fn def_keyword(bytes: &[u8], i: usize) -> Option<usize> {
    if i > 0 && is_name_byte(bytes[i - 1]) {
        return None;
    }
    for keyword in ["defp", "def"] {
        let after = i + keyword.len();
        if bytes
            .get(i..after)
            .is_some_and(|head| head == keyword.as_bytes())
            && bytes.get(after).is_some_and(u8::is_ascii_whitespace)
        {
            return Some(after);
        }
    }
    None
}

/// Inner span of the `def` head parens plus the closing offset.
fn head_parens(bytes: &[u8], from: usize) -> Option<((usize, usize), usize)> {
    let after_name = scan_name(bytes, skip_ws(bytes, from))?;
    let open = skip_ws(bytes, after_name);
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let close = match_paren(bytes, open)?;
    Some(((open + 1, close), close))
}

/// Offset past ASCII whitespace starting at `i`.
fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

/// Offset past a function name starting at `i`, if any.
fn scan_name(bytes: &[u8], i: usize) -> Option<usize> {
    let first = *bytes.get(i)?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut end = i + 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'?' | b'!'))
    {
        end += 1;
    }
    Some(end)
}

/// Offset of the paren closing `open`, tracking every bracket kind.
fn match_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut stack = vec![b'('];
    let mut i = open + 1;
    while let Some(byte) = bytes.get(i) {
        match byte {
            b'(' | b'[' | b'{' => stack.push(*byte),
            b')' | b']' | b'}' => {
                let opener = stack.pop()?;
                if !pair_matches(opener, *byte) {
                    return None;
                }
                if stack.is_empty() {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Matching bracket pairs for the head scan.
fn pair_matches(opener: u8, closer: u8) -> bool {
    matches!((opener, closer), (b'(', b')') | (b'[', b']') | (b'{', b'}'))
}

/// Classify each top-level comma-separated parameter of one head.
fn split_params(bytes: &[u8], inner: (usize, usize)) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut start = inner.0;
    let mut i = inner.0;
    while i < inner.1 {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                classify_param(bytes, start, i, &mut out);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    classify_param(bytes, start, inner.1, &mut out);
    out
}

/// Record the style when the parameter holds one top-level `=`.
fn classify_param(bytes: &[u8], start: usize, end: usize, out: &mut Vec<(String, usize, usize)>) {
    let Some(eq) = top_equals(bytes, start, end) else {
        return;
    };
    let (left_start, left_end) = trim_ws(bytes, start, eq);
    let (right_start, right_end) = trim_ws(bytes, eq + 1, end);
    if is_bare_var(&bytes[left_start..left_end]) {
        out.push(("before".to_owned(), left_start, left_end - left_start));
    } else if is_bare_var(&bytes[right_start..right_end]) {
        out.push(("after".to_owned(), right_start, right_end - right_start));
    }
}

/// First top-level `=` that is a match instead of `==`, `=>`, `<=`, `!=`.
fn top_equals(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    let mut depth = 0_usize;
    let mut i = start;
    while i < end {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'=' if depth == 0 && is_match_op(bytes, start, end, i) => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Bare `=` with no operator neighbors (`==`, `=>`, `<=`, `>=`, `!=`, `=~`).
fn is_match_op(bytes: &[u8], start: usize, end: usize, i: usize) -> bool {
    let prev_ok = i == start
        || bytes
            .get(i - 1)
            .is_none_or(|byte| !matches!(byte, b'=' | b'<' | b'>' | b'!' | b'~'));
    let next_ok = i + 1 >= end
        || bytes
            .get(i + 1)
            .is_none_or(|byte| !matches!(byte, b'=' | b'>' | b'~'));
    prev_ok && next_ok
}

/// Shrink `[start, end)` past ASCII whitespace.
fn trim_ws(bytes: &[u8], start: usize, end: usize) -> (usize, usize) {
    let mut left = start;
    while left < end && bytes[left].is_ascii_whitespace() {
        left += 1;
    }
    let mut right = end;
    while right > left && bytes[right - 1].is_ascii_whitespace() {
        right -= 1;
    }
    (left, right)
}

/// Bare capture variable with an optional trailing `?`/`!`.
fn is_bare_var(bytes: &[u8]) -> bool {
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first == b'_') {
        return false;
    }
    let mut core = rest;
    if let Some((&last, init)) = rest.split_last()
        && (last == b'?' || last == b'!')
    {
        core = init;
    }
    core.iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

/// Byte continuing an Elixir identifier (ASCII subset).
fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'?' | b'!')
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../../compatibility/cases/EX1004.json");

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
    fn minority_reports_are_deterministic_across_runs() {
        // The collect phase runs on a pool; repeated runs must agree
        // exactly (file order, issue order, counts).
        let files = vec![
            ProjectFile {
                filename: "a.ex".to_owned(),
                source: "defmodule M do\n  def f([a, b] = list), do: :ok\nend\n".to_owned(),
            },
            ProjectFile {
                filename: "b.ex".to_owned(),
                source: "defmodule N do\n  def g(list = [a, b]), do: :ok\nend\n".to_owned(),
            },
        ];
        let first = run(&files, &BTreeMap::new());
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].file, 1);
        for _ in 0..4 {
            assert_eq!(run(&files, &BTreeMap::new()), first);
        }
    }

    #[test]
    fn tie_breaks_toward_after_and_blames_before_file() {
        // Key space is `before` (`var = pattern`) / `after` (`pattern = var`);
        // smallest key `after` wins a 1-1 tie, so the file holding the
        // non-smallest `before` vote (file 0) is blamed regardless of order.
        let files = vec![
            ProjectFile {
                filename: "a.ex".to_owned(),
                source: "defmodule M do\n  def f(list = [a, b]), do: :ok\nend\n".to_owned(),
            },
            ProjectFile {
                filename: "b.ex".to_owned(),
                source: "defmodule N do\n  def g([a, b] = list), do: :ok\nend\n".to_owned(),
            },
        ];
        let issues = run(&files, &BTreeMap::new());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, 0);
        assert_eq!(issues[0].trigger, "list");
        assert_eq!(
            issues[0].message,
            "File has the variable name before the pattern while most of the files have the variable name after the pattern when naming parameter pattern matches"
        );
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
    fn oracle_pinned_single_file_votes() {
        // Guards the refute entries against vacuous passes from under-counting.
        let cases: [(&str, &[(&str, usize)]); 4] = [
            (
                "EX1004.upstream.mixed-left-right",
                &[("after", 3), ("before", 2)],
            ),
            ("EX1004.upstream.inconsistent-files.file1", &[("after", 1)]),
            ("EX1004.upstream.inconsistent-files.file5", &[("before", 1)]),
            ("EX1004.upstream.no-bindings", &[]),
        ];
        for (id, expected) in cases {
            let expected: BTreeMap<String, usize> = expected
                .iter()
                .map(|(kind, count)| ((*kind).to_owned(), *count))
                .collect();
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for found in collect(&source_of(id)) {
                *counts.entry(found.kind).or_insert(0) += 1;
            }
            assert_eq!(counts, expected, "counts for {id}");
        }
    }

    #[test]
    fn multibyte_literals_do_not_shift_lines() {
        let source = "defmodule M do\n  @doc \"μ def (a = b) μ\"\n  def test(foo = %{a: b}) do\n    nil\n  end\nend\n";
        let found = collect(source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].name, "foo");
    }
}
