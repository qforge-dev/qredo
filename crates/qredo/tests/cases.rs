//! Generic upstream-case harness.
//!
//! Runs `compatibility/cases/<EXID>.json` corpora against rule kernels.
//! Only EXIDs listed in `compatibility/admitted.txt` gate the build; every
//! other corpus is parsed for schema health but does not fail. Promote a
//! check by fixing its kernel until its corpus fully passes, then adding its
//! EXID to `admitted.txt` in the same commit.
//!
//! Multi-file group entries are deferred by the kernel gate and evaluated by
//! the project gate instead: EXIDs in `compatibility/admitted_project.txt`
//! run each entry group (entries sharing `group`, or singletons) through
//! `run_project_check` once, attributing issues back per file.
//!
//! Param encoding (JSON value -> kernel string param):
//! bool -> "true"/"false"; number -> decimal; ":atom" string -> atom name
//! without the colon; other strings verbatim; arrays/objects (lists, tuples,
//! regexes `{"regex": s}`, ranges `{"range": [a,b]}`) -> compact JSON, parsed
//! by per-check code as fixes land (see `compatibility/params.md`).

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Deserialize)]
struct Entry {
    id: String,
    origin: String,
    assert: String,
    source: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    sources: Vec<serde_json::Value>,
    #[serde(default)]
    params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    findings: Vec<FindingExpected>,
    #[serde(default)]
    excluded_reason: Option<String>,
}

/// Why an entry is not kernel-gateable (needs pipeline/project evaluation).
fn deferral(entry: &Entry) -> Option<&'static str> {
    if !entry.sources.is_empty() {
        Some("multi-source project entry")
    } else if entry.filename.is_some() {
        Some("filename-dependent entry")
    } else if entry.group.is_some() {
        Some("multi-file group entry")
    } else {
        None
    }
}

#[derive(Deserialize)]
struct FindingExpected {
    line: Option<usize>,
    column: Option<usize>,
    #[serde(default)]
    filename: Option<String>,
    trigger: String,
    message: String,
}

fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compatibility/cases")
}

fn inventory_rule(exid: &str) -> Option<String> {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compatibility/upstream/inventory.json"),
    )
    .ok()?;
    let inventory: serde_json::Value = serde_json::from_str(&text).ok()?;
    inventory["rules"].as_array()?.iter().find_map(|rule| {
        if rule["upstream_id"] == exid {
            rule["id"].as_str().map(ToOwned::to_owned)
        } else {
            None
        }
    })
}

fn admitted() -> BTreeSet<String> {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compatibility/admitted.txt"),
    )
    .unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
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
    filename: &str,
    trigger: &str,
    message: &str,
) -> String {
    format!(
        "{}|{}|{filename}|{trigger}|{message}",
        line.unwrap_or(0),
        column.unwrap_or(0)
    )
}

fn check_entry(rule: &str, entry: &Entry) -> Vec<String> {
    let mut problems = Vec::new();
    assert!(!entry.origin.is_empty(), "{}", entry.id);
    assert!(!entry.assert.is_empty(), "{}", entry.id);
    if entry.excluded_reason.is_some() {
        return problems;
    }
    if deferral(entry).is_some() {
        return problems;
    }
    let params: BTreeMap<String, String> = entry
        .params
        .iter()
        .map(|(key, value)| (key.clone(), param_string(value)))
        .collect();
    let findings = match qredo::check_kernel_with_params(rule, &entry.source, &params) {
        Ok(findings) => findings,
        Err(error) => {
            problems.push(format!("{}: kernel error: {:?}", entry.id, error));
            return problems;
        }
    };
    let mut actual: Vec<String> = findings.iter().map(actual_key).collect();
    let mut expected: Vec<String> = entry.findings.iter().map(expected_key).collect();
    actual.sort();
    expected.sort();
    if actual != expected {
        problems.push(format!(
            "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
            entry.id
        ));
    }
    let _ = entry.filename.as_deref();
    problems
}

