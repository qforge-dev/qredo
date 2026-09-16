//! Native-shaped `explain` output: check docs and per-issue explanations.
//!
//! Mirrors the pinned upstream (`work/ref/credo` @ `ea1ccb9`; verbatim
//! captures in `work/p6-siblings-probe.md` §4 plus follow-up probes for the
//! gaps the probe left open). Scope is `explain` only: `diff` and `gen.*`
//! belong elsewhere.
//!
//! Wiring contract for `main.rs` (the owner wires dispatch; this module only
//! renders and runs the location analysis):
//!
//! ```ignore
//! mod cmd_explain; // top of main.rs
//! mod cmd_browse;  // for `mod_name` reuse below
//!
//! // Target routing mirrors `ExplainCommand.call/2`: the FIRST positional
//! // decides. `contains_line_no?` is a segment count over `:` (`2` or `3`
//! // parts; `3` or `4` when the target contains `:/` for Windows paths).
//! // A location splits into path + line (+ accepted-but-ignored column);
//! // anything else is a check name; no positional is usage.
//! // `Check.defined?("Elixir.#{name}")` is `qredo::check_docs::doc_for`.
//! let target = match first_positional {
//!     None => None,
//!     Some(raw) if is_location_target(raw) => {
//!         let (path, line, _column) = split_location(raw)?; // non-integers crash, see R-EXPLAIN-8
//!         Some(cmd_explain::ExplainTarget::Location { path, line })
//!     }
//!     Some(name) => Some(cmd_explain::ExplainTarget::Check(name)),
//! };
//!
//! // `files` must be the loaded analysis set (discovery + reads, like
//! // `suggest`); `working_dir` is the resolved root (`--working-dir` or the
//! // process CWD, absolute). For native-exact issue paths the files should
//! // carry absolute `filename`s, or root-relative names that resolve under
//! // `working_dir` (see R-EXPLAIN-7).
//! let ctx = cmd_explain::ExplainContext {
//!     target,
//!     working_dir,
//!     config_name: args.config_name.clone(),
//!     config_source: config_source.clone(),
//!     files,
//!     min_priority,
//!     selection,
//! };
//! // `--format json` selects the JSON renderer; every other `--format`
//! // value (including unknown ones) renders the default text, like native.
//! let (stdout, stderr, code) = if format == "json" {
//!     cmd_explain::run_explain_json(&ctx)
//! } else {
//!     cmd_explain::run_explain(&ctx)
//! };
//! print!("{stdout}"); eprint!("{stderr}"); code
//! ```
//!
//! Provenance (upstream files consulted after probing):
//! `cli/command/explain/explain_command.ex` (routing, containment, lookup,
//! exit rules), `cli/command/explain/explain_output.ex` (usage text),
//! `cli/command/explain/output/default.ex` (all text rendering),
//! `cli/command/explain/output/json.ex` (JSON shapes), `cli/output.ex`
//! (tags, arrows, priority names), `cli/output/ui.ex` (`edge`, truncation),
//! `cli/filename.ex` (`contains_line_no?`), `priority.ex` (`to_atom`).
//!
//! Established by probing (see the module tests):
//! - Wrapping rule: explanation prose is NOT re-wrapped. Each raw line is
//!   emitted verbatim under a 7-space edge prefix; blank lines keep the
//!   trailing spaces. (Determined by diffing probe bytes against the raw
//!   `check_docs` strings.)
//! - Unknown check names (including `Elixir.`-prefixed ones) print the
//!   usage text, exit 0 — same as no target.
//! - `--format json` switches both modes to compact
//!   `{"explanations":[...]}` shapes; `flycheck`/`sarif`/`oneline`/unknown
//!   values render the default text silently. Help still wins over `--format
//!   json` for unknown/no targets.
//! - Location column is accepted but ignored for matching; the rendered
//!   `file:line:col` always shows the issue's own column.
//! - `--only`/`--ignore`/`--min-priority`/`--strict` shape the analysis (and
//!   hence the exit status and the lookup set); the context carries only
//!   `min_priority` (see R-EXPLAIN-6).
//!
//! Known residuals vs native (byte-match limits of this module):
//!
//! - R-EXPLAIN-1: piped plain text only: no TTY colors, fixed 80 columns.
//! - R-EXPLAIN-2: excerpt truncation counts Unicode scalars, native counts
//!   graphemes; identical for ASCII (same residual as `list`).
//! - R-EXPLAIN-3: multi-issue locations print in runner order
//!   `(check, filename, line)`; native prints `Execution.get_issues/1`
//!   order. Both agreed on every probed fixture.
//! - R-EXPLAIN-4: the outside-dir echo reconstructs `{path}:{line}`; a
//!   `:col` suffix on the original target is dropped (native echoes it).
//! - R-EXPLAIN-5: relative location targets resolve against `working_dir`;
//!   native expands against the process CWD (identical when they agree).
//!   Symlinked path prefixes are canonicalized when the files exist,
//!   otherwise compared lexically.
//! - R-EXPLAIN-6: only `min_priority` shapes the analysis. Native also
//!   applies `--only`/`--ignore`/tags/`--files-included` etc. to the
//!   explain analysis; extend `ExplainContext` with a `Selection` to close.
//! - R-EXPLAIN-7: issue paths render from the stored `RunnerFile` names
//!   (absolutized under `working_dir` when relative). Pass absolute names
//!   for native-exact output.
//! - R-EXPLAIN-8: non-integer `line`/`column` segments crash native
//!   (exit 1, `ArgumentError` + stack). That parsing lives in the caller
//!   (the target enum carries a parsed `usize`); the caller must reproduce
//!   the crash shape (see the wiring sketch).
//! - R-EXPLAIN-9: JSON string escaping follows `serde_json`; native uses
//!   Jason. Both agree on ASCII; exotic controls/escapes may differ.
//! - R-EXPLAIN-10: pipeline errors and unserved configs fail closed
//!   (`unsupported config: …`, exit 2 / `error: …`, exit 2), mirroring the
//!   `suggest`/`list` caller convention rather than native analysis.

