//! Byte-exact DEFAULT (`suggest`) human formatter.
//!
//! Mirrors `Credo.CLI.Command.Suggest.Output.Default` with
//! `Credo.CLI.Output.Summary` for piped (non-TTY) runs at 80 columns.
//! Everything here is derived from empirical captures of the pinned
//! upstream (`work/ref/credo` @ `ea1ccb9`, Elixir 1.20.2 / OTP 29;
//! see `work/p1-formats-probe.md`, `work/p1-flags-probe.md` and the
//! `/tmp/p6/logs` captures), cross-checked against the upstream source
//! listed under "Provenance" below.
//!
//! Scope: default format only. `oneline`/`flycheck`/`json`/`sarif` live
//! elsewhere. Non-verbose, non-`first-run` runs only (the binary exposes
//! neither flag): issue messages render without the verbose
//! `[CheckModule]` suffix and without source excerpts, and the
//! first-run hint never prints. Version-gated skips and invalid-file
//! complaints go to stderr in native and stay the caller's job.
//!
//! Provenance (upstream files consulted after probing):
//! `cli/command/suggest/output/default.ex` (sections, truncation,
//! ordering, wrapping entry), `cli/output/summary.ex` (cry, timing,
//! summary parts, hints), `cli/output/ui.ex` (`edge`, `wrap_at`),
//! `cli/output/shell.ex` + `bunt` (color stripping and the trailing
//! reset), `cli/output.ex` (tags, colors, arrows), `cli/sorter.ex`
//! (category order), `cli/filename.ex` (`pos_suffix`), `priority.ex`
//! (integer to arrow/color mapping).
//!
//! Wiring contract for `main.rs`: replace the current `print_text` /
//! `print_json` dispatch for `Format::Default` with
//! `print!("{}", format_default::render(&report, &files, &context))`
//! and drop the ad-hoc `println!("No files found!")` (the renderer owns
//! the whole stdout, including that line). Build the context as:
//! - `check_count`: checks executed (native post-`PrepareChecksToRun`
//!   count; for served configs the enabled-check count).
//! - `load_microseconds` / `run_microseconds`: measured wall times;
//!   native divides each by `10_000` before formatting (see [`render`]).
//! - `color`: `Some(true)` only, and only when stdout is a real TTY
//!   (native never colorizes pipes, even with `--color`).
//! - `show_all`: `--all`, or anything implying it (`--strict` sets
//!   `min_priority` to -99, and native shows all when
//!   `min_priority <= -99`).
//! - `strict_hint`: resolved `min_priority < 0` (native prints the
//!   default "Showing..." hint iff `min_priority >= 0`).
//! - `locale_utf8`: false under a C/POSIX locale (native then emits
//!   `\x{HEX}` escapes for codepoints above U+00FF).
//!
//! Open questions / known limits:
//! - Full-tie display order (same priority, severity, filename and line)
//!   reverses native execution order; the input here arrives check-sorted,
//!   so such ties reverse check-name order instead. No probed case hits it.
//! - `mods/funs` counts tree-sitter `defmodule` + `def`-family calls; the
//!   probed edge cases agree (including `defmacrop`/`defguard`/
//!   `defprotocol` exclusion), but grammar-vs-compiler parse differences on
//!   exotic files remain a differential follow-up.
//! - C-locale bytes 0x80-0xFF (e.g. `é` in a filename) stay UTF-8 here;
//!   native re-encodes them to single latin-1 bytes, which a Rust `String`
//!   cannot hold. Only the `\x{HEX}` escapes above U+00FF are reproduced.
//! - Terminal widths other than 80 are not modeled; headers always pad to
//!   80 and messages wrap at 72.

use crate::issue::{Category, Issue};
use crate::runner::{RunReport, RunnerFile};

/// Rendering inputs that the report and files do not carry.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatContext {
    /// Checks executed, shown in the timing line.
    pub check_count: usize,
    /// Load-phase microseconds (`credo.time.source_files`).
    pub load_microseconds: u64,
    /// Analysis-phase microseconds (`credo.time.run_checks`).
    pub run_microseconds: u64,
    /// Emit TTY escape sequences. Defaults to false: piped output never
    /// carries them.
    pub color: bool,
    /// Disable the 5-per-category cap (`--all`, implied by `--strict`).
    pub show_all: bool,
    /// Hint variant: true selects the `--strict` line, false the default
    /// "Showing priority issues..." line.
    pub strict_hint: bool,
    /// False reproduces the C-locale `\x{HEX}` rendering.
    pub locale_utf8: bool,
}

impl Default for FormatContext {
    fn default() -> Self {
        Self {
            check_count: 0,
            load_microseconds: 0,
            run_microseconds: 0,
            color: false,
            show_all: false,
            strict_hint: false,
            locale_utf8: true,
        }
    }
}

