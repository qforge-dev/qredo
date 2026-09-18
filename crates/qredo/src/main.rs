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

mod cmd_browse;
mod cmd_explain;
mod cmd_gen;
mod help_texts;

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
    List(Box<SuggestArgs>),
    Categories,
    Info(Box<InfoArgs>),
    Explain(Box<ExplainArgs>),
    Diff(Box<DiffArgs>),
    GenConfig,
    GenCheck(Option<String>),
    Version,
    Help,
    SuggestHelp,
    HelpFor(&'static str),
}

/// `info` options (subset of the suggest surface plus `--verbose`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct InfoArgs {
    paths: Vec<String>,
    working_dir: Option<PathBuf>,
    config_file: Option<PathBuf>,
    config_name: String,
    files_included: Vec<String>,
    files_excluded: Vec<String>,
    verbose: bool,
}

/// Native subcommands by implementation status.
fn lookup_command(word: &str) -> Option<Command> {
    match word {
        "suggest" => Some(Command::Suggest(Box::default())),
        "list" => Some(Command::List(Box::default())),
        "categories" => Some(Command::Categories),
        "info" => Some(Command::Info(Box::default())),
        "explain" => Some(Command::Explain(Box::default())),
        "diff" => Some(Command::Diff(Box::default())),
        "gen.config" => Some(Command::GenConfig),
        "gen.check" => Some(Command::GenCheck(None)),
        "version" => Some(Command::Version),
        "help" => Some(Command::Help),
        _ => None,
    }
}

/// `explain` options: target positional plus analysis-shaping flags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ExplainArgs {
    target: Option<String>,
    working_dir: Option<PathBuf>,
    config_file: Option<PathBuf>,
    config_name: String,
    strict: bool,
    min_priority: Option<String>,
    format: ExplainFormat,
    only: Vec<String>,
    ignore: Vec<String>,
    checks_with_tag: Vec<String>,
    enable_disabled: Vec<String>,
}

/// `explain` output format: only `json` switches rendering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ExplainFormat {
    #[default]
    Default,
    Json,
}

/// `diff` options: git comparison plus the suggest selection surface.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DiffArgs {
    positional_ref: Option<String>,
    from_git_ref: Option<String>,
    from_dir: Option<PathBuf>,
    from_git_merge_base: Option<String>,
    since: Option<String>,
    show_fixed: bool,
    show_kept: bool,
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
    stale: bool,
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
            stale: false,
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
    // Bare `-v`/`--version` alone prints the version; combined with other
    // flags it reads as an unknown switch, like native.
    if raw.len() == 1 && (raw[0] == "-v" || raw[0] == "--version") {
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
            None => {
                let (words, help) = strip_help(words);
                if help {
                    Command::SuggestHelp
                } else {
                    Command::Suggest(Box::new(parse_suggest_vec(words)?))
                }
            }
            Some(command) => parse_command_tail(command, words[1..].to_vec())?,
        },
    };
    Ok(Args { command })
}

/// Parse suggest-family words: `--help` wins, otherwise the wrapped
/// parser runs over the remaining words.
fn with_help_stripped(
    rest: Vec<String>,
    help: Command,
    run: impl FnOnce(Vec<String>) -> Result<Command, ParseError>,
) -> Result<Command, ParseError> {
    let (rest, is_help) = strip_help(rest);
    if is_help { Ok(help) } else { run(rest) }
}