fn actual_key(finding: &qredo::Finding) -> String {
    finding_key(
        Some(finding.line),
        finding.column,
        "",
        match &finding.trigger {
            qredo::Trigger::NoTrigger => "no_trigger",
            qredo::Trigger::Text(text) => text,
        },
        &finding.message,
    )
}

fn expected_key(finding: &FindingExpected) -> String {
    finding_key(
        finding.line,
        finding.column,
        finding.filename.as_deref().unwrap_or(""),
        &finding.trigger,
        &finding.message,
    )
}

#[test]
fn corpora_parse_and_cover_inventory() {
    let mut seen = BTreeSet::new();
    let mut entries = 0_usize;
    let mut excluded = 0_usize;
    let paths = std::fs::read_dir(cases_dir()).expect("cases dir readable");
    for path in paths {
        let path = path.expect("dir entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("case file readable");
        let parsed: Vec<Entry> = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{}: invalid JSON: {error}", path.display()));
        for entry in &parsed {
            assert!(!entry.id.is_empty(), "{}", path.display());
            entries += 1;
            if entry.excluded_reason.is_some() {
                excluded += 1;
            }
            if let Some((exid, _)) = entry.id.split_once('.') {
                seen.insert(exid.to_owned());
            }
        }
    }
    assert!(entries > 0, "corpora must not be empty");
    eprintln!(
        "cases: {entries} entries ({excluded} excluded) across {} files",
        seen.len()
    );
}

#[test]
fn admitted_corpora_fully_pass() {
    let allow = admitted();
    for exid in &allow {
        assert!(
            cases_dir().join(format!("{exid}.json")).exists(),
            "admitted {exid} has no corpus file"
        );
        assert!(
            inventory_rule(exid).is_some(),
            "admitted {exid} missing from inventory"
        );
    }
    let mut failures = Vec::new();
    let mut checked = 0_usize;
    let mut deferred = 0_usize;
    let paths = std::fs::read_dir(cases_dir()).expect("cases dir readable");
    for path in paths {
        let path = path.expect("dir entry").path();
        let Some(exid) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !allow.contains(&exid) {
            continue;
        }
        let rule = inventory_rule(&exid).expect("admitted rule in inventory");
        let text = std::fs::read_to_string(&path).expect("case file readable");
        let parsed: Vec<Entry> = serde_json::from_str(&text).expect("valid JSON");
        let mut evaluated = 0_usize;
        for entry in &parsed {
            checked += 1;
            if entry.excluded_reason.is_none() && deferral(entry).is_none() {
                evaluated += 1;
            } else {
                deferred += 1;
            }
            failures.extend(check_entry(&rule, entry));
        }
        assert!(
            evaluated > 0,
            "admitted {exid} has no kernel-gateable entries"
        );
    }
    eprintln!(
        "admitted gate: {checked} entries checked ({deferred} deferred), {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn admitted_project() -> BTreeSet<String> {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compatibility/admitted_project.txt"),
    )
    .unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
}

fn project_finding_key(issue: &qredo::ProjectIssue) -> String {
    finding_key(issue.line, issue.column, "", &issue.trigger, &issue.message)
}

fn expected_project_key(finding: &FindingExpected) -> String {
    finding_key(
        finding.line,
        finding.column,
        finding.filename.as_deref().unwrap_or(""),
        &finding.trigger,
        &finding.message,
    )
}