/// Piped terminal width; headers pad to it, messages wrap 8 columns short.
const TERM_WIDTH: usize = 80;
/// Location-line and continuation-line content indent.
const INDENT: usize = 8;
/// File counts above this warn the run might take a while.
const MANY_FILES: usize = 60;
/// Default per-category issue cap without `--all`.
const PER_CATEGORY: usize = 5;
/// Microseconds per native timing centisecond.
const MICROS_PER_CENTI: u64 = 10_000;

/// Render the complete default-format stdout, ending with one newline.
///
/// Timings are caller-measured wall times: native divides each by `10_000`
/// and formats `<0.1s` with two decimals, the rest with one. File counts
/// and `mods/funs` cover valid files only (invalid ones leave all three,
/// like native). Sections print every reported issue, grouped and ordered
/// exactly like native.
#[must_use]
pub fn render(report: &RunReport, files: &[RunnerFile], context: &FormatContext) -> String {
    let valid: Vec<&RunnerFile> = files
        .iter()
        .filter(|file| {
            !report
                .skipped_invalid
                .iter()
                .any(|skipped| skipped == &file.filename)
        })
        .collect();
    let mut out = String::new();
    push_header(&mut out, valid.len(), context);
    for category in [
        Category::Design,
        Category::Readability,
        Category::Refactor,
        Category::Warning,
        Category::Consistency,
    ] {
        let mut selected: Vec<&Issue> = report
            .issues
            .iter()
            .filter(|issue| issue.category == category)
            .collect();
        if selected.is_empty() {
            continue;
        }
        // Native sorts ascending by the tuple, then reverses the whole
        // list; the stable sort plus reverse below matches that exactly.
        selected.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.severity.total_cmp(&right.severity))
                .then_with(|| left.filename.cmp(&right.filename))
                .then_with(|| left.line_no.cmp(&right.line_no))
        });
        selected.reverse();
        push_section(&mut out, category, &selected, context);
    }
    push_summary(&mut out, report, &valid, context);
    out
}

/// Count `defmodule` + `def`/`defp`/`defmacro` definitions in one file,
/// mirroring native `scope_count/1` over the parsed AST. Unparseable input
/// contributes nothing; the pipeline already excludes such files with a
/// complaint.
#[must_use]
pub fn count_mods_funs(source: &str) -> usize {
    let Some(tree) = crate::ts_parser::parse(source) else {
        return 0;
    };
    if tree.root_node().has_error() {
        return 0;
    }
    let facts = crate::facts::extract(&tree, source);
    facts.modules.len() + facts.defs.len()
}

/// One styled line: escape codes stay raw while every text span passes
/// locale escaping. A trailing reset is appended iff color is on and at
/// least one sequence was emitted (mirroring `IO.ANSI.format/2`).
struct Line<'a> {
    context: &'a FormatContext,
    parts: Vec<Part>,
    saw_sequence: bool,
}

/// Raw escape code (never locale-escaped) or text (always escaped).
enum Part {
    Sequence(&'static str),
    Text(String),
}

impl<'a> Line<'a> {
    fn new(context: &'a FormatContext) -> Self {
        Self {
            context,
            parts: Vec::new(),
            saw_sequence: false,
        }
    }

    fn seq(&mut self, code: &'static str) -> &mut Self {
        self.parts.push(Part::Sequence(code));
        self.saw_sequence = true;
        self
    }

    fn text(&mut self, text: impl Into<String>) -> &mut Self {
        self.parts.push(Part::Text(text.into()));
        self
    }

