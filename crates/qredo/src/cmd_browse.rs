//! Native-shaped `list`, `categories` and `info` output.
//!
//! Plain-text (piped) renderers for the three read-only browse subcommands,
//! mirroring the pinned upstream (`work/ref/credo` @ `ea1ccb9`; verbatim
//! captures in `work/p6-siblings-probe.md`). Scope is `list`/`categories`/
//! `info` only: `explain`, `diff` and `gen.*` belong elsewhere.
//!
//! Wiring contract for `main.rs` (the owner wires dispatch; this module only
//! renders):
//!
//! ```ignore
//! mod cmd_browse; // top of main.rs
//!
//! // `list`: same discovery, file reads, timing and check-count inputs as
//! // `suggest`; the renderer owns the whole stdout including the header.
//! let (stdout, exit) = cmd_browse::run_list(
//!     &files,
//!     &report,
//!     &cmd_browse::ListContext {
//!         check_count,
//!         load_microseconds,
//!         run_microseconds,
//!         strict_hint: min_priority < 0,
//!         mute_exit_status: args.mute_exit_status,
//!     },
//! );
//!
//! // `categories`: no inputs at all.
//! let (stdout, exit) = cmd_browse::run_categories();
//!
//! // `info`: versions and the verbose inventory come from the caller; checks
//! // are read off the already-parsed `qredo::CredoConfig` here.
//! let (stdout, exit) = cmd_browse::run_info(&cmd_browse::InfoContext {
//!     credo_version: env!("CARGO_PKG_VERSION"),
//!     elixir_version: "1.20.2",
//!     erlang_version: "29",
//!     verbose: args.verbose,
//!     embedded_inspect: &embedded_default_config(),
//!     config: &config,
//!     config_path: &config_path.to_string_lossy(),
//!     config_source: &config_source,
//!     files: &resolved_files,
//! });
//! ```
//!
//! `info` file resolution (from `lib/credo/cli/options.ex` `split_args`/
//! `extract_path` with `treat_unknown_args_as_files: false`): only the FIRST
//! positional is ever used. When it exists (or matches a glob), resolve just
//! that one pattern; otherwise resolve `--files-included` when given, else
//! the config `included` set. A nonexistent positional is ignored (never a
//! crash, unlike `suggest`/`list`). Apply `files.excluded` as usual. The
//! resolved absolute display paths are what `files` carries.
//!
//! Known residuals vs native (byte-match limits of this module):
//!
//! - R-LIST-1: the `No files found!` header prints unconditionally, matching
//!   native (whose pre-analysis `Filter.important/1` is empty at that point).
//! - R-LIST-2: a file section title uses the first issue in report order for
//!   that file; native uses first-in-execution order. Both reduce to the
//!   module name and agree in practice.
//! - R-LIST-3: piped plain text only: no TTY colors, no C-locale `\x{}`
//!   escapes, fixed 80 columns.
//! - R-LIST-4: excerpt truncation counts Unicode scalars, native counts
//!   graphemes; identical for ASCII.
//! - R-LIST-5: machine formats (`--format json/flycheck/sarif/oneline`) for
//!   `list` are not rendered here.
//! - R-INFO-1: the `## :credo` entry embeds the checkout `.credo.exs` at
//!   native compile time (a 4243-byte dump); qredo has no such artifact, so
//!   the content is caller-provided (see the wiring sketch).
//! - R-INFO-2: config-source inspect escaping covers `"`, `\`, newline,
//!   carriage return and tab; other controls and non-ASCII escaping may
//!   differ from Elixir `inspect/2`, which stays raw UTF-8.
//! - R-INFO-3: `Checks:` lists enabled config checks in file order without
//!   applying CLI selection (`--only`/`--ignore`/tags).
//! - R-INFO-4: tool versions are caller-provided constants; there is no BEAM
//!   to query at runtime.
//! - R-INFO-5: `--format json` for `info`/`categories` is not rendered here.

