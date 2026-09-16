use crate::Finding;

/// `EX3011`: avoid `alias Foo.{Bar, Baz}` multi-alias syntax.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let mut search = 0_usize;
        while let Some(pos) = line[search..].find("alias") {
            let base = search + pos;
            let after_kw = base + "alias".len();
            if is_word_boundary(line, base)
                && line[after_kw..].starts_with(|c: char| c.is_whitespace())
                && let Some(hit) = first_expansion(&lines, idx, &line[after_kw..], after_kw)
            {
                findings.push(Finding::with_trigger(
                    hit.line,
                    hit.column,
                    "Avoid grouping aliases in '{ ... }'; please specify one fully-qualified alias per line.",
                    hit.trigger,
                ));
                break;
            }
            search = after_kw;
            if search >= line.len() {
                break;
            }
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// One multi-alias hit: 1-based line/column and first inner module text.
struct Hit {
    line: usize,
    column: Option<usize>,
    trigger: String,
}

/// If the text after `alias` is `<Base>.{<First>, ...}`, return the hit.
/// When `{` ends the line, the first inner module is read from the
/// following lines (reported at the directive line with no column, like
/// upstream); otherwise the hit stays on the directive line.
fn first_expansion(lines: &[&str], idx: usize, after: &str, after_kw: usize) -> Option<Hit> {
    let brace = open_brace(after)?;
    let inner_ws = after[brace..]
        .find(|c: char| !c.is_whitespace())
        .map_or(after[brace..].len(), |p| p);
    let cursor = brace + inner_ws;
    if cursor < after.len() {
        return same_line_hit(after, after_kw, idx, cursor);
    }
    // `{` at end of line: first inner module lives on a following line.
    continued_hit(lines, idx)
}

/// Offset just past `<Base>.{` in the text after `alias`, if present.
fn open_brace(after: &str) -> Option<usize> {
    let mut cursor = after
        .find(|c: char| !c.is_whitespace())
        .map_or(after.len(), |p| p);
    let path_start = cursor;
    while let Some(c) = after[cursor..].chars().next() {
        if !is_path_char(c) {
            break;
        }
        cursor += c.len_utf8();
    }
    if cursor == path_start {
        return None;
    }
    let skipped_ws = after[cursor..]
        .find(|c: char| !c.is_whitespace())
        .map_or(after[cursor..].len(), |p| p);
    cursor += skipped_ws;
    if !after[cursor..].starts_with('{') {
        return None;
    }
    Some(cursor + 1)
}

/// Hit for a first inner module starting at `cursor` on the directive line.
fn same_line_hit(after: &str, after_kw: usize, idx: usize, mut cursor: usize) -> Option<Hit> {
    let name_start = cursor;
    while let Some(c) = after[cursor..].chars().next() {
        if !is_path_char(c) {
            break;
        }
        cursor += c.len_utf8();
    }
    if cursor == name_start {
        return None;
    }
    Some(Hit {
        line: idx + 1,
        column: Some(after_kw + name_start + 1),
        trigger: after[name_start..cursor].to_owned(),
    })
}

/// Hit for a first inner module on the lines below a trailing `{`.
fn continued_hit(lines: &[&str], idx: usize) -> Option<Hit> {
    for next in lines.iter().skip(idx + 1) {
        let mut start = next
            .find(|c: char| !c.is_whitespace())
            .map_or(next.len(), |p| p);
        if start == next.len() {
            continue;
        }
        // A closing brace before any name means no expansion.
        if next[start..].starts_with('}') {
            return None;
        }
        let name_start = start;
        while let Some(c) = next[start..].chars().next() {
            if !is_path_char(c) || c == '.' {
                break;
            }
            start += c.len_utf8();
        }
        if start == name_start {
            return None;
        }
        return Some(Hit {
            line: idx + 1,
            column: None,
            trigger: next[name_start..start].to_owned(),
        });
    }
    None
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

fn is_word_boundary(line: &str, base: usize) -> bool {
    let before_ok = base == 0
        || !line[..base]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
    let after_ok = line[base + "alias".len()..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric() && c != '_');
    before_ok && after_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_alias_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("alias Foo.Bar\n")).is_empty());
    }

    #[test]
    fn reports_multi_alias() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("alias Foo.{Bar, Baz}\n")).len(),
            1
        );
    }

    #[test]
    fn trigger_is_first_inner_module() {
        let out = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  alias App.Module2.{Module3}\nend\n",
        ));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
        assert_eq!(out[0].column, Some(22));
        assert_eq!(out[0].trigger, crate::Trigger::Text("Module3".to_owned()));
    }

    #[test]
    fn brace_at_eol_with_parts_on_following_lines() {
        // MA-ML: `alias Base.{` at end of line, parts below. Upstream
        // reports the directive line with the first inner module.
        let out = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  alias Labqoat.AWS.{\n    BuildLogs,\n    Deploys\n  }\nend\n",
        ));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
        assert_eq!(out[0].trigger, crate::Trigger::Text("BuildLogs".to_owned()));
    }
}
