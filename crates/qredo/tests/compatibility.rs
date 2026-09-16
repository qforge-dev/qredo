use qredo::{
    ConfigSource, GeneralParams, SelectedOutcome, Selection, SourceSnapshot, Trigger, check_kernel,
    check_kernel_with_params, run_trailing_blank_line_selected,
};
use serde::Deserialize;
use std::collections::BTreeMap;

const RULE: &str = "Credo.Check.Readability.TrailingBlankLine";

#[derive(Deserialize)]
struct Case {
    id: String,
    source: String,
    findings: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    line: usize,
    message: String,
    trigger: String,
}

#[test]
fn ex3028_matches_upstream_examples_and_native_boundary_findings() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../compatibility/trailing_blank_line.json")).unwrap();
    for case in cases {
        let findings = check_kernel(RULE, &case.source).unwrap();
        assert_eq!(findings.len(), case.findings.len(), "{}", case.id);
        for (actual, expected) in findings.iter().zip(&case.findings) {
            assert_eq!(actual.line, expected.line, "{}", case.id);
            assert_eq!(actual.message, expected.message, "{}", case.id);
            assert_eq!(actual.trigger, Trigger::NoTrigger, "{}", case.id);
            assert_eq!(expected.trigger, "no_trigger", "{}", case.id);
        }
    }
}

#[test]
fn all_kernels_are_implemented_without_clean_result_confusion() {
    let ledger: serde_json::Value =
        serde_json::from_str(include_str!("../compatibility/rules.json")).unwrap();
    let inventory: serde_json::Value =
        serde_json::from_str(include_str!("../compatibility/upstream/inventory.json")).unwrap();
    let rules = ledger["rules"].as_array().unwrap();
    let upstream = inventory["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 120);
    assert_eq!(rules.len(), upstream.len());
    for (rule, reference) in rules.iter().zip(upstream) {
        assert_eq!(rule["id"], reference["id"]);
        assert_eq!(rule["pipeline"], "unsupported");
        let id = rule["id"].as_str().unwrap();
        assert_eq!(rule["kernel"], "verified", "{id}");
        // Every implemented kernel must be callable; unknown IDs still error
        // so a missing rule can never report a clean result.
        assert!(check_kernel(id, "x = 1\n").is_ok(), "{id}");
    }
    assert!(check_kernel("not-a-credo-rule", "").is_err());
}

#[derive(Deserialize)]
struct KernelCase {
    id: String,
    origin: String,
    source: String,
    #[serde(default = "default_ignore_strings")]
    ignore_strings: bool,
    findings: Vec<KernelExpected>,
}

#[derive(Deserialize)]
struct KernelExpected {
    line: usize,
    column: Option<usize>,
    trigger: String,
    message: String,
}

fn default_ignore_strings() -> bool {
    true
}

#[test]
fn ex3029_matches_reviewed_native_findings() {
    let cases: Vec<KernelCase> =
        serde_json::from_str(include_str!("../compatibility/trailing_white_space.json")).unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let mut params = BTreeMap::new();
        params.insert(
            "ignore_strings".to_owned(),
            (if case.ignore_strings { "true" } else { "false" }).to_owned(),
        );
        assert_kernel_findings(
            "Credo.Check.Readability.TrailingWhiteSpace",
            &ParamKernelCase {
                id: case.id,
                origin: case.origin,
                source: case.source,
                params,
                findings: case.findings,
            },
        );
    }
}

#[derive(Deserialize)]
struct ParamKernelCase {
    id: String,
    origin: String,
    source: String,
    #[serde(default)]
    params: BTreeMap<String, String>,
    findings: Vec<KernelExpected>,
}

#[test]
fn ex3019_matches_reviewed_native_findings() {
    let cases: Vec<ParamKernelCase> =
        serde_json::from_str(include_str!("../compatibility/redundant_blank_lines.json")).unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_kernel_findings("Credo.Check.Readability.RedundantBlankLines", &case);
    }
}

#[test]
fn ex3020_matches_reviewed_native_findings() {
    let cases: Vec<ParamKernelCase> =
        serde_json::from_str(include_str!("../compatibility/semicolons.json")).unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_kernel_findings("Credo.Check.Readability.Semicolons", &case);
    }
}