use std::path::{Path, PathBuf};

/// Explain target routed from the first positional, mirroring
/// `ExplainCommand.call/2`. A `path:line[:col]` location carries the file
/// path and line (the column is accepted by the CLI but dropped: native
/// matches on file+line only). Anything without a line suffix is a check
/// name; `None` (no positional) prints usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplainTarget {
    Check(String),
    Location { path: String, line: usize },
}

/// Rendering + analysis inputs for one `explain` run.
#[derive(Debug, Clone)]
pub struct ExplainContext {
    /// Routed first positional (`None` prints usage).
    pub target: Option<ExplainTarget>,
    /// Resolved root for containment and relative names (absolute).
    pub working_dir: PathBuf,
    /// Active config name (`default` unless `-C`/`--config-name`).
    pub config_name: String,
    /// Raw text of the active config file.
    pub config_source: String,
    /// Loaded analysis set (filenames + sources).
    pub files: Vec<qredo::RunnerFile>,
    /// Minimum priority to report (`0` default, `-99` for `--strict`).
    pub min_priority: i32,
    /// CLI check selection shaping the analysis (`--only`/`--ignore`/tags).
    pub selection: qredo::Selection,
}

/// Piped terminal width; headers pad to it.
const TERM_WIDTH: usize = 80;
/// Explanation/source edge indent.
const INDENT: usize = 8;
/// Minimum params-table indent before rounding.
const PARAMS_MIN_INDENT: usize = 10;

/// Render the complete `explain` stdout with its stderr and exit status.
///
/// Check mode never runs analysis (exit 0). Location mode runs the full
/// analysis over `ctx.files`: unknown locations stay silent with the
/// analysis exit status; found issues render with that same status.
#[must_use]
pub fn run_explain(ctx: &ExplainContext) -> (String, String, i32) {
    run_with_format(ctx, false)
}

/// JSON variant of [`run_explain`], selected by `--format json` only.
/// Unknown/no targets still print the usage text, like native.
#[must_use]
pub fn run_explain_json(ctx: &ExplainContext) -> (String, String, i32) {
    run_with_format(ctx, true)
}

/// Shared dispatch for both formats.
fn run_with_format(ctx: &ExplainContext, json: bool) -> (String, String, i32) {
    match &ctx.target {
        None => (usage_text(), String::new(), 0),
        Some(ExplainTarget::Check(name)) => match qredo::check_docs::doc_for(name) {
            Some(doc) => {
                if json {
                    (render_check_json(doc), String::new(), 0)
                } else {
                    (render_check(doc), String::new(), 0)
                }
            }
            // Unknown check names (including `Elixir.`-prefixed ones)
            // fall through to usage, exit 0.
            None => (usage_text(), String::new(), 0),
        },
        Some(ExplainTarget::Location { path, line }) => run_location(ctx, path, *line, json),
    }
}

/// Location branch: containment (128), source lookup (128), full analysis,
/// then silent-or-rendered results with the analysis exit status.
fn run_location(
    ctx: &ExplainContext,
    path: &str,
    line: usize,
    json: bool,
) -> (String, String, i32) {
    // Echo reconstructs `{path}:{line}`; a `:col` suffix is dropped
    // (R-EXPLAIN-4).
    let raw = format!("{path}:{line}");
    if !inside_dir(&ctx.working_dir, path) {
        return (
            String::new(),
            format!(
                "** (explain) Given location is not part of the working dir.\n\n  Location:     {raw}\n  Working dir:  {}\n\n",
                ctx.working_dir.to_string_lossy()
            ),
            128,
        );
    }
    let Some(matched) = match_file(&ctx.files, &ctx.working_dir, path) else {
        return (
            String::new(),
            format!("** (explain) Could not find source file: {path}\n"),
            128,
        );
    };
    let report = match analyze(ctx) {
        Ok(report) => report,
        Err((stderr, code)) => return (String::new(), stderr, code),
    };
    render_matched(&report, matched, &ctx.working_dir, path, line, json)
}

/// Render matched location issues (or the empty-shape) with the analysis
/// exit status: report, file, dir, target and mode travel together.
#[allow(clippy::too_many_arguments)]
fn render_matched(
    report: &qredo::RunReport,
    matched: &qredo::RunnerFile,
    working_dir: &Path,
    path: &str,
    line: usize,
    json: bool,
) -> (String, String, i32) {
    let exit = report.exit_status;
    let target = absolute_under(working_dir, path);
    let display = display_name(working_dir, matched);
    let shown: Vec<&qredo::Issue> = report
        .issues
        .iter()
        .filter(|issue| {
            absolute_under(working_dir, &issue.filename) == target && issue.line_no == Some(line)
        })
        .collect();
    if shown.is_empty() {
        if json {
            return ("{\"explanations\":[]}\n".to_owned(), String::new(), exit);
        }
        return (String::new(), String::new(), exit);
    }
    if json {
        (
            render_location_json(&shown, &matched.source, &display),
            String::new(),
            exit,
        )
    } else {
        (
            render_location(&shown, &matched.source, &display),
            String::new(),
            exit,
        )
    }
}

