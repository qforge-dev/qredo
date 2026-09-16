//! `qredo`: native Elixir linting with Credo-compatible behavior.
//!
//! Usage: `qredo [command|paths...] [options]` (default command `suggest`).
//!
//! Reads the static `.credo.exs` subset qredo serves, lints the selected
//! files and prints issues. Configs outside the served subset fail closed
//! with an explicit reason and exit code 2 instead of running partial
//! analysis. Exit status is the OR-combined issue status (`0` when clean),
//! matching `mix credo suggest`; error codes mirror it too (129 missing
//! or malformed config, 1 crash, 130 invalid options, 2 qredo refusals).

use std::path::{Path, PathBuf};

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    command: Command,
}

/// Top-level command. The first word names a subcommand when it is a
/// known command; anything else falls through to `suggest` with
/// positional paths, mirroring native dispatch.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Suggest(Box<SuggestArgs>),
    Version,
    Help,
    SuggestHelp,
    Unimplemented(&'static str),
}

/// Native subcommands by implementation status.
fn lookup_command(word: &str) -> Option<Command> {
    match word {
        "suggest" => Some(Command::Suggest(Box::default())),
        "version" => Some(Command::Version),
        "help" => Some(Command::Help),
        "list" | "categories" | "info" | "explain" | "diff" | "gen.check" | "gen.config" => Some(
            Command::Unimplemented("not yet implemented in this preview"),
        ),
        _ => None,
    }
}

/// `suggest` options. One bool per CLI switch, mirroring native flags.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SuggestArgs {
    paths: Vec<String>,
    working_dir: Option<PathBuf>,
    config_file: Option<PathBuf>,
    config_name: String,
    strict: bool,
    all_priorities: bool,
    all: bool,
    min_priority: Option<String>,
    mute_exit_status: bool,
    only: Vec<String>,
    ignore: Vec<String>,
    checks_with_tag: Vec<String>,
    checks_without_tag: Vec<String>,
    enable_disabled: Vec<String>,
    files_included: Vec<String>,
    files_excluded: Vec<String>,
    color: Option<bool>,
    format: Format,
}

impl Default for SuggestArgs {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            working_dir: None,
            config_file: None,
            config_name: "default".to_owned(),
            strict: false,
            all_priorities: false,
            all: false,
            min_priority: None,
            mute_exit_status: false,
            only: Vec::new(),
            ignore: Vec::new(),
            checks_with_tag: Vec::new(),
            checks_without_tag: Vec::new(),
            enable_disabled: Vec::new(),
            files_included: Vec::new(),
            files_excluded: Vec::new(),
            color: None,
            format: Format::Default,
        }
    }
}

/// Output format. Unknown names fall back to `Default` silently, like native.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Format {
    #[default]
    Default,
    Oneline,
    Flycheck,
    Json,
    Sarif,
}

/// General help text.
fn help_text() -> &'static str {
    "qredo: native Elixir linting with Credo-compatible behavior\n\
     \n\
     Usage: qredo [command] [paths...] [options] (default command `suggest`)\n\
     \n\
     Commands:\n\
     \n\
     suggest     Suggest code objects to look at next (default)\n\
     list        List all issues grouped by files\n\
     version     Show qredo's version number\n\
     help        Show this help message\n\
     \n\
     Use `qredo suggest --help` for suggest options.\n"
}

/// Suggest-command help text.
fn suggest_help_text() -> &'static str {
    "Usage: qredo suggest [paths...] [options]\n\
     \n\
     Suggests objects from every category that qredo thinks can be improved.\n\
     \n\
     Suggest options:\n\
       --config-file FILE        Use the given config file\n\
       --config-name NAME        Use the given config instead of \"default\"\n\
       --strict                  Alias for --all-priorities\n\
       --all-priorities          Show all issues including low priority ones\n\
       --min-priority LEVEL      Minimum priority (higher,high,normal,low,ignore or number)\n\
       --mute-exit-status        Exit with status zero even if there are issues\n\
       --only CHECKS             Only include checks matching the given strings\n\
       --ignore CHECKS           Ignore checks matching the given strings\n\
       --format FORMAT           Display format (json,oneline,flycheck,sarif)\n"
}