    fn finish(self, out: &mut String) {
        for part in self.parts {
            match part {
                Part::Sequence(code) => {
                    if self.context.color {
                        out.push_str("\x1b[");
                        out.push_str(code);
                        out.push('m');
                    }
                }
                Part::Text(text) => push_locale(out, &text, self.context.locale_utf8),
            }
        }
        if self.context.color && self.saw_sequence {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
}

/// Append text, escaping non-latin-1 codepoints as `\x{HEX}` (uppercase,
/// unpadded) outside UTF-8 locales. Codepoints at or below U+00FF stay as
/// Unicode scalars (native re-encodes them to latin-1 bytes, which a Rust
/// `String` cannot hold; see the module docs).
fn push_locale(out: &mut String, text: &str, utf8: bool) {
    if utf8 || text.is_ascii() {
        out.push_str(text);
        return;
    }
    for c in text.chars() {
        if (c as u32) <= 0xFF {
            out.push(c);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "\\x{{{:X}}}", c as u32);
        }
    }
}

/// `Checking ...` header plus its trailing blank line. Invalid files are
/// already excluded from the count, like native post-validation state.
fn push_header(out: &mut String, file_count: usize, context: &FormatContext) {
    let mut line = Line::new(context);
    match file_count {
        0 => {
            line.text("No files found!");
        }
        1 => {
            line.text("Checking 1 source file ...");
        }
        count => {
            line.text(format!("Checking {count} source files"));
            if count > MANY_FILES {
                line.text(" (this might take a while)");
            }
            line.text(" ...");
        }
    }
    line.finish(out);
}

/// Padded section header, bare edge, up to five issues and the overflow
/// hint for one category.
fn push_section(
    out: &mut String,
    category: Category,
    selected: &[&Issue],
    context: &FormatContext,
) {
    let style = category_style(category);
    out.push('\n');
    let mut title = format!(" {}", style.title);
    while title.chars().count() < TERM_WIDTH - 1 {
        title.push(' ');
    }
    let mut head = Line::new(context);
    head.seq("1")
        .seq(style.background)
        .seq(style.color)
        .text(" ")
        .seq(style.foreground)
        .seq("22")
        .text(title);
    head.finish(out);
    let mut edge = Line::new(context);
    edge.seq("0").seq(style.color).text("┃ ");
    edge.finish(out);
    let shown = if context.show_all {
        selected.len()
    } else {
        selected.len().min(PER_CATEGORY)
    };
    for issue in &selected[..shown] {
        push_issue(out, issue, context);
    }
    if selected.len() > shown {
        let mut hint = Line::new(context);
        hint.seq("0")
            .seq(style.color)
            .text("┃ ")
            .seq("2")
            .text(format!(
                " ...  ({} more, use `--all` to show them)",
                selected.len() - shown
            ));
        hint.finish(out);
    }
}

/// Per-issue display attributes: category color, priority arrow and tag
/// weight (faint when the issue color matches the category color).
struct IssueStyle {
    outer: &'static str,
    arrow: &'static str,
    tag_style: &'static str,
}

impl IssueStyle {
    fn of(issue: &Issue) -> Self {
        let (arrow, inner) = priority_style(issue.priority);
        let outer = category_style(issue.category).color;
        Self {
            outer,
            arrow,
            tag_style: if inner == outer { "2" } else { "1" },
        }
    }
}

/// One issue message (wrapped) plus its `path:line:col #(Scope)` line.
fn push_issue(out: &mut String, issue: &Issue, context: &FormatContext) {
    let style = IssueStyle::of(issue);
    push_message_lines(out, issue, &style, context);
    push_location(out, issue, &style, context);
}

/// Wrapped message lines: tagged first line, indented continuations.
fn push_message_lines(
    out: &mut String,
    issue: &Issue,
    style: &IssueStyle,
    context: &FormatContext,
) {
    let chunks = wrap_at(&issue.message, TERM_WIDTH - INDENT);
    let Some((first, rest)) = chunks.split_first() else {
        return;
    };
    let mut message = Line::new(context);
    message
        .seq("0")
        .seq(style.outer)
        .text("┃ ")
        .seq(style.outer)
        .seq(style.tag_style)
        .text(format!("[{}]", category_tag(issue.category)))
        .text(" ")
        .text(style.arrow)
        .seq("22")
        .seq(style.outer)
        .text(" ")
        .text(first.clone());
    message.finish(out);
    for chunk in rest {
        // Empty chunks print nothing, like the native `""` clause.
        if chunk.is_empty() {
            continue;
        }
        let mut continued = Line::new(context);
        continued
            .seq("0")
            .seq(style.outer)
            .text("┃ ")
            .seq(style.outer)
            .text("     ")
            .seq("22")
            .seq(style.outer)
            .text(" ")
            .text(chunk.clone());
        continued.finish(out);
    }
}

/// `path:line:col #(Scope)` location line under the message.
fn push_location(out: &mut String, issue: &Issue, style: &IssueStyle, context: &FormatContext) {
    let mut location = Line::new(context);
    location
        .seq("0")
        .seq(style.outer)
        .text("┃       ")
        .seq("39")
        .seq("2")
        .text(issue.filename.clone())
        .seq("39")
        .seq("2")
        .text(pos_suffix(issue.line_no, issue.column))
        .seq("8")
        .text(" #")
        .seq("0")
        .seq("2")
        .text(format!("({})", issue.scope.as_deref().unwrap_or("")));
    location.finish(out);
}

/// Cry for help, timing line, `mods/funs` summary and the hint line.
fn push_summary(
    out: &mut String,
    report: &RunReport,
    valid: &[&RunnerFile],
    context: &FormatContext,
) {
    out.push('\n');
    let mut cry = Line::new(context);
    cry.seq("2")
        .text("Please report incorrect results: https://github.com/rrrene/credo/issues");
    cry.finish(out);
    out.push('\n');
    let mut timing = Line::new(context);
    timing.seq("2").text(timing_text(context, valid.len()));
    timing.finish(out);
    push_found(out, report, valid, context);
    out.push('\n');
    let mut hint = Line::new(context);
    hint.seq("2").text(if context.strict_hint {
        "Use `mix credo explain` to explain issues, `mix credo --help` for options."
    } else {
        "Showing priority issues: ↑ ↗ →  (use `mix credo explain` to explain issues, `mix credo --help` for options)."
    });
    hint.finish(out);
}

/// `Analysis took ...` with native centisecond formatting and singulars.
fn timing_text(context: &FormatContext, file_count: usize) -> String {
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

/// Native `format_in_seconds/1` over centiseconds: two decimals below
/// 0.1s, one above.
fn format_seconds(centis: u64) -> String {
    if centis < 10 {
        format!("0.0{centis}")
    } else {
        let decis = centis / 10;
        format!("{}.{}", decis / 10, decis % 10)
    }
}

/// `N mods/funs, found ... .` with per-category colors in native order.
fn push_found(
    out: &mut String,
    report: &RunReport,
    valid: &[&RunnerFile],
    context: &FormatContext,
) {
    let mods: usize = valid.iter().map(|file| count_mods_funs(&file.source)).sum();
    let mut line = Line::new(context);
    line.seq("32").text(format!("{mods} mods/funs, "));
    line.seq("0").text("found ");
    let mut parts: Vec<(&str, String)> = Vec::new();
    for (category, singular, plural, color) in summary_parts() {
        let count = report
            .issues
            .iter()
            .filter(|issue| issue.category == category)
            .count();
        if count == 0 {
            continue;
        }
        let text = if count == 1 {
            format!("1 {singular}, ")
        } else {
            format!("{count} {plural}, ")
        };
        parts.push((color, text));
    }
    if parts.is_empty() {
        line.text("no issues");
    } else {
        if let Some((_, last)) = parts.last_mut() {
            last.truncate(last.len() - ", ".len());
        }
        for (color, text) in parts {
            line.seq(color).text(text);
        }
    }
    line.text(".");
    line.finish(out);
}

/// Summary wording and colors in native `@category_wording` order.
fn summary_parts() -> [(Category, &'static str, &'static str, &'static str); 5] {
    [
        (
            Category::Consistency,
            "consistency issue",
            "consistency issues",
            "36",
        ),
        (Category::Warning, "warning", "warnings", "31"),
        (
            Category::Refactor,
            "refactoring opportunity",
            "refactoring opportunities",
            "33",
        ),
        (
            Category::Readability,
            "code readability issue",
            "code readability issues",
            "34",
        ),
        (
            Category::Design,
            "software design suggestion",
            "software design suggestions",
            "38;5;100",
        ),
    ]
}

/// Native `Filename.pos_suffix/2`.
fn pos_suffix(line_no: Option<usize>, column: Option<usize>) -> String {
    match (line_no, column) {
        (None, None) => String::new(),
        (Some(line), None) => format!(":{line}"),
        (Some(line), Some(column)) => format!(":{line}:{column}"),
        // Unreachable (columns imply lines); mirrors `":#{nil}:#{column}"`.
        (None, Some(column)) => format!("::{column}"),
    }
}

/// Category display style: foreground, background, header foreground,
/// section title and issue tag.
struct CategoryStyle {
    color: &'static str,
    background: &'static str,
    foreground: &'static str,
    title: &'static str,
    tag: &'static str,
}

/// 256-color olive used for the design category.
const OLIVE: &str = "38;5;100";
/// 256-color olive background.
const OLIVE_BACKGROUND: &str = "48;5;100";

/// Native `@category_colors`, `@category_titles` and check tags
/// (`refactor` maps to `F`, the rest to first letters).
fn category_style(category: Category) -> CategoryStyle {
    match category {
        Category::Design => CategoryStyle {
            color: OLIVE,
            background: OLIVE_BACKGROUND,
            foreground: "37",
            title: "Software Design",
            tag: "D",
        },
        Category::Readability => CategoryStyle {
            color: "34",
            background: "44",
            foreground: "37",
            title: "Code Readability",
            tag: "R",
        },
        Category::Refactor => CategoryStyle {
            color: "33",
            background: "43",
            foreground: "30",
            title: "Refactoring opportunities",
            tag: "F",
        },
        Category::Warning => CategoryStyle {
            color: "31",
            background: "41",
            foreground: "37",
            title: "Warnings - please take a look",
            tag: "W",
        },
        Category::Consistency => CategoryStyle {
            color: "36",
            background: "46",
            foreground: "30",
            title: "Consistency",
            tag: "C",
        },
    }
}

/// Tag letter for one issue's category.
fn category_tag(category: Category) -> &'static str {
    category_style(category).tag
}

/// Native arrow and issue color per integer priority
/// (`Priority.to_atom/1` ranges with `Output.priority_arrow/1` and
/// `Output.issue_color/1`).
fn priority_style(priority: i32) -> (&'static str, &'static str) {
    if priority > 19 {
        ("\u{2191}", "31")
    } else if priority >= 10 {
        ("\u{2197}", "31")
    } else if priority >= 0 {
        ("\u{2192}", "33")
    } else if priority >= -10 {
        ("\u{2198}", "34")
    } else {
        ("\u{2193}", "35")
    }
}

/// Split a message the way `UI.wrap_at/2` does: each chunk is the longest
/// prefix of at most `width` chars ending after horizontal whitespace
/// (or, at the width, swallowing one following space), else a hard cut.
/// One `\r\n`/`\n` is consumed after each chunk; bare breaks yield empty
/// chunks. Probed against native: `[69, 70, 70, 29]`, `[72, 8]`,
/// `[73, 4]` for the double-space boundary.
fn wrap_at(text: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < chars.len() {
        if chars[pos] == '\n' {
            out.push(String::new());
            pos += 1;
            continue;
        }
        if chars[pos] == '\r' && chars.get(pos + 1) == Some(&'\n') {
            out.push(String::new());
            pos += 2;
            continue;
        }
        // `.` never crosses `\n`, so the grab stops before one.
        let to_break = chars[pos..].iter().take_while(|c| **c != '\n').count();
        let max = width.min(chars.len() - pos).min(to_break).max(1);
        let (len, extra) = search_chunk(&chars, pos, max);
        out.push(chars[pos..pos + len + extra].iter().collect());
        pos += len + extra;
        // `(?:\r?\n)?`: consume one line break after the chunk.
        if chars.get(pos) == Some(&'\n') {
            pos += 1;
        } else if chars.get(pos) == Some(&'\r') && chars.get(pos + 1) == Some(&'\n') {
            pos += 2;
        }
    }
    out
}

/// Descending chunk-length search mirroring the atomic group's internal
/// backtrack: longest prefix of at most `max` chars ending after
/// horizontal whitespace (or, at the width, swallowing one following
/// space), else a hard cut. Returns the length plus any extra consumed
/// trailing space.
fn search_chunk(chars: &[char], pos: usize, max: usize) -> (usize, usize) {
    /// Boundary whitespace: `[^\S\r\n]`, i.e. whitespace but never a break.
    fn boundary_ws(c: char) -> bool {
        c != '\r' && c != '\n' && c.is_whitespace()
    }

    let mut n = max;
    loop {
        let end = pos + n;
        if end == chars.len() {
            return (n, 0);
        }
        let last_ws = boundary_ws(chars[end - 1]);
        let next_is_break =
            chars[end] == '\n' || (chars[end] == '\r' && chars.get(end + 1) == Some(&'\n'));
        let next_ws = !next_is_break && boundary_ws(chars[end]);
        if last_ws {
            return (n, usize::from(next_ws));
        }
        if n == max && next_ws {
            // `[^\S\r\n]` consumes the boundary space after a full grab.
            return (n + 1, 0);
        }
        if next_is_break {
            return (n, 0);
        }
        if n == 1 {
            // Second alternation branch: hard cut.
            return (max, 0);
        }
        n -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::IssueTrigger;

    const SMELLS: &str =
        "defmodule Smells do\n  def f(x) do\n    IO.inspect(x)\n    dbg(x)\n  end\nend\n";
    const CLEAN: &str = "defmodule Clean do\n  @moduledoc \"Clean.\"\n  def f(x), do: x\nend\n";
    const TEST_FILE: &str = "defmodule SmellsTest do\n  use ExUnit.Case\nend\n";

    /// Eight fields mirror the full `Issue` shape the builder fills in.
    #[allow(clippy::too_many_arguments)]
    fn issue(
        check: &str,
        category: Category,
        priority: i32,
        message: &str,
        filename: &str,
        line: usize,
        column: usize,
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
            exit_status: category.default_exit_status(),
            trigger: IssueTrigger::Text(String::new()),
            scope: Some(scope.to_owned()),
        }
    }

    fn three_issues() -> Vec<Issue> {
        let file = "/tmp/p6/fx/lib/smells.ex";
        vec![
            issue(
                "Credo.Check.Readability.ModuleDoc",
                Category::Readability,
                1,
                "Modules should have a @moduledoc tag.",
                file,
                1,
                11,
                "Smells",
            ),
            issue(
                "Credo.Check.Warning.IoInspect",
                Category::Warning,
                12,
                "There should be no calls to `IO.inspect/1`.",
                file,
                3,
                5,
                "Smells.f",
            ),
            issue(
                "Credo.Check.Warning.Dbg",
                Category::Warning,
                12,
                "There should be no calls to `dbg/1`.",
                file,
                4,
                5,
                "Smells.f",
            ),
        ]
    }

    fn three_files() -> Vec<RunnerFile> {
        vec![
            RunnerFile {
                filename: "/tmp/p6/fx/lib/smells.ex".to_owned(),
                source: SMELLS.to_owned(),
            },
            RunnerFile {
                filename: "/tmp/p6/fx/lib/clean.ex".to_owned(),
                source: CLEAN.to_owned(),
            },
            RunnerFile {
                filename: "/tmp/p6/fx/test/smells_test.exs".to_owned(),
                source: TEST_FILE.to_owned(),
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
        }
    }

    /// Pinned verbatim: `--strict` 3-issue run (timings zeroed; the
    /// `┃` is U+2503, `→` U+2192, `↗` U+2197). Headers are 80 columns.
    #[test]
    fn strict_three_issue_fixture_is_byte_exact() {
        let context = FormatContext {
            check_count: 3,
            strict_hint: true,
            ..FormatContext::default()
        };
        let rendered = render(&report(three_issues()), &three_files(), &context);
        let expected = "Checking 3 source files ...\n\
            \n\
            \x20 Code Readability                                                              \n\
            ┃ \n\
            ┃ [R] → Modules should have a @moduledoc tag.\n\
            ┃       /tmp/p6/fx/lib/smells.ex:1:11 #(Smells)\n\
            \n\
            \x20 Warnings - please take a look                                                 \n\
            ┃ \n\
            ┃ [W] ↗ There should be no calls to `dbg/1`.\n\
            ┃       /tmp/p6/fx/lib/smells.ex:4:5 #(Smells.f)\n\
            ┃ [W] ↗ There should be no calls to `IO.inspect/1`.\n\
            ┃       /tmp/p6/fx/lib/smells.ex:3:5 #(Smells.f)\n\
            \n\
            Please report incorrect results: https://github.com/rrrene/credo/issues\n\
            \n\
            Analysis took 0.00 seconds (0.00s to load, 0.00s running 3 checks on 3 files)\n\
            5 mods/funs, found 2 warnings, 1 code readability issue.\n\
            \n\
            Use `mix credo explain` to explain issues, `mix credo --help` for options.\n";
        assert_eq!(rendered, expected);
    }

    /// Pinned verbatim: 7 warnings without `--all` show 5 plus the hint.
    #[test]
    fn truncation_cap_and_hint_are_byte_exact() {
        let file = "/tmp/p6/fx2/lib/many.ex";
        let issues = (4..=10)
            .map(|line| {
                issue(
                    "Credo.Check.Warning.IoInspect",
                    Category::Warning,
                    12,
                    "There should be no calls to `IO.inspect/1`.",
                    file,
                    line,
                    5,
                    "Many.f",
                )
            })
            .collect();
        let context = FormatContext {
            check_count: 1,
            ..FormatContext::default()
        };
        let files = vec![RunnerFile {
            filename: file.to_owned(),
            source: "defmodule Many do\n  @moduledoc \"M.\"\n  def f(x) do\n    1\n  end\nend\n"
                .to_owned(),
        }];
        let rendered = render(&report(issues), &files, &context);
        assert!(rendered.contains("┃  ...  (2 more, use `--all` to show them)\n"));
        assert!(rendered.contains("2 mods/funs, found 7 warnings.\n"));
        assert!(rendered.ends_with("Showing priority issues: ↑ ↗ →  (use `mix credo explain` to explain issues, `mix credo --help` for options).\n"));
        assert_eq!(rendered.matches("There should be no calls").count(), 5);
    }

    /// Pinned verbatim: clean single-file run.
    #[test]
    fn clean_run_is_byte_exact() {
        let context = FormatContext {
            check_count: 3,
            strict_hint: true,
            ..FormatContext::default()
        };
        let files = vec![RunnerFile {
            filename: "/tmp/p6/fx/lib/clean.ex".to_owned(),
            source: CLEAN.to_owned(),
        }];
        let rendered = render(&report(Vec::new()), &files, &context);
        let expected = "Checking 1 source file ...\n\
            \n\
            Please report incorrect results: https://github.com/rrrene/credo/issues\n\
            \n\
            Analysis took 0.00 seconds (0.00s to load, 0.00s running 3 checks on 1 file)\n\
            2 mods/funs, found no issues.\n\
            \n\
            Use `mix credo explain` to explain issues, `mix credo --help` for options.\n";
        assert_eq!(rendered, expected);
    }

    /// Pinned verbatim shape: no files at all.
    #[test]
    fn no_files_found_shape_is_byte_exact() {
        let context = FormatContext {
            check_count: 3,
            strict_hint: true,
            ..FormatContext::default()
        };
        let rendered = render(&report(Vec::new()), &[], &context);
        let expected = "No files found!\n\
            \n\
            Please report incorrect results: https://github.com/rrrene/credo/issues\n\
            \n\
            Analysis took 0.00 seconds (0.00s to load, 0.00s running 3 checks on 0 files)\n\
            0 mods/funs, found no issues.\n\
            \n\
            Use `mix credo explain` to explain issues, `mix credo --help` for options.\n";
        assert_eq!(rendered, expected);
    }

    /// `--all` (and `--strict`) disables the 5-per-category cap.
    #[test]
    fn show_all_disables_truncation() {
        let file = "/tmp/p6/fx2/lib/many.ex";
        let issues = (4..=10)
            .map(|line| {
                issue(
                    "Credo.Check.Warning.IoInspect",
                    Category::Warning,
                    12,
                    "There should be no calls to `IO.inspect/1`.",
                    file,
                    line,
                    5,
                    "Many.f",
                )
            })
            .collect();
        let context = FormatContext {
            check_count: 1,
            show_all: true,
            strict_hint: true,
            ..FormatContext::default()
        };
        let files = vec![RunnerFile {
            filename: file.to_owned(),
            source: "defmodule Many do\nend\n".to_owned(),
        }];
        let rendered = render(&report(issues), &files, &context);
        assert_eq!(rendered.matches("There should be no calls").count(), 7);
        assert!(!rendered.contains("more, use `--all`"));
    }

    /// C locale renders box-drawing and arrows as ASCII escapes.
    #[test]
    fn c_locale_escapes_non_latin1() {
        let context = FormatContext {
            check_count: 1,
            strict_hint: true,
            locale_utf8: false,
            ..FormatContext::default()
        };
        let files = vec![RunnerFile {
            filename: "/tmp/p6/fx2/lib/solo.ex".to_owned(),
            source: "defmodule Solo do\n  @moduledoc \"S.\"\n  def f(x) do\n    IO.inspect(x)\n  end\nend\n"
                .to_owned(),
        }];
        let solo = issue(
            "Credo.Check.Warning.IoInspect",
            Category::Warning,
            12,
            "There should be no calls to `IO.inspect/1`.",
            "/tmp/p6/fx2/lib/solo.ex",
            4,
            5,
            "Solo.f",
        );
        let rendered = render(&report(vec![solo]), &files, &context);
        assert!(rendered.contains("\\x{2503} [W] \\x{2197} There should be no calls"));
        assert!(!rendered.contains("┃"));
        assert!(!rendered.contains("↗"));
    }

    /// TTY color composes the exact native escape sequences.
    #[test]
    fn colorized_output_uses_native_sequences() {
        let context = FormatContext {
            check_count: 1,
            strict_hint: true,
            color: true,
            ..FormatContext::default()
        };
        let files = vec![RunnerFile {
            filename: "/tmp/p6/fx2/lib/solo.ex".to_owned(),
            source: "defmodule Solo do\n  @moduledoc \"S.\"\n  def f(x) do\n    IO.inspect(x)\n  end\nend\n"
                .to_owned(),
        }];
        let solo = issue(
            "Credo.Check.Warning.IoInspect",
            Category::Warning,
            12,
            "There should be no calls to `IO.inspect/1`.",
            "/tmp/p6/fx2/lib/solo.ex",
            4,
            5,
            "Solo.f",
        );
        let rendered = render(&report(vec![solo]), &files, &context);
        assert!(
            rendered
                .contains("\x1b[1m\x1b[41m\x1b[31m \x1b[37m\x1b[22m Warnings - please take a look")
        );
        assert!(rendered.contains("\x1b[0m\x1b[31m┃ \x1b[0m\n"));
        assert!(rendered.contains("\x1b[31m\x1b[2m[W]"));
        assert!(rendered.contains("\x1b[39m\x1b[2m/tmp/p6/fx2/lib/solo.ex"));
        assert!(rendered.contains("\x1b[32m2 mods/funs, \x1b[0mfound \x1b[31m1 warning.\x1b[0m\n"));
    }

    /// `wrap_at` mirrors `UI.wrap_at/2`: word-boundary chunks keep the
    /// trailing space, hard cuts split at the width.
    #[test]
    fn wrap_at_matches_native_chunks() {
        let message =
            "Found a TODO tag in a comment: # TODO: ".to_owned() + &"word ".repeat(39) + "word";
        assert_eq!(
            wrap_at(&message, 72)
                .iter()
                .map(String::len)
                .collect::<Vec<_>>(),
            vec![69, 70, 70, 29]
        );
        assert_eq!(
            wrap_at(&"a".repeat(80), 72)
                .iter()
                .map(String::len)
                .collect::<Vec<_>>(),
            vec![72, 8]
        );
        assert_eq!(wrap_at("hello world", 72), vec!["hello world".to_owned()]);
    }

    /// Scope counting matches the native `5 mods/funs` fixture total and
    /// the `defmacrop`/`defguard`/`defprotocol` edge total of 6.
    #[test]
    fn mods_funs_counts_match_native() {
        assert_eq!(count_mods_funs(SMELLS), 2);
        assert_eq!(count_mods_funs(CLEAN), 2);
        assert_eq!(count_mods_funs(TEST_FILE), 1);
        let edge = "defmodule Edge do\n  @moduledoc \"E.\"\n  def a, do: 1\n  defp b, do: 2\n  defmacro c, do: 3\n  defmacrop d, do: 4\n  defguard e(x) when is_integer(x)\n  defprotocol P do\n    def f(t)\n  end\n  defimpl P, for: Atom do\n    def f(_), do: 1\n  end\nend\n";
        assert_eq!(count_mods_funs(edge), 6);
    }

    /// Invalid files leave the header, timing and scope counts.
    #[test]
    fn skipped_invalid_files_leave_counts() {
        let context = FormatContext {
            check_count: 2,
            strict_hint: true,
            ..FormatContext::default()
        };
        let files = vec![
            RunnerFile {
                filename: "/tmp/p6/fx4/lib/ok.ex".to_owned(),
                source: "defmodule Ok1 do\n  @moduledoc \"O.\"\n  def f(x), do: x\nend\n"
                    .to_owned(),
            },
            RunnerFile {
                filename: "/tmp/p6/fx4/lib/broken.ex".to_owned(),
                source: "def broken( do\n".to_owned(),
            },
        ];
        let mut gone = report(Vec::new());
        gone.skipped_invalid = vec!["/tmp/p6/fx4/lib/broken.ex".to_owned()];
        let rendered = render(&gone, &files, &context);
        assert!(rendered.starts_with("Checking 1 source file ...\n"));
        assert!(rendered.contains("running 2 checks on 1 file"));
        assert!(rendered.contains("2 mods/funs, found no issues."));
    }

    /// More than 60 files warns the run might take a while.
    #[test]
    fn many_files_warn_they_take_a_while() {
        let files: Vec<RunnerFile> = (0..61)
            .map(|n| RunnerFile {
                filename: format!("lib/{n}.ex"),
                source: "defmodule M do\nend\n".to_owned(),
            })
            .collect();
        let rendered = render(&report(Vec::new()), &files, &FormatContext::default());
        assert!(rendered.starts_with("Checking 61 source files (this might take a while) ...\n"));
    }

    /// Design and refactor sections carry their tags, arrows and summary
    /// wording (`[D]`/`[F]`, `→`/`↘`, fixed summary order).
    #[test]
    fn design_and_refactor_sections_render() {
        let file = "/tmp/p6/fx3/lib/a.ex";
        let issues = vec![
            issue(
                "Credo.Check.Design.TagTODO",
                Category::Design,
                8,
                "Found a TODO tag in a comment: # TODO: fix",
                file,
                3,
                3,
                "Des.f",
            ),
            issue(
                "Credo.Check.Refactor.DoubleBooleanNegation",
                Category::Refactor,
                -5,
                "Double boolean negation found.",
                file,
                4,
                6,
                "Des.f",
            ),
        ];
        let context = FormatContext {
            check_count: 3,
            strict_hint: true,
            ..FormatContext::default()
        };
        let files = vec![RunnerFile {
            filename: file.to_owned(),
            source: "defmodule Des do\n  def f(x), do: x\nend\n".to_owned(),
        }];
        let rendered = render(&report(issues), &files, &context);
        let design_at = rendered.find("Software Design").expect("design section");
        let refactor_at = rendered
            .find("Refactoring opportunities")
            .expect("refactor section");
        assert!(design_at < refactor_at);
        assert!(rendered.contains("┃ [D] → Found a TODO tag in a comment: # TODO: fix\n"));
        assert!(rendered.contains("┃ [F] ↘ Double boolean negation found.\n"));
        assert!(rendered.contains(
            "2 mods/funs, found 1 refactoring opportunity, 1 software design suggestion.\n"
        ));
    }
}