/// Full analysis over the provided files at the requested minimum
/// priority, mirroring the `ExplainIssue` pipeline stages
/// (`PrepareChecksToRun`, `RunChecks`, `SetRelevantIssues`). Callers pass
/// discovery names matching the config form (relative or absolute), as
/// the CLI does; issue paths absolutize back under the working dir for
/// lookup and display.
fn analyze(ctx: &ExplainContext) -> Result<qredo::RunReport, (String, i32)> {
    let files = ctx.files.clone();
    match qredo::integration::execute_selected(
        &ctx.config_source,
        &ctx.config_name,
        &files,
        ctx.min_priority,
        ctx.selection.clone(),
    ) {
        Err(fallback) => Err((format!("unsupported config: {}\n", fallback.reason), 2)),
        Ok(report) => {
            if report.errors.is_empty() {
                Ok(report)
            } else {
                let mut stderr = String::new();
                for error in &report.errors {
                    use std::fmt::Write as _;
                    let _ = writeln!(stderr, "error: {error:?}");
                }
                Err((stderr, 2))
            }
        }
    }
}

/// Absolute view of a file name: absolute names verbatim, relative names
/// resolved under the working dir.
fn absolute_under(dir: &Path, name: &str) -> PathBuf {
    let path = Path::new(name);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    }
}

/// Lexically absolute path without touching the filesystem.
fn lexical_absolute(base: &Path, raw: &str) -> PathBuf {
    let joined = absolute_under(base, raw);
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

/// Containment of a location path in the working dir: canonicalized when
/// both sides resolve, lexical otherwise (R-EXPLAIN-5).
fn inside_dir(dir: &Path, raw: &str) -> bool {
    let base = lexical_absolute(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        &dir.to_string_lossy(),
    );
    let absolute = lexical_absolute(&base, raw);
    match (
        std::fs::canonicalize(&base),
        std::fs::canonicalize(&absolute),
    ) {
        (Ok(canonical_base), Ok(canonical_target)) => canonical_target.starts_with(canonical_base),
        _ => absolute.starts_with(&base),
    }
}

/// Find the analysis file for a location path: absolute-lexical equality
/// first, then canonicalized equality for symlinked prefixes.
fn match_file<'f>(
    files: &'f [qredo::RunnerFile],
    dir: &Path,
    raw: &str,
) -> Option<&'f qredo::RunnerFile> {
    let target = lexical_absolute(dir, raw);
    if let Some(found) = files
        .iter()
        .find(|file| lexical_absolute(dir, &file.filename) == target)
    {
        return Some(found);
    }
    let canonical_target = std::fs::canonicalize(&target).ok()?;
    files.iter().find(|file| {
        std::fs::canonicalize(lexical_absolute(dir, &file.filename))
            .is_ok_and(|candidate| candidate == canonical_target)
    })
}

/// Display filename for a matched file: the stored name when absolute,
/// resolved under the working dir otherwise (R-EXPLAIN-7).
fn display_name(dir: &Path, matched: &qredo::RunnerFile) -> String {
    absolute_under(dir, &matched.filename)
        .to_string_lossy()
        .into_owned()
}

/// Tag letter for one issue's category (`refactor` maps to `F`).
fn category_tag(category: qredo::Category) -> &'static str {
    match category {
        qredo::Category::Consistency => "C",
        qredo::Category::Design => "D",
        qredo::Category::Readability => "R",
        qredo::Category::Refactor => "F",
        qredo::Category::Warning => "W",
    }
}

/// Priority name from `Priority.to_atom/1` boundaries: above 19 higher,
/// 10-19 high, 0-9 normal, -10 to -1 low, below -10 ignore.
fn priority_name(priority: i32) -> &'static str {
    if priority > 19 {
        "higher"
    } else if priority >= 10 {
        "high"
    } else if priority >= 0 {
        "normal"
    } else if priority >= -10 {
        "low"
    } else {
        "ignore"
    }
}

/// Priority arrow from the same boundaries.
fn priority_arrow(priority: i32) -> char {
    if priority > 19 {
        '\u{2191}'
    } else if priority >= 10 {
        '\u{2197}'
    } else if priority >= 0 {
        '\u{2192}'
    } else if priority >= -10 {
        '\u{2198}'
    } else {
        '\u{2193}'
    }
}