use qredo::{Category, Issue, RunReport, RunnerFile};

/// Piped terminal width; headers pad to it, excerpts truncate 8 short.
const TERM_WIDTH: usize = 80;
/// Location-line and excerpt indent.
const INDENT: usize = 8;
/// Microseconds per native timing centisecond.
const MICROS_PER_CENTI: u64 = 10_000;

/// Rendering inputs for `list` that the report and files do not carry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListContext {
    /// Checks executed, shown in the timing line.
    pub check_count: usize,
    /// Load-phase microseconds.
    pub load_microseconds: u64,
    /// Analysis-phase microseconds.
    pub run_microseconds: u64,
    /// True when the resolved minimum priority is negative: selects the
    /// `--strict` hint line instead of the default one.
    pub strict_hint: bool,
    /// Exit zero despite issues (`--mute-exit-status`).
    pub mute_exit_status: bool,
}

/// Rendering inputs for `info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoContext<'a> {
    /// Credo version for the `System:` section.
    pub credo_version: &'a str,
    /// Elixir version for the `System:` section.
    pub elixir_version: &'a str,
    /// OTP release for the `System:` section.
    pub erlang_version: &'a str,
    /// Append the `Configuration:` inventory (`--verbose`).
    pub verbose: bool,
    /// Inspect-rendered `## :credo` entry body (see R-INFO-1).
    pub embedded_inspect: &'a str,
    /// Already-parsed active config; enabled checks feed `Checks:`.
    pub config: &'a qredo::CredoConfig,
    /// Display path of the active config file.
    pub config_path: &'a str,
    /// Raw text of the active config file.
    pub config_source: &'a str,
    /// Resolved absolute display paths for `Files:` (first positional only,
    /// `--files-included`, or the config set; see the module docs).
    pub files: &'a [String],
}

/// Render the complete `list` stdout and its exit status.
///
/// Header (always `No files found!`), one per-file section per file with
/// issues (sorted by filename, issues sorted by line), then the
/// suggest-shaped summary.
#[must_use]
pub fn run_list(files: &[RunnerFile], report: &RunReport, context: &ListContext) -> (String, i32) {
    // Unconditional: native prints the pre-analysis important-file count,
    // which is empty at this pipeline point, on every `list` run.
    let mut out = String::from("\nNo files found!\n");
    let mut names: Vec<&str> = files
        .iter()
        .filter(|file| {
            !report
                .skipped_invalid
                .iter()
                .any(|skipped| skipped == &file.filename)
        })
        .map(|file| file.filename.as_str())
        .collect();
    names.sort_unstable();
    for name in names {
        let source = files
            .iter()
            .find(|file| file.filename.as_str() == name)
            .map_or("", |file| file.source.as_str());
        let mut issues: Vec<&Issue> = report
            .issues
            .iter()
            .filter(|issue| issue.filename == name)
            .collect();
        if issues.is_empty() {
            continue;
        }
        // Stable ascending line order within the file, like native.
        issues.sort_by(|left, right| left.line_no.cmp(&right.line_no));
        push_file_section(&mut out, source, &issues);
    }
    push_summary(&mut out, files, report, context);
    let exit = if context.mute_exit_status {
        0
    } else {
        report.exit_status
    };
    (out, exit)
}

