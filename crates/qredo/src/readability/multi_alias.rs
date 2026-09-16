use crate::Finding;

/// `EX3011`: avoid `alias Foo.{Bar, Baz}` multi-alias syntax.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let mut search = 0_usize;
        while let Some(pos) = line[search..].find("alias") {
            let base = search + pos;
            let after_kw = base + "alias".len();
            if is_word_boundary(line, base)
                && line[after_kw..].starts_with(|c: char| c.is_whitespace())
                && let Some((col, trigger)) = first_expansion(&line[after_kw..], after_kw)
            {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(col),
                    "Avoid grouping aliases in '{ ... }'; please specify one fully-qualified alias per line.",
                    trigger,
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

/// If the text after `alias` is `<Base>.{<First>, ...}`, return the 1-based
/// column and text of the first expanded module.
fn first_expansion(after: &str, after_kw: usize) -> Option<(usize, String)> {
    let mut idx = after
        .find(|c: char| !c.is_whitespace())
        .map_or(after.len(), |p| p);
    let path_start = idx;
    while let Some(c) = after[idx..].chars().next() {
        if !is_path_char(c) {
            break;
        }
        idx += c.len_utf8();
    }
    if idx == path_start {
        return None;
    }
    let skipped_ws = after[idx..]
        .find(|c: char| !c.is_whitespace())
        .map_or(after[idx..].len(), |p| p);
    idx += skipped_ws;
    if after[idx..].starts_with('{') {
        idx += 1;
    } else {
        return None;
    }
    let inner_ws = after[idx..]
        .find(|c: char| !c.is_whitespace())
        .map_or(after[idx..].len(), |p| p);
    idx += inner_ws;
    let name_start = idx;
    while let Some(c) = after[idx..].chars().next() {
        if !is_path_char(c) {
            break;
        }
        idx += c.len_utf8();
    }
    if idx == name_start {
        return None;
    }
    Some((after_kw + name_start + 1, after[name_start..idx].to_owned()))
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
}
