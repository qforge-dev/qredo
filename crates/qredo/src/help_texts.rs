//! Verbatim upstream per-command help texts.
//!
//! Byte-exact captures of `mix credo --help` (general), `mix credo suggest
//! --help`, `list --help`, `info --help`, `explain --help` and `diff --help`
//! from the pinned upstream (`work/ref/credo` @ `ea1ccb9`, `MIX_ENV=dev`,
//! Elixir 1.20.2 / OTP 29, captured live on 2026-09-16; every run exit 0
//! with empty stderr). `mix credo help` is byte-identical to
//! `mix credo --help` at this pin, so one text serves both.
//!
//! Wiring contract for `main.rs` (the owner wires dispatch; this module only
//! serves text):
//!
//! ```ignore
//! mod help_texts; // top of main.rs
//!
//! // `--help` with no command, or the `help` command:
//! print!("{}", help_texts::general());
//! // `qredo <command> --help` (or `help <command>`):
//! print!("{}", help_texts::suggest()); // list/info/explain/diff likewise
//! ```
//!
//! Invocation spellings inside the texts stay `mix credo ...` exactly as
//! upstream prints them; they are NOT rewritten to `qredo`.
//!
//! Known residuals vs native:
//!
//! - R-HELP-1: upstream colorizes TTY output; these captures are piped
//!   plain text (no ANSI escapes), matching non-TTY runs.
//! - R-HELP-2: the texts embed the upstream version (`1.8.0-dev`) and
//!   hexdocs links; they are pinned snapshots, not generated.

/// General help (`mix credo --help`, identical to `mix credo help`).
#[must_use]
pub fn general() -> &'static str {
    GENERAL
}

/// `mix credo suggest --help`.
#[must_use]
pub fn suggest() -> &'static str {
    SUGGEST
}

/// `mix credo list --help`.
#[must_use]
pub fn list() -> &'static str {
    LIST
}

/// `mix credo info --help`.
#[must_use]
pub fn info() -> &'static str {
    INFO
}

/// `mix credo explain --help`.
#[must_use]
pub fn explain() -> &'static str {
    EXPLAIN
}

/// `mix credo diff --help`.
#[must_use]
pub fn diff() -> &'static str {
    DIFF
}

/// Pinned `mix credo --help` stdout (3571 bytes, leading blank line).
const GENERAL: &str = r"
                    ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇     ▇▇▇▇▇    ▇▇▇
                    ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▅▅▇▇▇▇▇▅▅▅ ▇▇▇
                       ▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
                      ▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
               ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▇▇▇▇▇▇▇▇▅▅▅▇▇▇▅▅▅▅▅▅▅▅▅▅▅
         ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▇▇▇▇▇▇▇▇▅▅▅▇▇▇▅▅▅▅▅▅▅▅▅▅▅
         ▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
                      ▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
  ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
  ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▇▇▇▇▇▇▇▇▇▇▅▅▅▇▇▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
       ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▇▇▇▇▇▇▇▇▇▇▅▅▅▇▇▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
                      ▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
      ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
      ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▇▇▅▅▅▇▇▇▇▇▇▅▅▅▅▅▅▅▅▅▅
             ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▇▇▅▅▅▇▇▇▇▇▇▅▅▅▅▅▅▅▅▅▅
                      ▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
                       ▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅
                    ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▅▅▅▅▇▇▇▇▇▅▅▅▅ ▇▇▇
                    ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇    ▇▇▇▇▇     ▇▇▇


Credo Version 1.8.0-dev
Usage: $ mix credo <command> [options]

Commands:

  suggest     Suggest code objects to look at next (default)
  explain     Show code object and explain why it is/might be an issue
  categories  Show and explain all issue categories
  diff        Suggest code objects to look at next (based on git-diff)
  gen.check   Create a new custom check
  gen.config  Initialize a new .credo.exs exec file in the current directory
  info        Show useful debug information
  list        List all issues grouped by files
  version     Show Credo's version number
  help        Show this help message

Use `--help` on any command to get further information.
For example, `mix credo suggest --help` for help on the default command.
";

/// Pinned `mix credo suggest --help` stdout (2094 bytes).
const SUGGEST: &str = r#"
Usage: mix credo suggest [options]

Suggests objects from every category that Credo thinks can be improved.

Examples:
  $ mix credo suggest --format json
  $ mix credo suggest "lib/**/*.ex" --only consistency --all
  $ mix credo suggest --checks-without-tag formatter --checks-without-tag controversial

Arrows (↑ ↗ → ↘ ↓) hint at the importance of an issue.

