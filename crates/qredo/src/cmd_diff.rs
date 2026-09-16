//! Native-shaped `mix credo diff` (default format, piped plain text).
//!
//! Compares the working dir against a previous side (default `HEAD`) using
//! the native pipeline on both sides, matches issues with native
//! `same_issue?`, and renders the `Diffing ...` / marked-issue / `Changes
//! between ...` output. Pinned upstream: `work/ref/credo` @ `ea1ccb9`
//! (`lib/credo/cli/command/diff/`); verbatim captures in
//! `work/p6-siblings-probe.md` §6.
//!
//! Wiring contract for `main.rs` (the owner wires dispatch; this module only
//! runs and renders):
//!
//! ```ignore
//! // lib.rs
//! pub mod cmd_diff;
//!
//! // main.rs: parse `diff` flags into `qredo::cmd_diff::DiffOptions`
//! // (see `diff_help_text` sketch in the report) and call:
//! let (stdout, stderr, code) = qredo::cmd_diff::run_diff(&opts);
//! print!("{stdout}"); eprint!("{stderr}"); code
//! ```
//!
//! Previous-side flag semantics (probed on scratch repos under `/tmp`,
//! process CWD forced into the scratch repo via `File.cd!` so ref
//! resolution happens there; see the report):
//!
//! - default / `--from-git-ref <ref>` / positional `<ref>`: clone the repo
//!   root to `$TMPDIR/credo-diff-*` (left behind, like native), checkout
//!   the ref, run the previous analysis there. Header shows the given ref,
//!   summary shows the same ref.
//! - `--from-git-merge-base <base>`: resolve `git merge-base <base> HEAD`
//!   to a sha, clone/checkout that sha. Header shows the base name,
//!   summary shows the resolved sha.
//! - `--since <datetime>`: resolve `git rev-list --reverse --after
//!   <datetime> HEAD` to its first line, or `HEAD` when empty.
//!   Clone/checkout that. Header shows the datetime string, summary shows
//!   the resolved sha (or `HEAD`).
//! - `--from-dir <path>`: no clone; run the previous analysis directly on
//!   the given dir. Header and summary show the path as given.
//! - Precedence mirrors native: `--since` > `--from-dir` >
//!   `--from-git-ref` > `--from-git-merge-base` > positional ref >
//!   default `HEAD`.
//!
//! Quirks and deviations:
//!
//! - Q1 reproduced exactly: `fixed` is the exact-struct difference
//!   `previous -- old` on clone-absolute paths, while `kept`/`new` use
//!   `same_issue?` (relative-filename OR line equality AND
//!   column/category/message/trigger/scope equality). Line-matched issues
//!   therefore count as BOTH fixed and kept, like native.
//! - Q2 reproduced exactly: an explicit `--config-file` is kept for the
//!   previous run, so absolute `included` paths pointing at the working
//!   repo make the previous side analyze the working dir (self-compare).
//! - Q3 deliberately NOT reproduced: the working dir is canonicalized
//!   (symlinks resolved, e.g. macOS `/tmp` -> `/private/tmp`) before the
//!   root comparison, so the clone subdir is found. Native string-compares
//!   `git rev-parse --show-toplevel` (physical) against the logical
//!   `--working-dir` and falls through to a nonexistent dir.
//! - DEV-1: all git invocations run with the working dir as CWD
//!   (`git -C <working_dir> ...`). Native resolves `--from-git-ref` /
//!   `--from-git-merge-base` / `--since` refs in the *process* CWD, which
//!   only coincides when run from inside the target repo.
//! - DEV-2: outside a repo only the deterministic `** (MatchError)` lines
//!   are emitted (native appends PID-independent but version-specific
//!   stack frames; the frames carry no information).
//! - R-DIFF-1: piped plain text only (no TTY colors, no C-locale escapes,
//!   fixed 80 columns), default format only.
//! - R-DIFF-2: no source excerpts (native prints them only when verbose,
//!   and only for new issues).
//! - R-DIFF-3: `--files-included` / `--files-excluded` are accepted for
//!   compatibility but ignored for discovery, like qredo `suggest`.
//! - R-DIFF-4: positional file patterns other than the ref are not
//!   supported; discovery uses the config `included` set. Use an explicit
//!   config with narrow `included` to scope a diff.
//! - R-DIFF-5: `--format` is not rendered here (default format only).
//! - R-DIFF-6: `--show-fixed` / `--show-kept` are honored (fixed/old
//!   detail blocks render like native) but default to off, matching the
//!   probed default output.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::{Category, Issue, RunReport, RunnerFile};

/// Options for one `diff` run, mirroring the suggest surface plus the
/// previous-side selectors.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    /// Repo working dir (canonicalized before the root comparison).
    pub working_dir: PathBuf,
    /// `--from-git-ref <ref>` (also covers the bare positional ref form
    /// when `positional_ref` is unset; explicit flag wins).
    pub from_ref: Option<String>,
    /// `--from-dir <path>`: previous analysis runs directly on this dir.
    pub from_dir: Option<PathBuf>,
    /// `--from-git-merge-base <base>`: previous side is the merge base of
    /// this ref and `HEAD`.
    pub from_merge_base: Option<String>,
    /// `--since <datetime>`: previous side is the first commit after this
    /// point in time (or `HEAD` when there is none).
    pub since: Option<String>,
    /// Bare positional ref (`mix credo diff <ref>`). Ignored when any
    /// `--from-*` / `--since` flag is set.
    pub positional_ref: Option<String>,
    /// Explicit config file, kept for the previous run (Q2).
    pub config_file: Option<PathBuf>,
    /// Config name (default `"default"`).
    pub config_name: String,
    /// `--strict` (implies lowest-priority visibility).
    pub strict: bool,
    /// `--all-priorities` (same visibility effect as `--strict`).
    pub all_priorities: bool,
    /// `--all` (disables the 5-per-category display cap).
    pub all: bool,
    /// `--min-priority <level>`: explicit value beats the strict flags.
    pub min_priority: Option<String>,
    /// `--only` / `--checks` patterns (case-insensitive regex).
    pub only: Vec<String>,
    /// `--ignore` / `--ignore-checks` patterns.
    pub ignore: Vec<String>,
    /// `--checks-with-tag` patterns.
    pub checks_with_tag: Vec<String>,
    /// `--checks-without-tag` (accepted; native ignores it).
    pub checks_without_tag: Vec<String>,
    /// `--enable-disabled-checks` patterns.
    pub enable_disabled: Vec<String>,
    /// Accepted but ignored for discovery (R-DIFF-3).
    pub files_included: Vec<String>,
    /// Accepted but ignored for discovery (R-DIFF-3).
    pub files_excluded: Vec<String>,
    /// `--show-fixed`: render fixed-issue detail blocks.
    pub show_fixed: bool,
    /// `--show-kept`: render kept-issue detail blocks.
    pub show_kept: bool,
    /// `--mute-exit-status`: exit zero despite new issues.
    pub mute_exit_status: bool,
}

