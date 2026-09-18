//! Upstream-exact file selection for checks.
//!
//! Mirrors `Credo.Sources` discovery filtering and `Check.Runner` per-check
//! filtering with `Path.wildcard/2` (Erlang `filelib`) semantics, verified
//! against native probes (Elixir 1.20.2, pin `ea1ccb9`, 2026-09-15; every
//! case below was observed, including dotfile/brace/class edges):
//!
//! - Patterns without wildcard characters match by equality, or as a
//!   directory prefix for non-`.ex`/`.exs` patterns (`"test/"`, `"test"`).
//! - `*` spans within one segment, `?` one character, `**` zero or more
//!   whole segments, `{a,b}` expands textually (nested braces raise
//!   upstream, so they error here too).
//! - Segments containing wildcard characters never match dotfiles, even
//!   explicit ones (`.*.ex`, `[.]x`); literal segments do (`.hdir/*.ex`).
//! - `[...]` classes take ranges and literal `-`/`]`/`!`/`^` (no negation);
//!   unclosed `[` is literal. Runs of `*` collapse unless the segment is
//!   exactly `**`; backslashes are literal.
//! - Matching is byte-exact (upstream depends on the filesystem for case).
//!
//! Check-level `files` defaults come from each check's `param_defaults`
//! (`EX2003`, `EX5025`, `EX5030`, `EX4031`; everything else runs everywhere
//! unless configured). Unlike the unit-test gates (which invoke `run`
//! directly), real executions filter files per check first.

use std::collections::BTreeMap;

use crate::config_file::FileEntry;
use crate::pipeline::GeneralParams;

/// A file-pattern problem naming the offending pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternError(pub String);

/// True when `path` is selected by `pattern` (both project-relative, or
/// both already expanded; byte-exact).
///
/// # Errors
/// Returns [`PatternError`] for nested braces or uncompilable classes,
/// mirroring upstream raises.
pub fn wildcard_match(pattern: &str, path: &str) -> Result<bool, PatternError> {
    CompiledGlob::compile(pattern).is_match(path)
}