/// Parse the words after a known command word into its command.
fn parse_command_tail(command: Command, rest: Vec<String>) -> Result<Command, ParseError> {
    match command {
        Command::Suggest(_) => with_help_stripped(rest, Command::SuggestHelp, |rest| {
            Ok(Command::Suggest(Box::new(parse_suggest_vec(rest)?)))
        }),
        Command::List(_) => with_help_stripped(rest, Command::HelpFor("list"), |rest| {
            Ok(Command::List(Box::new(parse_suggest_vec(rest)?)))
        }),
        Command::Info(_) => with_help_stripped(rest, Command::HelpFor("info"), |rest| {
            Ok(Command::Info(Box::new(parse_info_vec(rest)?)))
        }),
        Command::Categories => Ok(Command::Categories),
        Command::Version => {
            if let Some(flag) = rest.iter().find(|arg| arg.starts_with('-')) {
                return Err(ParseError::InvalidOption(unknown_switch("version", flag)));
            }
            Ok(Command::Version)
        }
        Command::Explain(_) => with_help_stripped(rest, Command::HelpFor("explain"), |rest| {
            Ok(Command::Explain(Box::new(parse_explain_vec(rest)?)))
        }),
        Command::Diff(_) => with_help_stripped(rest, Command::HelpFor("diff"), |rest| {
            Ok(Command::Diff(Box::new(parse_diff_vec(rest)?)))
        }),
        Command::GenConfig => {
            if let Some(flag) = rest.iter().find(|arg| arg.starts_with('-')) {
                return Err(ParseError::InvalidOption(unknown_switch(
                    "gen.config",
                    flag,
                )));
            }
            Ok(Command::GenConfig)
        }
        Command::GenCheck(_) => parse_gen_check(rest),
        Command::Help => Ok(Command::Help),
        Command::SuggestHelp => Ok(Command::SuggestHelp),
        Command::HelpFor(_) => Ok(command),
    }
}

/// Parse `gen.check` positionals: exactly one name is accepted; anything
/// else prints usage. Any switch is rejected, like native.
fn parse_gen_check(rest: Vec<String>) -> Result<Command, ParseError> {
    if let Some(flag) = rest.iter().find(|arg| arg.starts_with('-')) {
        return Err(ParseError::InvalidOption(unknown_switch("gen.check", flag)));
    }
    Ok(Command::GenCheck(rest.into_iter().next()))
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

/// Parse explain flags and positionals: the full suggest surface (native
/// reuses the suggest switches); the first positional routes the target.
fn parse_explain_vec(words: Vec<String>) -> Result<ExplainArgs, ParseError> {
    let suggest = parse_suggest_vec(words)?;
    let (format, target) = (suggest.format, suggest.paths.first().cloned());
    Ok(ExplainArgs {
        target,
        working_dir: suggest.working_dir,
        config_file: suggest.config_file,
        config_name: suggest.config_name,
        strict: suggest.strict,
        min_priority: suggest.min_priority,
        format: match format {
            Format::Json => ExplainFormat::Json,
            _ => ExplainFormat::Default,
        },
        only: suggest.only,
        ignore: suggest.ignore,
        checks_with_tag: suggest.checks_with_tag,
        enable_disabled: suggest.enable_disabled,
    })
}

/// Parse diff flags and positionals: one optional positional ref plus
/// git selectors and the suggest selection surface.
fn parse_diff_vec(words: Vec<String>) -> Result<DiffArgs, ParseError> {
    let mut args = DiffArgs {
        config_name: "default".to_owned(),
        ..DiffArgs::default()
    };
    let mut words = words.into_iter().peekable();
    let mut positional = Vec::new();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--show-fixed" => args.show_fixed = true,
            "--show-kept" => args.show_kept = true,
            "--strict" => args.strict = true,
            "--all-priorities" | "-A" => args.all_priorities = true,
            "--all" | "-a" => args.all = true,
            "--mute-exit-status" => args.mute_exit_status = true,
            "--from-git-ref"
            | "--from-dir"
            | "--from-git-merge-base"
            | "--since"
            | "--config-file"
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
            | "--enable-disabled-checks" => {
                parse_diff_valued(&mut args, word.as_str(), &mut words)?;
            }
            flag if flag.starts_with('-') => {
                return Err(ParseError::InvalidOption(unknown_switch("diff", flag)));
            }
            _ => positional.push(word),
        }
    }
    // First positional is the ref; extras are ignored like native.
    args.positional_ref = positional.into_iter().next();
    Ok(args)
}

