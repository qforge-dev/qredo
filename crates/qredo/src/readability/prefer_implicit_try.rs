use crate::Finding;

/// `EX3017`: prefer implicit `try` in `def` over explicit `try do`.
///
/// Upstream only flags `try` when it is the entire body of a
/// `def`/`defp`/`defmacro`. Textually: a `try do` line directly under a
/// `def ... do` head whose matching `end` is immediately closed by the
/// definition's own `end`.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if !is_try_open(trimmed) {
            continue;
        }
        let Some(def_indent) = prev_def_indent(&lines[..idx]) else {
            continue;
        };
        if def_indent >= indent {
            continue;
        }
        let Some(try_end) = try_end_index(&lines[idx + 1..], indent) else {
            continue;
        };
        if !def_closes_next(&lines[idx + 1 + try_end + 1..], indent) {
            continue;
        }
        findings.push(Finding::with_trigger(
            idx + 1,
            Some(line.find("try").unwrap_or(indent) + 1),
            "Prefer using an implicit `try` rather than explicit `try`.",
            "try".to_owned(),
        ));
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// A `try do` block opener (bare `try` counts; `tryfoo` does not).
fn is_try_open(trimmed: &str) -> bool {
    if trimmed == "try" {
        return true;
    }
    trimmed.strip_prefix("try").is_some_and(|t| {
        t.starts_with(|c: char| c.is_whitespace()) && t.trim_start().starts_with("do")
    })
}

/// Indent of the nearest preceding non-blank line when it opens a
/// `def`/`defp`/`defmacro` block with `do`.
fn prev_def_indent(before: &[&str]) -> Option<usize> {
    let line = before.iter().rev().find(|l| !l.trim().is_empty())?;
    let trimmed = line.trim_start();
    let is_def = ["defmacro ", "defp ", "def "]
        .iter()
        .any(|op| trimmed.starts_with(op));
    if !is_def {
        return None;
    }
    // Masked comments leave trailing blanks; compare the code ending.
    let code = trimmed.trim_end();
    if code == "do" || code.ends_with(" do") || code.ends_with("\tdo") {
        Some(line.len() - trimmed.len())
    } else {
        None
    }
}

/// Offset (relative to `after`) of the `try` block's closing `end`: the first
/// non-blank line at or above the `try` indent must be `end` at equal indent.
fn try_end_index(after: &[&str], indent: usize) -> Option<usize> {
    for (off, line) in after.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let trimmed = line.trim_start();
        let this = line.len() - trimmed.len();
        if this > indent {
            continue;
        }
        if this == indent && is_end(trimmed) {
            return Some(off);
        }
        // `rescue`/`catch`/`after`/`else` clauses sit at the `try` indent.
        if this == indent && is_clause(trimmed) {
            continue;
        }
        return None;
    }
    None
}

/// Whether the next non-blank line closes the definition (`end` dedented).
fn def_closes_next(after: &[&str], indent: usize) -> bool {
    for line in after {
        if line.trim().is_empty() {
            continue;
        }
        let trimmed = line.trim_start();
        let this = line.len() - trimmed.len();
        return this < indent && is_end(trimmed);
    }
    false
}

fn is_end(trimmed: &str) -> bool {
    trimmed == "end"
        || trimmed
            .strip_prefix("end")
            .is_some_and(|t| t.starts_with(|c: char| !c.is_alphanumeric() && c != '_'))
}

fn is_clause(trimmed: &str) -> bool {
    ["rescue", "catch", "after", "else"].iter().any(|kw| {
        trimmed == *kw
            || trimmed
                .strip_prefix(kw)
                .is_some_and(|t| t.starts_with(|c: char| !c.is_alphanumeric() && c != '_'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn implicit_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "def f do\n  x\nrescue\n  y\nend\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_explicit_try() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "def f do\n  try do\n  end\nend\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn nested_try_is_clean() {
        let src = "defmodule M do\n  def f(first) do\n    other()\n    str =\n      try do\n        to_string(first)\n      rescue\n        _ -> :x\n      end\n    to_atom(str)\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn direct_body_try_reports_try_line() {
        let src = "defmodule M do\n  def f(first) do\n    try do\n      to_string(first)\n    rescue\n      _ -> :x\n    end\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].line, out[0].column), (3, Some(5)));
    }
}