/// Textual brace expansion; nested braces raise upstream, so they error here.
fn expand_braces(pattern: &str) -> Result<Vec<String>, PatternError> {
    let Some(open) = pattern.find('{') else {
        return Ok(vec![pattern.to_owned()]);
    };
    let bytes = pattern.as_bytes();
    let mut depth = 0_usize;
    let mut close = None;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => {
                if index > open {
                    return Err(PatternError("nested braces in file pattern".to_owned()));
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return Err(PatternError("unbalanced braces in file pattern".to_owned()));
    };
    let mut out = Vec::new();
    for alternative in pattern[open + 1..close].split(',') {
        let expanded = format!(
            "{}{}{}",
            &pattern[..open],
            alternative,
            &pattern[close + 1..]
        );
        out.extend(expand_braces(&expanded)?);
    }
    Ok(out)
}

/// One precompiled glob alternative.
#[derive(Debug, Clone)]
enum CompiledAlternative {
    /// Literal directory prefix (`"lib/"`, bare `"test"`): exact-or-prefix
    /// segment equality, never wildcards.
    DirPrefix(Vec<String>),
    /// Segment matcher with `**` recursion.
    Segments(Vec<CompiledSegment>),
}

/// A glob pattern compiled once and matched many times. Brace errors are
/// retained and surfaced only when a filename is actually matched, just as
/// `wildcard_match` did before compilation was cached.
///
/// Backs hot per-(check, file) selection: brace expansion and segment
/// regexes build once instead of per file. Compile errors retain
/// `wildcard_match`'s lazy evaluation order.
#[derive(Debug, Clone)]
struct CompiledGlob {
    alternatives: Result<Vec<CompiledAlternative>, PatternError>,
}

/// One precompiled glob segment.
#[derive(Debug, Clone)]
enum CompiledSegment {
    /// `**`: zero or more non-dot segments.
    Recursive,
    /// No wildcard characters: plain equality (dotfiles match).
    Literal(String),
    /// Wildcard segment: anchored regex (dotfiles never match).
    Pattern(Result<regex::Regex, PatternError>),
}

impl CompiledGlob {
    /// Compile one glob, retaining errors for lazy evaluation.
    fn compile(pattern: &str) -> Self {
        let alternatives = expand_braces(pattern).map(|expanded| {
            expanded
                .iter()
                .map(|alternative| compile_alternative(alternative))
                .collect()
        });
        Self { alternatives }
    }

    /// True when `path` matches.
    ///
    /// # Errors
    /// Returns a retained pattern error if evaluation reaches it.
    fn is_match(&self, path: &str) -> Result<bool, PatternError> {
        let segments: Vec<&str> = path.split('/').collect();
        let alternatives = self.alternatives.as_ref().map_err(Clone::clone)?;
        for alternative in alternatives {
            let matches = match alternative {
                CompiledAlternative::DirPrefix(prefix) => {
                    !prefix.is_empty()
                        && segments.len() >= prefix.len()
                        && segments[..prefix.len()]
                            .iter()
                            .zip(prefix.iter())
                            .all(|(segment, expected)| *segment == expected)
                }
                CompiledAlternative::Segments(compiled) => match_compiled(compiled, &segments)?,
            };
            if matches {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Compile one brace-expanded alternative, mirroring the `match_segments`
/// entry checks (trailing-slash and bare-directory forms stay literal
/// exact-or-prefix matches).
fn compile_alternative(alternative: &str) -> CompiledAlternative {
    let mut parts: Vec<&str> = alternative.split('/').collect();
    let mut dir_prefix = false;
    while parts.last() == Some(&"") {
        dir_prefix = true;
        parts.pop();
    }
    if dir_prefix {
        return CompiledAlternative::DirPrefix(parts.iter().map(ToString::to_string).collect());
    }
    // Bare directory patterns (`"test"`, like trailing-slash `"test/"`)
    // match the directory itself and everything below it; bare `.ex`/`.exs`
    // file patterns match exactly (mirrors `recurse_path`/`File.dir?`).
    // Upstream compares extensions case-sensitively.
    #[allow(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "mirrors upstream String.ends_with?([\".ex\", \".exs\"])"
    )]
    if !parts.iter().any(|part| part.contains(['*', '?', '[', '{']))
        && !parts
            .last()
            .is_some_and(|last| last.ends_with(".ex") || last.ends_with(".exs"))
    {
        return CompiledAlternative::DirPrefix(parts.iter().map(ToString::to_string).collect());
    }
    let mut compiled = Vec::new();
    for part in parts {
        if part == "**" {
            compiled.push(CompiledSegment::Recursive);
        } else if !part.contains(['*', '?', '[']) {
            compiled.push(CompiledSegment::Literal(part.to_owned()));
        } else {
            compiled.push(CompiledSegment::Pattern(segment_regex(part)));
        }
    }
    CompiledAlternative::Segments(compiled)
}

/// Recursive segment match with `**` spanning zero or more non-dot segments.
fn match_compiled(patterns: &[CompiledSegment], segments: &[&str]) -> Result<bool, PatternError> {
    let Some((head, rest)) = patterns.split_first() else {
        return Ok(segments.is_empty());
    };
    if matches!(head, CompiledSegment::Recursive) {
        for skip in 0..=segments.len() {
            if segments[..skip]
                .iter()
                .any(|segment| segment.starts_with('.'))
            {
                continue;
            }
            if match_compiled(rest, &segments[skip..])? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    let Some((segment, remaining)) = segments.split_first() else {
        return Ok(false);
    };
    let matched = match head {
        CompiledSegment::Literal(expected) => expected == segment,
        CompiledSegment::Pattern(_) if segment.starts_with('.') => false,
        CompiledSegment::Pattern(expression) => {
            expression.as_ref().map_err(Clone::clone)?.is_match(segment)
        }
        CompiledSegment::Recursive => false,
    };
    if !matched {
        return Ok(false);
    }
    match_compiled(rest, remaining)
}

/// Anchored regex source for one wildcard segment: runs of `*` collapse
/// (verified `a**b` ≡ `a*b`); a full `**` segment never reaches here.
fn segment_regex_source(pattern: &str) -> String {
    let mut regex = String::from("^");
    let bytes = pattern.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        match bytes[index] {
            b'*' => {
                while bytes.get(index) == Some(&b'*') {
                    index += 1;
                }
                regex.push_str("[^/]*");
            }
            b'?' => {
                regex.push_str("[^/]");
                index += 1;
            }
            b'[' => {
                index = push_class(pattern, index, &mut regex);
            }
            _ => {
                let char = pattern[index..].chars().next().unwrap_or_default();
                push_escaped(&mut regex, char);
                index += char.len_utf8();
            }
        }
    }
    regex.push('$');
    regex
}

/// Compiled anchored regex for one wildcard segment.
fn segment_regex(pattern: &str) -> Result<regex::Regex, PatternError> {
    let source = segment_regex_source(pattern);
    regex::Regex::new(&source)
        .map_err(|error| PatternError(format!("invalid file pattern `{pattern}`: {error}")))
}

/// One precompiled config file entry.
#[derive(Debug, Clone)]
enum CompiledEntry {
    Glob(CompiledGlob),
    Regex(Result<regex::Regex, PatternError>),
}

impl CompiledEntry {
    /// Compile one entry, retaining errors for lazy evaluation.
    fn compile(entry: &FileEntry) -> Self {
        match entry {
            FileEntry::Glob(pattern) => Self::Glob(CompiledGlob::compile(pattern)),
            FileEntry::Regex(source) => {
                Self::Regex(regex::Regex::new(source).map_err(|error| {
                    PatternError(format!("invalid file regex `{source}`: {error}"))
                }))
            }
        }
    }

    /// Match one entry, surfacing a retained error only if reached.
    fn is_match(&self, filename: &str) -> Result<bool, PatternError> {
        match self {
            Self::Glob(glob) => glob.is_match(filename),
            Self::Regex(expression) => Ok(expression
                .as_ref()
                .map_err(Clone::clone)?
                .is_match(filename)),
        }
    }
}

/// Per-check file selection compiled once per run.
///
/// Hot lanes evaluate selection per file; compiling once removes per-file
/// brace expansion and regex builds. Error behavior mirrors
/// `check_runs_on_entries`: the first bad pattern in evaluation order wins,
/// and a bad `files.excluded` with empty `files.included` never surfaces
/// (the excluded list is unreachable then, exactly as before).
#[derive(Debug, Clone)]
pub(crate) struct CheckFileMatcher {
    general_included: Vec<CompiledEntry>,
    general_excluded: Vec<CompiledEntry>,
    check_included: Vec<CompiledGlob>,
    check_excluded: Vec<CompiledGlob>,
}

impl CheckFileMatcher {
    /// Compile selection for one check. Invalid patterns are retained so
    /// they surface in the same lazy evaluation order as the old matcher.
    #[must_use]
    pub(crate) fn compile(
        rule: &str,
        general_included: &[FileEntry],
        general_excluded: &[FileEntry],
        check_params: &BTreeMap<String, String>,
    ) -> Self {
        let compiled_general_included = general_included
            .iter()
            .map(CompiledEntry::compile)
            .collect();
        let compiled_general_excluded = general_excluded
            .iter()
            .map(CompiledEntry::compile)
            .collect();
        let (default_included, default_excluded) = default_check_files(rule);
        let included = check_params.get("files.included").map_or_else(
            || default_included.unwrap_or_default(),
            |value| split_globs(value),
        );
        let compiled_included = included
            .iter()
            .map(|pattern| CompiledGlob::compile(pattern))
            .collect();
        let excluded = check_params
            .get("files.excluded")
            .map_or(default_excluded, |value| split_globs(value));
        let compiled_excluded = excluded
            .iter()
            .map(|pattern| CompiledGlob::compile(pattern))
            .collect();
        Self {
            general_included: compiled_general_included,
            general_excluded: compiled_general_excluded,
            check_included: compiled_included,
            check_excluded: compiled_excluded,
        }
    }

    /// Match one file using cached patterns.
    ///
    /// # Errors
    /// Returns the first pattern error reached in upstream evaluation order.
    pub(crate) fn matches(&self, filename: &str) -> Result<bool, PatternError> {
        if !self.general_included.is_empty()
            && !any_entry_matches(&self.general_included, filename)?
        {
            return Ok(false);
        }
        if any_entry_matches(&self.general_excluded, filename)? {
            return Ok(false);
        }
        if self.check_included.is_empty() {
            return Ok(true);
        }
        for pattern in &self.check_included {
            if pattern.is_match(filename)? {
                for excluded in &self.check_excluded {
                    if excluded.is_match(filename)? {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// `Iterator::any` with short-circuiting error propagation.
fn any_entry_matches(entries: &[CompiledEntry], filename: &str) -> Result<bool, PatternError> {
    for entry in entries {
        if entry.is_match(filename)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Append a `[...]` class starting at `index` (which points at `[`);
/// returns the index past `]`. Unclosed `[` is literal (verified).
/// Ranges fold only when ordered; reversed ranges degrade to literals
/// (verified `[z-a]` matches exactly `z`, `-`, `a`). There is no negation:
/// leading `!`/`^` are literal (verified).
fn push_class(pattern: &str, index: usize, regex: &mut String) -> usize {
    let (items, cursor, closed) = read_class_body(pattern, index);
    if !closed {
        regex.push_str("\\[");
        return index + 1;
    }
    let mut out = String::from("[");
    let mut rest = items.as_slice();
    if let [first, tail @ ..] = rest
        && matches!(first, '!' | '^')
    {
        push_escaped(&mut out, *first);
        rest = tail;
    }
    while let [head, middle @ ..] = rest {
        if let (lo, ['-', hi, tail @ ..]) = (head, middle)
            && lo <= hi
        {
            push_escaped(&mut out, *lo);
            out.push('-');
            push_escaped(&mut out, *hi);
            rest = tail;
        } else {
            push_escaped(&mut out, *head);
            rest = middle;
        }
    }
    out.push(']');
    regex.push_str(&out);
    cursor
}

/// Literal class items plus end cursor; `closed` is false without `]`.
fn read_class_body(pattern: &str, index: usize) -> (Vec<char>, usize, bool) {
    let bytes = pattern.as_bytes();
    let mut items: Vec<char> = Vec::new();
    let mut cursor = index + 1;
    if bytes.get(cursor) == Some(&b']') {
        items.push(']');
        cursor += 1;
    }
    while cursor < bytes.len() {
        match bytes[cursor] {
            b']' => return (items, cursor + 1, true),
            b'\\' => {
                let Some(next) = pattern[cursor..].chars().nth(1) else {
                    break;
                };
                items.push(next);
                cursor += 1 + next.len_utf8();
            }
            _ => {
                let char = pattern[cursor..].chars().next().unwrap_or_default();
                items.push(char);
                cursor += char.len_utf8();
            }
        }
    }
    (items, cursor, false)
}

/// Append one literal char, escaping regex metacharacters.
fn push_escaped(out: &mut String, char: char) {
    if char.is_ascii_punctuation() && char != '_' {
        out.push('\\');
    }
    out.push(char);
}

/// Per-check `files` defaults: `(included, excluded)` globs, or `None`
/// included meaning every file. Sourced from each check's `param_defaults`
/// (`lib/credo/check/...` in the pinned checkout).
fn default_check_files(rule: &str) -> (Option<Vec<String>>, Vec<String>) {
    match rule {
        "Credo.Check.Design.SkipTestWithoutComment"
        | "Credo.Check.Refactor.PassAsyncInTestCases" => (
            Some(vec![
                "test/**/*_test.exs".to_owned(),
                "apps/**/test/**/*_test.exs".to_owned(),
            ]),
            Vec::new(),
        ),
        "Credo.Check.Warning.WrongTestFileExtension" => (
            Some(vec![
                "test/**/*_test.ex".to_owned(),
                "apps/**/test/**/*_test.ex".to_owned(),
            ]),
            Vec::new(),
        ),
        "Credo.Check.Warning.WrongTestFilename" => (
            Some(vec!["test/".to_owned()]),
            vec![
                "test/**/*_test.exs".to_owned(),
                "apps/**/test/**/*_test.exs".to_owned(),
            ],
        ),
        _ => (None, Vec::new()),
    }
}

/// Whether `filename` reaches `rule`'s `run`: config-level general selection
/// AND per-check files (defaults overridden by `files.included`/
/// `files.excluded` entry params) must both match.
///
/// # Errors
/// Returns [`PatternError`] for uncompilable patterns.
pub fn check_runs_on_file(
    rule: &str,
    filename: &str,
    general: &GeneralParams,
    check_params: &BTreeMap<String, String>,
) -> Result<bool, PatternError> {
    let included: Vec<FileEntry> = general
        .files_included
        .iter()
        .map(|pattern| FileEntry::Glob(pattern.clone()))
        .collect();
    let excluded: Vec<FileEntry> = general
        .files_excluded
        .iter()
        .map(|pattern| FileEntry::Glob(pattern.clone()))
        .collect();
    check_runs_on_entries(rule, filename, &included, &excluded, check_params)
}

/// Whether `filename` reaches `rule`'s `run` under config file entries.
///
/// # Errors
/// Returns [`PatternError`] for uncompilable patterns.
pub fn check_runs_on_entries(
    rule: &str,
    filename: &str,
    general_included: &[FileEntry],
    general_excluded: &[FileEntry],
    check_params: &BTreeMap<String, String>,
) -> Result<bool, PatternError> {
    if !general_included.is_empty() {
        let mut selected = false;
        for entry in general_included {
            if entry_matches(entry, filename)? {
                selected = true;
                break;
            }
        }
        if !selected {
            return Ok(false);
        }
    }
    for entry in general_excluded {
        if entry_matches(entry, filename)? {
            return Ok(false);
        }
    }
    let (default_included, default_excluded) = default_check_files(rule);
    let included = check_params.get("files.included").map_or_else(
        || default_included.unwrap_or_default(),
        |value| split_globs(value),
    );
    let excluded = check_params
        .get("files.excluded")
        .map_or(default_excluded, |value| split_globs(value));
    // Explicit empty `included` falls back to every file, mirroring
    // `find_in_dir` with `[]` (default sources glob over our universe).
    if included.is_empty() {
        return Ok(true);
    }
    for pattern in &included {
        if wildcard_match(pattern, filename)? {
            for exclude in &excluded {
                if wildcard_match(exclude, filename)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// Split comma-separated glob lists (mirrors `GeneralParams` encoding).
fn split_globs(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Filenames from `files` that match any included pattern and no excluded
/// one; used to exercise selection composition in tests.
#[allow(dead_code, reason = "selection composition helper for tests")]
fn select_filenames(
    filenames: &[&str],
    included: &[FileEntry],
    excluded: &[FileEntry],
) -> Result<Vec<String>, PatternError> {
    let mut out = Vec::new();
    for filename in filenames {
        if included.is_empty() {
            continue;
        }
        let mut selected = false;
        for entry in included {
            if entry_matches(entry, filename)? {
                selected = true;
                break;
            }
        }
        if !selected {
            continue;
        }
        let mut kept = true;
        for entry in excluded {
            if entry_matches(entry, filename)? {
                kept = false;
                break;
            }
        }
        if kept {
            out.push((*filename).to_owned());
        }
    }
    Ok(out)
}

/// One config file entry against a filename.
fn entry_matches(entry: &FileEntry, filename: &str) -> Result<bool, PatternError> {
    match entry {
        FileEntry::Glob(pattern) => wildcard_match(pattern, filename),
        FileEntry::Regex(source) => regex::Regex::new(source)
            .map_err(|error| PatternError(format!("invalid file regex `{source}`: {error}")))
            .map(|expression| expression.is_match(filename)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (pattern, path, expected) — every row observed via `Path.wildcard/2`.
    const MATCH_CASES: &[(&str, &str, bool)] = &[
        ("lib/", "lib/a.ex", true),
        ("lib/", "lib", true),
        ("lib/", "library/a.ex", false),
        ("test/", "test/a_test.exs", true),
        ("test", "test/a.ex", true),
        ("test", "other/a.ex", false),
        ("lib/a.ex", "lib/a.ex", true),
        ("lib/a.ex", "lib/b.ex", false),
        ("test/**/*_test.exs", "test/a_test.exs", true),
        ("test/**/*_test.exs", "test/b/a_test.exs", true),
        ("test/**/*_test.exs", "test/a_test.ex", false),
        ("test/**/*_test.exs", "other/a_test.exs", false),
        ("test/**/*_test.ex", "test/a_test.ex", true),
        ("lib/**/*.ex", "lib/a.ex", true),
        ("lib/**/*.ex", "lib/f/b.ex", true),
        ("lib/**/*.ex", "lib/a.exs", false),
        ("**/*_test.exs", "test/a_test.exs", true),
        ("**/*.ex", "lib/a.ex", true),
        ("lib/*.ex", "lib/a.ex", true),
        ("lib/*.ex", "lib/f/a.ex", false),
        ("**", "lib/a.ex", true),
        ("{lib,test}/*.exs", "test/c_test.exs", true),
        ("{lib,test}/*.exs", "other/c_test.exs", false),
        ("test/{bar,c}_test.exs", "test/c_test.exs", true),
        ("test/{bar,c}_test.exs", "test/bar/d_test.exs", false),
        ("lib/[a-z].ex", "lib/a.ex", true),
        ("lib/[!a].ex", "lib/a.ex", true),
        ("lib/[^a].ex", "lib/a.ex", true),
        ("lib/[]a].ex", "lib/a.ex", true),
        ("lib/[A-Z].ex", "lib/a.ex", false),
        ("lib/[z-a].ex", "lib/a.ex", true),
        ("lib/[z-a].ex", "lib/f.ex", false),
        ("lib/[abc", "lib/[abc", true),
        ("lib/[abc", "lib/a.ex", false),
        ("lib/?.ex", "lib/a.ex", true),
        ("lib/?.ex", "lib/ab.ex", false),
        ("lib/a**b.ex", "lib/axb.ex", true),
        ("lib/***.ex", "lib/a.ex", true),
        ("lib/\\*.ex", "lib/a.ex", false),
        ("**/*.ex", "lib/.hidden.ex", false),
        ("lib/*.ex", "lib/.hidden.ex", false),
        ("**/f.ex", ".hdir/f.ex", false),
        (".hdir/*.ex", ".hdir/f.ex", true),
        ("lib/.*.ex", "lib/.hidden.ex", false),
        ("**/b.ex", "lib/foo/b.ex", true),
        ("lib/**/b.ex", "lib/foo/b.ex", true),
        ("lib/sp ace.ex", "lib/sp ace.ex", true),
    ];

    #[test]
    fn wildcard_matches_native_observations() {
        for (pattern, path, expected) in MATCH_CASES {
            assert_eq!(
                wildcard_match(pattern, path).expect("valid pattern"),
                *expected,
                "{pattern} vs {path}"
            );
        }
    }

    #[test]
    fn nested_braces_are_explicit_errors() {
        assert!(wildcard_match("lib/{foo,{bar}}.ex", "lib/foo.ex").is_err());
    }

    #[test]
    fn check_files_defaults_match_upstream() {
        let (included, excluded) = default_check_files("Credo.Check.Design.SkipTestWithoutComment");
        assert_eq!(
            included,
            Some(vec![
                "test/**/*_test.exs".to_owned(),
                "apps/**/test/**/*_test.exs".to_owned()
            ])
        );
        assert!(excluded.is_empty());
        let (included, _) = default_check_files("Credo.Check.Warning.WrongTestFileExtension");
        assert_eq!(
            included,
            Some(vec![
                "test/**/*_test.ex".to_owned(),
                "apps/**/test/**/*_test.ex".to_owned()
            ])
        );
        let (included, excluded) = default_check_files("Credo.Check.Warning.WrongTestFilename");
        assert_eq!(included, Some(vec!["test/".to_owned()]));
        assert_eq!(
            excluded,
            vec![
                "test/**/*_test.exs".to_owned(),
                "apps/**/test/**/*_test.exs".to_owned()
            ]
        );
        let (included, _) = default_check_files("Credo.Check.Refactor.PassAsyncInTestCases");
        assert_eq!(
            included,
            Some(vec![
                "test/**/*_test.exs".to_owned(),
                "apps/**/test/**/*_test.exs".to_owned()
            ])
        );
        let (included, excluded) = default_check_files("Credo.Check.Warning.IoInspect");
        assert_eq!(included, None);
        assert!(excluded.is_empty());
    }

    #[test]
    fn wrong_test_filename_skips_lib_files() {
        // Unit-test gates invoke run directly (lib files report there); real
        // executions never feed lib files to this check.
        let general = GeneralParams::default();
        let params = BTreeMap::new();
        assert!(
            check_runs_on_file(
                "Credo.Check.Warning.WrongTestFilename",
                "test/my_module_text.exs",
                &general,
                &params
            )
            .expect("valid patterns")
        );
        assert!(
            !check_runs_on_file(
                "Credo.Check.Warning.WrongTestFilename",
                "lib/my_module.ex",
                &general,
                &params
            )
            .expect("valid patterns")
        );
        assert!(
            !check_runs_on_file(
                "Credo.Check.Warning.WrongTestFilename",
                "test/my_module_test.exs",
                &general,
                &params
            )
            .expect("valid patterns")
        );
    }

    #[test]
    fn general_and_check_selection_compose() {
        let mut map = BTreeMap::new();
        map.insert("files.excluded".to_owned(), "test/foo".to_owned());
        let general = GeneralParams::from_map(&map);
        let params = BTreeMap::new();
        // General exclusion wins even where the check would run.
        assert!(
            !check_runs_on_file(
                "Credo.Check.Design.SkipTestWithoutComment",
                "test/foo/a_test.exs",
                &general,
                &params
            )
            .expect("valid patterns")
        );
        assert!(
            check_runs_on_file(
                "Credo.Check.Design.SkipTestWithoutComment",
                "test/a_test.exs",
                &general,
                &params
            )
            .expect("valid patterns")
        );
    }

    #[test]
    fn entry_params_override_check_files() {
        let general = GeneralParams::default();
        let mut params = BTreeMap::new();
        params.insert("files.included".to_owned(), "special/".to_owned());
        assert!(
            check_runs_on_file(
                "Credo.Check.Warning.IoInspect",
                "special/a.ex",
                &general,
                &params
            )
            .expect("valid patterns")
        );
        assert!(
            !check_runs_on_file(
                "Credo.Check.Warning.IoInspect",
                "lib/a.ex",
                &general,
                &params
            )
            .expect("valid patterns")
        );
    }

    #[test]
    fn config_file_entries_select() {
        let files = ["lib/a.ex", "lib/.hidden.ex", "test/b_test.exs"];
        let selected = select_filenames(
            &files,
            &[FileEntry::Glob("lib/".to_owned())],
            &[FileEntry::Regex("/\\.".to_owned())],
        )
        .expect("valid patterns");
        assert_eq!(selected, vec!["lib/a.ex".to_owned()]);
    }

    const MATCHER_FILENAMES: &[&str] = &[
        "lib/a.ex",
        "lib/old/b.ex",
        "lib/_build/c.ex",
        "test/a_test.exs",
        "test/my_module_text.exs",
        "special/a.ex",
        "special/old/b.ex",
        "other/b.ex",
    ];

    fn assert_compiled_agrees(
        rule: &str,
        included: &[FileEntry],
        excluded: &[FileEntry],
        params: &BTreeMap<String, String>,
    ) {
        let matcher = CheckFileMatcher::compile(rule, included, excluded, params);
        for filename in MATCHER_FILENAMES {
            assert_eq!(
                matcher.matches(filename),
                check_runs_on_entries(rule, filename, included, excluded, params),
                "{rule} vs {filename}"
            );
        }
    }

    #[test]
    fn compiled_matcher_agrees_with_entries() {
        // The precompiled matcher must agree with `check_runs_on_entries`
        // on every (general entries, check params, filename) combination,
        // including check-level file defaults.
        let lib_only: Vec<FileEntry> = vec![FileEntry::Glob("lib/".to_owned())];
        let with_regex_excluded: Vec<FileEntry> = vec![
            FileEntry::Glob("lib/".to_owned()),
            FileEntry::Regex("/_build/".to_owned()),
        ];
        let mut special_params = BTreeMap::new();
        special_params.insert("files.included".to_owned(), "special/".to_owned());
        let mut excluded_params = BTreeMap::new();
        excluded_params.insert("files.excluded".to_owned(), "special/old/".to_owned());
        let empty = BTreeMap::new();
        assert_compiled_agrees("Credo.Check.Warning.IoInspect", &[], &[], &empty);
        assert_compiled_agrees("Credo.Check.Warning.IoInspect", &lib_only, &[], &empty);
        assert_compiled_agrees(
            "Credo.Check.Warning.IoInspect",
            &with_regex_excluded,
            &[],
            &empty,
        );
        assert_compiled_agrees("Credo.Check.Warning.IoInspect", &[], &[], &special_params);
        assert_compiled_agrees(
            "Credo.Check.Warning.IoInspect",
            &lib_only,
            &[],
            &excluded_params,
        );
        assert_compiled_agrees(
            "Credo.Check.Design.SkipTestWithoutComment",
            &[],
            &[],
            &empty,
        );
        assert_compiled_agrees("Credo.Check.Warning.WrongTestFilename", &[], &[], &empty);
    }

    #[test]
    fn compiled_matcher_error_order_matches_entries() {
        // First bad pattern in evaluation order wins, exactly like
        // `check_runs_on_entries` — including the unreachable-excluded
        // carve-out (empty `files.included` never evaluates `files.excluded`).
        let bad_included =
            BTreeMap::from([("files.included".to_owned(), "lib/{a,{b}}.ex".to_owned())]);
        let bad_excluded =
            BTreeMap::from([("files.excluded".to_owned(), "lib/{a,{b}}.ex".to_owned())]);
        let empty_included_bad_excluded = BTreeMap::from([
            ("files.included".to_owned(), String::new()),
            ("files.excluded".to_owned(), "lib/{a,{b}}.ex".to_owned()),
        ]);
        let bad_regex = [FileEntry::Regex("([".to_owned())];
        let empty = BTreeMap::new();
        assert_compiled_agrees("Credo.Check.Warning.IoInspect", &[], &[], &bad_included);
        assert_compiled_agrees("Credo.Check.Warning.IoInspect", &[], &[], &bad_excluded);
        assert_compiled_agrees("Credo.Check.Warning.IoInspect", &[], &bad_regex, &empty);
        // Empty `files.included` falls back to every file without ever
        // compiling `files.excluded`: no error, everything selected.
        let matcher = CheckFileMatcher::compile(
            "Credo.Check.Warning.IoInspect",
            &[],
            &[],
            &empty_included_bad_excluded,
        );
        assert!(matcher.matches("lib/a.ex").expect("excluded unreachable"));
        assert!(
            check_runs_on_entries(
                "Credo.Check.Warning.IoInspect",
                "lib/a.ex",
                &[],
                &[],
                &empty_included_bad_excluded,
            )
            .expect("no error")
        );

        // A later invalid pattern remains unobserved when an earlier one
        // already selects the file.
        let lazy_general = [
            FileEntry::Glob("lib/".to_owned()),
            FileEntry::Regex("([".to_owned()),
        ];
        assert_compiled_agrees("Credo.Check.Warning.IoInspect", &lazy_general, &[], &empty);
    }

    #[test]
    fn invalid_patterns_are_explicit() {
        let general = GeneralParams::default();
        let mut params = BTreeMap::new();
        params.insert("files.included".to_owned(), "lib/{a,{b}}.ex".to_owned());
        assert!(
            check_runs_on_file(
                "Credo.Check.Warning.IoInspect",
                "lib/a.ex",
                &general,
                &params
            )
            .is_err()
        );
    }
}
