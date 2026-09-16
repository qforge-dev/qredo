use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX4010`: function arity.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let max_arity: usize = params
        .get("max_arity")
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let ignore_defp = helpers::param_bool(params, "ignore_defp", false);
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let trimmed = line.trim_start();
        let op = def_op(trimmed);
        if op.is_none() {
            continue;
        }
        if ignore_defp && op == Some("defp") {
            continue;
        }
        let Some(name) = def_name(trimmed) else {
            continue;
        };
        let Some(arity) = def_arity(line) else {
            continue;
        };
        if arity > max_arity {
            findings.push(
                Finding::with_trigger(
                    idx + 1,
                    credo_column(line, &name),
                    format!(
                        "Function takes too many parameters (arity is {arity}, max is {max_arity})."
                    ),
                    name,
                )
                .with_severity(crate::check_meta::severity_count(arity, max_arity)),
            );
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

fn def_op(trimmed: &str) -> Option<&'static str> {
    for op in ["defmacro ", "defp ", "def "] {
        if trimmed.starts_with(op) {
            return Some(op.trim_end());
        }
    }
    None
}

fn def_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.split_whitespace().nth(1)?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

fn def_arity(line: &str) -> Option<usize> {
    let open = line.find('(')?;
    let close = match_paren(line, open)?;
    let inside = line.get(open + 1..close)?.trim();
    if inside.is_empty() {
        return Some(0);
    }
    Some(split_top_level(inside).len())
}

fn match_paren(line: &str, open: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut depth = 0_usize;
    let mut idx = open;
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn split_top_level(inside: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (idx, byte) in inside.bytes().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(inside.get(start..idx).unwrap_or(""));
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(inside.get(start..).unwrap_or(""));
    parts
}

/// Credo `SourceFile.column/3`: 1-based column of `trigger` when surrounded by
/// whitespace, parens, commas or word boundaries; `None` otherwise.
fn credo_column(line: &str, trigger: &str) -> Option<usize> {
    if trigger.is_empty() {
        return None;
    }
    let lchars: Vec<char> = line.chars().collect();
    let tchars: Vec<char> = trigger.chars().collect();
    if lchars.len() < tchars.len() {
        return None;
    }
    for idx in 0..=lchars.len() - tchars.len() {
        if lchars[idx..idx + tchars.len()] != tchars[..] {
            continue;
        }
        let before_ok = if idx == 0 {
            is_word(tchars[0])
        } else {
            before_ok(lchars[idx - 1], tchars[0])
        };
        let after = idx + tchars.len();
        let after_ok = if after == lchars.len() {
            is_word(tchars[tchars.len() - 1])
        } else {
            after_ok(tchars[tchars.len() - 1], lchars[after])
        };
        if before_ok && after_ok {
            return Some(idx + 1);
        }
    }
    None
}

fn before_ok(prev: char, first: char) -> bool {
    prev.is_whitespace()
        || prev == '('
        || prev == ')'
        || prev == ','
        || is_word(prev) != is_word(first)
}

fn after_ok(last: char, next: char) -> bool {
    next.is_whitespace()
        || next == '('
        || next == ')'
        || next == ','
        || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_arity_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo(a, b), do: 1\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_large_arity() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo(a,b,c,d,e,f,g,h,i), do: 1\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn reports_trigger_column_like_upstream() {
        // EX4010.upstream.violation: column points at the function name.
        let src = "defmodule Credo.Sample.Module do\n  def some_function(p1, p2, p3, p4, p5, p6, p7, p8, p9) do\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, Some(7));
    }
    #[test]
    fn max_arity_param_is_honored() {
        let src = "def foo(a, b, c), do: 1\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
        let params: BTreeMap<String, String> = [("max_arity".to_owned(), "2".to_owned())]
            .into_iter()
            .collect();
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params).len(),
            1
        );
    }
    #[test]
    fn ignore_defp_param_is_honored() {
        let src = "defp foo(a,b,c,d,e,f,g,h,i), do: 1\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
        let params: BTreeMap<String, String> = [("ignore_defp".to_owned(), "true".to_owned())]
            .into_iter()
            .collect();
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
        let pub_src = "def foo(a,b,c,d,e,f,g,h,i), do: 1\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(pub_src), &params).len(),
            1
        );
    }
}
