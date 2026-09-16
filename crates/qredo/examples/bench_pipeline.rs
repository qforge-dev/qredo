//! Minimal full-pipeline driver for `scripts/credo-pipeline-bench`.
//!
//! Usage: `bench_pipeline <approot> <credo-exs>` — reads the project
//! config, lints `lib/` + `test/` (`.ex`/`.exs`, sorted,
//! project-relative names) with every enabled check at `--strict`
//! priority, and prints one JSON object per issue plus a summary line.
//! Always exits 0 on a completed run (the summary carries the exit
//! status); exits 2 when the config cannot be read.

use std::path::{Path, PathBuf};

/// Sorted recursive collection of `.ex`/`.exs` files with project-relative names.
fn collect(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext == "ex" || ext == "exs")
        {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            out.push((relative, path));
        }
    }
}

/// Profiling entry: the hotpath guard (feature-gated, profiling builds
/// only) prints the timing report at exit. Plain builds see a plain
/// `main` with identical behavior.
#[cfg_attr(feature = "hotpath", hotpath::main)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: bench_pipeline <approot> <credo-exs>");
        std::process::exit(64);
    }
    let root = PathBuf::from(&args[1]);
    let config_source = std::fs::read_to_string(&args[2]).unwrap_or_else(|error| {
        eprintln!("cannot read config: {error}");
        std::process::exit(2);
    });
    let config = qredo::parse_config(&config_source, "default").unwrap_or_else(|error| {
        eprintln!("cannot parse config: {error:?}");
        std::process::exit(2);
    });
    let mut named = Vec::new();
    collect(&root.join("lib"), &root, &mut named);
    collect(&root.join("test"), &root, &mut named);
    let files: Vec<qredo::RunnerFile> = named
        .iter()
        .map(|(relative, path)| qredo::RunnerFile {
            filename: relative.clone(),
            source: std::fs::read_to_string(path).unwrap_or_default(),
        })
        .collect();
    let report = qredo::run_checks(&files, &runner_config(config));
    print_report(&files, &report);
}

/// Runner configuration from a parsed project config.
fn runner_config(config: qredo::CredoConfig) -> qredo::RunnerConfig {
    let checks: Vec<qredo::CheckEntry> = config
        .checks
        .into_iter()
        .filter(|check| check.enabled)
        .map(|check| qredo::CheckEntry {
            module: check.module,
            enabled: true,
            params: check.params,
        })
        .collect();
    qredo::RunnerConfig {
        checks,
        files_included: config
            .files_included
            .into_iter()
            .map(qredo::FileEntry::Glob)
            .collect(),
        files_excluded: config.files_excluded,
        selection: qredo::Selection::default(),
        min_priority: -99,
        general: qredo::GeneralParams::default(),
    }
}

/// One JSON object per issue plus the summary line.
fn print_report(files: &[qredo::RunnerFile], report: &qredo::RunReport) {
    for issue in &report.issues {
        println!(
            "{}",
            serde_json::json!({
                "check": issue.check,
                "filename": issue.filename,
                "line_no": issue.line_no,
            })
        );
    }
    println!(
        "summary files={} issues={} exit={} errors={} skipped_invalid={}",
        files.len(),
        report.issues.len(),
        report.exit_status,
        report.errors.len(),
        report.skipped_invalid.len()
    );
    for error in &report.errors {
        println!("error: {error:?}");
    }
}