/// Apply one `diff` flag that takes a value.
fn parse_diff_valued(
    args: &mut DiffArgs,
    flag: &str,
    words: &mut std::iter::Peekable<impl Iterator<Item = String>>,
) -> Result<(), ParseError> {
    match flag {
        "--from-git-ref" => {
            args.from_git_ref = Some(take_value(words, "diff", flag)?);
        }
        "--from-dir" => {
            args.from_dir = Some(PathBuf::from(take_value(words, "diff", flag)?));
        }
        "--from-git-merge-base" => {
            args.from_git_merge_base = Some(take_value(words, "diff", flag)?);
        }
        "--since" => {
            args.since = Some(take_value(words, "diff", flag)?);
        }
        "--config-file" => {
            args.config_file = Some(PathBuf::from(take_value(words, "diff", flag)?));
        }
        "--config-name" | "-C" => {
            args.config_name = take_value(words, "diff", flag)?;
        }
        "--working-dir" => {
            args.working_dir = Some(PathBuf::from(take_value(words, "diff", flag)?));
        }
        "--min-priority" => {
            args.min_priority = Some(take_value(words, "diff", flag)?);
        }
        "--only" | "--checks" | "-c" => {
            args.only
                .extend(split_list(&take_value(words, "diff", flag)?));
        }
        "--ignore" | "--ignore-checks" | "-i" => {
            args.ignore
                .extend(split_list(&take_value(words, "diff", flag)?));
        }
        "--checks-with-tag" => {
            args.checks_with_tag
                .extend(split_list(&take_value(words, "diff", flag)?));
        }
        "--checks-without-tag" => {
            args.checks_without_tag
                .extend(split_list(&take_value(words, "diff", flag)?));
        }
        "--enable-disabled-checks" => {
            args.enable_disabled
                .extend(split_list(&take_value(words, "diff", flag)?));
        }
        _ => {
            return Err(ParseError::InvalidOption(unknown_switch("diff", flag)));
        }
    }
    Ok(())
}

