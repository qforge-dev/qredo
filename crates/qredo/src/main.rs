//! `qredo`: native Elixir linting with Credo-compatible behavior.
//!
//! Usage: `qredo [PATH] [--config-file FILE] [--strict] [--format text|json]`
//!
//! Reads the static `.credo.exs` subset qredo serves, lints `lib/` + `test/`
//! (`.ex`/`.exs`, sorted, project-relative names) and prints one line per
//! issue. Configs outside the served subset fail closed with an explicit
//! reason and exit code 2 instead of running partial analysis. Exit status
//! is the OR-combined issue status (`0` when clean), like Credo.

use std::path::{Path, PathBuf};

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    approot: PathBuf,
    config_file: Option<PathBuf>,
    strict: bool,
    format: Format,
}

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Format {
    #[default]
    Text,
    Json,
}

/// Usage error text.
fn usage() -> &'static str {
    "usage: qredo [PATH] [--config-file FILE] [--strict] [--format text|json]"
}

/// Parse argv (without the program name).
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut approot = PathBuf::from(".");
    let mut config_file = None;
    let mut strict = false;
    let mut format = Format::Text;
    let mut positional = Vec::new();
    let mut iter = argv.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--strict" => strict = true,
            "--config-file" => {
                let value = iter.next().ok_or("--config-file needs a value")?;
                config_file = Some(PathBuf::from(value));
            }
            "--format" => {
                let value = iter.next().ok_or("--format needs a value")?;
                format = match value.as_str() {
                    "text" => Format::Text,
                    "json" => Format::Json,
                    _ => return Err(format!("unknown --format `{value}`")),
                };
            }
            "--help" | "-h" => return Err(usage().to_owned()),
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "unknown argument `{arg}`\n{usage}",
                    usage = usage()
                ));
            }
            _ => positional.push(arg),
        }
    }
    if positional.len() > 1 {
        return Err(usage().to_owned());
    }
    if let Some(root) = positional.first() {
        approot = PathBuf::from(root);
    }
    Ok(Args {
        approot,
        config_file,
        strict,
        format,
    })
}

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

/// One text line per issue, Credo-oneline style.
fn print_text(report: &qredo::RunReport) {
    for issue in &report.issues {
        let line = issue.line_no.map_or("-".to_owned(), |n| n.to_string());
        let column = issue.column.map_or(String::new(), |n| format!(":{n}"));
        println!(
            "{}:{}{} [{}] {}",
            issue.filename, line, column, issue.check, issue.message
        );
    }
}

/// One JSON object per issue plus a summary line.
fn print_json(files: &[qredo::RunnerFile], report: &qredo::RunReport) {
    for issue in &report.issues {
        println!(
            "{}",
            serde_json::json!({
                "check": issue.check,
                "filename": issue.filename,
                "line_no": issue.line_no,
                "column": issue.column,
                "message": issue.message,
                "priority": issue.priority,
            })
        );
    }
    println!(
        "{}",
        serde_json::json!({
            "summary": {
                "files": files.len(),
                "issues": report.issues.len(),
                "exit": report.exit_status,
                "errors": report.errors.len(),
                "skipped_invalid": report.skipped_invalid,
            }
        })
    );
}

fn main() {
    std::process::exit(run());
}

/// Execute the CLI, returning the process exit code.
fn run() -> i32 {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            return 64;
        }
    };
    let config_path = args
        .config_file
        .clone()
        .unwrap_or_else(|| args.approot.join(".credo.exs"));
    let config_source = match std::fs::read_to_string(&config_path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "cannot read config {}: {error}",
                config_path.to_string_lossy()
            );
            return 2;
        }
    };
    let files = read_inputs(&args.approot);
    let min_priority = if args.strict { -99 } else { 0 };
    match qredo::integration::execute(&config_source, "default", &files, min_priority) {
        Err(fallback) => {
            eprintln!("unsupported config: {}", fallback.reason);
            2
        }
        Ok(report) => {
            for error in &report.errors {
                eprintln!("error: {error:?}");
            }
            match args.format {
                Format::Text => print_text(&report),
                Format::Json => print_json(&files, &report),
            }
            if !report.skipped_invalid.is_empty() {
                eprintln!("skipped invalid: {}", report.skipped_invalid.join(", "));
            }
            report.exit_status
        }
    }
}

/// Read `lib/` + `test/` sources under the approot.
fn read_inputs(approot: &Path) -> Vec<qredo::RunnerFile> {
    let mut named = Vec::new();
    collect(&approot.join("lib"), approot, &mut named);
    collect(&approot.join("test"), approot, &mut named);
    named
        .iter()
        .map(|(relative, path)| qredo::RunnerFile {
            filename: relative.clone(),
            source: std::fs::read_to_string(path).unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn defaults_to_current_dir_text() {
        let parsed = parse_args(&args(&[])).expect("parses");
        assert_eq!(parsed.approot, PathBuf::from("."));
        assert_eq!(parsed.format, Format::Text);
        assert!(!parsed.strict);
        assert_eq!(parsed.config_file, None);
    }

    #[test]
    fn parses_all_options() {
        let parsed = parse_args(&args(&[
            "apps/web",
            "--config-file",
            "custom.credo.exs",
            "--strict",
            "--format",
            "json",
        ]))
        .expect("parses");
        assert_eq!(parsed.approot, PathBuf::from("apps/web"));
        assert_eq!(parsed.config_file, Some(PathBuf::from("custom.credo.exs")));
        assert!(parsed.strict);
        assert_eq!(parsed.format, Format::Json);
    }

    #[test]
    fn rejects_unknown_arguments() {
        assert!(parse_args(&args(&["--nope"])).is_err());
        assert!(parse_args(&args(&["a", "b"])).is_err());
        assert!(parse_args(&args(&["--format", "yaml"])).is_err());
        assert!(parse_args(&args(&["--config-file"])).is_err());
    }
}
