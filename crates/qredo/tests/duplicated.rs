//! EX2002 oracle contracts and an opt-in release performance probe.
use qredo::{ProjectFile, run_project_check};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const RULE: &str = "Credo.Check.Design.DuplicatedCode";

#[test]
fn ex2002_excluded_macros_do_not_exclude_nested_keyword_lists() {
    let files: Vec<ProjectFile> = ["A", "B"]
        .into_iter()
        .map(|name| ProjectFile {
            filename: format!("{}.ex", name.to_lowercase()),
            source: format!(
                "defmodule {name} do\n def f(x) do\nfoo(do: [do: bar(x)])\n end\nend\n"
            ),
        })
        .collect();
    let params = BTreeMap::from([
        ("mass_threshold".to_owned(), "1".to_owned()),
        ("excluded_macros".to_owned(), "[\"bar\"]".to_owned()),
    ]);
    let issues = run_project_check(RULE, &files, &params).unwrap();
    // Pinned Credo reports both the outer do-pair and its enclosing list;
    // the nested value is a list, not a directly excluded macro call.
    for (file, peer) in [(0, "b.ex"), (1, "a.ex")] {
        let expected = format!("Duplicate code found in {peer}:3 (mass: 4).");
        let matching: Vec<_> = issues
            .iter()
            .filter(|i| i.file == file && i.message == expected)
            .collect();
        assert_eq!(matching.len(), 2);
        assert!(matching.iter().all(|i| i.line == Some(3)));
    }
}

#[test]
fn ex2002_nil_locations_and_json_sentinel() {
    let files: Vec<qredo::RunnerFile> = ["lib/a.ex", "lib/b.ex"]
        .into_iter()
        .map(|filename| qredo::RunnerFile {
            filename: filename.to_owned(),
            source: "42\n".to_owned(),
        })
        .collect();
    let config = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Design.DuplicatedCode, [mass_threshold: 0]}]}}]}";
    let report = qredo::integration::execute(config, "default", &files, -99).unwrap();
    assert_eq!(report.issues.len(), 2);
    assert_eq!(report.exit_status, 2);
    for issue in &report.issues {
        assert_eq!(issue.line_no, None);
        assert_eq!(issue.scope, None);
        assert_eq!(issue.priority, 20);
        assert_eq!(issue.severity.to_bits(), 2.0_f64.to_bits());
    }
    let json = qredo::format_machine::render_json(
        &report,
        &qredo::format_machine::MachineContext::new("."),
    );
    let data: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        data["issues"][0]["trigger"],
        serde_json::json!(["__no_trigger__"])
    );
}

#[test]
fn ex2002_reviewed_projects() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../compatibility/duplicated.json")).unwrap();
    verify_projects(corpus["projects"].as_array().unwrap());
}

#[test]
#[ignore = "development oracle campaign: QREDO_PROJECTS=oracle.json"]
fn ex2002_external_projects() {
    let corpus: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("QREDO_PROJECTS").unwrap()).unwrap(),
    )
    .unwrap();
    verify_projects(corpus["projects"].as_array().unwrap());
}

fn verify_projects(cases: &[serde_json::Value]) {
    for case in cases {
        let files: Vec<ProjectFile> = case["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| ProjectFile {
                filename: f["filename"].as_str().unwrap().to_owned(),
                source: f["source"].as_str().unwrap().to_owned(),
            })
            .collect();
        let params = case["params"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, val)| (key.clone(), val.to_string()))
            .collect();
        let issues = run_project_check(RULE, &files, &params).unwrap();
        let mut actual: Vec<serde_json::Value> = issues
            .iter()
            .map(|i| {
                serde_json::json!({
                    "filename": files[i.file].filename, "line": i.line, "column": i.column,
                    "message": i.message, "trigger": i.trigger, "severity": i.severity,
                })
            })
            .collect();
        let mut expected = case["findings"].as_array().unwrap().clone();
        actual.sort_by_key(serde_json::Value::to_string);
        expected.sort_by_key(serde_json::Value::to_string);
        assert_eq!(actual, expected, "{}", case["id"]);
    }
}

#[test]
#[ignore = "release-only timing probe: cargo test --release --test duplicated ex2002_performance -- --ignored --nocapture"]
fn ex2002_performance() {
    for count in [30, 120, 480] {
        let files: Vec<ProjectFile> = (0..count).map(|i| ProjectFile {
            filename: format!("lib/m{i}.ex"),
            source: format!("defmodule M{i} do\n{}\nend\n", (0..30).fold(String::new(), |mut out, j| { let _ = writeln!(out, "def f{j}(x), do: x |> Enum.map(&(&1 + {i})) |> Enum.filter(&(&1 > {j}))"); out })),
        }).collect();
        let start = std::time::Instant::now();
        let mut findings = 0;
        for _ in 0..3 {
            findings +=
                std::hint::black_box(run_project_check(RULE, &files, &BTreeMap::new()).unwrap())
                    .len();
        }
        eprintln!(
            "EX2002 files={count} runs=3 elapsed={:?} findings={findings}",
            start.elapsed()
        );
    }
}

#[test]
fn ex2002_chunk_boundaries_are_deterministic() {
    for count in [29, 30, 31, 60, 61] {
        let files = duplicated_files(count);
        let issues = run_project_check(RULE, &files, &BTreeMap::new()).unwrap();
        assert_eq!(issues.len(), count, "EX2002.chunks.{count}");
        let mut order: Vec<usize> = (0..count).rev().collect();
        order.sort_by_key(|i| i / 30);
        for issue in &issues {
            let peers: Vec<String> = order
                .iter()
                .filter(|i| **i != issue.file)
                .map(|i| format!("lib/m{i:03}.ex:2"))
                .collect();
            assert_eq!(issue.line, Some(2));
            assert_eq!(
                issue.message,
                format!("Duplicate code found in {} (mass: 86).", peers.join(", "))
            );
        }
        assert_eq!(
            issues,
            run_project_check(RULE, &files, &BTreeMap::new()).unwrap()
        );
        for (key, value) in [
            ("mass_threshold", "87".to_owned()),
            ("nodes_threshold", (count + 1).to_string()),
            ("excluded_macros", "[\"def\"]".to_owned()),
        ] {
            assert!(
                run_project_check(RULE, &files, &BTreeMap::from([(key.to_owned(), value)]))
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(
            issues,
            run_project_check(
                RULE,
                &files,
                &BTreeMap::from([("mass_threshold".to_owned(), "86".to_owned())])
            )
            .unwrap()
        );
    }
}

fn duplicated_files(count: usize) -> Vec<ProjectFile> {
    let body = (0..20).fold(String::new(), |mut out, i| {
        let _ = writeln!(out, "x = call(x, {i})");
        out
    });
    (0..count)
        .map(|i| ProjectFile {
            filename: format!("lib/m{i:03}.ex"),
            source: format!("defmodule M{i} do\ndef f(x) do\n{body}end\nend\n"),
        })
        .collect()
}