Suggest options:
  -a, --all                     Show all issues
  -A, --all-priorities          Show all issues including low priority ones
  -c, --checks                  Only include checks that match the given strings
      --checks-with-tag         Only include checks that match the given tag (can be used multiple times)
      --checks-without-tag      Ignore checks that match the given tag (can be used multiple times)
      --config-file             Use the given config file
  -C, --config-name             Use the given config instead of "default"
      --enable-disabled-checks  Re-enable disabled checks that match the given strings
      --files-included          Only include these files (accepts globs, can be used multiple times)
      --files-excluded          Exclude these files (accepts globs, can be used multiple times)
      --format                  Display the list in a specific format (json,flycheck,sarif,oneline)
  -i, --ignore-checks           Ignore checks that match the given strings
      --ignore                  Alias for --ignore-checks
      --min-priority            Minimum priority to show issues (higher,high,normal,low,ignore or number)
      --mute-exit-status        Exit with status zero even if there are issues
      --only                    Alias for --checks
      --strict                  Alias for --all-priorities

General options:
      --[no-]color              Toggle colored output
  -v, --version                 Show version
  -h, --help                    Show this help

Find advanced usage instructions and more examples here:
  https://hexdocs.pm/credo/suggest_command.html

Give feedback and open an issue here:
  https://github.com/rrrene/credo/issues
"#;

/// Pinned `mix credo list --help` stdout (2079 bytes).
const LIST: &str = r#"
Usage: mix credo list [options]

Lists objects that Credo thinks can be improved ordered by their priority.

Examples:
  $ mix credo list --format json
  $ mix credo list "lib/**/*.ex" --only consistency --all
  $ mix credo list --checks-without-tag formatter --checks-without-tag controversial

Arrows (↑ ↗ → ↘ ↓) hint at the importance of an issue.

List options:
  -a, --all                     Show all issues
  -A, --all-priorities          Show all issues including low priority ones
  -c, --checks                  Only include checks that match the given strings
      --checks-with-tag         Only include checks that match the given tag (can be used multiple times)
      --checks-without-tag      Ignore checks that match the given tag (can be used multiple times)
      --config-file             Use the given config file
  -C, --config-name             Use the given config instead of "default"
      --enable-disabled-checks  Re-enable disabled checks that match the given strings
      --files-included          Only include these files (accepts globs, can be used multiple times)
      --files-excluded          Exclude these files (accepts globs, can be used multiple times)
      --format                  Display the list in a specific format (json,flycheck,sarif,oneline)
  -i, --ignore-checks           Ignore checks that match the given strings
      --ignore                  Alias for --ignore-checks
      --min-priority            Minimum priority to show issues (higher,high,normal,low,ignore or number)
      --mute-exit-status        Exit with status zero even if there are issues
      --only                    Alias for --checks
      --strict                  Alias for --all-priorities

General options:
      --[no-]color              Toggle colored output
  -v, --version                 Show version
  -h, --help                    Show this help

Find advanced usage instructions and more examples here:
  https://hexdocs.pm/credo/list_command.html

Give feedback and open an issue here:
  https://github.com/rrrene/credo/issues
"#;

/// Pinned `mix credo info --help` stdout (1504 bytes).
const INFO: &str = r#"
Usage: mix credo info [options]

Shows information about Credo and its environment.

Example: $ mix credo info --format=json --verbose

Info options:
  -c, --checks                  Only include checks that match the given strings
      --checks-with-tag         Only include checks that match the given tag (can be used multiple times)
      --checks-without-tag      Ignore checks that match the given tag (can be used multiple times)
      --config-file             Use the given config file
  -C, --config-name             Use the given config instead of "default"
      --enable-disabled-checks  Re-enable disabled checks that match the given strings
      --files-included          Only include these files (accepts globs, can be used multiple times)
      --files-excluded          Exclude these files (accepts globs, can be used multiple times)
      --format                  Display the list in a specific format (json,flycheck,sarif,oneline)
  -i, --ignore-checks           Ignore checks that match the given strings
      --ignore                  Alias for --ignore-checks
      --min-priority            Minimum priority to show issues (higher,high,normal,low,ignore or number)
      --only                    Alias for --checks
      --verbose                 Display more information (e.g. checked files)

General options:
  -v, --version                 Show version
  -h, --help                    Show this help

Feedback:
  Open an issue here: https://github.com/rrrene/credo/issues
"#;

/// Pinned `mix credo explain --help` stdout (711 bytes).
const EXPLAIN: &str = r"
Usage: mix credo explain <check_name_or_path_line_no_column> [options]

Explain the given check or issue.

Examples:
  $ mix credo explain lib/foo/bar.ex:13:6
  $ mix credo explain lib/foo/bar.ex:13:6 --format json
  $ mix credo explain Credo.Check.Refactor.Nesting

Explain options:
      --format            Display the list in a specific format (json,flycheck,sarif,oneline)

General options:
      --[no-]color        Toggle colored output
  -v, --version           Show version
  -h, --help              Show this help

Find advanced usage instructions and more examples here:
  https://hexdocs.pm/credo/explain_command.html

