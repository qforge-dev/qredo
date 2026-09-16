//! `qredo`: native Elixir linting with Credo-compatible behavior.
//!
//! Usage: `qredo [PATH] [options]`
//!
//! Reads the static `.credo.exs` subset qredo serves, lints the configured
//! files (`.ex`/`.exs`, sorted, project-relative names) and prints one line
//! per issue. Configs outside the served subset fail closed with an explicit
//! reason and exit code 2 instead of running partial analysis. Exit status
//! is the OR-combined issue status (`0` when clean), like Credo.

use std::path::{Path, PathBuf};

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    approot: PathBuf,
    config_file: Option<PathBuf>,
    config_name: String,
    strict: bool,
    min_priority: Option<i32>,
    mute_exit_status: bool,
    only: Vec<String>,
    ignore: Vec<String>,
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
    "usage: qredo [PATH] [--config-file FILE] [--config-name NAME] [--strict] [--min-priority N] [--mute-exit-status] [--only CHECK,...] [--ignore CHECK,...] [--format text|json]"
}

/// Split a comma-separated flag value, dropping empties.
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Parse argv (without the program name).
fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut args = Args {
        approot: PathBuf::from("."),
        config_file: None,
        config_name: "default".to_owned(),
        strict: false,
        min_priority: None,
        mute_exit_status: false,
        only: Vec::new(),
        ignore: Vec::new(),
        format: Format::Text,
    };
    let mut positional = Vec::new();
    let mut iter = raw.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg.starts_with('-') {
            parse_flag(&mut args, arg, &mut iter)?;
        } else {
            positional.push(arg);
        }
    }
    if positional.len() > 1 {
        return Err(usage().to_owned());
    }
    if let Some(root) = positional.first() {
        args.approot = PathBuf::from(root);
    }
    Ok(args)
}

/// Apply one `--flag` (driving the value iterator for flags with values).
fn parse_flag(
    args: &mut Args,
    arg: &str,
    iter: &mut std::iter::Peekable<std::slice::Iter<'_, String>>,
) -> Result<(), String> {
    match arg {
        "--strict" => args.strict = true,
        "--mute-exit-status" => args.mute_exit_status = true,
        "--config-file" => {
            let value = take_value(iter, "--config-file")?;
            args.config_file = Some(PathBuf::from(value));
        }
        "--config-name" => {
            args.config_name = take_value(iter, "--config-name")?;
        }
        "--min-priority" => {
            let value = take_value(iter, "--min-priority")?;
            args.min_priority = Some(
                value
                    .parse::<i32>()
                    .map_err(|_| format!("invalid --min-priority `{value}`"))?,
            );
        }
        "--only" => {
            args.only.extend(split_list(&take_value(iter, "--only")?));
        }
        "--ignore" => {
            args.ignore
                .extend(split_list(&take_value(iter, "--ignore")?));
        }
        "--format" => {
            let value = take_value(iter, "--format")?;
            args.format = match value.as_str() {
                "text" => Format::Text,
                "json" => Format::Json,
                _ => return Err(format!("unknown --format `{value}`")),
            };
        }
        "--help" | "-h" => return Err(usage().to_owned()),
        _ => {
            return Err(format!(
                "unknown argument `{arg}`\n{usage}",
                usage = usage()
            ));
        }
    }
    Ok(())
}

/// Next argv value for a flag that requires one.
fn take_value(
    iter: &mut std::iter::Peekable<std::slice::Iter<'_, String>>,
    flag: &str,
) -> Result<String, String> {
    iter.next()
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// Directories never descended into during discovery.
const PRUNED_DIRS: &[&str] = &["_build", "deps", "node_modules"];

/// Sorted recursive collection of `.ex`/`.exs` files with project-relative
/// names, skipping build artefacts and hidden directories.
fn collect(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name.starts_with('.') || PRUNED_DIRS.contains(&name.as_str()) {
                continue;
            }
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

/// One config file entry against a project-relative filename.
fn entry_matches(entry: &qredo::FileEntry, filename: &str) -> bool {
    match entry {
        qredo::FileEntry::Glob(pattern) => {
            qredo::wildcard_match(pattern, filename).unwrap_or(false)
        }
        qredo::FileEntry::Regex(source) => regex::Regex::new(source)
            .map(|expression| expression.is_match(filename))
            .unwrap_or(false),
    }
}

/// Files the run covers: config `files.included` (defaulting to `lib/`
/// and `test/` when the config lists none) minus `files.excluded`.
fn discover(
    approot: &Path,
    included: &[String],
    excluded: &[qredo::FileEntry],
) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    if included.is_empty() {
        collect(&approot.join("lib"), approot, &mut found);
        collect(&approot.join("test"), approot, &mut found);
    } else {
        collect(approot, approot, &mut found);
        found.retain(|(relative, _)| {
            included
                .iter()
                .any(|pattern| qredo::wildcard_match(pattern, relative).unwrap_or(false))
        });
    }
    found.retain(|(relative, _)| !excluded.iter().any(|entry| entry_matches(entry, relative)));
    found
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
    let selection = qredo::Selection {
        only: args.only.clone(),
        ignore: args.ignore.clone(),
    };
    if let Err(invalid) = selection.validate() {
        eprintln!("invalid selection: {invalid}");
        return 2;
    }
    // Discovery needs the static config; an unreadable one fails here
    // with the same explicit reason as the pipeline below.
    let (included, excluded) = match qredo::parse_config(&config_source, &args.config_name) {
        Ok(config) => (config.files_included, config.files_excluded),
        Err(unsupported) => {
            eprintln!("unsupported config: {}", unsupported.0);
            return 2;
        }
    };
    let files = read_inputs(&args.approot, &included, &excluded);
    report_exit(&args, &files, &config_source, selection)
}

/// Run the pipeline over discovered files and print the report,
/// returning the process exit code.
fn report_exit(
    args: &Args,
    files: &[qredo::RunnerFile],
    config_source: &str,
    selection: qredo::Selection,
) -> i32 {
    let min_priority = args
        .min_priority
        .unwrap_or(if args.strict { -99 } else { 0 });
    match qredo::integration::execute_selected(
        config_source,
        &args.config_name,
        files,
        min_priority,
        selection,
    ) {
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
                Format::Json => print_json(files, &report),
            }
            if !report.skipped_invalid.is_empty() {
                eprintln!("skipped invalid: {}", report.skipped_invalid.join(", "));
            }
            if !report.errors.is_empty() {
                return 2;
            }
            if args.mute_exit_status {
                0
            } else {
                report.exit_status
            }
        }
    }
}

