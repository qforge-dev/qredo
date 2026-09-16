//! End-to-end pipeline checks over a small fixture application.
//!
//! These run the same `integration::execute` entry the `qredo` binary
//! uses: a served config must report the expected issues, and anything
//! outside the served subset must fail closed with an explicit reason
//! instead of running partial analysis.

use std::path::PathBuf;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/small_app")
            .join(name),
    )
    .expect("fixture readable")
}

fn files() -> Vec<qredo::RunnerFile> {
    ["lib/clean.ex", "lib/smells.ex", "test/smells_test.exs"]
        .into_iter()
        .map(|name| qredo::RunnerFile {
            filename: name.to_owned(),
            source: fixture(name),
        })
        .collect()
}

fn issue_keys(report: &qredo::RunReport) -> Vec<(String, String, Option<usize>)> {
    let mut keys: Vec<(String, String, Option<usize>)> = report
        .issues
        .iter()
        .map(|issue| (issue.check.clone(), issue.filename.clone(), issue.line_no))
        .collect();
    keys.sort();
    keys
}

#[test]
fn served_config_reports_expected_issues() {
    let config = fixture(".credo.exs");
    let report =
        qredo::integration::execute(&config, "default", &files(), -99).expect("config served");
    assert!(report.errors.is_empty());
    assert!(report.skipped_invalid.is_empty());
    assert_eq!(
        issue_keys(&report),
        vec![
            (
                "Credo.Check.Readability.ModuleDoc".to_owned(),
                "lib/smells.ex".to_owned(),
                Some(1),
            ),
            (
                "Credo.Check.Warning.Dbg".to_owned(),
                "lib/smells.ex".to_owned(),
                Some(4),
            ),
            (
                "Credo.Check.Warning.IoInspect".to_owned(),
                "lib/smells.ex".to_owned(),
                Some(3),
            ),
        ]
    );
    assert_ne!(report.exit_status, 0);
}

#[test]
fn clean_file_stays_clean() {
    let config = fixture(".credo.exs");
    let files = vec![qredo::RunnerFile {
        filename: "lib/clean.ex".to_owned(),
        source: fixture("lib/clean.ex"),
    }];
    let report =
        qredo::integration::execute(&config, "default", &files, -99).expect("config served");
    assert!(report.issues.is_empty());
    assert_eq!(report.exit_status, 0);
}

#[test]
fn unknown_check_fails_closed() {
    let config =
        "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Nope, []}]}}]}\n";
    let error =
        qredo::integration::execute(config, "default", &files(), -99).expect_err("falls back");
    assert!(
        error.reason.starts_with("unsupported-check:"),
        "unexpected reason: {}",
        error.reason
    );
}

#[test]
fn only_narrows_to_one_check() {
    let config = fixture(".credo.exs");
    let selection = qredo::Selection {
        only: vec!["IoInspect".to_owned()],
        ignore: Vec::new(),
    };
    let report = qredo::integration::execute_selected(&config, "default", &files(), -99, selection)
        .expect("config served");
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].check, "Credo.Check.Warning.IoInspect");
}

#[test]
fn ignore_drops_checks() {
    let config = fixture(".credo.exs");
    let selection = qredo::Selection {
        only: Vec::new(),
        ignore: vec!["IoInspect".to_owned(), "Dbg".to_owned()],
    };
    let report = qredo::integration::execute_selected(&config, "default", &files(), -99, selection)
        .expect("config served");
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].check, "Credo.Check.Readability.ModuleDoc");
}

#[test]
fn missing_config_name_fails_closed() {
    let config = fixture(".credo.exs");
    let error = qredo::integration::execute(&config, "ci", &files(), -99).expect_err("falls back");
    assert!(
        error.reason.starts_with("unsupported-credo-config:"),
        "unexpected reason: {}",
        error.reason
    );
}