/// Run one `diff`, returning `(stdout, stderr, exit_code)`.
#[must_use]
pub fn run_diff(opts: &DiffOptions) -> (String, String, i32) {
    // Logical paths throughout: native displays `--working-dir` as given
    // (`/tmp/...`, not `/private/tmp/...`). Symlinks resolve only inside
    // the previous-side root comparison (`clone_checkout`), never for
    // display or the current analysis (Q3 deliberately not reproduced).
    let working = match std::env::current_dir() {
        Ok(cwd) => absolutize(&cwd, &opts.working_dir),
        Err(_) => opts.working_dir.clone(),
    };
    let git_root = match git_toplevel(&working) {
        Ok(root) => root,
        Err(output) => return (String::new(), outside_repo_stderr(&output), 1),
    };
    if let Err(message) = compile_selection(&opts.only, &opts.ignore) {
        return (String::new(), message + "\n", 1);
    }
    let min_priority = match resolve_min_priority(opts) {
        Ok(priority) => priority,
        Err(message) => return (String::new(), message + "\n", 1),
    };
    let previous = match resolve_previous(opts, &working, &git_root) {
        Ok(previous) => previous,
        Err((stderr, code)) => return (String::new(), stderr, code),
    };
    match execute_both(opts, &working, &previous, min_priority) {
        Ok((stdout, exit)) => (stdout, String::new(), exit),
        Err((stderr, code)) => (String::new(), stderr, code),
    }
}

/// Display flags derived from priority/selection options.
#[allow(
    clippy::struct_excessive_bools,
    reason = "one bool per CLI switch, mirroring native flags"
)]
#[derive(Debug, Clone, Copy)]
struct DisplayFlags {
    show_all: bool,
    strict_hint: bool,
    show_fixed: bool,
    show_kept: bool,
}

/// Run both sides, partition, render and score the exit status.
fn execute_both(
    opts: &DiffOptions,
    working: &Path,
    previous: &Previous,
    min_priority: i32,
) -> Result<(String, i32), (String, i32)> {
    let selection = crate::Selection {
        only: opts.only.clone(),
        ignore: opts.ignore.clone(),
        checks_with_tag: opts.checks_with_tag.clone(),
        enable_disabled: opts.enable_disabled.clone(),
    };
    let (current, current_check_count) = run_side(working, opts, &selection, min_priority)?;
    let (prev_side, _) = run_side(&previous.dirname, opts, &selection, min_priority)?;
    let part = partition(
        &current.report.issues,
        &prev_side.report.issues,
        &previous.dirname,
    );
    let flags = DisplayFlags {
        show_all: opts.all || min_priority <= -99,
        strict_hint: min_priority < 0,
        show_fixed: opts.show_fixed,
        show_kept: opts.show_kept,
    };
    let stdout = render(previous, &current, &part, current_check_count, flags);
    Ok((stdout, exit_for(&part, opts.mute_exit_status)))
}

/// Exit status from new issues only (unless muted).
fn exit_for(part: &Partition, mute_exit_status: bool) -> i32 {
    if mute_exit_status {
        0
    } else {
        part.new
            .iter()
            .fold(0, |status, issue| status | issue.exit_status)
    }
}

/// Previous-side location plus the two ref displays.
struct Previous {
    /// Canonical dir the previous analysis ran in (clone subdir or dir).
    dirname: PathBuf,
    /// Header display (`Diffing ... with <this> ...`): the given ref/path.
    header_ref: String,
    /// Summary display (`Changes between <this> and working dir:`):
    /// resolved sha for merge-base/since, otherwise the given ref/path.
    summary_ref: String,
    /// The as-given `--from-dir` path for `(dir:...)` locations, if any.
    dir_display: Option<String>,
}

/// Resolve the previous side (clone when git-backed, direct for `--from-dir`).
fn resolve_previous(
    opts: &DiffOptions,
    working: &Path,
    git_root: &Path,
) -> Result<Previous, (String, i32)> {
    if opts.since.is_some() {
        return previous_since(opts, working, git_root);
    }
    if opts.from_dir.is_some() {
        return previous_dir(opts);
    }
    if opts.from_ref.is_some() {
        return previous_ref(opts, working, git_root);
    }
    if opts.from_merge_base.is_some() {
        return previous_merge_base(opts, working, git_root);
    }
    if opts.positional_ref.is_some() {
        return previous_positional(opts, working, git_root);
    }
    previous_head(working, git_root)
}

/// Previous side for `--since <datetime>`.
fn previous_since(
    opts: &DiffOptions,
    working: &Path,
    git_root: &Path,
) -> Result<Previous, (String, i32)> {
    let datetime = opts.since.clone().unwrap_or_default();
    let summary = git_first_after(working, &datetime).unwrap_or_else(|| "HEAD".to_owned());
    let dirname = clone_checkout(working, git_root, &summary).map_err(|message| (message, 1))?;
    Ok(Previous {
        dirname,
        header_ref: datetime,
        summary_ref: summary,
        dir_display: None,
    })
}

/// Previous side for `--from-dir <path>` (no clone).
fn previous_dir(opts: &DiffOptions) -> Result<Previous, (String, i32)> {
    let dir = opts.from_dir.clone().unwrap_or_default();
    if !dir.exists() {
        return Err((
            format!("** (diff) could not find given path: {}\n", dir.display()),
            128,
        ));
    }
    let display = dir.to_string_lossy().into_owned();
    Ok(Previous {
        dirname: canonicalize(&dir),
        header_ref: display.clone(),
        summary_ref: display.clone(),
        dir_display: Some(display),
    })
}

