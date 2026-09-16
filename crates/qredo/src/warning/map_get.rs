use crate::Finding;

/// `EX5009`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    let mut search = 0_usize;
    while search < masked.len() {
        let Some(rel) = masked[search..].find("Map.get") else {
            break;
        };
        let base = search + rel;
        search = base + 1;
        if !before_ok(masked, base) || !masked[base + "Map.get".len()..].starts_with('(') {
            continue;
        }
        let Some(args) = paren_args(masked, base + "Map.get".len()) else {
            continue;
        };
        if args.len() > 2 {
            continue;
        }
        let piped_into = fed_by_pipe(masked, base);
        let piped_to_enum = feeds_enum(masked, args_close(masked, base));
        // `/1` flags when piped through into `Enum`; `/2` flags as a pipe
        // start into `Enum`. Other shapes are safe or out of scope.
        let hit =
            piped_to_enum && ((args.len() == 1 && piped_into) || (args.len() == 2 && !piped_into));
        if !hit {
            continue;
        }
        let (line, column) = line_col(masked, base);
        findings.push(Finding::with_trigger(
            line,
            Some(column),
            "`Map.get` with no default return value is potentially unsafe in pipes, use `Map.get/3` instead.",
            "Map.get".to_owned(),
        ));
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

/// End (byte index past `(`) helper: closing paren of the call at `base`.
fn args_close(masked: &str, base: usize) -> usize {
    paren_close(masked, base + "Map.get".len()).unwrap_or(masked.len())
}

/// Byte index just past the closing paren of the call opening at `open`.
fn paren_close(masked: &str, open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (rel, chr) in masked[open..].char_indices() {
        match chr {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + rel + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Byte ranges of top-level comma-separated arguments in `(...)`.
fn paren_args(masked: &str, open: usize) -> Option<Vec<(usize, usize)>> {
    let mut args = Vec::new();
    let mut depth = 0_usize;
    let mut arg_start = 0_usize;
    for (rel, chr) in masked[open..].char_indices() {
        let idx = open + rel;
        match chr {
            '(' | '[' | '{' => {
                if depth == 0 {
                    arg_start = idx + 1;
                }
                depth += 1;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    args.push((arg_start, idx));
                    return Some(args);
                }
            }
            ',' if depth == 1 => {
                args.push((arg_start, idx));
                arg_start = idx + 1;
            }
            _ => {}
        }
    }
    None
}

/// Whether `Map.get` is fed by a pipe (`... |> Map.get`).
fn fed_by_pipe(masked: &str, base: usize) -> bool {
    masked[..base].trim_end().ends_with("|>")
}

/// Whether the call ending at `close` is directly piped into `Enum.fun`.
fn feeds_enum(masked: &str, close: usize) -> bool {
    let rest = masked[close..].trim_start();
    let Some(pipe) = rest.strip_prefix("|>") else {
        return false;
    };
    let target = pipe.trim_start();
    // `Enum.fun` (lowercase call); `Enum.Sub.fun` is a different module.
    target.starts_with("Enum.")
        && target["Enum.".len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
}

/// The char before `Map` must not continue another name or remote path.
fn before_ok(masked: &str, base: usize) -> bool {
    if base == 0 {
        return true;
    }
    !masked[..base]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// 1-based `(line, column-in-chars)` of the byte offset (must be a boundary).
fn line_col(masked: &str, byte_idx: usize) -> (usize, usize) {
    let upto = &masked[..byte_idx];
    let line = upto.matches('\n').count() + 1;
    let column = upto
        .rsplit('\n')
        .next()
        .map_or(1, |last| last.chars().count() + 1);
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "x |> Map.get(:a, :default)\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_piped_into_enum() {
        let src = "defmodule CredoSampleModule do\n  def some_function() do\n\n    %{}\n    |> Map.get(:foo)\n    |> Enum.sum\n\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 5);
        assert_eq!(out[0].column, Some(8));
    }
    #[test]
    fn non_enum_pipe_is_clean() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1) do\n\n      %{}\n      |> Map.get(:foo)\n      |> some_arbitrary_function\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
}
