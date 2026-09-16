use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3099`: prefer anonymous functions over bare captures where configured.
///
/// Upstream flags every outer `&` capture except a capture assigned directly
/// (`y = &...`, which the AST walk prunes) and the two opt-in shapes:
/// `& &1.field` / `& &1[...]` (`allow_field_access`) and remote `&Mod.fun/1`
/// (`allow_function_with_arity`, arity exactly 1).
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let allow_field = helpers::param_bool(params, "allow_field_access", false);
    let allow_arity = helpers::param_bool(params, "allow_function_with_arity", false);
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for (n, (b, c)) in chars.iter().enumerate() {
            if *c != '&' {
                continue;
            }
            let prev = n.checked_sub(1).map(|p| chars[p].1);
            let next = chars.get(n + 1).map(|(_, c)| *c);
            // `&&` / `&&&` boolean operators, not captures.
            if prev == Some('&') || next == Some('&') {
                continue;
            }
            // `&1` placeholders inside a surrounding capture.
            if next.is_some_and(|c| c.is_ascii_digit()) {
                continue;
            }
            // Direct assignment right-hand side (`y = &...`).
            if is_assigned(line, *b) {
                continue;
            }
            let rest = line[*b + 1..].trim_start();
            if allow_field && is_field_shape(rest) {
                continue;
            }
            if allow_arity && is_remote_arity_one(rest) {
                continue;
            }
            findings.push(Finding::with_trigger(
                idx + 1,
                Some(b + 1),
                "Use an anonymous function instead of the capture operator.",
                "&",
            ));
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Whether the `&` at byte offset `amp` is the direct right-hand side of `=`.
fn is_assigned(line: &str, amp: usize) -> bool {
    let pre = &line[..amp];
    let Some(eq) = pre.rfind('=') else {
        return false;
    };
    if !pre[eq + 1..].trim().is_empty() {
        return false;
    }
    !pre[..eq].ends_with(['=', '!', '<', '>', '|', '&', '+', '-', '*', '/', ':'])
}

/// `&1.field` (field call without arguments) or `&1[...]`, ignoring a
/// wrapping open paren from the same capture.
fn is_field_shape(rest: &str) -> bool {
    let mut s = rest.trim_start();
    while let Some(inner) = s.strip_prefix('(') {
        s = inner.trim_start();
    }
    if let Some(after) = s.strip_prefix("&1.") {
        let end = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(after.len(), |p| p);
        if end == 0 {
            return false;
        }
        let tail = after[end..].trim_start();
        return tail.is_empty() || tail.starts_with([')', ',', ']', '}', '|']);
    }
    s.starts_with("&1[")
}

/// Remote capture with arity exactly 1: `&Mod.fun/1`.
fn is_remote_arity_one(rest: &str) -> bool {
    let mut s = rest.trim_start();
    while let Some(inner) = s.strip_prefix('(') {
        s = inner.trim_start();
    }
    let Some(slash) = s.find('/') else {
        return false;
    };
    let (left, right) = (&s[..slash], &s[slash + 1..]);
    if left.is_empty()
        || !left.contains('.')
        || !left
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '?' || c == '!')
    {
        return false;
    }
    right
        .strip_prefix('1')
        .is_some_and(|t| t.starts_with(|c: char| !c.is_alphanumeric() && c != '_') || t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_capture_is_pruned() {
        // Upstream prunes captures assigned directly (`{:=, _, [_, {:&, ...}]}`).
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("f = &(&1 + 1)\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_bare_capture() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("Enum.map(x, &(&1 + 1))\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }

    #[test]
    fn allows_arity_capture_when_configured() {
        let mut params = BTreeMap::new();
        params.insert("allow_function_with_arity".to_owned(), "true".to_owned());
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("f = &String.downcase/1\n"),
                &params
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_field_capture_without_opt_in() {
        let src = "defmodule M do\n  def f(x) do\n    Enum.map(x, & &1.name)\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].line, out[0].column), (3, Some(17)));
    }
    #[test]
    fn allows_field_capture_when_configured() {
        let src = "defmodule M do\n  def f(x) do\n    Enum.map(x, & &1.name)\n  end\nend\n";
        let mut params = BTreeMap::new();
        params.insert("allow_field_access".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
    #[test]
    fn assigned_capture_is_clean() {
        let src = "defmodule M do\n  def f do\n    y = & &1\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }
}