/// qredo version line.
fn version_text() -> String {
    format!("{}\n", env!("CARGO_PKG_VERSION"))
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

/// `** (credo)`-shaped unknown-switch error for one command.
fn unknown_switch(command: &str, flag: &str) -> String {
    format!("** (credo) Unknown switch for `{command}` command: {flag}")
}

/// Parse errors with their process exit codes.
#[derive(Debug, PartialEq, Eq)]
enum ParseError {
    /// Unknown switch or usage problem: exit 130 like native.
    InvalidOption(String),
}

/// Parse argv (without the program name).
fn parse_args(raw: &[String]) -> Result<Args, ParseError> {
    if raw.iter().any(|arg| arg == "-v" || arg == "--version") {
        return Ok(Args {
            command: Command::Version,
        });
    }
    let words: Vec<String> = raw.to_vec();
    // `-h`/`--help` alongside suggest resolves to suggest help; strip
    // before flag parsing so it never reads as an unknown switch.
    let command = match words.first() {
        None => Command::Suggest(Box::default()),
        Some(word) if word.starts_with('-') => {
            let (words, help) = strip_help(words);
            if help {
                Command::SuggestHelp
            } else {
                Command::Suggest(Box::new(parse_suggest_vec(words)?))
            }
        }
        Some(word) => match lookup_command(word) {
            Some(Command::Suggest(_)) => {
                let (rest, help) = strip_help(words[1..].to_vec());
                if help {
                    Command::SuggestHelp
                } else {
                    Command::Suggest(Box::new(parse_suggest_vec(rest)?))
                }
            }
            Some(other) => other,
            None => {
                let (words, help) = strip_help(words);
                if help {
                    Command::SuggestHelp
                } else {
                    Command::Suggest(Box::new(parse_suggest_vec(words)?))
                }
            }
        },
    };
    Ok(Args { command })
}

/// Split `-h`/`--help` out of suggest words, reporting its presence.
fn strip_help(words: Vec<String>) -> (Vec<String>, bool) {
    let mut help = false;
    let kept = words
        .into_iter()
        .filter(|word| {
            if word == "-h" || word == "--help" {
                help = true;
                false
            } else {
                true
            }
        })
        .collect();
    (kept, help)
}

/// Parse suggest flags and positionals from owned words.
fn parse_suggest_vec(words: Vec<String>) -> Result<SuggestArgs, ParseError> {
    let mut args = SuggestArgs::default();
    let mut words = words.into_iter().peekable();
    let mut positional = Vec::new();
    while let Some(word) = words.next() {
        if word.starts_with('-') {
            parse_suggest_flag(&mut args, &word, &mut words)?;
        } else {
            positional.push(word);
        }
    }
    args.paths = positional;
    Ok(args)
}

/// Apply one suggest `--flag` (driving the value iterator for valued flags).
fn parse_suggest_flag(
    args: &mut SuggestArgs,
    flag: &str,
    words: &mut std::iter::Peekable<impl Iterator<Item = String>>,
) -> Result<(), ParseError> {
    match flag {
        "--strict" => args.strict = true,
        "--all-priorities" | "-A" => args.all_priorities = true,
        "--all" | "-a" => args.all = true,
        "--mute-exit-status" => args.mute_exit_status = true,
        "--color" => args.color = Some(true),
        "--no-color" => args.color = Some(false),
        "--format" => {
            let value = take_value(words, "suggest", flag)?;
            args.format = match value.as_str() {
                "json" | "jsonl" => Format::Json,
                "oneline" => Format::Oneline,
                "flycheck" => Format::Flycheck,
                "sarif" => Format::Sarif,
                _ => Format::Default,
            };
        }
        "--config-file"
        | "--config-name"
        | "-C"
        | "--working-dir"
        | "--min-priority"
        | "--only"
        | "--checks"
        | "-c"
        | "--ignore"
        | "--ignore-checks"
        | "-i"
        | "--checks-with-tag"
        | "--checks-without-tag"
        | "--enable-disabled-checks"
        | "--files-included"
        | "--files-excluded" => {
            parse_valued_flag(args, flag, words)?;
        }
        _ => {
            return Err(ParseError::InvalidOption(unknown_switch("suggest", flag)));
        }
    }
    Ok(())
}

/// Apply one suggest flag that takes a comma-separated or path value.
fn parse_valued_flag(
    args: &mut SuggestArgs,
    flag: &str,
    words: &mut std::iter::Peekable<impl Iterator<Item = String>>,
) -> Result<(), ParseError> {
    match flag {
        "--config-file" => {
            args.config_file = Some(PathBuf::from(take_value(words, "suggest", flag)?));
        }
        "--config-name" | "-C" => {
            args.config_name = take_value(words, "suggest", flag)?;
        }
        "--working-dir" => {
            args.working_dir = Some(PathBuf::from(take_value(words, "suggest", flag)?));
        }
        "--min-priority" => {
            args.min_priority = Some(take_value(words, "suggest", flag)?);
        }
        "--only" | "--checks" | "-c" => {
            args.only
                .extend(split_list(&take_value(words, "suggest", flag)?));
        }
        "--ignore" | "--ignore-checks" | "-i" => {
            args.ignore
                .extend(split_list(&take_value(words, "suggest", flag)?));
        }
        "--checks-with-tag" => {
            args.checks_with_tag
                .extend(split_list(&take_value(words, "suggest", flag)?));
        }
        "--checks-without-tag" => {
            // Accepted for compatibility; native ignores this flag.
            args.checks_without_tag
                .extend(split_list(&take_value(words, "suggest", flag)?));
        }
        "--enable-disabled-checks" => {
            args.enable_disabled
                .extend(split_list(&take_value(words, "suggest", flag)?));
        }
        "--files-included" => {
            // Accepted for compatibility; native ignores it for `suggest`.
            args.files_included
                .extend(split_list(&take_value(words, "suggest", flag)?));
        }
        "--files-excluded" => {
            args.files_excluded
                .extend(split_list(&take_value(words, "suggest", flag)?));
        }
        _ => {
            return Err(ParseError::InvalidOption(unknown_switch("suggest", flag)));
        }
    }
    Ok(())
}

/// Next argv value for a flag that requires one; a missing value reads
/// as an unknown switch, mirroring native behavior.
fn take_value<I>(
    words: &mut std::iter::Peekable<I>,
    command: &str,
    flag: &str,
) -> Result<String, ParseError>
where
    I: Iterator<Item = String>,
{
    words
        .next()
        .ok_or_else(|| ParseError::InvalidOption(unknown_switch(command, flag)))
}

/// Resolve the effective minimum priority: an explicit value beats
/// `--strict`/`--all-priorities` regardless of flag order.
fn resolve_min_priority(args: &SuggestArgs) -> Result<i32, String> {
    if let Some(raw) = &args.min_priority {
        return match raw.as_str() {
            "higher" => Ok(20),
            "high" => Ok(10),
            "normal" => Ok(1),
            "low" => Ok(-10),
            "ignore" => Ok(-100),
            _ => raw.parse::<i32>().map_err(|_| invalid_priority(raw)),
        };
    }
    if args.strict || args.all_priorities {
        Ok(-99)
    } else {
        Ok(0)
    }
}

/// Native crash shape for an invalid priority value.
fn invalid_priority(raw: &str) -> String {
    format!(
        "** (RuntimeError) Got an invalid priority: :{raw}  (valid are numbers or higher/high/normal/low/ignore)"
    )
}

/// Pre-compile selection regexes like native option parsing: an invalid
/// pattern crashes before any analysis.
fn compile_selection(only: &[String], ignore: &[String]) -> Result<(), String> {
    for pattern in only.iter().chain(ignore.iter()) {
        if regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .is_err()
        {
            return Err(match_error(pattern));
        }
    }
    Ok(())
}

/// Native crash shape for an invalid match pattern.
fn match_error(pattern: &str) -> String {
    format!(
        "** (MatchError) no match of right hand side value:\n\n    {pattern:?} is not a valid regular expression"
    )
}

/// Missing config file shape with exit code.
fn missing_config(path: &Path) -> (String, i32) {
    (
        format!(
            "** (config) Given config file does not exist:\n  {}",
            path.to_string_lossy()
        ),
        129,
    )
}

/// Missing-file crash shape: first deterministic line only (native
/// appends process-specific stack frames).
fn unreadable_file(path: &Path) -> String {
    format!(
        "** (File.Error) could not read file \"{}\": no such file or directory",
        path.to_string_lossy()
    )
}

/// Lexically absolute path without touching symlinks (native expands
/// paths the same way for display).
fn absolutize(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut out = PathBuf::new();
    for part in joined.components() {
        use std::path::Component::{CurDir, Normal, ParentDir, Prefix, RootDir};
        match part {
            Prefix(prefix) => out.push(prefix.as_os_str()),
            RootDir => out.push("/"),
            CurDir => {}
            ParentDir => {
                out.pop();
            }
            Normal(text) => out.push(text),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
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

/// Resolve positional patterns to absolute display paths. Literals that
/// exist are taken as-is (dirs recurse, any extension); globs expand;
/// a missing `*.ex`/`*.exs` literal crashes like native; anything else
/// unmatched contributes nothing.
fn resolve_positionals(root: &Path, patterns: &[String]) -> Result<Vec<PathBuf>, PathBuf> {
    let mut found = Vec::new();
    for pattern in patterns {
        if has_magic(pattern) {
            found.extend(expand_glob(root, pattern));
        } else {
            let path = absolutize(root, Path::new(pattern));
            match std::fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => collect_dir_files(&path, &mut found),
                Ok(_) => found.push(path),
                Err(_) => {
                    if is_elixir_path(pattern) {
                        return Err(path);
                    }
                }
            }
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
}

/// True for glob patterns (magic characters).
fn has_magic(pattern: &str) -> bool {
    pattern.contains(['*', '?', '['])
}

/// True for `.ex`/`.exs` paths (crash candidates when missing).
/// Extensions are lowercase-only, matching native file discovery.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn is_elixir_path(pattern: &str) -> bool {
    pattern.ends_with(".ex") || pattern.ends_with(".exs")
}

/// Expand one glob against the filesystem, mirroring native wildcard
/// semantics over absolute paths (plus the relative form as given).
fn expand_glob(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let walk_root = static_prefix(root, pattern);
    let mut candidates = Vec::new();
    collect_all(&walk_root, &mut candidates);
    candidates
        .into_iter()
        .filter(|path| {
            let text = path.to_string_lossy().into_owned();
            qredo::wildcard_match(pattern, &text).unwrap_or(false)
        })
        .collect()
}

/// Longest leading static directory of a glob pattern.
fn static_prefix(root: &Path, pattern: &str) -> PathBuf {
    let absolute = absolutize(root, Path::new(pattern));
    let mut prefix = PathBuf::from("/");
    for part in absolute.parent().unwrap_or(Path::new("/")).components() {
        use std::path::Component::Normal;
        match part {
            Normal(text) if has_magic(&text.to_string_lossy()) => break,
            Normal(text) => prefix.push(text),
            _ => {}
        }
    }
    if prefix.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        prefix
    }
}

/// Sorted recursive collection of every file (any extension).
fn collect_all(dir: &Path, out: &mut Vec<PathBuf>) {
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
            collect_all(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Sorted recursive collection of `.ex`/`.exs` files (absolute paths).
fn collect_dir_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut named = Vec::new();
    collect(dir, dir, &mut named);
    out.extend(named.into_iter().map(|(_, path)| path));
}

/// Files the run covers: positional patterns win over the config
/// `files.included` set (defaulting to `lib/` and `test/`), minus
/// `files.excluded`. Display names stay root-relative (matching the
/// differential contract); files outside the root keep absolute names.
fn discover(
    root: &Path,
    patterns: &[String],
    included: &[String],
    excluded: &[qredo::FileEntry],
) -> Result<Vec<(String, PathBuf)>, PathBuf> {
    let mut named: Vec<(String, PathBuf)> = if patterns.is_empty() {
        let mut collected = Vec::new();
        if included.is_empty() {
            collect(&root.join("lib"), root, &mut collected);
            collect(&root.join("test"), root, &mut collected);
        } else {
            collect_included(root, included, &mut collected);
        }
        collected
    } else {
        resolve_positionals(root, patterns)?
            .into_iter()
            .map(|path| (display_name(root, &path), path))
            .collect()
    };
    named.retain(|(relative, path)| {
        let absolute = path.to_string_lossy().into_owned();
        !excluded
            .iter()
            .any(|entry| entry_matches(entry, relative) || entry_matches(entry, &absolute))
    });
    named.sort();
    named.dedup_by(|a, b| a.1 == b.1);
    Ok(named)
}

/// Collect config `files.included` entries: plain directories are walked
/// directly (absolute or root-relative, inside or outside the root),
/// globs expand, and `.ex`/`.exs` files include literally. Mirrors native,
/// where included entries are search roots rather than filters.
fn collect_included(root: &Path, included: &[String], out: &mut Vec<(String, PathBuf)>) {
    for pattern in included {
        if has_magic(pattern) && Path::new(pattern).is_absolute() {
            for path in expand_glob(root, pattern) {
                out.push((display_name(root, &path), path));
            }
            continue;
        }
        if has_magic(pattern) {
            let mut walked = Vec::new();
            collect(root, root, &mut walked);
            walked
                .retain(|(relative, _)| qredo::wildcard_match(pattern, relative).unwrap_or(false));
            out.extend(walked);
            continue;
        }
        let candidate = absolutize(root, Path::new(pattern));
        match std::fs::metadata(&candidate) {
            Ok(meta) if meta.is_dir() => collect(&candidate, root, out),
            Ok(_) if is_elixir_path(pattern) => {
                out.push((display_name(root, &candidate), candidate));
            }
            Ok(_) | Err(_) => {}
        }
    }
    out.sort();
    out.dedup_by(|a, b| a.1 == b.1);
}

/// Display name for a discovered file: root-relative when inside the
/// root, absolute otherwise.
fn display_name(root: &Path, path: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(root) {
        relative.to_string_lossy().into_owned()
    } else {
        path.to_string_lossy().into_owned()
    }
}

/// Search upward from `dir` for `.credo.exs`.
fn discover_config(dir: &Path) -> Option<PathBuf> {
    let mut current = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(dir)
    };
    loop {
        let candidate = current.join(".credo.exs");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Rendered stdout for one `suggest` run: native-shape formatters over
/// absolute filenames (native resolves display paths absolutely).
fn print_report(
    args: &SuggestArgs,
    files: &[qredo::RunnerFile],
    report: &qredo::RunReport,
    context: &ReportContext,
) {
    match args.format {
        Format::Default => {
            let context = qredo::format_default::FormatContext {
                check_count: context.check_count,
                load_microseconds: context.load_microseconds,
                run_microseconds: context.run_microseconds,
                color: context.color,
                show_all: context.show_all,
                strict_hint: context.strict_hint,
                locale_utf8: context.locale_utf8,
            };
            print!("{}", qredo::format_default::render(report, files, &context));
        }
        Format::Oneline | Format::Flycheck | Format::Json | Format::Sarif => {
            let absolute = absolutized(report, &context.root);
            let machine = qredo::format_machine::MachineContext::new(context.root.clone());
            let text = match args.format {
                Format::Oneline => qredo::format_machine::render_oneline(&absolute, &machine),
                Format::Flycheck => qredo::format_machine::render_flycheck(&absolute, &machine),
                Format::Json => qredo::format_machine::render_json(&absolute, &machine),
                Format::Sarif => qredo::format_machine::render_sarif(&absolute, &machine),
                Format::Default => unreachable!("covered above"),
            };
            print!("{text}");
        }
    }
}

/// Report inputs the formatters need beyond issues and files: one bool
/// per display switch, mirroring native output flags.
#[allow(clippy::struct_excessive_bools)]
struct ReportContext {
    root: PathBuf,
    check_count: usize,
    load_microseconds: u64,
    run_microseconds: u64,
    color: bool,
    show_all: bool,
    strict_hint: bool,
    locale_utf8: bool,
}

/// Clone a report with root-absolute filenames for native-shape output.
fn absolutized(report: &qredo::RunReport, root: &Path) -> qredo::RunReport {
    let mut absolute = report.clone();
    for issue in &mut absolute.issues {
        let path = Path::new(&issue.filename);
        if !path.is_absolute() {
            issue.filename = root.join(path).to_string_lossy().into_owned();
        }
    }
    absolute
}

/// Saturating wall-time microseconds for timing lines: the `min` bounds
/// the value before conversion.
#[allow(clippy::cast_possible_truncation)]
fn micros(elapsed: std::time::Duration) -> u64 {
    elapsed.as_micros().min(u128::from(u64::MAX)) as u64
}

/// TTY color only: piped output never carries escapes, like native.
fn use_color(args: &SuggestArgs) -> bool {
    args.color
        .unwrap_or_else(|| std::io::IsTerminal::is_terminal(&std::io::stdout()))
}

/// C/POSIX locales render `\x{HEX}` escapes instead of Unicode glyphs.
fn utf8_locale() -> bool {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if value.is_empty() {
                continue;
            }
            return value != "C" && value != "POSIX" && !value.starts_with("C.");
        }
    }
    true
}

/// Checks the timing line reports: enabled, selected, version- and
/// priority-gated — the same predicates the runner applies.
fn check_count(
    config: &qredo::CredoConfig,
    selection: &qredo::Selection,
    min_priority: i32,
) -> usize {
    qredo::integration::enabled_modules(config, selection)
        .iter()
        .filter(|module| selection.should_run(module))
        .filter(|module| !qredo::version_skipped_on_pinned_toolchain(module))
        .filter(|module| qredo::runs_at_min_priority(module, min_priority))
        .count()
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run_with(&raw));
}

/// Execute the CLI over explicit argv, returning the exit code.
fn run_with(argv: &[String]) -> i32 {
    let parsed = match parse_args(argv) {
        Ok(parsed) => parsed,
        Err(ParseError::InvalidOption(message)) => {
            eprintln!("{message}");
            return 130;
        }
    };
    match parsed.command {
        Command::Version => {
            print!("{}", version_text());
            0
        }
        Command::Help => {
            print!("{help}", help = help_text());
            0
        }
        Command::SuggestHelp => {
            print!("{help}", help = suggest_help_text());
            0
        }
        Command::Unimplemented(name) => {
            eprintln!("{name}: not yet implemented in this preview");
            2
        }
        Command::Suggest(suggest) => run_suggest(&suggest),
    }
}

/// Execute one `suggest` run, returning the exit code.
fn run_suggest(args: &SuggestArgs) -> i32 {
    let Some(root) = working_root(args) else {
        return 2;
    };
    // A leading existing directory becomes the resolution root,
    // mirroring native behavior; the rest are file patterns.
    let (root, patterns) = split_root(&root, &args.paths);
    let Some(config_path) = config_path(args, &root) else {
        return 2;
    };
    if !config_path.is_file() {
        let (message, code) = missing_config(&config_path);
        eprintln!("{message}");
        return code;
    }
    let Some(config_source) = read_config(&config_path) else {
        return 2;
    };
    let Some(selection) = load_selection(args) else {
        return 1;
    };
    // Discovery needs the static config; an unreadable one fails here
    // with the same explicit reason as the pipeline below.
    let config = match qredo::parse_config(&config_source, &args.config_name) {
        Ok(config) => config,
        Err(unsupported) => {
            eprintln!("warning: failed to parse config file: {}", unsupported.0);
            return 129;
        }
    };
    let load_start = std::time::Instant::now();
    let found = match discover(
        &root,
        &patterns,
        &config.files_included,
        &config.files_excluded,
    ) {
        Ok(found) => found,
        Err(missing) => {
            eprintln!("{}", unreadable_file(&missing));
            return 1;
        }
    };
    let files = read_found(&found);
    let load_microseconds = micros(load_start.elapsed());
    match resolve_min_priority(args) {
        Ok(_) => {}
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    }
    let context = report_context(args, &config, &selection, root, load_microseconds);
    report_exit(args, &files, &config_source, selection, context)
}

/// CLI check selection with native regex pre-compilation: an invalid
/// pattern fails before any analysis.
fn load_selection(args: &SuggestArgs) -> Option<qredo::Selection> {
    if let Err(message) = compile_selection(&args.only, &args.ignore) {
        eprintln!("{message}");
        return None;
    }
    Some(qredo::Selection {
        only: args.only.clone(),
        ignore: args.ignore.clone(),
        checks_with_tag: args.checks_with_tag.clone(),
        enable_disabled: args.enable_disabled.clone(),
    })
}

/// Formatter inputs for one run: check counts mirror the runner gates.
fn report_context(
    args: &SuggestArgs,
    config: &qredo::CredoConfig,
    selection: &qredo::Selection,
    root: PathBuf,
    load_microseconds: u64,
) -> ReportContext {
    let min_priority = resolve_min_priority(args).unwrap_or(0);
    ReportContext {
        check_count: check_count(config, selection, min_priority),
        load_microseconds,
        run_microseconds: 0,
        color: use_color(args),
        show_all: args.all || min_priority <= -99,
        strict_hint: min_priority < 0,
        locale_utf8: utf8_locale(),
        root,
    }
}

/// Resolution root: `--working-dir` or the process working directory.
fn working_root(args: &SuggestArgs) -> Option<PathBuf> {
    if let Some(dir) = &args.working_dir {
        return Some(dir.clone());
    }
    match std::env::current_dir() {
        Ok(dir) => Some(dir),
        Err(error) => {
            eprintln!("cannot read working directory: {error}");
            None
        }
    }
}

/// Config path: explicit `--config-file` or upward `.credo.exs` discovery.
fn config_path(args: &SuggestArgs, root: &Path) -> Option<PathBuf> {
    if let Some(path) = &args.config_file {
        return Some(path.clone());
    }
    if let Some(path) = discover_config(root) {
        return Some(path);
    }
    eprintln!(
        "no .credo.exs found walking up from {}",
        root.to_string_lossy()
    );
    None
}

/// Read the config file to source text.
fn read_config(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(source) => Some(source),
        Err(error) => {
            eprintln!("cannot read config {}: {error}", path.to_string_lossy());
            None
        }
    }
}

/// Read discovered files into runner inputs.
fn read_found(found: &[(String, PathBuf)]) -> Vec<qredo::RunnerFile> {
    found
        .iter()
        .map(|(filename, path)| qredo::RunnerFile {
            filename: filename.clone(),
            source: std::fs::read_to_string(path).unwrap_or_default(),
        })
        .collect()
}

/// Split a leading existing directory off the patterns as resolution root.
fn split_root(root: &Path, patterns: &[String]) -> (PathBuf, Vec<String>) {
    let mut patterns = patterns.to_vec();
    let mut root = root.to_path_buf();
    if let Some(first) = patterns.first() {
        let candidate = absolutize(&root, Path::new(first));
        if candidate.is_dir() {
            root = candidate;
            patterns.remove(0);
        }
    }
    (root, patterns)
}

/// Run the pipeline over discovered files and print the report,
/// returning the process exit code.
fn report_exit(
    args: &SuggestArgs,
    files: &[qredo::RunnerFile],
    config_source: &str,
    selection: qredo::Selection,
    mut context: ReportContext,
) -> i32 {
    let min_priority = match resolve_min_priority(args) {
        Ok(priority) => priority,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let run_start = std::time::Instant::now();
    let outcome = qredo::integration::execute_selected(
        config_source,
        &args.config_name,
        files,
        min_priority,
        selection,
    );
    context.run_microseconds = micros(run_start.elapsed());
    match outcome {
        Err(fallback) => {
            eprintln!("unsupported config: {}", fallback.reason);
            2
        }
        Ok(report) => {
            for error in &report.errors {
                eprintln!("error: {error:?}");
            }
            print_report(args, files, &report, &context);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    fn suggest(raw: &[&str]) -> SuggestArgs {
        match parse_args(&argv(raw)).expect("parses").command {
            Command::Suggest(boxed) => *boxed,
            other => panic!("expected suggest, got {other:?}"),
        }
    }

    #[test]
    fn bare_invocation_is_default_suggest() {
        let parsed = parse_args(&argv(&[])).expect("parses");
        assert_eq!(parsed.command, Command::Suggest(Box::default()));
    }

    #[test]
    fn unknown_first_word_is_a_suggest_path() {
        let parsed = suggest(&["frobnicate"]);
        assert_eq!(parsed.paths, vec!["frobnicate".to_owned()]);
    }

    #[test]
    fn explicit_suggest_command_parses_rest() {
        let parsed = suggest(&["suggest", "--strict", "lib"]);
        assert!(parsed.strict);
        assert_eq!(parsed.paths, vec!["lib".to_owned()]);
    }

    #[test]
    fn version_flag_wins_anywhere() {
        assert_eq!(
            parse_args(&argv(&["--strict", "-v"]))
                .expect("parses")
                .command,
            Command::Version
        );
        assert_eq!(
            parse_args(&argv(&["version"])).expect("parses").command,
            Command::Version
        );
    }

    #[test]
    fn help_variants() {
        assert_eq!(
            parse_args(&argv(&["help"])).expect("parses").command,
            Command::Help
        );
        assert_eq!(
            parse_args(&argv(&["--help"])).expect("parses").command,
            Command::SuggestHelp
        );
        assert_eq!(
            parse_args(&argv(&["suggest", "--help"]))
                .expect("parses")
                .command,
            Command::SuggestHelp
        );
    }

    #[test]
    fn future_commands_error_explicitly() {
        for command in [
            "list",
            "categories",
            "info",
            "explain",
            "diff",
            "gen.check",
            "gen.config",
        ] {
            assert!(
                matches!(
                    parse_args(&argv(&[command])).expect("parses").command,
                    Command::Unimplemented(_)
                ),
                "{command}"
            );
        }
    }

    #[test]
    fn parses_suggest_output_options() {
        let parsed = suggest(&[
            "apps/web",
            "--config-file",
            "custom.credo.exs",
            "--config-name",
            "ci",
            "--strict",
            "--all-priorities",
            "--all",
            "--min-priority",
            "5",
            "--mute-exit-status",
            "--color",
            "--format",
            "json",
        ]);
        assert_eq!(parsed.paths, vec!["apps/web".to_owned()]);
        assert_eq!(parsed.config_file, Some(PathBuf::from("custom.credo.exs")));
        assert_eq!(parsed.config_name, "ci");
        assert!(parsed.strict);
        assert!(parsed.all_priorities);
        assert!(parsed.all);
        assert_eq!(parsed.min_priority, Some("5".to_owned()));
        assert!(parsed.mute_exit_status);
        assert_eq!(parsed.color, Some(true));
        assert_eq!(parsed.format, Format::Json);
    }

    #[test]
    fn parses_suggest_selection_options() {
        let parsed = suggest(&[
            "--only",
            "IoInspect,Dbg",
            "--ignore",
            "TagTODO",
            "--checks-with-tag",
            "formatter",
            "--checks-without-tag",
            "controversial",
            "--enable-disabled-checks",
            "ModuleDoc",
            "--files-included",
            "lib/",
            "--files-excluded",
            "gen/",
        ]);
        assert_eq!(parsed.only, vec!["IoInspect".to_owned(), "Dbg".to_owned()]);
        assert_eq!(parsed.ignore, vec!["TagTODO".to_owned()]);
        assert_eq!(parsed.checks_with_tag, vec!["formatter".to_owned()]);
        assert_eq!(parsed.checks_without_tag, vec!["controversial".to_owned()]);
        assert_eq!(parsed.enable_disabled, vec!["ModuleDoc".to_owned()]);
        assert_eq!(parsed.files_included, vec!["lib/".to_owned()]);
        assert_eq!(parsed.files_excluded, vec!["gen/".to_owned()]);
    }

    #[test]
    fn short_flags_and_aliases() {
        let parsed = suggest(&["-a", "-A", "-c", "IoInspect", "-i", "Dbg", "-C", "ci"]);
        assert!(parsed.all);
        assert!(parsed.all_priorities);
        assert_eq!(parsed.only, vec!["IoInspect".to_owned()]);
        assert_eq!(parsed.ignore, vec!["Dbg".to_owned()]);
        assert_eq!(parsed.config_name, "ci");
    }

    #[test]
    fn rejects_unknown_switches_with_130_shape() {
        let error = parse_args(&argv(&["--nope"])).expect_err("rejects");
        assert_eq!(
            error,
            ParseError::InvalidOption(
                "** (credo) Unknown switch for `suggest` command: --nope".to_owned()
            )
        );
    }

    #[test]
    fn missing_values_read_as_unknown_switch() {
        for flag in ["--config-file", "--only", "--format", "--min-priority"] {
            let error = parse_args(&argv(&[flag])).expect_err("rejects");
            assert_eq!(
                error,
                ParseError::InvalidOption(format!(
                    "** (credo) Unknown switch for `suggest` command: {flag}"
                )),
                "{flag}"
            );
        }
    }

    #[test]
    fn min_priority_table() {
        let priority = |args: SuggestArgs| resolve_min_priority(&args).expect("resolves");
        let mut base = SuggestArgs::default();
        assert_eq!(priority(base.clone()), 0);
        base.strict = true;
        assert_eq!(priority(base.clone()), -99);
        base.all_priorities = true;
        base.strict = false;
        assert_eq!(priority(base.clone()), -99);
        for (name, value) in [
            ("higher", 20),
            ("high", 10),
            ("normal", 1),
            ("low", -10),
            ("ignore", -100),
            ("5", 5),
            ("-99", -99),
            ("0", 0),
        ] {
            base.min_priority = Some(name.to_owned());
            base.strict = true;
            assert_eq!(priority(base.clone()), value, "{name}");
        }
    }

    #[test]
    fn invalid_priorities_crash_like_native() {
        for raw in ["banana", "Low", "highX", "1.5"] {
            let base = SuggestArgs {
                min_priority: Some(raw.to_owned()),
                ..SuggestArgs::default()
            };
            assert_eq!(
                resolve_min_priority(&base).expect_err("crashes"),
                format!(
                    "** (RuntimeError) Got an invalid priority: :{raw}  (valid are numbers or higher/high/normal/low/ignore)"
                ),
                "{raw}"
            );
        }
    }

    #[test]
    fn invalid_selection_regex_crashes_like_native() {
        assert!(compile_selection(&["([".to_owned()], &[]).is_err());
        assert!(compile_selection(&[], &["IoInspect".to_owned()]).is_ok());
    }

    #[test]
    fn split_list_drops_empties() {
        assert!(split_list("").is_empty());
        assert_eq!(split_list("a,, b ,"), vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn version_and_help_texts() {
        assert_eq!(version_text(), format!("{}\n", env!("CARGO_PKG_VERSION")));
        assert!(help_text().contains("suggest"));
        assert!(suggest_help_text().contains("--strict"));
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
            &[],
            &["lib/".to_owned()],
            &[qredo::FileEntry::Glob("lib/old/".to_owned())],
        )
        .expect("discovers");
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
        let found = discover(&root, &[], &[], &[]).expect("discovers");
        let mut names: Vec<&str> = found
            .iter()
            .map(|(relative, _)| relative.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["lib/a.ex", "test/a_test.exs"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discover_expands_globs_and_crashes_on_missing_sources() {
        let root = std::env::temp_dir().join("qredo-discover-glob-test");
        let _ = std::fs::remove_dir_all(&root);
        for file in ["lib/a.ex", "lib/b.ex"] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&path, "x = 1\n").expect("write");
        }
        let pattern = root.join("lib/*.ex").to_string_lossy().into_owned();
        let found = discover(&root, &[pattern], &[], &[]).expect("discovers");
        assert_eq!(found.len(), 2);
        let missing = root.join("nope.ex").to_string_lossy().into_owned();
        assert!(discover(&root, &[missing], &[], &[]).is_err());
        let missing_dir = root.join("nope").to_string_lossy().into_owned();
        assert!(
            discover(&root, &[missing_dir], &[], &[])
                .expect("dir")
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discover_matches_absolute_included_patterns() {
        // Real harnesses rewrite configs to absolute `included` paths while
        // running from another directory: absolute patterns must match.
        let root = std::env::temp_dir().join("qredo-discover-absinc-test");
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lib/a.ex");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "x = 1\n").expect("write");
        let absolute = path.to_string_lossy().into_owned();
        let found = discover(&root, &[], &[absolute], &[]).expect("discovers");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "lib/a.ex");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_with_missing_config_is_129() {
        let root = std::env::temp_dir().join("qredo-run-missing-cfg-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("lib")).expect("mkdir");
        let argv = argv(&[
            "--config-file",
            root.join("nope.credo.exs").to_str().expect("utf8"),
            root.to_str().expect("utf8"),
        ]);
        // Absolute positional dir outside the tree: resolution root moves
        // there; config discovery is skipped via --config-file.
        assert_eq!(run_with(&argv), 129);
        let _ = std::fs::remove_dir_all(&root);
    }
}