/// Check-mode priority display: the base name, with the `0` default
/// reading as `normal` (exactly `priority_name(to_atom(0))`).
fn check_priority(base: &str) -> (&'static str, char) {
    match base {
        "higher" => ("higher", '\u{2191}'),
        "high" => ("high", '\u{2197}'),
        "low" => ("low", '\u{2198}'),
        _ => ("normal", '\u{2192}'),
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

/// Usage text, byte-verbatim from the pinned upstream.
fn usage_text() -> String {
    "\nUsage: mix credo explain <check_name_or_path_line_no_column> [options]\n\
     \n\
     Explain the given check or issue.\n\
     \n\
     Examples:\n\
     \x20 $ mix credo explain lib/foo/bar.ex:13:6\n\
     \x20 $ mix credo explain lib/foo/bar.ex:13:6 --format json\n\
     \x20 $ mix credo explain Credo.Check.Refactor.Nesting\n\
     \n\
     Explain options:\n\
     \x20     --format            Display the list in a specific format (json,flycheck,sarif,oneline)\n\
     \n\
     General options:\n\
     \x20     --[no-]color        Toggle colored output\n\
     \x20 -v, --version           Show version\n\
     \x20 -h, --help              Show this help\n\
     \n\
     Find advanced usage instructions and more examples here:\n\
     \x20 https://hexdocs.pm/credo/explain_command.html\n\
     \n\
     Give feedback and open an issue here:\n\
     \x20 https://github.com/rrrene/credo/issues\n"
        .to_owned()
}

/// 80-column section title: one leading space plus the padded name.
fn push_header(out: &mut String, title: &str) {
    let mut head = format!(" {title}");
    while head.chars().count() < TERM_WIDTH - 1 {
        head.push(' ');
    }
    out.push(' ');
    out.push_str(&head);
    out.push('\n');
}

/// Bare `┃ ` edge line.
fn push_edge(out: &mut String) {
    out.push_str("┃ \n");
}

/// Indented `┃       ` edge line (faint, 8-wide).
fn push_edge_indented(out: &mut String) {
    out.push('┃');
    for _ in 0..INDENT - 1 {
        out.push(' ');
    }
    out.push('\n');
}

/// `[X] Category:` / arrow `Priority:` lines shared by both modes.
fn push_category_priority(out: &mut String, category: qredo::Category, name: &str, arrow: char) {
    use std::fmt::Write as _;
    let _ = writeln!(
        out,
        "┃   [{}] Category: {} ",
        category_tag(category),
        category.as_str()
    );
    let _ = writeln!(out, "┃    {arrow}  Priority: {name} ");
}

/// `__ WHY IT MATTERS` block: raw explanation lines verbatim (no
/// re-wrapping), each under the indented edge.
fn push_explanation(out: &mut String, explanation: &str) {
    out.push_str("┃    __ WHY IT MATTERS\n");
    push_edge(out);
    for line in explanation.trim().lines() {
        out.push('┃');
        for _ in 0..INDENT - 1 {
            out.push(' ');
        }
        out.push_str(line);
        out.push('\n');
    }
    push_edge(out);
}

/// Params-table indent: longest name (minimum 10) rounded up to the next
/// multiple of two.
fn params_indent(params: &[&qredo::check_docs::ParamDoc]) -> usize {
    let longest = params
        .iter()
        .map(|param| param.name.len())
        .fold(PARAMS_MIN_INDENT, usize::max);
    (longest / 2 + 1) * 2
}

/// Pad a table cell to `width` scalars with spaces.
fn pad_cell(text: &str, width: usize) -> String {
    let mut cell = text.to_owned();
    while cell.chars().count() < width {
        cell.push(' ');
    }
    cell
}

/// `__ CONFIGURATION OPTIONS` block. Only params with doc strings render
/// (undocumented ones are absent from native `explanations()[:params]`);
/// `(defaults to …)` lines are skipped for falsy defaults (`nil`,
/// `false`), mirroring Elixir `if default`.
fn push_params(out: &mut String, module: &str, params: &[qredo::check_docs::ParamDoc]) {
    use std::fmt::Write as _;
    out.push_str("┃    __ CONFIGURATION OPTIONS\n");
    push_edge(out);
    let shown: Vec<&qredo::check_docs::ParamDoc> = params
        .iter()
        .filter(|param| !param.doc.is_empty())
        .collect();
    if shown.is_empty() {
        out.push_str("┃       You can disable this check by using this tuple\n");
        push_edge(out);
        let _ = writeln!(out, "┃         {{{module}, false}}");
        push_edge(out);
        out.push_str("┃       There are no other configuration options.\n");
        push_edge(out);
        return;
    }
    out.push_str("┃       To configure this check, use this tuple\n");
    push_edge(out);
    let _ = writeln!(out, "┃         {{{module}, <params>}}");
    push_edge(out);
    out.push_str("┃       with <params> being false or any combination of these keywords:\n");
    push_edge(out);
    let plain: Vec<&qredo::check_docs::ParamDoc> = shown;
    let width = params_indent(&plain) + 3;
    for param in &plain {
        let head = pad_cell(&format!("  {}:", param.name), width);
        let mut lines = param.doc.split('\n');
        let _ = writeln!(out, "┃       {head}{}", lines.next().unwrap_or(""));
        for tail in lines {
            let _ = writeln!(out, "┃       {}{tail}", " ".repeat(width));
        }
        if param.default != "nil" && param.default != "false" {
            let _ = writeln!(
                out,
                "┃       {}(defaults to {})",
                " ".repeat(width),
                param.default
            );
        }
    }
}

/// Check-mode text: header, category/priority, explanation, params.
fn render_check(doc: &qredo::check_docs::CheckDoc) -> String {
    let mut out = String::from("\n");
    push_header(&mut out, &format!("Check: {}", doc.module));
    push_edge(&mut out);
    let category = category_for_doc(doc);
    let (name, arrow) = check_priority(doc.base_priority);
    push_category_priority(&mut out, category, name, arrow);
    push_edge(&mut out);
    push_edge_indented(&mut out);
    push_explanation(&mut out, doc.explanation);
    push_params(&mut out, doc.module, doc.params);
    push_edge(&mut out);
    out
}

/// Location-mode text: scope header once, then one block per issue.
fn render_location(issues: &[&qredo::Issue], source: &str, filename: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("\n");
    let scope = issues
        .first()
        .and_then(|issue| issue.scope.clone())
        .unwrap_or_default();
    push_header(&mut out, &crate::cmd_browse::mod_name(&scope));
    push_edge(&mut out);
    for issue in issues {
        push_category_priority(
            &mut out,
            issue.category,
            priority_name(issue.priority),
            priority_arrow(issue.priority),
        );
        push_edge(&mut out);
        let _ = writeln!(out, "┃       {}", issue.message);
        let _ = writeln!(
            out,
            "┃       {}{} ({})",
            filename,
            pos_suffix(issue.line_no, issue.column),
            issue.scope.as_deref().unwrap_or("")
        );
        if let Some(line_no) = issue.line_no {
            push_code_block(&mut out, source, line_no, issue.column, issue);
        }
        push_edge_indented(&mut out);
        let doc = qredo::check_docs::doc_for(&issue.check);
        push_explanation(
            &mut out,
            doc.map_or("TODO: Insert explanation", |found| found.explanation),
        );
        push_params(
            &mut out,
            &issue.check,
            doc.map_or(&[], |found| found.params),
        );
        push_edge(&mut out);
    }
    out
}

/// `__ CODE IN QUESTION`: source ±2 lines with line numbers plus the caret
/// underline when the issue carries a column.
fn push_code_block(
    out: &mut String,
    source: &str,
    line_no: usize,
    column: Option<usize>,
    issue: &qredo::Issue,
) {
    use std::fmt::Write as _;
    push_edge(out);
    out.push_str("┃    __ CODE IN QUESTION\n");
    push_edge(out);
    let lines: Vec<&str> = source.split('\n').collect();
    for number in line_no.saturating_sub(2)..=line_no + 2 {
        if number < 1 {
            continue;
        }
        if let Some(text) = lines.get(number - 1) {
            let gutter = format!("{:>width$}", format!("{number} "), width = INDENT - 2);
            let _ = writeln!(out, "┃ {gutter}{}", truncate(text, TERM_WIDTH - INDENT));
        }
        if number == line_no
            && let Some(column) = column
        {
            let width = match &issue.trigger {
                qredo::IssueTrigger::NoTrigger => 1,
                qredo::IssueTrigger::Text(trigger) => trigger.chars().count().max(1),
            };
            out.push('┃');
            for _ in 0..INDENT - 1 {
                out.push(' ');
            }
            for _ in 0..column.saturating_sub(1) {
                out.push(' ');
            }
            for _ in 0..width {
                out.push('^');
            }
            out.push('\n');
        }
    }
}

/// Category for a check doc (verified: the path segment, lowercased).
fn category_for_doc(doc: &qredo::check_docs::CheckDoc) -> qredo::Category {
    match doc.category {
        "consistency" => qredo::Category::Consistency,
        "design" => qredo::Category::Design,
        "readability" => qredo::Category::Readability,
        "refactor" => qredo::Category::Refactor,
        _ => qredo::Category::Warning,
    }
}

/// Minimal JSON string escaping matching Jason for the characters that
/// appear in check prose (quotes, backslashes, controls stay escaped;
/// everything else passes through raw UTF-8).
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Check-mode JSON: the raw base priority (atom name as string, `0` as a
/// number), the `Elixir.`-prefixed check, category and raw explanation.
fn render_check_json(doc: &qredo::check_docs::CheckDoc) -> String {
    let priority = if doc.base_priority == "0" {
        "0".to_owned()
    } else {
        json_string(doc.base_priority)
    };
    format!(
        "{{\"explanations\":[{{\"priority\":{priority},\"check\":{},\"category\":{},\"explanation_for_issue\":{}}}]}}\n",
        json_string(&format!("Elixir.{}", doc.module)),
        json_string(doc.category),
        json_string(doc.explanation),
    )
}

/// Location-mode JSON over the matched issues with `related_code` as
/// `[line, text]` pairs for the ±2 source window.
fn render_location_json(issues: &[&qredo::Issue], source: &str, filename: &str) -> String {
    use std::fmt::Write as _;
    let lines: Vec<&str> = source.split('\n').collect();
    let mut out = String::from("{\"explanations\":[");
    for (index, issue) in issues.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let related: Vec<String> = issue.line_no.map_or(Vec::new(), |line_no| {
            (line_no.saturating_sub(2)..=line_no + 2)
                .filter(|number| *number >= 1)
                .filter_map(|number| {
                    lines
                        .get(number - 1)
                        .map(|text| format!("[{number},{}]", json_string(text)))
                })
                .collect()
        });
        let optional =
            |value: Option<usize>| value.map_or("null".to_owned(), |number| number.to_string());
        let _ = write!(
            out,
            "{{\"message\":{},\"priority\":{},\"scope\":{},\"check\":{},\"filename\":{},\"category\":{},\"column\":{},\"line_no\":{},\"trigger\":{},\"explanation_for_issue\":{},\"related_code\":[{}]}}",
            json_string(&issue.message),
            issue.priority,
            issue
                .scope
                .as_deref()
                .map_or("null".to_owned(), json_string),
            json_string(&format!("Elixir.{}", issue.check)),
            json_string(filename),
            json_string(issue.category.as_str()),
            optional(issue.column),
            optional(issue.line_no),
            match &issue.trigger {
                qredo::IssueTrigger::NoTrigger => "null".to_owned(),
                qredo::IssueTrigger::Text(trigger) => json_string(trigger),
            },
            json_string(
                qredo::check_docs::doc_for(&issue.check)
                    .map_or("TODO: Insert explanation", |doc| doc.explanation)
            ),
            related.join(","),
        );
    }
    out.push_str("]}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMELLS: &str =
        "defmodule Smells do\n  def f(x) do\n    IO.inspect(x)\n    dbg(x)\n  end\nend\n";
    const CONFIG: &str = "%{configs: [%{name: \"default\", files: %{included: [\"lib\"]}, checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}, {Credo.Check.Warning.Dbg, []}, {Credo.Check.Readability.ModuleDoc, []}]}}]}";

    fn fixture_root() -> PathBuf {
        PathBuf::from("/tmp/qredo-explain-fixture")
    }

    fn smells_file() -> qredo::RunnerFile {
        qredo::RunnerFile {
            filename: "lib/smells.ex".to_owned(),
            source: SMELLS.to_owned(),
        }
    }

    fn context(target: Option<ExplainTarget>) -> ExplainContext {
        ExplainContext {
            target,
            working_dir: fixture_root(),
            config_name: "default".to_owned(),
            config_source: CONFIG.to_owned(),
            files: vec![smells_file()],
            min_priority: 0,
            selection: qredo::Selection::default(),
        }
    }

    /// Pinned verbatim: the no-args usage text (711 bytes, exit 0).
    #[test]
    fn no_target_prints_verbatim_usage() {
        let ctx = context(None);
        let (stdout, stderr, code) = run_explain(&ctx);
        assert_eq!(code, 0);
        assert_eq!(stderr, "");
        assert_eq!(stdout.len(), 711);
        assert_eq!(
            stdout,
            "\n\
             Usage: mix credo explain <check_name_or_path_line_no_column> [options]\n\
             \n\
             Explain the given check or issue.\n\
             \n\
             Examples:\n\
             \x20 $ mix credo explain lib/foo/bar.ex:13:6\n\
             \x20 $ mix credo explain lib/foo/bar.ex:13:6 --format json\n\
             \x20 $ mix credo explain Credo.Check.Refactor.Nesting\n\
             \n\
             Explain options:\n\
             \x20     --format            Display the list in a specific format (json,flycheck,sarif,oneline)\n\
             \n\
             General options:\n\
             \x20     --[no-]color        Toggle colored output\n\
             \x20 -v, --version           Show version\n\
             \x20 -h, --help              Show this help\n\
             \n\
             Find advanced usage instructions and more examples here:\n\
             \x20 https://hexdocs.pm/credo/explain_command.html\n\
             \n\
             Give feedback and open an issue here:\n\
             \x20 https://github.com/rrrene/credo/issues\n"
        );
    }

    /// Unknown check names (probed: plain and `Elixir.`-prefixed) print
    /// usage, exit 0 — never analysis.
    #[test]
    fn unknown_check_prints_usage() {
        for name in [
            "Nope.Nope",
            "Elixir.Credo.Check.Warning.IoInspect",
            "lib/smells.ex",
        ] {
            let ctx = context(Some(ExplainTarget::Check(name.to_owned())));
            let (stdout, stderr, code) = run_explain(&ctx);
            assert_eq!(code, 0, "{name}");
            assert_eq!(stderr, "", "{name}");
            assert!(stdout.starts_with("\nUsage: mix credo explain"), "{name}");
            assert_eq!(stdout.len(), 711, "{name}");
        }
    }

    /// Pinned verbatim: check-mode `ModuleDoc` full output, exit 0.
    #[test]
    fn check_mode_moduledoc_is_byte_exact() {
        let header = format!(
            "  Check: Credo.Check.Readability.ModuleDoc{}",
            " ".repeat(38)
        );
        let (stdout, stderr, code) = moduledoc_explained();
        assert_eq!(code, 0);
        assert_eq!(stderr, "");
        let mut expected = String::from("\n");
        expected.push_str(&header);
        expected.push('\n');
        expected.push_str(&MODULEDOC_LINES.join("\n"));
        expected.push('\n');
        assert_eq!(stdout, expected);
    }

    /// Full `ModuleDoc` check-mode output exercises the lines below.
    fn moduledoc_explained() -> (String, String, i32) {
        let ctx = context(Some(ExplainTarget::Check(
            "Credo.Check.Readability.ModuleDoc".to_owned(),
        )));
        let (stdout, stderr, code) = run_explain(&ctx);
        assert_eq!(stderr, "");
        (stdout, stderr, code)
    }

    /// Pinned `ModuleDoc` check-mode bytes (everything but the padded header).
    const MODULEDOC_LINES: &[&str] = &[
        // header spliced in code below,
        "┃ ",
        "┃   [R] Category: readability ",
        "┃    →  Priority: normal ",
        "┃ ",
        "┃       ",
        "┃    __ WHY IT MATTERS",
        "┃ ",
        "┃       Every module should contain comprehensive documentation.",
        "┃       ",
        "┃           # preferred",
        "┃       ",
        "┃           defmodule MyApp.Web.Search do",
        "┃             @moduledoc \"\"\"",
        "┃             This module provides a public API for all search queries originating",
        "┃             in the web layer.",
        "┃             \"\"\"",
        "┃           end",
        "┃       ",
        "┃           # also okay: explicitly say there is no documentation",
        "┃       ",
        "┃           defmodule MyApp.Web.Search do",
        "┃             @moduledoc false",
        "┃           end",
        "┃       ",
        "┃       Many times a sentence or two in plain english, explaining why the module",
        "┃       exists, will suffice. Documenting your train of thought this way will help",
        "┃       both your co-workers and your future-self.",
        "┃       ",
        "┃       Other times you will want to elaborate even further and show some",
        "┃       examples of how the module's functions can and should be used.",
        "┃       ",
        "┃       In some cases however, you might not want to document things about a module,",
        "┃       e.g. it is part of a private API inside your project. Since Elixir prefers",
        "┃       explicitness over implicit behaviour, you should \"tag\" these modules with",
        "┃       ",
        "┃           @moduledoc false",
        "┃       ",
        "┃       to make it clear that there is no intention in documenting it.",
        "┃       ",
        "┃       Like all `Readability` issues, this one is not a technical concern.",
        "┃       But you can improve the odds of others reading and liking your code by making",
        "┃       it easier to follow.",
        "┃ ",
        "┃    __ CONFIGURATION OPTIONS",
        "┃ ",
        "┃       To configure this check, use this tuple",
        "┃ ",
        "┃         {Credo.Check.Readability.ModuleDoc, <params>}",
        "┃ ",
        "┃       with <params> being false or any combination of these keywords:",
        "┃ ",
        "┃         ignore_names:          List of modules to ignore based on their name. Accepts atoms, strings and regexes.",
        "┃                                (defaults to [~r/(\\.\\w+Controller|\\.Endpoint|\\.\\w+Live(\\.\\w+)?|\\.Repo|\\.Router|\\.\\w+Socket|\\.\\w+View|\\.\\w+HTML|\\.\\w+JSON|\\.Telemetry|\\.Layouts|\\.Mailer)$/])",
        "┃         ignore_modules_using:  List of modules to ignore based on their `use` declarations. Accepts atoms, strings and regexes.",
        "┃                                (defaults to [Credo.Check, Ecto.Schema, Phoenix.LiveView, ~r/\\.Web$/])",
        "┃ ",
    ];

    /// Param-less checks show the disable tuple and no other options (with
    /// the doubled trailing edge native emits there).
    #[test]
    fn check_mode_paramless_shows_disable_tuple() {
        let ctx = context(Some(ExplainTarget::Check(
            "Credo.Check.Warning.IoInspect".to_owned(),
        )));
        let (stdout, stderr, code) = run_explain(&ctx);
        assert_eq!(code, 0);
        assert_eq!(stderr, "");
        assert!(stdout.contains("┃   [W] Category: warning \n"));
        assert!(stdout.contains("┃    ↗  Priority: high \n"));
        assert!(stdout.contains(
            "┃       You can disable this check by using this tuple\n\
             ┃ \n\
             ┃         {Credo.Check.Warning.IoInspect, false}\n\
             ┃ \n\
             ┃       There are no other configuration options.\n\
             ┃ \n\
             ┃ \n"
        ));
        assert!(stdout.ends_with("┃ \n┃ \n"));
    }

    /// Undocumented params stay hidden and falsy defaults (`nil`, `false`)
    /// print no `(defaults to …)` line.
    #[test]
    fn check_mode_hides_undocumented_and_falsy_defaults() {
        // `parens` (default `false`, empty doc) collapses to the disable
        // variant, like native `explanations()[:params]` being empty.
        let ctx = context(Some(ExplainTarget::Check(
            "Credo.Check.Readability.ParenthesesOnZeroArityDefs".to_owned(),
        )));
        let (stdout, _, code) = run_explain(&ctx);
        assert_eq!(code, 0);
        assert!(stdout.contains("{Credo.Check.Readability.ParenthesesOnZeroArityDefs, false}"));
        assert!(!stdout.contains("\n\u{2503}         parens:"));
        // `force` (default `nil`) renders without a defaults line.
        let ctx = context(Some(ExplainTarget::Check(
            "Credo.Check.Consistency.LineEndings".to_owned(),
        )));
        let (stdout, _, _) = run_explain(&ctx);
        assert!(stdout.contains("┃         force:       Force a choice"));
        assert!(!stdout.contains("defaults to"));
        // `ignore_specs` (default `false`, documented) renders without one;
        // its truthy siblings keep theirs.
        let ctx = context(Some(ExplainTarget::Check(
            "Credo.Check.Readability.MaxLineLength".to_owned(),
        )));
        let (stdout, _, _) = run_explain(&ctx);
        assert!(!stdout.contains("ignore_heredocs"));
        let specs_at = stdout.find("ignore_specs:").expect("specs row");
        let specs_row: String = stdout[specs_at..].chars().take(200).collect();
        assert!(!specs_row.contains("defaults to"), "{specs_row}");
        assert!(stdout.contains("(defaults to 120)"));
    }

    /// Pinned verbatim: location-mode `smells.ex:3` full output, exit 20.
    #[test]
    fn location_mode_ioinspect_is_byte_exact() {
        let ctx = context(Some(ExplainTarget::Location {
            path: "/tmp/qredo-explain-fixture/lib/smells.ex".to_owned(),
            line: 3,
        }));
        let (stdout, stderr, code) = run_explain(&ctx);
        assert_eq!(code, 20);
        assert_eq!(stderr, "");
        let file = "/tmp/qredo-explain-fixture/lib/smells.ex";
        let header = format!("  Smells{}", " ".repeat(72));
        let expected = format!(
            "\n\
             {header}\n\
             ┃ \n\
             ┃   [W] Category: warning \n\
             ┃    ↗  Priority: high \n\
             ┃ \n\
             ┃       There should be no calls to `IO.inspect/1`.\n\
             ┃       {file}:3:5 (Smells.f)\n\
             ┃ \n\
             ┃    __ CODE IN QUESTION\n\
             ┃ \n\
             ┃     1 defmodule Smells do\n\
             ┃     2   def f(x) do\n\
             ┃     3     IO.inspect(x)\n\
             ┃           ^^^^^^^^^^\n\
             ┃     4     dbg(x)\n\
             ┃     5   end\n\
             ┃       \n\
             ┃    __ WHY IT MATTERS\n\
             ┃ \n\
             ┃       While calls to IO.inspect might appear in some parts of production code,\n\
             ┃       most calls to this function are added during debugging sessions.\n\
             ┃       \n\
             ┃       This check warns about those calls, because they might have been committed\n\
             ┃       in error.\n\
             ┃ \n\
             ┃    __ CONFIGURATION OPTIONS\n\
             ┃ \n\
             ┃       You can disable this check by using this tuple\n\
             ┃ \n\
             ┃         {{Credo.Check.Warning.IoInspect, false}}\n\
             ┃ \n\
             ┃       There are no other configuration options.\n\
             ┃ \n\
             ┃ \n"
        );
        assert_eq!(stdout, expected);
    }

    /// Dead locations stay silent but keep the analysis exit status.
    #[test]
    fn dead_location_is_silent_with_analysis_status() {
        let ctx = context(Some(ExplainTarget::Location {
            path: "/tmp/qredo-explain-fixture/lib/smells.ex".to_owned(),
            line: 99,
        }));
        let (stdout, stderr, code) = run_explain(&ctx);
        assert_eq!((stdout.as_str(), stderr.as_str(), code), ("", "", 20));
    }

    /// Locations outside the working dir fail 128 with the exact shape.
    #[test]
    fn outside_dir_fails_128() {
        let mut ctx = context(Some(ExplainTarget::Location {
            path: "/tmp/elsewhere/lib/smells.ex".to_owned(),
            line: 3,
        }));
        ctx.working_dir = PathBuf::from("/tmp/qredo-explain-fixture");
        let (stdout, stderr, code) = run_explain(&ctx);
        assert_eq!(stdout, "");
        assert_eq!(code, 128);
        assert_eq!(
            stderr,
            "** (explain) Given location is not part of the working dir.\n\
             \n\
             \x20 Location:     /tmp/elsewhere/lib/smells.ex:3\n\
             \x20 Working dir:  /tmp/qredo-explain-fixture\n\
             \n"
        );
    }

    /// Missing files fail 128 with the exact shape.
    #[test]
    fn missing_file_fails_128() {
        let ctx = context(Some(ExplainTarget::Location {
            path: "/tmp/qredo-explain-fixture/does-not-exist.ex".to_owned(),
            line: 3,
        }));
        let (stdout, stderr, code) = run_explain(&ctx);
        assert_eq!(stdout, "");
        assert_eq!(code, 128);
        assert_eq!(
            stderr,
            "** (explain) Could not find source file: /tmp/qredo-explain-fixture/does-not-exist.ex\n"
        );
    }

    /// Check-mode JSON shape: string atoms, numeric zero, raw explanation.
    #[test]
    fn check_json_matches_native_shapes() {
        let ctx = context(Some(ExplainTarget::Check(
            "Credo.Check.Warning.IoInspect".to_owned(),
        )));
        let (stdout, stderr, code) = run_explain_json(&ctx);
        assert_eq!((stderr.as_str(), code), ("", 0));
        assert_eq!(
            stdout,
            "{\"explanations\":[{\"priority\":\"high\",\"check\":\"Elixir.Credo.Check.Warning.IoInspect\",\"category\":\"warning\",\"explanation_for_issue\":\"While calls to IO.inspect might appear in some parts of production code,\\nmost calls to this function are added during debugging sessions.\\n\\nThis check warns about those calls, because they might have been committed\\nin error.\\n\"}]}\n"
        );
        let ctx = context(Some(ExplainTarget::Check(
            "Credo.Check.Readability.ModuleDoc".to_owned(),
        )));
        let (stdout, _, code) = run_explain_json(&ctx);
        assert_eq!(code, 0);
        assert!(stdout.starts_with(
            "{\"explanations\":[{\"priority\":0,\"check\":\"Elixir.Credo.Check.Readability.ModuleDoc\""
        ));
        assert!(stdout.ends_with("it easier to follow.\\n\"}]}\n"));
    }

    /// Location-mode JSON shape over the single matched issue.
    #[test]
    fn location_json_matches_native_shape() {
        let ctx = context(Some(ExplainTarget::Location {
            path: "/tmp/qredo-explain-fixture/lib/smells.ex".to_owned(),
            line: 3,
        }));
        let (stdout, stderr, code) = run_explain_json(&ctx);
        assert_eq!((stderr.as_str(), code), ("", 20));
        let file = "/tmp/qredo-explain-fixture/lib/smells.ex";
        let explanation = qredo::check_docs::doc_for("Credo.Check.Warning.IoInspect")
            .expect("doc")
            .explanation;
        let raw = format!(
            "{{\"message\":\"There should be no calls to `IO.inspect/1`.\",\"priority\":{},\"scope\":\"Smells.f\",\"check\":\"Elixir.Credo.Check.Warning.IoInspect\",\"filename\":\"{file}\",\"category\":\"warning\",\"column\":5,\"line_no\":3,\"trigger\":\"IO.inspect\",\"explanation_for_issue\":{},\"related_code\":[[1,\"defmodule Smells do\"],[2,\"  def f(x) do\"],[3,\"    IO.inspect(x)\"],[4,\"    dbg(x)\"],[5,\"  end\"]]}}",
            issue_priority(),
            json_string(explanation),
        );
        assert_eq!(stdout, format!("{{\"explanations\":[{raw}]}}\n"));
    }

    /// Dead locations stay silent in JSON mode too.
    #[test]
    fn dead_location_json_is_empty_list() {
        let ctx = context(Some(ExplainTarget::Location {
            path: "/tmp/qredo-explain-fixture/lib/smells.ex".to_owned(),
            line: 99,
        }));
        let (stdout, stderr, code) = run_explain_json(&ctx);
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("{\"explanations\":[]}\n", "", 20)
        );
    }

    /// Native issue priority for the location fixture (drives the JSON
    /// expectation above); guards against silent scope-bonus drift.
    fn issue_priority() -> i32 {
        let ctx = context(Some(ExplainTarget::Location {
            path: "/tmp/qredo-explain-fixture/lib/smells.ex".to_owned(),
            line: 3,
        }));
        let report = analyze(&ctx).expect("served");
        report
            .issues
            .iter()
            .find(|issue| issue.line_no == Some(3))
            .expect("issue")
            .priority
    }

    /// Priority names and arrows follow `Priority.to_atom/1` boundaries.
    #[test]
    fn priority_names_follow_native_boundaries() {
        for priority in [20, 25] {
            assert_eq!(priority_name(priority), "higher");
        }
        for priority in [10, 12, 19] {
            assert_eq!(priority_name(priority), "high");
        }
        for priority in [0, 1, 9] {
            assert_eq!(priority_name(priority), "normal");
        }
        for priority in [-10, -1] {
            assert_eq!(priority_name(priority), "low");
        }
        for priority in [-11, -100] {
            assert_eq!(priority_name(priority), "ignore");
        }
        assert_eq!(check_priority("0"), ("normal", '→'));
        assert_eq!(check_priority("high"), ("high", '↗'));
    }

    /// Params-table indent rounds the longest name up to an even width.
    #[test]
    fn params_indent_rounds_up_even() {
        let doc = qredo::check_docs::doc_for("Credo.Check.Readability.ModuleDoc").expect("doc");
        let shown: Vec<&qredo::check_docs::ParamDoc> = doc
            .params
            .iter()
            .filter(|param| !param.doc.is_empty())
            .collect();
        assert_eq!(params_indent(&shown), 22);
    }
}
