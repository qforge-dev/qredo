use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3014`: zero-arity `def` should (not) have parens per `parens` param.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let want_parens = helpers::param_bool(params, "parens", false);
    let mut findings = Vec::new();
    for (idx, line) in prepared.masked().split('\n').enumerate() {
        // Longest operator first so `defmacro` is not read as `def`.
        for op in ["defmacro", "defp", "def"] {
            let mut search = 0_usize;
            while let Some(pos) = line[search..].find(op) {
                let base = search + pos;
                search = base + op.len();
                if !word_boundary(line, base, op.len()) {
                    continue;
                }
                let Some(head) = def_head(&line[search..]) else {
                    continue;
                };
                search += head.consumed;
                if head.has_args {
                    continue;
                }
                let col = base + op.len() + head.name_col0 + 1;
                if head.has_parens && !want_parens {
                    findings.push(Finding::with_trigger(
                        idx + 1,
                        Some(col),
                        "Do not use parentheses when defining a function which has no arguments.",
                        head.name,
                    ));
                } else if !head.has_parens && want_parens {
                    findings.push(Finding::with_trigger(
                        idx + 1,
                        Some(col),
                        "Use parentheses when defining a function which has no arguments.",
                        head.name,
                    ));
                }
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

struct DefHead {
    name: String,
    /// Byte offset of the name from the start of the searched remainder.
    name_col0: usize,
    /// Bytes consumed from the searched remainder (for continued scanning).
    consumed: usize,
    has_args: bool,
    has_parens: bool,
}

/// Parse ` <name>(...)` after a `def`-family keyword. Returns `None` for
/// `defp`-style variables (`defp = 1`) and `unquote` heads.
fn def_head(after_op: &str) -> Option<DefHead> {
    if !after_op.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let gap = after_op
        .find(|c: char| !c.is_whitespace())
        .map_or(after_op.len(), |p| p);
    let rest = &after_op[gap..];
    let mut idx = 0_usize;
    while let Some(c) = rest[idx..].chars().next() {
        if !(c.is_alphanumeric() || c == '_' || c == '?' || c == '!') {
            break;
        }
        idx += c.len_utf8();
    }
    let name = rest[..idx].to_owned();
    if name.is_empty() || name == "unquote" {
        return None;
    }
    let tail = rest[idx..].trim_start();
    Some(DefHead {
        name,
        name_col0: gap,
        consumed: gap + idx,
        has_args: declared_args(tail),
        has_parens: is_empty_parens(tail),
    })
}

/// A head takes arguments unless its parenthesized list (if any) is
/// empty or blank, mirroring upstream's `[{_, _, [_ | _]}]` skip: any
/// actual argument (including a single bare variable) counts.
fn declared_args(tail: &str) -> bool {
    if !tail.starts_with('(') {
        return false;
    }
    let inner: String = tail[1..].chars().take_while(|c| *c != ')').collect();
    let closed = tail[1..].contains(')');
    if !closed {
        return false;
    }
    !inner.trim().is_empty()
}

/// Upstream `~r/^\((\w*)\)(.)*/` on the line remainder after the name.
fn is_empty_parens(tail: &str) -> bool {
    let Some(inner) = tail.strip_prefix('(') else {
        return false;
    };
    let end = inner
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map_or(inner.len(), |p| p);
    inner[end..].starts_with(')')
}

fn word_boundary(line: &str, base: usize, op_len: usize) -> bool {
    let before_ok = base == 0
        || !line[..base]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
    let after_ok = line[base + op_len..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric() && c != '_');
    before_ok && after_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_simple_argument_is_not_zero_arity() {
        // `defp fetch_fields(attrs)` carries an argument (upstream AST
        // `[_ | _]` skip); only truly empty heads report.
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("defp fetch_fields(attrs), do: attrs\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo(), do: 1\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn no_parens_by_default_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo, do: 1\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_empty_parens() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo(), do: 1\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn messages_match_upstream() {
        let missing = check_prepared(
            &crate::batch::Prepared::lazy("defmodule M do\n  def run do\n    21\n  end\nend\n"),
            &{
                let mut p = BTreeMap::new();
                p.insert("parens".to_owned(), "true".to_owned());
                p
            },
        );
        assert_eq!(missing.len(), 1);
        assert_eq!(
            missing[0].message,
            "Use parentheses when defining a function which has no arguments."
        );
        assert_eq!((missing[0].line, missing[0].column), (2, Some(7)));
        let present = check_prepared(
            &crate::batch::Prepared::lazy("defmodule M do\n  def run() do\n    21\n  end\nend\n"),
            &BTreeMap::new(),
        );
        assert_eq!(present.len(), 1);
        assert_eq!(
            present[0].message,
            "Do not use parentheses when defining a function which has no arguments."
        );
    }
}