#[test]
fn ex3024_matches_reviewed_native_findings() {
    let cases: Vec<ParamKernelCase> =
        serde_json::from_str(include_str!("../compatibility/space_after_commas.json")).unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_kernel_findings("Credo.Check.Readability.SpaceAfterCommas", &case);
    }
}

#[test]
fn ex3007_matches_reviewed_native_findings() {
    let cases: Vec<ParamKernelCase> =
        serde_json::from_str(include_str!("../compatibility/max_line_length.json")).unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_kernel_findings("Credo.Check.Readability.MaxLineLength", &case);
    }
}

#[test]
fn ex2005_matches_reviewed_native_findings() {
    let cases: Vec<ParamKernelCase> =
        serde_json::from_str(include_str!("../compatibility/tag_todo.json")).unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_kernel_findings("Credo.Check.Design.TagTODO", &case);
    }
}

#[test]
fn ex2004_matches_reviewed_native_findings() {
    let cases: Vec<ParamKernelCase> =
        serde_json::from_str(include_str!("../compatibility/tag_fixme.json")).unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_kernel_findings("Credo.Check.Design.TagFIXME", &case);
    }
}

#[test]
fn ex4022_matches_reviewed_native_findings() {
    // Upstream ships zero test cases for PerceivedComplexity; these cases
    // were verified against native `run` on the pinned checkout (7/7 OK)
    // before committing and are reviewed artifacts, not snapshots.
    let cases: Vec<ParamKernelCase> =
        serde_json::from_str(include_str!("../compatibility/perceived_complexity.json")).unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_kernel_findings("Credo.Check.Refactor.PerceivedComplexity", &case);
    }
}

#[derive(Deserialize)]
struct FilenameCase {
    id: String,
    origin: String,
    filename: String,
    source: String,
    #[serde(default)]
    params: BTreeMap<String, String>,
    findings: Vec<KernelExpected>,
}

#[test]
fn ex5025_matches_reviewed_native_findings() {
    // Upstream defines zero test cases (the test module documents that
    // `files.included` selection makes it untestable in isolation). These
    // cases lock the native `run/2` issue shape (verified: 1 issue, line 1,
    // no trigger) behind the default `files.included` selection.
    let cases: Vec<FilenameCase> = serde_json::from_str(include_str!(
        "../compatibility/wrong_test_file_extension.json"
    ))
    .unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert!(!case.origin.is_empty(), "{}", case.id);
        let file = qredo::ProjectFile {
            filename: case.filename,
            source: case.source,
        };
        let issues = qredo::run_filename_check(
            "Credo.Check.Warning.WrongTestFileExtension",
            &file,
            &case.params,
        )
        .unwrap();
        assert_eq!(issues.len(), case.findings.len(), "{}", case.id);
        for (actual, expected) in issues.iter().zip(&case.findings) {
            assert_eq!(actual.line, Some(expected.line), "{}", case.id);
            assert_eq!(actual.column, expected.column, "{}", case.id);
            assert_eq!(actual.message, expected.message, "{}", case.id);
            assert_eq!(actual.trigger, expected.trigger, "{}", case.id);
        }
    }
}

fn assert_kernel_findings(rule: &str, case: &ParamKernelCase) {
    assert!(!case.origin.is_empty(), "{}", case.id);
    let findings = check_kernel_with_params(rule, &case.source, &case.params).unwrap();
    assert_eq!(findings.len(), case.findings.len(), "{}", case.id);
    for (actual, expected) in findings.iter().zip(&case.findings) {
        assert_eq!(actual.line, expected.line, "{}", case.id);
        assert_eq!(actual.column, expected.column, "{}", case.id);
        assert_eq!(actual.message, expected.message, "{}", case.id);
        match &actual.trigger {
            qredo::Trigger::Text(text) => {
                assert_eq!(text, &expected.trigger, "{}", case.id);
            }
            qredo::Trigger::NoTrigger => {
                assert_eq!(expected.trigger, "no_trigger", "{}", case.id);
            }
        }
    }
}

