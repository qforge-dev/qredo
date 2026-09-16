use crate::Finding;

/// `EX3034`: one-arity functions in pipes should have parens.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let mut search = 0_usize;
        while let Some(pos) = line[search..].find("|>") {
            let base = search + pos + 2;
            if let Some((col, name)) = bare_piped_name(&line[base..], base) {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(col),
                    "One arity functions should have parentheses in pipes.",
                    name,
                ));
            }
            search = base;
            if search >= line.len() {
                break;
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// If the pipe target is a bare local call with no parens and no arguments
/// (upstream `{name, _, nil}`), return its 1-based column and name. Remote
/// calls (`Mod.fun`), parenthesized calls and calls with arguments parse
/// with a non-nil argument list and are clean.
fn bare_piped_name(after: &str, base: usize) -> Option<(usize, String)> {
    let gap = after
        .find(|c: char| !c.is_whitespace())
        .map_or(after.len(), |p| p);
    let mut idx = gap;
    let first = after[idx..].chars().next()?;
    if !(first.is_ascii_lowercase() || first == '_') {
        return None;
    }
    while let Some(c) = after[idx..].chars().next() {
        if !(c.is_alphanumeric() || c == '_') {
            break;
        }
        idx += c.len_utf8();
    }
    while let Some(c) = after[idx..].chars().next() {
        if c != '?' && c != '!' {
            break;
        }
        idx += c.len_utf8();
    }
    let name = after[gap..idx].to_owned();
    let tail = after[idx..].trim_start();
    // `(`, `.`, extra arguments or another call segment mean the AST has a
    // non-nil argument list.
    if tail.starts_with('(')
        || tail.starts_with('.')
        || tail.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == ':' || c == '"')
    {
        return None;
    }
    Some((base + gap + 1, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parens_are_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x |> foo()\n")).is_empty());
    }
    #[test]
    fn reports_missing_parens() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("x |> foo\n")).len(),
            1
        );
    }
    #[test]
    fn column_points_at_name() {
        let src =
            "defmodule Test do\n  def f(arg) do\n    arg\n    |> foo()\n    |> bar\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].line, out[0].column), (5, Some(8)));
        assert_eq!(out[0].trigger, crate::Trigger::Text("bar".to_owned()));
    }
    #[test]
    fn case_block_is_clean() {
        let src = "defmodule Test do\n  def f(arg) do\n    arg\n    |> foo()\n    |> case do\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
}