/// Read configured sources under the approot.
fn read_inputs(
    approot: &Path,
    included: &[String],
    excluded: &[qredo::FileEntry],
) -> Vec<qredo::RunnerFile> {
    discover(approot, included, excluded)
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
        assert_eq!(parsed.min_priority, None);
        assert!(!parsed.mute_exit_status);
        assert!(parsed.only.is_empty());
        assert!(parsed.ignore.is_empty());
        assert_eq!(parsed.config_name, "default");
        assert_eq!(parsed.config_file, None);
    }

    #[test]
    fn parses_all_options() {
        let parsed = parse_args(&args(&[
            "apps/web",
            "--config-file",
            "custom.credo.exs",
            "--config-name",
            "ci",
            "--strict",
            "--min-priority",
            "5",
            "--mute-exit-status",
            "--only",
            "IoInspect,Dbg",
            "--ignore",
            "TagTODO",
            "--format",
            "json",
        ]))
        .expect("parses");
        assert_eq!(parsed.approot, PathBuf::from("apps/web"));
        assert_eq!(parsed.config_file, Some(PathBuf::from("custom.credo.exs")));
        assert_eq!(parsed.config_name, "ci");
        assert!(parsed.strict);
        assert_eq!(parsed.min_priority, Some(5));
        assert!(parsed.mute_exit_status);
        assert_eq!(parsed.only, vec!["IoInspect".to_owned(), "Dbg".to_owned()]);
        assert_eq!(parsed.ignore, vec!["TagTODO".to_owned()]);
        assert_eq!(parsed.format, Format::Json);
    }

    #[test]
    fn rejects_unknown_arguments() {
        assert!(parse_args(&args(&["--nope"])).is_err());
        assert!(parse_args(&args(&["a", "b"])).is_err());
        assert!(parse_args(&args(&["--format", "yaml"])).is_err());
        assert!(parse_args(&args(&["--config-file"])).is_err());
        assert!(parse_args(&args(&["--min-priority", "high"])).is_err());
    }

    #[test]
    fn split_list_drops_empties() {
        assert!(split_list("").is_empty());
        assert_eq!(split_list("a,, b ,"), vec!["a".to_owned(), "b".to_owned()]);
    }

    /// Discovery honors `files.included` over the conventional default:
    /// a config including only `lib/` never reads `test/`, and excluded
    /// paths are dropped.
    #[test]
    fn discover_respects_included_and_excluded() {
        let root = std::env::temp_dir().join("qredo-discover-test");
        let _ = std::fs::remove_dir_all(&root);
        for file in ["lib/a.ex", "lib/old/b.ex", "test/a_test.exs"] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&path, "x = 1\n").expect("write");
        }
        let found = discover(
            &root,
            &["lib/".to_owned()],
            &[qredo::FileEntry::Glob("lib/old/".to_owned())],
        );
        let names: Vec<&str> = found
            .iter()
            .map(|(relative, _)| relative.as_str())
            .collect();
        assert_eq!(names, vec!["lib/a.ex"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discover_defaults_to_lib_and_test() {
        let root = std::env::temp_dir().join("qredo-discover-default-test");
        let _ = std::fs::remove_dir_all(&root);
        for file in ["lib/a.ex", "test/a_test.exs", "other/b.ex"] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&path, "x = 1\n").expect("write");
        }
        let found = discover(&root, &[], &[]);
        let mut names: Vec<&str> = found
            .iter()
            .map(|(relative, _)| relative.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["lib/a.ex", "test/a_test.exs"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