Give feedback and open an issue here:
  https://github.com/rrrene/credo/issues
";

/// Pinned `mix credo diff --help` stdout (2174 bytes).
const DIFF: &str = r#"
Usage: mix credo diff [options]

Diffs objects against a point in Git's history.

Examples:
  $ mix credo diff v1.4.0
  $ mix credo diff main
  $ mix credo diff --from-git-ref HEAD --files-included "lib/**/*.ex"

Arrows (↑ ↗ → ↘ ↓) hint at the importance of an issue.

Diff options:
  -a, --all                     Show all new issues
  -A, --all-priorities          Show all new issues including low priority ones
  -c, --checks                  Only include checks that match the given strings
      --checks-with-tag         Only include checks that match the given tag (can be used multiple times)
      --checks-without-tag      Ignore checks that match the given tag (can be used multiple times)
      --config-file             Use the given config file
  -C, --config-name             Use the given config instead of "default"
      --enable-disabled-checks  Re-enable disabled checks that match the given strings
      --files-included          Only include these files (accepts globs, can be used multiple times)
      --files-excluded          Exclude these files (accepts globs, can be used multiple times)
      --format                  Display the list in a specific format (json)
      --from-dir                Diff from the given directory
      --from-git-ref            Diff from the given Git ref
      --from-git-merge-base     Diff from where the current HEAD branched off from the given merge base
  -i, --ignore-checks           Ignore checks that match the given strings
      --ignore                  Alias for --ignore-checks
      --min-priority            Minimum priority to show issues (higher,high,normal,low,ignore or number)
      --mute-exit-status        Exit with status zero even if there are issues
      --only                    Alias for --checks
      --since                   Diff from the given point in time (using Git)
      --strict                  Alias for --all-priorities

General options:
      --[no-]color              Toggle colored output
  -v, --version                 Show version
  -h, --help                    Show this help

Feedback:
  Open an issue here: https://github.com/rrrene/credo/issues
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_help_is_pinned() {
        let text = general();
        assert_eq!(text.len(), 3571);
        let mut lines = text.split('\n');
        assert_eq!(lines.next(), Some(""));
        assert!(lines.next().is_some_and(|line| line.contains("▇▇▇")));
        assert!(text.contains("Credo Version 1.8.0-dev"));
        assert!(text.contains("gen.check   Create a new custom check"));
        assert!(text.contains("gen.config  Initialize a new .credo.exs exec file"));
        assert!(text.ends_with(
            "For example, `mix credo suggest --help` for help on the default command.\n"
        ));
        assert!(text.contains("mix credo"), "upstream spellings are kept");
        assert!(!text.contains("qredo"), "never rewritten to qredo");
    }

    #[test]
    fn suggest_help_is_pinned() {
        let text = suggest();
        assert_eq!(text.len(), 2094);
        assert!(text.starts_with("\nUsage: mix credo suggest [options]\n"));
        assert!(text.contains("Suggest options:\n"));
        assert!(text.contains("https://hexdocs.pm/credo/suggest_command.html"));
        assert!(text.ends_with("  https://github.com/rrrene/credo/issues\n"));
    }

    #[test]
    fn list_help_is_pinned() {
        let text = list();
        assert_eq!(text.len(), 2079);
        assert!(text.starts_with("\nUsage: mix credo list [options]\n"));
        assert!(text.contains("List options:\n"));
        assert!(text.contains("https://hexdocs.pm/credo/list_command.html"));
        assert!(text.ends_with("  https://github.com/rrrene/credo/issues\n"));
    }

    #[test]
    fn info_help_is_pinned() {
        let text = info();
        assert_eq!(text.len(), 1504);
        assert!(text.starts_with("\nUsage: mix credo info [options]\n"));
        assert!(text.contains("Info options:\n"));
        assert!(text.contains("  -c, --checks"));
        assert!(text.ends_with("  Open an issue here: https://github.com/rrrene/credo/issues\n"));
    }

    #[test]
    fn explain_help_is_pinned() {
        let text = explain();
        assert_eq!(text.len(), 711);
        assert!(text.starts_with(
            "\nUsage: mix credo explain <check_name_or_path_line_no_column> [options]\n"
        ));
        assert!(text.contains("Explain options:\n"));
        assert!(text.contains("https://hexdocs.pm/credo/explain_command.html"));
        assert!(text.ends_with("  https://github.com/rrrene/credo/issues\n"));
    }

    #[test]
    fn diff_help_is_pinned() {
        let text = diff();
        assert_eq!(text.len(), 2174);
        assert!(text.starts_with("\nUsage: mix credo diff [options]\n"));
        assert!(text.contains("Diff options:\n"));
        assert!(text.contains("      --from-git-ref"));
        assert!(text.contains("--format"));
        assert!(text.ends_with("  Open an issue here: https://github.com/rrrene/credo/issues\n"));
    }
}