#[test]
fn admitted_project_corpora_fully_pass() {
    let allow = admitted_project();
    for exid in &allow {
        assert!(
            cases_dir().join(format!("{exid}.json")).exists(),
            "admitted {exid} has no corpus file"
        );
        assert!(
            inventory_rule(exid).is_some(),
            "admitted {exid} missing from inventory"
        );
    }
    let mut failures = Vec::new();
    let mut evaluated = 0_usize;
    let paths = std::fs::read_dir(cases_dir()).expect("cases dir readable");
    for path in paths {
        let path = path.expect("dir entry").path();
        evaluated += check_project_file(&path, &allow, &mut failures);
    }
    eprintln!(
        "project gate: {evaluated} entries evaluated, {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// One corpus file's subgroups; returns evaluated entry count.
fn check_project_file(
    path: &std::path::Path,
    allow: &BTreeSet<String>,
    failures: &mut Vec<String>,
) -> usize {
    let Some(exid) = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
    else {
        return 0;
    };
    if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !allow.contains(&exid) {
        return 0;
    }
    let rule = inventory_rule(&exid).expect("admitted rule in inventory");
    let text = std::fs::read_to_string(path).expect("case file readable");
    let parsed: Vec<Entry> = serde_json::from_str(&text).expect("valid JSON");
    let mut evaluated_here = 0_usize;
    // Subgroup by (group, params): one project run per subgroup.
    let mut subgroups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (index, entry) in parsed.iter().enumerate() {
        if entry.excluded_reason.is_some() {
            continue;
        }
        let group = entry.group.clone().unwrap_or_else(|| entry.id.clone());
        let params = serde_json::to_string(&entry.params).unwrap_or_default();
        subgroups.entry((group, params)).or_default().push(index);
    }
    for ((group, _), indexes) in &subgroups {
        evaluated_here += check_project_subgroup(&rule, group, &parsed, indexes, failures);
    }
    assert!(
        evaluated_here > 0,
        "admitted {exid} has no project-gateable entries"
    );
    evaluated_here
}

/// One project run for a subgroup; returns evaluated entry count.
fn check_project_subgroup(
    rule: &str,
    group: &str,
    parsed: &[Entry],
    indexes: &[usize],
    failures: &mut Vec<String>,
) -> usize {
    let first = &parsed[indexes[0]];
    if first
        .sources
        .iter()
        .any(|source| source.get("source").is_some())
    {
        return check_multisource_entries(rule, parsed, indexes, failures);
    }
    let params: BTreeMap<String, String> = first
        .params
        .iter()
        .map(|(key, value)| (key.clone(), param_string(value)))
        .collect();
    let files: Vec<qredo::ProjectFile> = indexes
        .iter()
        .map(|index| qredo::ProjectFile {
            filename: parsed[*index].id.clone(),
            source: parsed[*index].source.clone(),
        })
        .collect();
    let issues = match qredo::run_project_check(rule, &files, &params) {
        Ok(issues) => issues,
        Err(error) => {
            failures.push(format!("{group}: project error: {error:?}"));
            return 0;
        }
    };
    for (position, index) in indexes.iter().enumerate() {
        let entry = &parsed[*index];
        let mut actual: Vec<String> = issues
            .iter()
            .filter(|issue| issue.file == position)
            .map(project_finding_key)
            .collect();
        let mut expected: Vec<String> = entry.findings.iter().map(expected_project_key).collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            failures.push(format!(
                "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
                entry.id
            ));
        }
    }
    indexes.len()
}

/// Entries carrying their own `sources` arrays (multi-file projects encoded
/// per entry); each entry runs as an independent project keyed by filename.
fn check_multisource_entries(
    rule: &str,
    parsed: &[Entry],
    indexes: &[usize],
    failures: &mut Vec<String>,
) -> usize {
    let mut evaluated = 0_usize;
    for index in indexes {
        let entry = &parsed[*index];
        if entry.sources.is_empty() {
            continue;
        }
        evaluated += 1;
        check_multisource_entry(rule, entry, failures);
    }
    evaluated
}

/// One multi-source entry as an independent filename-keyed project.
fn check_multisource_entry(rule: &str, entry: &Entry, failures: &mut Vec<String>) {
    let params: BTreeMap<String, String> = entry
        .params
        .iter()
        .map(|(key, value)| (key.clone(), param_string(value)))
        .collect();
    let files: Vec<qredo::ProjectFile> = entry
        .sources
        .iter()
        .map(|source| qredo::ProjectFile {
            filename: source
                .get("filename")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned(),
            source: source
                .get("source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned(),
        })
        .collect();
    let issues = match qredo::run_project_check(rule, &files, &params) {
        Ok(issues) => issues,
        Err(error) => {
            failures.push(format!("{}: project error: {error:?}", entry.id));
            return;
        }
    };
    compare_multisource_issues(entry, &files, &issues, failures);
}

/// Sorted actual/expected comparison for one multi-source entry.
fn compare_multisource_issues(
    entry: &Entry,
    files: &[qredo::ProjectFile],
    issues: &[qredo::ProjectIssue],
    failures: &mut Vec<String>,
) {
    let mut actual: Vec<String> = issues
        .iter()
        .map(|issue| {
            let filename = files
                .get(issue.file)
                .map_or("", |file| file.filename.as_str());
            finding_key(
                Some(issue.line.unwrap_or(0)),
                issue.column,
                filename,
                &issue.trigger,
                &issue.message,
            )
        })
        .collect();
    let mut expected: Vec<String> = entry
        .findings
        .iter()
        .map(|finding| {
            finding_key(
                finding.line,
                finding.column,
                finding.filename.as_deref().unwrap_or(""),
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
            entry.id
        ));
    }
}

fn admitted_filename() -> BTreeSet<String> {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compatibility/admitted_filename.txt"),
    )
    .unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
}

#[test]
fn admitted_filename_corpora_fully_pass() {
    let allow = admitted_filename();
    for exid in &allow {
        assert!(
            cases_dir().join(format!("{exid}.json")).exists(),
            "admitted {exid} has no corpus file"
        );
        assert!(
            inventory_rule(exid).is_some(),
            "admitted {exid} missing from inventory"
        );
    }
    let mut failures = Vec::new();
    let mut evaluated = 0_usize;
    let paths = std::fs::read_dir(cases_dir()).expect("cases dir readable");
    for path in paths {
        let path = path.expect("dir entry").path();
        evaluated += check_filename_file(&path, &allow, &mut failures);
    }
    assert!(evaluated > 0, "filename gate evaluated nothing");
    eprintln!(
        "filename gate: {evaluated} entries evaluated, {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// One corpus file's filename entries; returns evaluated entry count.
fn check_filename_file(
    path: &std::path::Path,
    allow: &BTreeSet<String>,
    failures: &mut Vec<String>,
) -> usize {
    let mut evaluated = 0_usize;
    let Some(exid) = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
    else {
        return evaluated;
    };
    if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !allow.contains(&exid) {
        return evaluated;
    }
    let rule = inventory_rule(&exid).expect("admitted rule in inventory");
    let text = std::fs::read_to_string(path).expect("case file readable");
    let parsed: Vec<Entry> = serde_json::from_str(&text).expect("valid JSON");
    for entry in &parsed {
        if entry.excluded_reason.is_some() {
            continue;
        }
        let Some(filename) = entry.filename.clone() else {
            continue;
        };
        evaluated += 1;
        let params: BTreeMap<String, String> = entry
            .params
            .iter()
            .map(|(key, value)| (key.clone(), param_string(value)))
            .collect();
        let file = qredo::ProjectFile {
            filename,
            source: entry.source.clone(),
        };
        let issues = match qredo::run_filename_check(&rule, &file, &params) {
            Ok(issues) => issues,
            Err(error) => {
                failures.push(format!("{}: filename error: {error:?}", entry.id));
                continue;
            }
        };
        let mut actual: Vec<String> = issues.iter().map(project_finding_key).collect();
        let mut expected: Vec<String> = entry.findings.iter().map(expected_project_key).collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            failures.push(format!(
                "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
                entry.id
            ));
        }
    }
    evaluated
}