/// Previous side for `--from-git-ref <ref>`.
fn previous_ref(
    opts: &DiffOptions,
    working: &Path,
    git_root: &Path,
) -> Result<Previous, (String, i32)> {
    let given = opts.from_ref.clone().unwrap_or_default();
    ensure_git_ref(working, &given)?;
    let dirname = clone_checkout(working, git_root, &given).map_err(|message| (message, 1))?;
    Ok(Previous {
        dirname,
        header_ref: given.clone(),
        summary_ref: given,
        dir_display: None,
    })
}

/// Previous side for `--from-git-merge-base <base>`.
fn previous_merge_base(
    opts: &DiffOptions,
    working: &Path,
    git_root: &Path,
) -> Result<Previous, (String, i32)> {
    let base = opts.from_merge_base.clone().unwrap_or_default();
    ensure_git_ref(working, &base)?;
    let sha = git_merge_base(working, &base).ok_or_else(|| (merge_base_stderr(&base, ""), 128))?;
    let dirname = clone_checkout(working, git_root, &sha).map_err(|message| (message, 1))?;
    Ok(Previous {
        dirname,
        header_ref: base,
        summary_ref: sha,
        dir_display: None,
    })
}

/// Previous side for a bare positional ref or path.
fn previous_positional(
    opts: &DiffOptions,
    working: &Path,
    git_root: &Path,
) -> Result<Previous, (String, i32)> {
    let first = opts.positional_ref.clone().unwrap_or_default();
    if git_ref_exists(working, &first) {
        let dirname = clone_checkout(working, git_root, &first).map_err(|message| (message, 1))?;
        return Ok(Previous {
            dirname,
            header_ref: first.clone(),
            summary_ref: first,
            dir_display: None,
        });
    }
    let path = PathBuf::from(&first);
    if path.exists() {
        return Ok(Previous {
            dirname: canonicalize(&path),
            header_ref: first.clone(),
            summary_ref: first.clone(),
            dir_display: Some(first),
        });
    }
    Err((
        format!("** (diff) given ref is not a Git ref or local path: {first}\n"),
        128,
    ))
}

/// Previous side for the default `HEAD`.
fn previous_head(working: &Path, git_root: &Path) -> Result<Previous, (String, i32)> {
    ensure_git_ref(working, "HEAD").map_err(|_| {
        (
            "** (diff) given ref is not a Git ref or local path: \n".to_owned(),
            128,
        )
    })?;
    let dirname = clone_checkout(working, git_root, "HEAD").map_err(|message| (message, 1))?;
    Ok(Previous {
        dirname,
        header_ref: "HEAD".to_owned(),
        summary_ref: "HEAD".to_owned(),
        dir_display: None,
    })
}

/// `** (diff)` halt shape for a bad git ref.
fn ensure_git_ref(working: &Path, given: &str) -> Result<(), (String, i32)> {
    if git_ref_exists(working, given) {
        Ok(())
    } else {
        Err((
            format!("** (diff) given value is not a Git ref: {given}\n"),
            128,
        ))
    }
}

/// `** (diff)` halt shape for an unresolvable merge base.
fn merge_base_stderr(base: &str, output: &str) -> String {
    format!("** (diff) Could not determine merge base for `{base}`: \"{output}\"\n")
}

/// Clone the git root to a fresh `credo-diff-*` temp dir (left behind, like
/// native), checkout the ref, and return the previous working dir.
fn clone_checkout(working: &Path, git_root: &Path, git_ref: &str) -> Result<PathBuf, String> {
    let tmp = fresh_clone_dir();
    let status = std::process::Command::new("git")
        .args(["clone", &git_root.to_string_lossy(), &tmp.to_string_lossy()])
        .current_dir(working)
        .output()
        .map_err(|error| format!("** (diff) could not run `git clone`: {error}\n"))?;
    if !status.status.success() {
        return Err("** (diff) could not clone the repository\n".to_owned());
    }
    let checkout = std::process::Command::new("git")
        .args(["checkout", git_ref])
        .current_dir(&tmp)
        .output()
        .map_err(|error| format!("** (diff) could not run `git checkout`: {error}\n"))?;
    if !checkout.status.success() {
        return Err("** (diff) could not checkout the given ref\n".to_owned());
    }
    let canon_root = canonicalize(git_root);
    let canon_work = canonicalize(working);
    if canon_work == canon_root {
        return Ok(canonicalize(&tmp));
    }
    match canon_work.strip_prefix(&canon_root) {
        Ok(relative) => Ok(canonicalize(&tmp.join(relative))),
        Err(_) => Ok(canonicalize(&tmp)),
    }
}

/// Fresh `credo-diff-<nanos>-<pid>` dir under the temp dir (never removed).
fn fresh_clone_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let pid = std::process::id();
    std::env::temp_dir().join(format!("credo-diff-{nanos}-{pid}"))
}

/// `git rev-parse --show-toplevel` under `dir`; `Err` carries combined output.
fn git_toplevel(dir: &Path) -> Result<PathBuf, String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .map_err(|_| "could not run `git`\n".to_owned())?;
    if !output.status.success() {
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        return Err(combined);
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(PathBuf::from(root))
}