#[derive(Deserialize)]
struct PipelineCase {
    id: String,
    source: String,
    filename: String,
    #[serde(default)]
    general: BTreeMap<String, String>,
    #[serde(default)]
    selection: PipelineSelection,
    #[serde(default = "default_config")]
    config: String,
    #[serde(default = "default_outcome")]
    outcome: String,
    status: String,
    issues: Vec<PipelineExpected>,
}

#[derive(Deserialize, Default)]
struct PipelineSelection {
    #[serde(default)]
    only: Vec<String>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    checks_with_tag: Vec<String>,
    #[serde(default)]
    enable_disabled: Vec<String>,
}

fn default_config() -> String {
    "default".to_owned()
}

fn default_outcome() -> String {
    "ran".to_owned()
}

#[derive(Deserialize)]
struct PipelineExpected {
    line_no: Option<usize>,
    column: Option<usize>,
    scope: Option<String>,
    priority: i32,
    exit_status: i32,
    category: String,
    trigger: String,
    message: String,
}

#[test]
fn ex3028_pipeline_matches_reviewed_full_issue_expectations() {
    let cases: Vec<PipelineCase> = serde_json::from_str(include_str!(
        "../compatibility/trailing_blank_line_pipeline.json"
    ))
    .unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_pipeline_case(&case);
    }
}

fn assert_pipeline_case(case: &PipelineCase) {
    let snapshot = SourceSnapshot::parse(&case.source, &case.filename);
    let general = GeneralParams::from_map(&case.general);
    let selection = Selection {
        only: case.selection.only.clone(),
        ignore: case.selection.ignore.clone(),
        checks_with_tag: case.selection.checks_with_tag.clone(),
        enable_disabled: case.selection.enable_disabled.clone(),
    };
    let config = if let Some(path) = case.config.strip_prefix("executable:") {
        ConfigSource::ExecutableFile(path.to_owned())
    } else {
        ConfigSource::Default
    };
    match run_trailing_blank_line_selected(&snapshot, &general, &selection, &config) {
        SelectedOutcome::FilteredOut => {
            assert_eq!(case.outcome, "filtered_out", "{}", case.id);
            assert!(case.issues.is_empty(), "{}", case.id);
        }
        SelectedOutcome::NeedsNativeConfig(path) => {
            assert_eq!(case.outcome, "needs_native_config", "{}", case.id);
            assert!(case.issues.is_empty(), "{}", case.id);
            let _ = path;
        }
        SelectedOutcome::InvalidSelection(pattern) => {
            assert_eq!(case.outcome, "invalid_selection", "{}", case.id);
            assert!(case.issues.is_empty(), "{}", case.id);
            let _ = pattern;
        }
        SelectedOutcome::Ran(result) => {
            assert_eq!(case.outcome, "ran", "{}", case.id);
            let status = format!("{:?}", result.status).to_lowercase();
            assert_eq!(status, case.status, "{}", case.id);
            assert_eq!(result.issues.len(), case.issues.len(), "{}", case.id);
            for (actual, expected) in result.issues.iter().zip(&case.issues) {
                assert_pipeline_issue(actual, expected, &case.id);
            }
        }
    }
}

fn assert_pipeline_issue(actual: &qredo::Issue, expected: &PipelineExpected, case_id: &str) {
    assert_eq!(actual.line_no, expected.line_no, "{case_id}");
    assert_eq!(actual.column, expected.column, "{case_id}");
    assert_eq!(actual.scope, expected.scope, "{case_id}");
    assert_eq!(actual.priority, expected.priority, "{case_id}");
    assert_eq!(actual.exit_status, expected.exit_status, "{case_id}");
    assert_eq!(actual.category.as_str(), expected.category, "{case_id}");
    assert_eq!(actual.message, expected.message, "{case_id}");
    match &actual.trigger {
        qredo::IssueTrigger::NoTrigger => {
            assert_eq!(expected.trigger, "no_trigger", "{case_id}");
        }
        qredo::IssueTrigger::Text(text) => {
            assert_eq!(text, &expected.trigger, "{case_id}");
        }
    }
}