/// Render the complete `categories` stdout; always exit 0.
#[must_use]
pub fn run_categories() -> (String, i32) {
    let mut out = String::new();
    for (title, body) in CATEGORIES {
        out.push('\n');
        push_padded_title(&mut out, title);
        out.push_str("┃ \n");
        for line in body.split('\n') {
            out.push_str("┃  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    (out, 0)
}

/// Render the complete `info` stdout; always exit 0.
///
/// Without `verbose` only the `System:` section prints. With `verbose` the
/// `Configuration:` inventory follows: the `## :credo` entry plus the active
/// config file listed twice (native append-twice quirk, reproduced
/// deliberately), then `Files:`, then `Checks:`.
#[must_use]
pub fn run_info(context: &InfoContext<'_>) -> (String, i32) {
    let mut out = format!(
        "System:\n  Credo: {}\n  Elixir: {}\n  Erlang: {}",
        context.credo_version, context.elixir_version, context.erlang_version
    );
    if context.verbose {
        out.push_str("\nConfiguration:\n  Configs:");
        push_config_entry(&mut out, "## :credo", context.embedded_inspect);
        let inspected = elixir_inspect_string(context.config_source);
        let label = format!("## file: {}", context.config_path);
        // Native appends the active config file twice (the CLI-to-config
        // conversion runs in two pipeline stages); reproduced deliberately.
        push_config_entry(&mut out, &label, &inspected);
        push_config_entry(&mut out, &label, &inspected);
        out.push_str("\n  Files:");
        for file in context.files {
            out.push_str("\n    - ");
            out.push_str(file);
        }
        out.push_str("\n  Checks:");
        for entry in &context.config.checks {
            if entry.enabled {
                out.push_str("\n    - Elixir.");
                out.push_str(&entry.module);
            }
        }
    }
    out.push('\n');
    (out, 0)
}

/// Elixir `inspect/2` rendering of a config source string for the
/// `Configuration:` dump: double-quoted with `\`, `"`, newline, carriage
/// return and tab escaped, then the `pretty: true` printable limit —
/// the first 4096 characters verbatim plus `" <> ...` when longer
/// (verified against `inspect/2` on this toolchain).
#[must_use]
pub fn elixir_inspect_string(source: &str) -> String {
    /// `Inspect.Opts` default `printable_limit`.
    const PRINTABLE_LIMIT: usize = 4096;
    let truncated = source.chars().count() > PRINTABLE_LIMIT;
    let mut out = String::with_capacity(source.len() + 2);
    out.push('"');
    for c in source.chars().take(PRINTABLE_LIMIT) {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    if truncated {
        out.push_str(" <> ...");
    }
    out
}

/// Module part of a scope name (`Smells.f` to `Smells`), mirroring
/// `Credo.Code.Scope.mod_name/1`: a trailing lowercase/underscore segment is
/// a function name and is dropped.
#[must_use]
pub fn mod_name(scope: &str) -> String {
    let mut parts: Vec<&str> = scope.split('.').collect();
    let function_segment = parts
        .last()
        .and_then(|base| base.chars().next())
        .is_some_and(|first| first == '_' || first.is_ascii_lowercase());
    if function_segment {
        parts.pop();
        parts.join(".")
    } else {
        scope.to_owned()
    }
}

/// The five fixed categories in native display order with their titles and
/// description bodies (heredoc text with leading indentation stripped).
const CATEGORIES: &[(&str, &str)] = &[
    (
        "Code Readability",
        "Readability checks do not concern themselves with the technical correctness\n\
         of your code, but how easy it is to digest.\n",
    ),
    (
        "Software Design",
        "While refactor checks show you possible problems, these checks try to\n\
         highlight possibilities, like - potentially intended - duplicated code or\n\
         TODO and FIXME comments.\n",
    ),
    (
        "Refactoring opportunities",
        "The Refactor checks show you opportunities to avoid future problems and\n\
         technical debt.\n",
    ),
    (
        "Warnings - please take a look",
        "These checks warn you about things that are potentially dangerous, like a\n\
         missed call to `IEx.pry` you put in during a debugging session or a call\n\
         to String.downcase without using the result.\n",
    ),
    (
        "Consistency",
        "These checks take a look at your code and ensure a consistent coding style.\n\
         Using tabs or spaces? Both is fine, just don't mix them or Credo will tell\n\
         you.\n",
    ),
];

/// 80-column section title: one leading space plus the padded name.
fn push_padded_title(out: &mut String, title: &str) {
    let mut head = format!(" {title}");
    while head.chars().count() < TERM_WIDTH - 1 {
        head.push(' ');
    }
    out.push(' ');
    out.push_str(&head);
    out.push('\n');
}

/// One `list` file section: blank line, module-name title, bare edge, then
/// each issue in line order.
fn push_file_section(out: &mut String, source: &str, issues: &[&Issue]) {
    let title = issues.first().map_or(String::new(), |first| {
        mod_name(first.scope.as_deref().unwrap_or(""))
    });
    out.push('\n');
    push_padded_title(out, &title);
    out.push_str("┃ \n");
    for issue in issues {
        push_issue(out, source, issue);
    }
}

/// One `list` issue: message line, `path:line:col (Scope)` line, source
/// excerpt with caret underline, and the indented closing edge.
fn push_issue(out: &mut String, source: &str, issue: &Issue) {
    let tag = category_tag(issue.category);
    let arrow = priority_arrow(issue.priority);
    out.push_str("┃ [");
    out.push_str(tag);
    out.push_str("] ");
    out.push_str(arrow);
    out.push(' ');
    out.push_str(&issue.message);
    out.push('\n');
    out.push_str("┃       ");
    out.push_str(&issue.filename);
    out.push_str(&pos_suffix(issue.line_no, issue.column));
    out.push_str(" (");
    out.push_str(issue.scope.as_deref().unwrap_or(""));
    out.push_str(")\n");
    if let Some(line_no) = issue.line_no {
        let raw = line_no
            .checked_sub(1)
            .and_then(|index| source.split('\n').nth(index))
            .unwrap_or("");
        let trimmed = raw.trim();
        out.push_str("┃ \n");
        out.push_str("┃       ");
        out.push_str(&truncate(trimmed, TERM_WIDTH - INDENT));
        out.push('\n');
        if issue.column.is_some() {
            let offset = raw.chars().count() - trimmed.chars().count();
            let column = issue.column.unwrap_or(0);
            let indent = column.saturating_sub(offset).saturating_sub(1);
            let width = trigger_width(issue);
            out.push_str("┃       ");
            for _ in 0..indent {
                out.push(' ');
            }
            for _ in 0..width {
                out.push('^');
            }
            out.push('\n');
        }
    }
    out.push_str("┃       \n");
}

/// Caret width: trigger length in characters, one when there is no trigger.
fn trigger_width(issue: &Issue) -> usize {
    match &issue.trigger {
        qredo::IssueTrigger::NoTrigger => 1,
        qredo::IssueTrigger::Text(trigger) => trigger.chars().count(),
    }
}

/// Native `Filename.pos_suffix/2`.
fn pos_suffix(line_no: Option<usize>, column: Option<usize>) -> String {
    match (line_no, column) {
        (None, None) => String::new(),
        (Some(line), None) => format!(":{line}"),
        (Some(line), Some(column)) => format!(":{line}:{column}"),
        (None, Some(column)) => format!("::{column}"),
    }
}

/// Native `UI.truncate/2` with the `…` ellipsis over scalar counts.
fn truncate(line: &str, max: usize) -> String {
    if line.chars().count() <= max {
        return line.to_owned();
    }
    if max <= 1 {
        return "…".to_owned();
    }
    let kept: String = line.chars().take(max - 1).collect();
    format!("{kept}…")
}

/// Tag letter for one issue's category (`refactor` maps to `F`).
fn category_tag(category: Category) -> &'static str {
    match category {
        Category::Consistency => "C",
        Category::Design => "D",
        Category::Readability => "R",
        Category::Refactor => "F",
        Category::Warning => "W",
    }
}

/// Native arrow per integer priority.
fn priority_arrow(priority: i32) -> &'static str {
    if priority > 19 {
        "\u{2191}"
    } else if priority >= 10 {
        "\u{2197}"
    } else if priority >= 0 {
        "\u{2192}"
    } else if priority >= -10 {
        "\u{2198}"
    } else {
        "\u{2193}"
    }
}

/// One verbose `info` config entry: label line, blank line, inspect dump.
fn push_config_entry(out: &mut String, label: &str, inspected: &str) {
    out.push_str("\n    - ");
    out.push_str(label);
    out.push_str("\n\n");
    out.push_str(inspected);
    out.push('\n');
}

/// Cry for help, timing line, `mods/funs` summary and hint line, mirroring
/// the suggest summary exactly.
fn push_summary(out: &mut String, files: &[RunnerFile], report: &RunReport, context: &ListContext) {
    let valid: Vec<&RunnerFile> = files
        .iter()
        .filter(|file| {
            !report
                .skipped_invalid
                .iter()
                .any(|skipped| skipped == &file.filename)
        })
        .collect();
    out.push('\n');
    out.push_str("Please report incorrect results: https://github.com/rrrene/credo/issues\n");
    out.push('\n');
    out.push_str(&timing_text(context, valid.len()));
    out.push('\n');
    out.push_str(&found_text(&valid, report));
    out.push('\n');
    out.push('\n');
    if context.strict_hint {
        out.push_str(
            "Use `mix credo explain` to explain issues, `mix credo --help` for options.\n",
        );
    } else {
        out.push_str("Showing priority issues: ↑ ↗ →  (use `mix credo explain` to explain issues, `mix credo --help` for options).\n");
    }
}

/// `Analysis took ...` with native centisecond formatting and singulars.
fn timing_text(context: &ListContext, file_count: usize) -> String {
    let load = context.load_microseconds / MICROS_PER_CENTI;
    let run = context.run_microseconds / MICROS_PER_CENTI;
    let total = format_seconds(load + run);
    let total_text = if total == "1.0" {
        "1 second".to_owned()
    } else {
        format!("{total} seconds")
    };
    let checks = if context.check_count == 1 {
        "1 check".to_owned()
    } else {
        format!("{} checks", context.check_count)
    };
    let files = if file_count == 1 {
        "1 file".to_owned()
    } else {
        format!("{file_count} files")
    };
    format!(
        "Analysis took {total_text} ({}s to load, {}s running {checks} on {files})",
        format_seconds(load),
        format_seconds(run)
    )
}

/// Native `format_in_seconds/1` over centiseconds.
fn format_seconds(centis: u64) -> String {
    if centis < 10 {
        format!("0.0{centis}")
    } else {
        let decis = centis / 10;
        format!("{}.{}", decis / 10, decis % 10)
    }
}

/// `N mods/funs, found ... .` with per-category wording in native order.
fn found_text(valid: &[&RunnerFile], report: &RunReport) -> String {
    let mods: usize = valid
        .iter()
        .map(|file| qredo::format_default::count_mods_funs(&file.source))
        .sum();
    let mut parts = Vec::new();
    for (category, singular, plural) in summary_parts() {
        let count = report
            .issues
            .iter()
            .filter(|issue| issue.category == category)
            .count();
        if count == 0 {
            continue;
        }
        if count == 1 {
            parts.push(format!("1 {singular}"));
        } else {
            parts.push(format!("{count} {plural}"));
        }
    }
    if parts.is_empty() {
        format!("{mods} mods/funs, found no issues.")
    } else {
        format!("{mods} mods/funs, found {}.", parts.join(", "))
    }
}

/// Summary wording in native `@category_wording` order.
fn summary_parts() -> [(Category, &'static str, &'static str); 5] {
    [
        (
            Category::Consistency,
            "consistency issue",
            "consistency issues",
        ),
        (Category::Warning, "warning", "warnings"),
        (
            Category::Refactor,
            "refactoring opportunity",
            "refactoring opportunities",
        ),
        (
            Category::Readability,
            "code readability issue",
            "code readability issues",
        ),
        (
            Category::Design,
            "software design suggestion",
            "software design suggestions",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use qredo::{Category, IssueTrigger};

    const SMELLS: &str =
        "defmodule Smells do\n  def f(x) do\n    IO.inspect(x)\n    dbg(x)\n  end\nend\n";
    const CLEAN: &str = "defmodule Clean do\n  @moduledoc \"Clean.\"\n  def f(x), do: x\nend\n";
    const SMELLS_TEST: &str = "defmodule SmellsTest do\n  use ExUnit.Case\nend\n";

    /// Eight fields mirror the full `Issue` shape the builder fills in.
    #[allow(clippy::too_many_arguments)]
    fn issue(
        check: &str,
        category: Category,
        priority: i32,
        exit_status: i32,
        message: &str,
        filename: &str,
        line: usize,
        column: usize,
        trigger: &str,
        scope: &str,
    ) -> Issue {
        Issue {
            check: check.to_owned(),
            category,
            priority,
            severity: 1.0,
            message: message.to_owned(),
            filename: filename.to_owned(),
            line_no: Some(line),
            column: Some(column),
            exit_status,
            trigger: IssueTrigger::Text(trigger.to_owned()),
            scope: Some(scope.to_owned()),
        }
    }

    fn smells_issues() -> Vec<Issue> {
        let file = "/tmp/probe-sib/lib/smells.ex";
        vec![
            issue(
                "Credo.Check.Readability.ModuleDoc",
                Category::Readability,
                1,
                4,
                "Modules should have a @moduledoc tag.",
                file,
                1,
                11,
                "Smells",
                "Smells",
            ),
            issue(
                "Credo.Check.Warning.IoInspect",
                Category::Warning,
                11,
                16,
                "There should be no calls to `IO.inspect/1`.",
                file,
                3,
                5,
                "IO.inspect",
                "Smells.f",
            ),
            issue(
                "Credo.Check.Warning.Dbg",
                Category::Warning,
                11,
                16,
                "There should be no calls to `dbg/1`.",
                file,
                4,
                5,
                "dbg",
                "Smells.f",
            ),
        ]
    }

    fn three_files() -> Vec<RunnerFile> {
        vec![
            RunnerFile {
                filename: "/tmp/probe-sib/lib/clean.ex".to_owned(),
                source: CLEAN.to_owned(),
            },
            RunnerFile {
                filename: "/tmp/probe-sib/lib/smells.ex".to_owned(),
                source: SMELLS.to_owned(),
            },
            RunnerFile {
                filename: "/tmp/probe-sib/test/smells_test.exs".to_owned(),
                source: SMELLS_TEST.to_owned(),
            },
        ]
    }

    fn report(issues: Vec<Issue>) -> RunReport {
        let exit_status = issues
            .iter()
            .fold(0, |status, issue| status | issue.exit_status);
        RunReport {
            issues,
            exit_status,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
            mods_funs: None,
        }
    }

    /// Pinned verbatim: the probe `list` run over the three fixture files
    /// (timings stand in for the probed `0.00s to load, 0.01s running`).
    #[test]
    fn list_three_issue_fixture_is_byte_exact() {
        let context = ListContext {
            check_count: 3,
            load_microseconds: 0,
            run_microseconds: 15_000,
            ..ListContext::default()
        };
        let (rendered, exit) = run_list(&three_files(), &report(smells_issues()), &context);
        assert_eq!(exit, 20);
        let title = format!("  Smells{}", " ".repeat(72));
        let expected = format!(
            "\n\
             No files found!\n\
             \n\
             {title}\n\
             ┃ \n\
             ┃ [R] → Modules should have a @moduledoc tag.\n\
             ┃       /tmp/probe-sib/lib/smells.ex:1:11 (Smells)\n\
             ┃ \n\
             ┃       defmodule Smells do\n\
             ┃                 ^^^^^^\n\
             ┃       \n\
             ┃ [W] ↗ There should be no calls to `IO.inspect/1`.\n\
             ┃       /tmp/probe-sib/lib/smells.ex:3:5 (Smells.f)\n\
             ┃ \n\
             ┃       IO.inspect(x)\n\
             ┃       ^^^^^^^^^^\n\
             ┃       \n\
             ┃ [W] ↗ There should be no calls to `dbg/1`.\n\
             ┃       /tmp/probe-sib/lib/smells.ex:4:5 (Smells.f)\n\
             ┃ \n\
             ┃       dbg(x)\n\
             ┃       ^^^\n\
             ┃       \n\
             \n\
             Please report incorrect results: https://github.com/rrrene/credo/issues\n\
             \n\
             Analysis took 0.01 seconds (0.00s to load, 0.01s running 3 checks on 3 files)\n\
             5 mods/funs, found 2 warnings, 1 code readability issue.\n\
             \n\
             Showing priority issues: ↑ ↗ →  (use `mix credo explain` to explain issues, `mix credo --help` for options).\n"
        );
        assert_eq!(rendered, expected);
    }

    /// Pinned verbatim: the probe `list` run over the clean file only.
    #[test]
    fn list_clean_run_is_byte_exact() {
        let context = ListContext {
            check_count: 3,
            ..ListContext::default()
        };
        let files = vec![RunnerFile {
            filename: "/tmp/probe-sib/lib/clean.ex".to_owned(),
            source: CLEAN.to_owned(),
        }];
        let (rendered, exit) = run_list(&files, &report(Vec::new()), &context);
        assert_eq!(exit, 0);
        let expected = "\n\
             No files found!\n\
             \n\
             Please report incorrect results: https://github.com/rrrene/credo/issues\n\
             \n\
             Analysis took 0.00 seconds (0.00s to load, 0.00s running 3 checks on 1 file)\n\
             2 mods/funs, found no issues.\n\
             \n\
             Showing priority issues: ↑ ↗ →  (use `mix credo explain` to explain issues, `mix credo --help` for options).\n";
        assert_eq!(rendered, expected);
    }

    /// Pinned verbatim: the probe `categories` run (headers are 80 columns;
    /// description lines carry two spaces after `┃`).
    #[test]
    fn categories_full_text_is_byte_exact() {
        let (rendered, exit) = run_categories();
        assert_eq!(exit, 0);
        let header = |title: &str| format!("  {title}{}", " ".repeat(78 - title.len()));
        let expected = format!(
            "\n\
             {readability}\n\
             ┃ \n\
             ┃  Readability checks do not concern themselves with the technical correctness\n\
             ┃  of your code, but how easy it is to digest.\n\
             ┃  \n\
             \n\
             {design}\n\
             ┃ \n\
             ┃  While refactor checks show you possible problems, these checks try to\n\
             ┃  highlight possibilities, like - potentially intended - duplicated code or\n\
             ┃  TODO and FIXME comments.\n\
             ┃  \n\
             \n\
             {refactor}\n\
             ┃ \n\
             ┃  The Refactor checks show you opportunities to avoid future problems and\n\
             ┃  technical debt.\n\
             ┃  \n\
             \n\
             {warning}\n\
             ┃ \n\
             ┃  These checks warn you about things that are potentially dangerous, like a\n\
             ┃  missed call to `IEx.pry` you put in during a debugging session or a call\n\
             ┃  to String.downcase without using the result.\n\
             ┃  \n\
             \n\
             {consistency}\n\
             ┃ \n\
             ┃  These checks take a look at your code and ensure a consistent coding style.\n\
             ┃  Using tabs or spaces? Both is fine, just don't mix them or Credo will tell\n\
             ┃  you.\n\
             ┃  \n",
            readability = header("Code Readability"),
            design = header("Software Design"),
            refactor = header("Refactoring opportunities"),
            warning = header("Warnings - please take a look"),
            consistency = header("Consistency"),
        );
        assert_eq!(rendered, expected);
    }

    /// Pinned verbatim: default `info` prints only the `System:` section.
    #[test]
    fn info_default_is_system_only() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: []}}]}\n";
        let config = qredo::parse_config(source, "default").expect("parses");
        let context = InfoContext {
            credo_version: "1.8.0-dev",
            elixir_version: "1.20.2",
            erlang_version: "29",
            verbose: false,
            embedded_inspect: "\"embedded\"",
            config: &config,
            config_path: "/tmp/probe-sib/abs.credo.exs",
            config_source: source,
            files: &[],
        };
        let (rendered, exit) = run_info(&context);
        assert_eq!(exit, 0);
        assert_eq!(
            rendered,
            "System:\n  Credo: 1.8.0-dev\n  Elixir: 1.20.2\n  Erlang: 29\n"
        );
    }

    /// Smoke over verbose `info`: config inventory with the double-listed
    /// config file, first-positional-only `Files:`, `Elixir.`-prefixed
    /// `Checks:` read off the parsed config.
    #[test]
    fn info_verbose_lists_config_twice_and_first_file_only() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}, {Credo.Check.Warning.Dbg, []}, {Credo.Check.Readability.ModuleDoc, []}]}}]}";
        let config = qredo::parse_config(source, "default").expect("parses");
        let files = vec!["/tmp/probe-sib/lib/clean.ex".to_owned()];
        let context = InfoContext {
            credo_version: "1.8.0-dev",
            elixir_version: "1.20.2",
            erlang_version: "29",
            verbose: true,
            embedded_inspect: "\"embedded\"",
            config: &config,
            config_path: "/tmp/probe-sib/abs.credo.exs",
            config_source: source,
            files: &files,
        };
        let (rendered, exit) = run_info(&context);
        assert_eq!(exit, 0);
        assert!(rendered.contains("System:\n  Credo: 1.8.0-dev\n"));
        assert!(rendered.contains("Configuration:\n  Configs:\n"));
        assert!(rendered.contains("    - ## :credo\n"));
        assert_eq!(
            rendered
                .matches("    - ## file: /tmp/probe-sib/abs.credo.exs\n")
                .count(),
            2,
            "config file is listed twice (native append-twice quirk)"
        );
        assert!(rendered.contains("  Files:\n    - /tmp/probe-sib/lib/clean.ex\n"));
        assert!(
            !rendered.contains("smells.ex"),
            "only the first positional is listed"
        );
        assert!(rendered.contains("  Checks:\n    - Elixir.Credo.Check.Warning.IoInspect\n"));
        assert!(rendered.contains("    - Elixir.Credo.Check.Warning.Dbg\n"));
        assert!(rendered.contains("    - Elixir.Credo.Check.Readability.ModuleDoc\n"));
    }

    #[test]
    fn scope_module_name_drops_function_segment() {
        assert_eq!(mod_name("Smells"), "Smells");
        assert_eq!(mod_name("Smells.f"), "Smells");
        assert_eq!(mod_name("A.B.c"), "A.B");
        assert_eq!(mod_name("A.B"), "A.B");
        assert_eq!(mod_name(""), "");
    }

    #[test]
    fn inspect_string_escapes_like_elixir() {
        assert_eq!(elixir_inspect_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(elixir_inspect_string("x\ny"), "\"x\\ny\"");
    }

    #[test]
    fn inspect_string_applies_printable_limit() {
        // `inspect/2` keeps the first 4096 characters, then `" <> ...`.
        let long = "a".repeat(5000);
        let rendered = elixir_inspect_string(&long);
        assert_eq!(rendered.len(), 1 + 4096 + 8);
        assert!(rendered.ends_with("\" <> ..."));
        assert_eq!(
            elixir_inspect_string(&"a".repeat(4096)),
            format!("\"{}\"", "a".repeat(4096))
        );
    }
}