/// Parse info flags and positionals from owned words.
fn parse_info_vec(words: Vec<String>) -> Result<InfoArgs, ParseError> {
    let mut args = InfoArgs {
        config_name: "default".to_owned(),
        ..InfoArgs::default()
    };
    // `..Default::default()` above would also work; the explicit name keeps
    // the default visible next to the suggest parser default.
    let mut words = words.into_iter().peekable();
    let mut positional = Vec::new();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--verbose" => args.verbose = true,
            "--config-file" => {
                args.config_file = Some(PathBuf::from(take_value(
                    &mut words,
                    "info",
                    "--config-file",
                )?));
            }
            "--config-name" | "-C" => {
                args.config_name = take_value(&mut words, "info", "--config-name")?;
            }
            "--working-dir" => {
                args.working_dir = Some(PathBuf::from(take_value(
                    &mut words,
                    "info",
                    "--working-dir",
                )?));
            }
            "--files-included" => {
                args.files_included.extend(split_list(&take_value(
                    &mut words,
                    "info",
                    "--files-included",
                )?));
            }
            "--files-excluded" => {
                args.files_excluded.extend(split_list(&take_value(
                    &mut words,
                    "info",
                    "--files-excluded",
                )?));
            }
            flag if flag.starts_with('-') => {
                return Err(ParseError::InvalidOption(unknown_switch("info", flag)));
            }
            _ => positional.push(word),
        }
    }
    args.paths = positional;
    Ok(args)
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
        "--stale" => args.stale = true,
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
        // Display names follow the pattern form: absolute patterns yield
        // absolute names (so check file-selection matches), relative
        // patterns yield root-relative names.
        let absolute_pattern = Path::new(pattern).is_absolute();
        if has_magic(pattern) && absolute_pattern {
            for path in expand_glob(root, pattern) {
                let text = path.to_string_lossy().into_owned();
                out.push((text, path));
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
            Ok(meta) if meta.is_dir() => {
                let mut walked = Vec::new();
                collect(&candidate, root, &mut walked);
                if absolute_pattern {
                    for (_, path) in walked {
                        let text = path.to_string_lossy().into_owned();
                        out.push((text, path));
                    }
                } else {
                    out.extend(walked);
                }
            }
            Ok(_) if is_elixir_path(pattern) => {
                let display = if absolute_pattern {
                    candidate.to_string_lossy().into_owned()
                } else {
                    display_name(root, &candidate)
                };
                out.push((display, candidate));
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

/// Execute the pipeline, using the incremental disk cache under `--stale`.
#[allow(
    clippy::too_many_arguments,
    reason = "one call-site spine shared by suggest and list"
)]
fn execute_cached(
    args: &SuggestArgs,
    config_source: &str,
    files: &[qredo::RunnerFile],
    min_priority: i32,
    selection: qredo::Selection,
    root: &Path,
) -> Result<qredo::RunReport, qredo::integration::Fallback> {
    if args.stale {
        qredo::stale::execute_stale(
            config_source,
            &args.config_name,
            files,
            min_priority,
            selection,
            root,
        )
    } else {
        qredo::integration::execute_selected(
            config_source,
            &args.config_name,
            files,
            min_priority,
            selection,
        )
    }
}
/// Rendered stdout for one `suggest` run: native-shape formatters over
/// absolute filenames (native resolves display paths absolutely).
fn print_report(
    args: &SuggestArgs,
    files: &[qredo::RunnerFile],
    report: &mut qredo::RunReport,
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
            absolutize_in_place(report, &context.root);
            let mut machine = qredo::format_machine::MachineContext::new(context.root.clone());
            if matches!(args.format, Format::Sarif) {
                let mut seen = std::collections::BTreeSet::new();
                for issue in &report.issues {
                    if seen.insert(issue.check.clone()) {
                        machine = machine.with_rule_doc(
                            issue.check.clone(),
                            qredo::check_docs::sarif_rule_doc(&issue.check),
                        );
                    }
                }
            }
            let text = match args.format {
                Format::Oneline => qredo::format_machine::render_oneline(report, &machine),
                Format::Flycheck => qredo::format_machine::render_flycheck(report, &machine),
                Format::Json => qredo::format_machine::render_json(report, &machine),
                Format::Sarif => qredo::format_machine::render_sarif(report, &machine),
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

/// Make report filenames root-absolute in place for native-shape output.
fn absolutize_in_place(report: &mut qredo::RunReport, root: &Path) {
    for issue in &mut report.issues {
        let path = Path::new(&issue.filename);
        if !path.is_absolute() {
            issue.filename = root.join(path).to_string_lossy().into_owned();
        }
    }
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
    execute(parsed.command)
}

/// Execute one parsed command, returning the exit code.
fn execute(command: Command) -> i32 {
    match command {
        Command::Version => {
            print!("{}", version_text());
            0
        }
        Command::Help => {
            print!("{help}", help = help_text());
            0
        }
        Command::SuggestHelp => {
            print!("{}", help_texts::suggest());
            0
        }
        Command::Suggest(suggest) => run_suggest(&suggest),
        Command::List(list) => run_list(&list),
        Command::Categories => {
            let (text, code) = cmd_browse::run_categories();
            print!("{text}");
            code
        }
        Command::Info(info) => run_info(&info),
        Command::Explain(explain) => run_explain(&explain),
        Command::Diff(diff) => run_diff(&diff),
        Command::GenConfig => {
            let (text, code) = cmd_gen::run_gen_config();
            print!("{text}");
            code
        }
        Command::GenCheck(name) => {
            let (text, code) = cmd_gen::run_gen_check(name.as_deref());
            print!("{text}");
            code
        }
        Command::HelpFor(which) => {
            print!(
                "{}",
                match which {
                    "list" => help_texts::list(),
                    "info" => help_texts::info(),
                    "explain" => help_texts::explain(),
                    "diff" => help_texts::diff(),
                    _ => help_texts::general(),
                }
            );
            0
        }
    }
}

/// Execute one `suggest` run, returning the exit code.
fn run_suggest(args: &SuggestArgs) -> i32 {
    let loaded = match load_run(
        &args.paths,
        args.working_dir.as_ref(),
        args.config_file.as_ref(),
        &args.config_name,
    ) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let Some(selection) = load_selection(args) else {
        return 1;
    };
    let Ok((files, load_microseconds)) = discover_files(&loaded) else {
        return 1;
    };
    match resolve_min_priority(args) {
        Ok(_) => {}
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    }
    let context = report_context(
        args,
        &loaded.config,
        &selection,
        loaded.root,
        load_microseconds,
    );
    report_exit(args, &files, &loaded.config_source, selection, context)
}

/// Shared load preamble for file-analyzing commands: resolution root,
/// config discovery and static parse. Prints the failure reason and
/// yields its exit code on `Err`.
fn load_run(
    paths: &[String],
    working_dir: Option<&PathBuf>,
    config_file: Option<&PathBuf>,
    config_name: &str,
) -> Result<LoadedRun, i32> {
    let base = match working_dir {
        Some(dir) => (*dir).clone(),
        None => match std::env::current_dir() {
            Ok(dir) => dir,
            Err(error) => {
                eprintln!("cannot read working directory: {error}");
                return Err(2);
            }
        },
    };
    // A leading existing directory becomes the resolution root,
    // mirroring native behavior; the rest are file patterns.
    let (root, patterns) = split_root(&base, paths);
    let config_path = if let Some(path) = config_file {
        (*path).clone()
    } else {
        let Some(path) = discover_config(&root) else {
            eprintln!(
                "no .credo.exs found walking up from {}",
                root.to_string_lossy()
            );
            return Err(2);
        };
        path
    };
    if !config_path.is_file() {
        let (message, code) = missing_config(&config_path);
        eprintln!("{message}");
        return Err(code);
    }
    let Some(config_source) = read_config(&config_path) else {
        return Err(2);
    };
    // Discovery needs the static config; an unreadable one fails here
    // with the same explicit reason as the pipeline below.
    match qredo::parse_config(&config_source, config_name) {
        Ok(config) => Ok(LoadedRun {
            root,
            patterns,
            config_path,
            config_source,
            config,
        }),
        Err(unsupported) => {
            eprintln!("warning: failed to parse config file: {}", unsupported.0);
            Err(129)
        }
    }
}

/// Loaded run inputs shared by the file-analyzing commands.
struct LoadedRun {
    root: PathBuf,
    patterns: Vec<String>,
    config_path: PathBuf,
    config_source: String,
    config: qredo::CredoConfig,
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

/// Execute one `list` run: same discovery and pipeline as suggest,
/// per-file grouped rendering.
fn run_list(args: &SuggestArgs) -> i32 {
    let loaded = match load_run(
        &args.paths,
        args.working_dir.as_ref(),
        args.config_file.as_ref(),
        &args.config_name,
    ) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let Some(selection) = load_selection(args) else {
        return 1;
    };
    let Ok((files, load_microseconds)) = discover_files(&loaded) else {
        return 1;
    };
    let min_priority = match resolve_min_priority(args) {
        Ok(priority) => priority,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let context = cmd_browse::ListContext {
        check_count: check_count(&loaded.config, &selection, min_priority),
        load_microseconds,
        run_microseconds: 0,
        strict_hint: min_priority < 0,
        mute_exit_status: args.mute_exit_status,
    };
    let run_start = std::time::Instant::now();
    let outcome = execute_cached(
        args,
        &loaded.config_source,
        &files,
        min_priority,
        selection,
        &loaded.root,
    );
    let mut context = context;
    context.run_microseconds = micros(run_start.elapsed());
    finish_list(&files, outcome, &context)
}

/// Print one `list` outcome, returning the exit code.
fn finish_list(
    files: &[qredo::RunnerFile],
    outcome: Result<qredo::RunReport, qredo::integration::Fallback>,
    context: &cmd_browse::ListContext,
) -> i32 {
    match outcome {
        Err(fallback) => {
            eprintln!("unsupported config: {}", fallback.reason);
            2
        }
        Ok(report) => {
            for error in &report.errors {
                eprintln!("error: {error:?}");
            }
            let (text, code) = cmd_browse::run_list(files, &report, context);
            print!("{text}");
            if !report.errors.is_empty() {
                return 2;
            }
            code
        }
    }
}

/// Execute one `explain` run: usage, check docs, or a located issue.
fn run_explain(args: &ExplainArgs) -> i32 {
    let target = match route_explain_target(args) {
        Ok(target) => target,
        Err(code) => return code,
    };
    let loaded = match load_run(
        &[],
        args.working_dir.as_ref(),
        args.config_file.as_ref(),
        &args.config_name,
    ) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let selection = match explain_selection(args) {
        Ok(selection) => selection,
        Err(code) => return code,
    };
    let min_priority = match explicit_min_priority(args.strict, args.min_priority.as_deref()) {
        Ok(priority) => priority,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let files = match discover(
        &loaded.root,
        &[],
        &loaded.config.files_included,
        &loaded.config.files_excluded,
    ) {
        Ok(found) => read_found(&found),
        Err(_) => Vec::new(),
    };
    let context = cmd_explain::ExplainContext {
        target,
        working_dir: loaded.root,
        config_name: args.config_name.clone(),
        config_source: loaded.config_source,
        files,
        min_priority,
        selection,
    };
    let (stdout, stderr, code) = match args.format {
        ExplainFormat::Json => cmd_explain::run_explain_json(&context),
        ExplainFormat::Default => cmd_explain::run_explain(&context),
    };
    print!("{stdout}");
    eprint!("{stderr}");
    code
}

/// Route the first positional to an explain target; usage needs none.
/// Prints the crash shape for malformed locations.
fn route_explain_target(args: &ExplainArgs) -> Result<Option<cmd_explain::ExplainTarget>, i32> {
    // The first positional routes the target; a `path:line[:col]`
    // location splits there, anything else is a check name.
    match args.target.as_deref() {
        None => Ok(None),
        Some(raw) if is_location_target(raw) => match split_location_target(raw) {
            Ok((path, line)) => Ok(Some(cmd_explain::ExplainTarget::Location { path, line })),
            Err(message) => {
                eprintln!("{message}");
                Err(1)
            }
        },
        Some(name) => Ok(Some(cmd_explain::ExplainTarget::Check(name.to_owned()))),
    }
}

/// Explain check selection with native regex pre-compilation.
fn explain_selection(args: &ExplainArgs) -> Result<qredo::Selection, i32> {
    if let Err(message) = compile_selection(&args.only, &args.ignore) {
        eprintln!("{message}");
        return Err(1);
    }
    Ok(qredo::Selection {
        only: args.only.clone(),
        ignore: args.ignore.clone(),
        checks_with_tag: args.checks_with_tag.clone(),
        enable_disabled: args.enable_disabled.clone(),
    })
}
/// True for `path:line[:col]` targets: two or three `:` segments, all
/// non-empty.
fn is_location_target(raw: &str) -> bool {
    let parts: Vec<&str> = raw.rsplit(':').collect();
    matches!(parts.len(), 2 | 3) && parts.iter().all(|part| !part.is_empty())
}

/// Split a location target; non-integer line/column crashes like native.
fn split_location_target(raw: &str) -> Result<(String, usize), String> {
    let mut parts: Vec<&str> = raw.rsplitn(3, ':').collect();
    parts.reverse();
    let (path, line_raw, column_raw) = match parts.as_slice() {
        [path, line] => (*path, *line, None),
        [path, line, column] => (*path, *line, Some(*column)),
        _ => return Err(format!("** (ArgumentError) invalid location: {raw}")),
    };
    let line: usize = line_raw.parse().map_err(|_| {
        format!("** (ArgumentError) argument error\n    {raw} is not a valid location")
    })?;
    if let Some(column) = column_raw {
        column.parse::<usize>().map_err(|_| {
            format!("** (ArgumentError) argument error\n    {raw} is not a valid location")
        })?;
    }
    Ok((path.to_owned(), line))
}

/// Minimum priority from explicit flags: a `--min-priority` value beats
/// `--strict` regardless of order, mirroring the suggest surface.
fn explicit_min_priority(strict: bool, min_priority: Option<&str>) -> Result<i32, String> {
    if let Some(raw) = min_priority {
        return match raw {
            "higher" => Ok(20),
            "high" => Ok(10),
            "normal" => Ok(1),
            "low" => Ok(-10),
            "ignore" => Ok(-100),
            _ => raw.parse::<i32>().map_err(|_| invalid_priority(raw)),
        };
    }
    if strict { Ok(-99) } else { Ok(0) }
}

/// Execute one `diff` run against git HEAD (or the selected previous side).
fn run_diff(args: &DiffArgs) -> i32 {
    let working_dir = match &args.working_dir {
        Some(dir) => dir.clone(),
        None => match std::env::current_dir() {
            Ok(dir) => dir,
            Err(error) => {
                eprintln!("cannot read working directory: {error}");
                return 2;
            }
        },
    };
    let options = qredo::cmd_diff::DiffOptions {
        working_dir,
        positional_ref: args.positional_ref.clone(),
        from_ref: args.from_git_ref.clone(),
        from_dir: args.from_dir.clone(),
        from_merge_base: args.from_git_merge_base.clone(),
        since: args.since.clone(),
        config_file: args.config_file.clone(),
        config_name: args.config_name.clone(),
        strict: args.strict,
        all_priorities: args.all_priorities,
        all: args.all,
        min_priority: args.min_priority.clone(),
        only: args.only.clone(),
        ignore: args.ignore.clone(),
        checks_with_tag: args.checks_with_tag.clone(),
        checks_without_tag: args.checks_without_tag.clone(),
        enable_disabled: args.enable_disabled.clone(),
        files_included: Vec::new(),
        files_excluded: Vec::new(),
        show_fixed: args.show_fixed,
        show_kept: args.show_kept,
        mute_exit_status: args.mute_exit_status,
    };
    let (stdout, stderr, code) = qredo::cmd_diff::run_diff(&options);
    print!("{stdout}");
    eprint!("{stderr}");
    code
}

/// Execute one `info` run: config and file inventory without analysis.
fn run_info(args: &InfoArgs) -> i32 {
    let loaded = match load_run(
        &[],
        args.working_dir.as_ref(),
        args.config_file.as_ref(),
        &args.config_name,
    ) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    // Only the first positional feeds `info`; the rest are ignored.
    let patterns: Vec<String> = args.paths.first().cloned().into_iter().collect();
    let (root, patterns) = split_root(&loaded.root, &patterns);
    let config = loaded.config;
    let files = info_files(&root, &patterns, &config, &args.files_included);
    let context = cmd_browse::InfoContext {
        credo_version: env!("CARGO_PKG_VERSION"),
        elixir_version: "1.20.2",
        erlang_version: "29",
        verbose: args.verbose,
        embedded_inspect: &embedded_default_config(),
        config: &config,
        config_path: &loaded.config_path.to_string_lossy(),
        config_source: &loaded.config_source,
        files: &files,
    };
    let (text, code) = cmd_browse::run_info(&context);
    print!("{text}");
    code
}

/// Native `info --verbose` embeds its compile-time `.credo.exs` (the
/// pinned upstream default config, vendored under
/// `compatibility/upstream`), inspected with the printable limit.
fn embedded_default_config() -> String {
    cmd_browse::elixir_inspect_string(include_str!(
        "../compatibility/upstream/credo_default_config.exs"
    ))
}

/// File inventory for `info`: first positional only, else live
/// `--files-included`, else the config set; nonexistent positionals are
/// silently ignored (never crash).
fn info_files(
    root: &Path,
    patterns: &[String],
    config: &qredo::CredoConfig,
    files_included: &[String],
) -> Vec<String> {
    let found: Vec<(String, PathBuf)> = if patterns.is_empty() && files_included.is_empty() {
        discover(root, &[], &config.files_included, &config.files_excluded).unwrap_or_default()
    } else if patterns.is_empty() {
        // `--files-included` is live for `info` (unlike `suggest`).
        let mut collected = Vec::new();
        collect_included(root, files_included, &mut collected);
        collected
            .into_iter()
            .filter(|(relative, path)| {
                let absolute = path.to_string_lossy().into_owned();
                !config
                    .files_excluded
                    .iter()
                    .any(|entry| entry_matches(entry, relative) || entry_matches(entry, &absolute))
            })
            .collect()
    } else {
        match resolve_positionals(root, patterns) {
            Ok(paths) => paths
                .into_iter()
                .map(|path| (display_name(root, &path), path))
                .collect(),
            Err(_) => Vec::new(),
        }
    };
    found
        .into_iter()
        .map(|(_, path)| path.to_string_lossy().into_owned())
        .collect()
}

/// Discover and read one loaded run's files with load-phase timing.
fn discover_files(loaded: &LoadedRun) -> Result<(Vec<qredo::RunnerFile>, u64), i32> {
    let load_start = std::time::Instant::now();
    let found = match discover(
        &loaded.root,
        &loaded.patterns,
        &loaded.config.files_included,
        &loaded.config.files_excluded,
    ) {
        Ok(found) => found,
        Err(missing) => {
            eprintln!("{}", unreadable_file(&missing));
            return Err(1);
        }
    };
    let files = read_found(&found);
    Ok((files, micros(load_start.elapsed())))
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
    let outcome = execute_cached(
        args,
        config_source,
        files,
        min_priority,
        selection,
        &context.root,
    );
    context.run_microseconds = micros(run_start.elapsed());
    match outcome {
        Err(fallback) => {
            eprintln!("unsupported config: {}", fallback.reason);
            2
        }
        Ok(mut report) => {
            for error in &report.errors {
                eprintln!("error: {error:?}");
            }
            print_report(args, files, &mut report, &context);
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
    fn version_flag_only_alone() {
        // Bare `-v`/`--version` prints the version; combined with other
        // flags it reads as an unknown switch, like native.
        assert_eq!(
            parse_args(&argv(&["-v"])).expect("parses").command,
            Command::Version
        );
        assert_eq!(
            parse_args(&argv(&["--version"])).expect("parses").command,
            Command::Version
        );
        assert_eq!(
            parse_args(&argv(&["version"])).expect("parses").command,
            Command::Version
        );
        assert_eq!(
            parse_args(&argv(&["version", "foo"]))
                .expect("parses")
                .command,
            Command::Version
        );
        assert!(parse_args(&argv(&["--strict", "-v"])).is_err());
        assert!(parse_args(&argv(&["version", "--help"])).is_err());
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
    fn generated_commands_parse() {
        assert!(matches!(
            parse_args(&argv(&["explain"])).expect("parses").command,
            Command::Explain(_)
        ));
        assert!(matches!(
            parse_args(&argv(&["diff"])).expect("parses").command,
            Command::Diff(_)
        ));
        assert_eq!(
            parse_args(&argv(&["gen.config"])).expect("parses").command,
            Command::GenConfig
        );
        assert_eq!(
            parse_args(&argv(&["gen.check", "Foo.Bar"]))
                .expect("parses")
                .command,
            Command::GenCheck(Some("Foo.Bar".to_owned()))
        );
        assert_eq!(
            parse_args(&argv(&["gen.check"])).expect("parses").command,
            Command::GenCheck(None)
        );
    }

    #[test]
    fn browse_commands_parse() {
        assert!(matches!(
            parse_args(&argv(&["list"])).expect("parses").command,
            Command::List(_)
        ));
        assert_eq!(
            parse_args(&argv(&["categories"])).expect("parses").command,
            Command::Categories
        );
        match parse_args(&argv(&["info", "--verbose"]))
            .expect("parses")
            .command
        {
            Command::Info(info) => assert!(info.verbose),
            other => panic!("expected info, got {other:?}"),
        }
        match parse_args(&argv(&["info"])).expect("parses").command {
            Command::Info(info) => {
                assert!(!info.verbose);
                assert_eq!(info.config_name, "default");
            }
            other => panic!("expected info, got {other:?}"),
        }
    }

    #[test]
    fn info_rejects_unknown_switches() {
        let error = parse_args(&argv(&["info", "--nope"])).expect_err("rejects");
        assert_eq!(
            error,
            ParseError::InvalidOption(
                "** (credo) Unknown switch for `info` command: --nope".to_owned()
            )
        );
    }

    #[test]
    fn stale_flag_parses() {
        let parsed = suggest(&["--stale"]);
        assert!(parsed.stale);
        assert!(!SuggestArgs::default().stale);
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
        assert!(help_texts::suggest().contains("--strict"));
    }

    #[test]
    fn machine_output_absolutizes_in_place() {
        let mut report = qredo::RunReport {
            issues: vec![qredo::Issue {
                check: "Credo.Check.Warning.IoInspect".to_owned(),
                category: qredo::Category::Warning,
                priority: 0,
                severity: 1.0,
                message: "message".to_owned(),
                filename: "lib/a.ex".to_owned(),
                line_no: Some(1),
                column: Some(1),
                exit_status: 16,
                trigger: qredo::IssueTrigger::NoTrigger,
                scope: None,
            }],
            exit_status: 16,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
        };
        let allocation = report.issues.as_ptr();
        absolutize_in_place(&mut report, Path::new("/project"));
        assert_eq!(report.issues.as_ptr(), allocation, "must not clone issues");
        assert_eq!(report.issues[0].filename, "/project/lib/a.ex");
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
        // running from another directory: absolute patterns match and
        // yield absolute names (so check file-selection matches).
        let root = std::env::temp_dir().join("qredo-discover-absinc-test");
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("lib/a.ex");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "x = 1\n").expect("write");
        let absolute = path.to_string_lossy().into_owned();
        let found = discover(&root, &[], std::slice::from_ref(&absolute), &[]).expect("discovers");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, absolute);
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