/// True when `git show <ref>` succeeds under `dir`.
fn git_ref_exists(dir: &Path, git_ref: &str) -> bool {
    std::process::Command::new("git")
        .args(["show", git_ref])
        .current_dir(dir)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// First commit after `datetime` (`git rev-list --reverse --after ... HEAD`).
fn git_first_after(dir: &Path, datetime: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-list", "--reverse", "--after", datetime, "HEAD"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
}

/// Merge-base sha of `base` and `HEAD`, or `None` when it cannot be read.
fn git_merge_base(dir: &Path, base: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["merge-base", base, "HEAD"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if sha.is_empty() { None } else { Some(sha) }
}

/// Deterministic outside-repo stderr (stack frames omitted, DEV-2).
fn outside_repo_stderr(git_output: &str) -> String {
    let escaped = elixir_escape(git_output);
    format!("** (MatchError) no match of right hand side value:\n\n    {{\"{escaped}\", 128}}\n")
}

/// Elixir `inspect`-style escaping for the embedded git output: the probed
/// message only needs backslash/quote/newline handling.
fn elixir_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// Canonicalize, falling back to the input when the path does not exist.
fn canonicalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// One analyzed side: its report plus phase timings.
struct Side {
    report: RunReport,
    files: Vec<RunnerFile>,
    load_microseconds: u64,
    run_microseconds: u64,
}

/// Run the full native pipeline rooted at `root`.
fn run_side(
    root: &Path,
    opts: &DiffOptions,
    selection: &crate::Selection,
    min_priority: i32,
) -> Result<(Side, usize), (String, i32)> {
    let load_start = std::time::Instant::now();
    let (config_source, config) = load_config(root, opts)?;
    let found = discover(root, &config);
    let files = read_found(&found);
    let load_microseconds = micros(load_start.elapsed());
    let check_count = check_count(&config, selection, min_priority);
    let run_start = std::time::Instant::now();
    let report = crate::integration::execute_selected(
        &config_source,
        &opts.config_name,
        &files,
        min_priority,
        selection.clone(),
    )
    .map_err(|fallback| (format!("unsupported config: {}\n", fallback.reason), 2))?;
    let run_microseconds = micros(run_start.elapsed());
    if !report.errors.is_empty() {
        return Err((format!("pipeline errors: {:?}\n", report.errors), 2));
    }
    // Absolutize against the side root: matching (Q1/Q2) and rendering
    // need clone/working-absolute paths, while the pipeline itself runs
    // on root-relative names for file selection.
    let mut report = report;
    absolutize_report(&mut report, root);
    Ok((
        Side {
            report,
            files,
            load_microseconds,
            run_microseconds,
        },
        check_count,
    ))
}

/// Config discovery: explicit `--config-file` is kept verbatim for both
/// sides (Q2); otherwise walk up from the side root.
fn load_config(
    root: &Path,
    opts: &DiffOptions,
) -> Result<(String, crate::CredoConfig), (String, i32)> {
    let path = match &opts.config_file {
        Some(path) => path.clone(),
        None => discover_config(root).ok_or_else(|| {
            (
                format!("no .credo.exs found walking up from {}\n", root.display()),
                2,
            )
        })?,
    };
    if !path.is_file() {
        return Err((
            format!(
                "** (config) Given config file does not exist:\n  {}\n",
                path.display()
            ),
            129,
        ));
    }
    let source = std::fs::read_to_string(&path).map_err(|error| {
        (
            format!("cannot read config {}: {error}\n", path.display()),
            2,
        )
    })?;
    let name = if opts.config_name.is_empty() {
        "default"
    } else {
        opts.config_name.as_str()
    };
    match crate::parse_config(&source, name) {
        Ok(config) => Ok((source, config)),
        Err(unsupported) => Err((
            format!("warning: failed to parse config file: {}\n", unsupported.0),
            129,
        )),
    }
}

/// Rewrite root-relative issue filenames to side-absolute paths.
/// (`skipped_invalid` stays root-relative like `files`, so the header
/// valid-count comparison still lines up.)
fn absolutize_report(report: &mut RunReport, root: &Path) {
    for issue in &mut report.issues {
        if Path::new(&issue.filename).is_relative() {
            issue.filename = root.join(&issue.filename).to_string_lossy().into_owned();
        }
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

/// Files the side covers: config `included` (defaulting to `lib/`+`test/`),
/// minus `excluded`. Names are root-relative when inside the root (what the
/// pipeline's file selection matches on), absolute otherwise.
fn discover(root: &Path, config: &crate::CredoConfig) -> Vec<(String, PathBuf)> {
    let mut collected = Vec::new();
    if config.files_included.is_empty() {
        collect(&root.join("lib"), root, &mut collected);
        collect(&root.join("test"), root, &mut collected);
    } else {
        collect_included(root, &config.files_included, &mut collected);
    }
    collected.retain(|(relative, path)| {
        let absolute = path.to_string_lossy().into_owned();
        !config
            .files_excluded
            .iter()
            .any(|entry| entry_matches(entry, relative) || entry_matches(entry, &absolute))
    });
    collected.sort();
    collected.dedup_by(|a, b| a.1 == b.1);
    collected
}

/// Read discovered files into runner inputs.
fn read_found(found: &[(String, PathBuf)]) -> Vec<RunnerFile> {
    found
        .iter()
        .map(|(filename, path)| RunnerFile {
            filename: filename.clone(),
            source: std::fs::read_to_string(path).unwrap_or_default(),
        })
        .collect()
}

/// Sorted recursive collection of `.ex`/`.exs` files.
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
            if name.starts_with('.') || ["_build", "deps", "node_modules"].contains(&name.as_str())
            {
                continue;
            }
            collect(&path, root, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext == "ex" || ext == "exs")
        {
            out.push((display_name(root, &path), path));
        }
    }
}

/// Collect config `included` entries: plain directories walked directly
/// (absolute or root-relative), globs expanded, `.ex`/`.exs` files literal.
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
            walked.retain(|(_, path)| {
                let text = path.to_string_lossy().into_owned();
                crate::wildcard_match(pattern, &text).unwrap_or(false)
            });
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
/// root (what pipeline file selection matches on), absolute otherwise
/// (which is also what reproduces Q2: absolute `included` paths keep
/// working-dir-absolute names even when rooted at the clone).
fn display_name(root: &Path, path: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(root) {
        relative.to_string_lossy().into_owned()
    } else {
        path.to_string_lossy().into_owned()
    }
}

/// One config file entry against a filename.
fn entry_matches(entry: &crate::FileEntry, filename: &str) -> bool {
    match entry {
        crate::FileEntry::Glob(pattern) => {
            crate::wildcard_match(pattern, filename).unwrap_or(false)
        }
        crate::FileEntry::Regex(source) => regex::Regex::new(source)
            .map(|expression| expression.is_match(filename))
            .unwrap_or(false),
    }
}

/// Lexically absolute path without touching symlinks.
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

/// True for glob patterns.
fn has_magic(pattern: &str) -> bool {
    pattern.contains(['*', '?', '['])
}

/// True for `.ex`/`.exs` paths.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn is_elixir_path(pattern: &str) -> bool {
    pattern.ends_with(".ex") || pattern.ends_with(".exs")
}

/// Expand one absolute glob against the filesystem.
fn expand_glob(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let walk_root = static_prefix(root, pattern);
    let mut candidates = Vec::new();
    collect_all(&walk_root, &mut candidates);
    candidates
        .into_iter()
        .filter(|path| {
            let text = path.to_string_lossy().into_owned();
            crate::wildcard_match(pattern, &text).unwrap_or(false)
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
            if name.starts_with('.') || ["_build", "deps", "node_modules"].contains(&name.as_str())
            {
                continue;
            }
            collect_all(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Resolve the effective minimum priority.
fn resolve_min_priority(opts: &DiffOptions) -> Result<i32, String> {
    if let Some(raw) = &opts.min_priority {
        return match raw.as_str() {
            "higher" => Ok(20),
            "high" => Ok(10),
            "normal" => Ok(1),
            "low" => Ok(-10),
            "ignore" => Ok(-100),
            _ => raw.parse::<i32>().map_err(|_| invalid_priority(raw)),
        };
    }
    if opts.strict || opts.all_priorities {
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

/// Pre-compile selection regexes: an invalid pattern fails before analysis.
fn compile_selection(only: &[String], ignore: &[String]) -> Result<(), String> {
    for pattern in only.iter().chain(ignore.iter()) {
        if regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .is_err()
        {
            return Err(format!(
                "** (MatchError) no match of right hand side value:\n\n    {pattern:?} is not a valid regular expression"
            ));
        }
    }
    Ok(())
}

/// Checks the timing line reports.
fn check_count(
    config: &crate::CredoConfig,
    selection: &crate::Selection,
    min_priority: i32,
) -> usize {
    crate::integration::enabled_modules(config, selection)
        .iter()
        .filter(|module| selection.should_run(module))
        .filter(|module| !crate::version_skipped_on_pinned_toolchain(module))
        .filter(|module| crate::runs_at_min_priority(module, min_priority))
        .count()
}

/// Saturating wall-time microseconds.
#[allow(clippy::cast_possible_truncation)]
fn micros(elapsed: std::time::Duration) -> u64 {
    elapsed.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Partitioned issues: new/kept from `same_issue?`, fixed by struct difference.
struct Partition {
    new: Vec<Issue>,
    fixed: Vec<Issue>,
    old: Vec<Issue>,
}

/// Match current against previous with Q1 semantics.
fn partition(current: &[Issue], previous: &[Issue], previous_dirname: &Path) -> Partition {
    let prev_prefix = previous_dirname.to_string_lossy().into_owned();
    let old: Vec<Issue> = current
        .iter()
        .filter(|issue| {
            previous
                .iter()
                .any(|prev| same_issue(issue, prev, &prev_prefix))
        })
        .cloned()
        .collect();
    let new: Vec<Issue> = current
        .iter()
        .filter(|issue| {
            !previous
                .iter()
                .any(|prev| same_issue(issue, prev, &prev_prefix))
        })
        .cloned()
        .collect();
    // Q1: struct difference against the kept (current-shaped) issues, so
    // line-matched-but-repathed previous issues are both kept and fixed.
    let fixed: Vec<Issue> = previous
        .iter()
        .filter(|prev| !old.iter().any(|kept| kept == *prev))
        .cloned()
        .collect();
    Partition { new, fixed, old }
}

/// Native `same_issue?`: relative-filename OR line equality, AND
/// column/category/message/trigger/scope equality.
fn same_issue(current: &Issue, previous: &Issue, previous_dirname: &str) -> bool {
    let same_file_or_line = current.filename == relative_to(&previous.filename, previous_dirname)
        || current.line_no == previous.line_no;
    same_file_or_line
        && current.column == previous.column
        && current.category == previous.category
        && current.message == previous.message
        && current.trigger == previous.trigger
        && current.scope == previous.scope
}

/// Elixir `Path.relative_to/2`: strip the prefix plus one separator, else
/// return the path unchanged (this is what makes Q1/Q2 fall out).
fn relative_to(path: &str, prefix: &str) -> String {
    let trimmed = prefix.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return path.trim_start_matches('/').to_owned();
    }
    if path == trimmed {
        return String::new();
    }
    if let Some(rest) = path.strip_prefix(trimmed)
        && let Some(stripped) = rest.strip_prefix('/')
    {
        return stripped.to_owned();
    }
    path.to_owned()
}

/// Diff marker for one partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    New,
    Old,
    Fixed,
}

impl Marker {
    fn text(self) -> &'static str {
        match self {
            Self::New => "+ ",
            Self::Old => "~ ",
            Self::Fixed => "✔ ",
        }
    }

    fn weight(self) -> u8 {
        match self {
            Self::Fixed => 0,
            Self::Old => 1,
            Self::New => 2,
        }
    }
}

/// Piped terminal width; headers pad to it, messages wrap 8 short.
const TERM_WIDTH: usize = 80;
/// Location-line and continuation-line content indent.
const INDENT: usize = 8;
/// Default per-category display cap.
const PER_CATEGORY: usize = 5;
/// Microseconds per native timing centisecond.
const MICROS_PER_CENTI: u64 = 10_000;

/// Render the complete default-format diff stdout.
fn render(
    previous: &Previous,
    current: &Side,
    part: &Partition,
    check_count: usize,
    flags: DisplayFlags,
) -> String {
    let mut out = String::new();
    push_header(
        &mut out,
        &current.files,
        &current.report,
        &previous.header_ref,
    );
    push_displayed(&mut out, previous, part, flags);
    push_summary(
        &mut out,
        current,
        part,
        &SummaryCtx {
            summary_ref: &previous.summary_ref,
            check_count,
            strict_hint: flags.strict_hint,
        },
    );
    out
}

/// Inputs for the summary tail beyond issues and files.
struct SummaryCtx<'a> {
    summary_ref: &'a str,
    check_count: usize,
    strict_hint: bool,
}

/// Collect displayed issues, group by category, and push each section.
fn push_displayed(out: &mut String, previous: &Previous, part: &Partition, flags: DisplayFlags) {
    let mut displayed: Vec<(&Issue, Marker)> = Vec::new();
    for issue in &part.new {
        displayed.push((issue, Marker::New));
    }
    if flags.show_kept {
        for issue in &part.old {
            displayed.push((issue, Marker::Old));
        }
    }
    if flags.show_fixed {
        for issue in &part.fixed {
            displayed.push((issue, Marker::Fixed));
        }
    }
    for category in [
        Category::Design,
        Category::Readability,
        Category::Refactor,
        Category::Warning,
        Category::Consistency,
    ] {
        let mut selected: Vec<(&Issue, Marker)> = displayed
            .iter()
            .filter(|(issue, _)| issue.category == category)
            .map(|(issue, marker)| (*issue, *marker))
            .collect();
        if selected.is_empty() {
            continue;
        }
        selected.sort_by(|left, right| {
            left.1
                .weight()
                .cmp(&right.1.weight())
                .then_with(|| left.0.priority.cmp(&right.0.priority))
                .then_with(|| left.0.severity.total_cmp(&right.0.severity))
                .then_with(|| left.0.filename.cmp(&right.0.filename))
                .then_with(|| left.0.line_no.cmp(&right.0.line_no))
        });
        selected.reverse();
        push_category(out, category, &selected, previous, flags.show_all);
    }
}

/// `Diffing N ...` header (or `No files found!` when empty).
fn push_header(out: &mut String, files: &[RunnerFile], report: &RunReport, header_ref: &str) {
    let valid = files
        .iter()
        .filter(|file| {
            !report
                .skipped_invalid
                .iter()
                .any(|skipped| skipped == &file.filename)
        })
        .count();
    match valid {
        0 => out.push_str("No files found!\n"),
        1 => {
            let _ = writeln!(
                out,
                "Diffing 1 source file in working dir with {header_ref} ..."
            );
        }
        count => {
            if count > 60 {
                let _ = writeln!(
                    out,
                    "Diffing {count} source files in working dir with {header_ref} (this might take a while) ..."
                );
            } else {
                let _ = writeln!(
                    out,
                    "Diffing {count} source files in working dir with {header_ref} ..."
                );
            }
        }
    }
}

/// One category section with its marked issues and overflow hint.
fn push_category(
    out: &mut String,
    category: Category,
    selected: &[(&Issue, Marker)],
    previous: &Previous,
    show_all: bool,
) {
    out.push('\n');
    let mut inner = format!(" {}", category_title(category));
    while inner.chars().count() < TERM_WIDTH - 3 {
        inner.push(' ');
    }
    out.push_str("  ");
    out.push(' ');
    out.push_str(&inner);
    out.push('\n');
    out.push_str("  ┃ \n");
    let shown = if show_all {
        selected.len()
    } else {
        selected.len().min(PER_CATEGORY)
    };
    for (issue, marker) in &selected[..shown] {
        push_issue(out, issue, *marker, previous);
    }
    if selected.len() > shown {
        let _ = writeln!(
            out,
            "  ┃  ...  ({} other new issues, use `--all` to show them)",
            selected.len() - shown
        );
    }
}

/// One marked issue: wrapped message lines plus the location line.
fn push_issue(out: &mut String, issue: &Issue, marker: Marker, previous: &Previous) {
    let tag = category_tag(issue.category);
    let arrow = priority_arrow(issue.priority);
    let chunks = wrap_at(&issue.message, TERM_WIDTH - INDENT);
    if let Some((first, rest)) = chunks.split_first() {
        let _ = writeln!(out, "{}┃ [{tag}] {arrow} {first}", marker.text());
        for chunk in rest {
            if chunk.is_empty() {
                continue;
            }
            let _ = writeln!(out, "{}┃       {chunk}", marker.text());
        }
    }
    let location = match marker {
        Marker::Fixed => fixed_location(issue, previous),
        Marker::New | Marker::Old => issue.filename.clone(),
    };
    let scope = issue.scope.as_deref().unwrap_or("");
    let _ = writeln!(
        out,
        "{}┃       {location}{} #({scope})",
        marker.text(),
        pos_suffix(issue.line_no, issue.column)
    );
}

/// Fixed-issue location: `(git:<ref>) <clone-relative>` or
/// `(dir:<path>) <remainder>` (leading slash kept, like native).
fn fixed_location(issue: &Issue, previous: &Previous) -> String {
    let prefix = previous.dirname.to_string_lossy().into_owned();
    if let Some(dir) = &previous.dir_display {
        let relative = issue.filename.replace(prefix.as_str(), "");
        return format!("(dir:{dir}) {relative}");
    }
    let relative = relative_to(&issue.filename, &prefix)
        .trim_start_matches(['/', '\\'])
        .to_owned();
    format!("(git:{}) {relative}", previous.summary_ref)
}

/// Cry for help, timing line, `Changes between ...` counts and hint.
fn push_summary(out: &mut String, current: &Side, part: &Partition, ctx: &SummaryCtx<'_>) {
    out.push('\n');
    out.push_str("Please report incorrect results: https://github.com/rrrene/credo/issues\n");
    out.push('\n');
    out.push_str(&timing_text(current, ctx.check_count));
    out.push('\n');
    out.push('\n');
    let _ = writeln!(out, "Changes between {} and working dir:", ctx.summary_ref);
    out.push('\n');
    let _ = writeln!(out, "+  added {},", count_parts(&part.new, true));
    let _ = writeln!(out, "✔  fixed {}, and", count_parts(&part.fixed, false));
    let _ = writeln!(out, "~  kept {}.", count_parts(&part.old, false));
    out.push('\n');
    if ctx.strict_hint {
        out.push_str(
            "Use `mix credo explain` to explain issues, `mix credo diff --help` for options.\n",
        );
    } else {
        out.push_str("Showing priority issues: ↑ ↗ →  (use `mix credo explain` to explain issues, `mix credo diff --help` for options).\n");
    }
}

/// `Analysis took ...` with native centisecond formatting and singulars.
fn timing_text(current: &Side, check_count: usize) -> String {
    let load = current.load_microseconds / MICROS_PER_CENTI;
    let run = current.run_microseconds / MICROS_PER_CENTI;
    let total = format_seconds(load + run);
    let total_text = if total == "1.0" {
        "1 second".to_owned()
    } else {
        format!("{total} seconds")
    };
    let checks = if check_count == 1 {
        "1 check".to_owned()
    } else {
        format!("{check_count} checks")
    };
    let files = if current.files.len() == 1 {
        "1 file".to_owned()
    } else {
        format!("{} files", current.files.len())
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

/// `added/fixed/kept` count wording in native `@category_wording` order.
fn count_parts(issues: &[Issue], is_new: bool) -> String {
    let qualifier = if is_new { "new " } else { "" };
    let mut parts = Vec::new();
    for (category, singular, plural) in summary_parts() {
        let count = issues
            .iter()
            .filter(|issue| issue.category == category)
            .count();
        if count == 0 {
            continue;
        }
        if count == 1 {
            parts.push(format!("1 {qualifier}{singular}"));
        } else {
            parts.push(format!("{count} {qualifier}{plural}"));
        }
    }
    if parts.is_empty() {
        "no issues".to_owned()
    } else {
        parts.join(", ")
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

/// Section title per category.
fn category_title(category: Category) -> &'static str {
    match category {
        Category::Design => "Software Design",
        Category::Readability => "Code Readability",
        Category::Refactor => "Refactoring opportunities",
        Category::Warning => "Warnings - please take a look",
        Category::Consistency => "Consistency",
    }
}

/// Tag letter per category (`refactor` maps to `F`).
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
        "↑"
    } else if priority >= 10 {
        "↗"
    } else if priority >= 0 {
        "→"
    } else if priority >= -10 {
        "↘"
    } else {
        "↓"
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

/// Split a message the way `UI.wrap_at/2` does (72-column copy of the
/// suggest renderer; see `format_default` for the probed contract).
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
        let to_break = chars[pos..].iter().take_while(|c| **c != '\n').count();
        let max = width.min(chars.len() - pos).min(to_break).max(1);
        let (len, extra) = search_chunk(&chars, pos, max);
        out.push(chars[pos..pos + len + extra].iter().collect());
        pos += len + extra;
        if chars.get(pos) == Some(&'\n') {
            pos += 1;
        } else if chars.get(pos) == Some(&'\r') && chars.get(pos + 1) == Some(&'\n') {
            pos += 2;
        }
    }
    out
}

/// Descending chunk-length search mirroring the atomic group's backtrack.
fn search_chunk(chars: &[char], pos: usize, max: usize) -> (usize, usize) {
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
            return (n + 1, 0);
        }
        if next_is_break {
            return (n, 0);
        }
        if n == 1 {
            return (max, 0);
        }
        n -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IssueTrigger;

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
            exit_status: category.default_exit_status(),
            trigger: IssueTrigger::Text(trigger.to_owned()),
            scope: Some(scope.to_owned()),
        }
    }

    fn warning(filename: &str, line: usize, trigger: &str, message: &str) -> Issue {
        issue(
            "Credo.Check.Warning.IoInspect",
            Category::Warning,
            12,
            message,
            filename,
            line,
            5,
            trigger,
            "DiffA.f",
        )
    }

    /// Filename equality via the clone-relative form matches (the Q2
    /// self-compare shape: the previous filename is not under the clone
    /// prefix, so `relative_to` returns it unchanged and equal).
    #[test]
    fn same_issue_matches_relative_filename() {
        let current = warning("/work/lib/a.ex", 9, "IO.inspect", "msg");
        let previous = warning("/work/lib/a.ex", 3, "IO.inspect", "msg");
        assert!(same_issue(&current, &previous, "/clone"));
    }

    /// Line equality alone suffices when filenames differ entirely.
    #[test]
    fn same_issue_matches_line_alone() {
        let current = warning("/work/lib/a.ex", 3, "IO.inspect", "msg");
        let previous = warning("/clone/other/b.ex", 3, "IO.inspect", "msg");
        assert!(same_issue(&current, &previous, "/clone"));
    }

    /// Column/category/message/trigger/scope mismatches all break the match.
    #[test]
    fn same_issue_requires_full_key() {
        let base = warning("/work/lib/a.ex", 3, "IO.inspect", "msg");
        let mut col = base.clone();
        col.column = Some(6);
        let prev = warning("/clone/lib/a.ex", 3, "IO.inspect", "msg");
        assert!(!same_issue(&col, &prev, "/clone"));
        let mut msg = base.clone();
        msg.message = "other".to_owned();
        assert!(!same_issue(&msg, &prev, "/clone"));
        let mut trig = base.clone();
        trig.trigger = IssueTrigger::Text("other".to_owned());
        assert!(!same_issue(&trig, &prev, "/clone"));
        let mut scope = base.clone();
        scope.scope = Some("Other.f".to_owned());
        assert!(!same_issue(&scope, &prev, "/clone"));
        let mut cat = base.clone();
        cat.category = Category::Refactor;
        assert!(!same_issue(&cat, &prev, "/clone"));
        // Neither filename nor line matches.
        let mut line = base.clone();
        line.line_no = Some(4);
        let far = warning("/clone/other/b.ex", 3, "IO.inspect", "msg");
        assert!(!same_issue(&line, &far, "/clone"));
    }

    /// Q1: line-matched previous issues are BOTH kept and fixed.
    #[test]
    fn partition_reproduces_q1_double_count() {
        let prev_dir = "/clone";
        let previous = vec![warning("/clone/lib/a.ex", 3, "IO.inspect", "msg")];
        let current = vec![
            warning("/work/lib/a.ex", 3, "IO.inspect", "msg"),
            warning("/work/lib/a.ex", 4, "dbg", "msg2"),
        ];
        let part = partition(&current, &previous, Path::new(prev_dir));
        assert_eq!(part.new.len(), 1);
        assert_eq!(part.old.len(), 1);
        // Struct difference: the repathed previous issue is never `==`
        // to the kept current issue, so it is also fixed.
        assert_eq!(part.fixed.len(), 1);
    }

    /// Q2: identical absolute paths compare equal, so nothing is fixed.
    #[test]
    fn partition_self_compare_fixes_nothing() {
        let prev_dir = "/clone";
        let previous = vec![warning("/work/lib/a.ex", 3, "IO.inspect", "msg")];
        let current = vec![warning("/work/lib/a.ex", 3, "IO.inspect", "msg")];
        let part = partition(&current, &previous, Path::new(prev_dir));
        assert!(part.new.is_empty());
        assert_eq!(part.old.len(), 1);
        assert!(part.fixed.is_empty());
    }

    /// Count wording follows the singular/plural/no-issues rules.
    #[test]
    fn count_parts_wording() {
        assert_eq!(count_parts(&[], true), "no issues");
        assert_eq!(count_parts(&[], false), "no issues");
        let one = vec![warning("/w/a.ex", 3, "IO.inspect", "m")];
        assert_eq!(count_parts(&one, true), "1 new warning");
        assert_eq!(count_parts(&one, false), "1 warning");
        let two = vec![
            warning("/w/a.ex", 3, "IO.inspect", "m"),
            warning("/w/a.ex", 4, "IO.inspect", "m"),
        ];
        assert_eq!(count_parts(&two, true), "2 new warnings");
        let design = issue(
            "Credo.Check.Design.TagTODO",
            Category::Design,
            8,
            "todo",
            "/w/a.ex",
            1,
            1,
            "# TODO",
            "A",
        );
        assert_eq!(
            count_parts(&[design], false),
            "1 software design suggestion"
        );
    }

    /// Markers render before every detail line of their issue.
    #[test]
    fn issue_markers_prefix_detail_lines() {
        let previous = Previous {
            dirname: PathBuf::from("/clone"),
            header_ref: "HEAD".to_owned(),
            summary_ref: "HEAD".to_owned(),
            dir_display: None,
        };
        let new_issue = warning("/work/lib/a.ex", 3, "IO.inspect", "msg");
        let mut out = String::new();
        push_issue(&mut out, &new_issue, Marker::New, &previous);
        assert!(out.starts_with("+ ┃ [W]"));
        assert!(out.contains("\n+ ┃       /work/lib/a.ex:3:5 #(DiffA.f)\n"));
        let mut kept = String::new();
        push_issue(&mut kept, &new_issue, Marker::Old, &previous);
        assert!(kept.starts_with("~ ┃ [W]"));
        let fixed = warning("/clone/lib/a.ex", 3, "IO.inspect", "msg");
        let mut fixed_out = String::new();
        push_issue(&mut fixed_out, &fixed, Marker::Fixed, &previous);
        assert!(fixed_out.contains("✔ ┃       (git:HEAD) lib/a.ex:3:5 #(DiffA.f)\n"));
    }

    /// Outside-repo stderr carries only the deterministic lines.
    #[test]
    fn outside_repo_stderr_is_deterministic() {
        let stderr = outside_repo_stderr(
            "fatal: not a git repository (or any of the parent directories): .git\n",
        );
        assert_eq!(
            stderr,
            "** (MatchError) no match of right hand side value:\n\n    {\"fatal: not a git repository (or any of the parent directories): .git\\n\", 128}\n"
        );
    }

    /// Unique scratch dir under the temp dir for one git test.
    fn scratch_dir(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }

    /// True when `git` runs on this machine (else the caller skips).
    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_ok()
    }

    /// Init a scratch repo with probe identity config.
    fn init_repo(root: &Path) {
        std::fs::create_dir_all(root.join("lib")).expect("mkdir");
        for args in [
            ["init", "-q"].as_slice(),
            ["config", "user.email", "probe@example.com"].as_slice(),
            ["config", "user.name", "probe"].as_slice(),
            ["config", "commit.gpgsign", "false"].as_slice(),
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git runs");
            assert!(status.success(), "{args:?}");
        }
    }

    /// Stage and commit everything with `message`.
    fn commit_all(root: &Path, message: &str) {
        assert!(
            std::process::Command::new("git")
                .args(["add", "-A"])
                .current_dir(root)
                .status()
                .expect("add")
                .success()
        );
        assert!(
            std::process::Command::new("git")
                .args(["commit", "-qm", message])
                .current_dir(root)
                .status()
                .expect("commit")
                .success()
        );
    }

    /// Serial git integration: scratch repo under the temp dir with a
    /// unique name. Skips gracefully when `git` is unavailable. Run
    /// serially (`--test-threads=1`) alongside other git tests.
    #[test]
    fn serial_git_diff_reports_new_warning() {
        if !git_available() {
            return;
        }
        let root = scratch_dir("qredo-diff-test");
        let _ = std::fs::remove_dir_all(&root);
        init_repo(&root);
        std::fs::write(
            root.join(".credo.exs"),
            "%{configs: [%{name: \"default\", files: %{included: [\"lib/\"]}, checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}]}}]}\n",
        )
        .expect("config");
        std::fs::write(
            root.join("lib/a.ex"),
            "defmodule DiffA do\n  def f(x), do: x\nend\n",
        )
        .expect("clean");
        commit_all(&root, "clean");
        std::fs::write(
            root.join("lib/a.ex"),
            "defmodule DiffA do\n  def f(x) do\n    IO.inspect(x)\n    x\n  end\nend\n",
        )
        .expect("dirty");
        let opts = DiffOptions {
            working_dir: root.clone(),
            config_name: "default".to_owned(),
            ..DiffOptions::default()
        };
        let (stdout, stderr, code) = run_diff(&opts);
        assert!(stderr.is_empty(), "stderr: {stderr}");
        assert_eq!(code, 16, "stdout:\n{stdout}");
        assert!(stdout.contains("Diffing 1 source file in working dir with HEAD ...\n"));
        assert!(stdout.contains("+ ┃ [W]"));
        assert!(stdout.contains("+  added 1 new warning,\n"));
        assert!(stdout.contains("✔  fixed no issues, and\n"));
        assert!(stdout.contains("~  kept no issues.\n"));
        assert!(stdout.contains("`mix credo diff --help` for options)."));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Serial git integration: outside any repo reports the deterministic
    /// `MatchError` on stderr with exit 1 and empty stdout.
    #[test]
    fn serial_git_diff_outside_repo_is_match_error() {
        if !git_available() {
            return;
        }
        let root = scratch_dir("qredo-diff-outside");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("lib")).expect("mkdir");
        std::fs::write(root.join("lib/a.ex"), "defmodule A do\nend\n").expect("write");
        let opts = DiffOptions {
            working_dir: root.clone(),
            config_name: "default".to_owned(),
            ..DiffOptions::default()
        };
        let (stdout, stderr, code) = run_diff(&opts);
        assert!(stdout.is_empty());
        assert_eq!(code, 1);
        assert!(
            stderr.starts_with("** (MatchError) no match of right hand side value:"),
            "stderr: {stderr}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
