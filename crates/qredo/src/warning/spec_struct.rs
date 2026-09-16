use crate::Finding;

/// `EX5014`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    let mut search = 0_usize;
    while search < masked.len() {
        let Some(rel) = masked[search..].find("@spec") else {
            break;
        };
        let base = search + rel;
        search = base + 1;
        if !spec_boundary(masked, base) {
            continue;
        }
        let end = spec_end(masked, base);
        for (pct, name) in structs_in(&masked[base..end]) {
            let (line, column) = line_col(masked, base + pct);
            findings.push(Finding::with_trigger(
                line,
                Some(column),
                format!("Struct %{name}{{}} found in `@spec`."),
                format!("%{name}{{"),
            ));
        }
        search = end.max(base + 1);
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

/// Whether `@spec` at `base` is a real attribute (not `@special`, ...).
fn spec_boundary(masked: &str, base: usize) -> bool {
    if base > 0
        && masked[..base]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '@')
    {
        return false;
    }
    masked[base + "@spec".len()..]
        .chars()
        .next()
        .is_none_or(|c| c.is_whitespace() || c == '(')
}

/// Byte index where the `@spec` starting at `base` ends (first newline at
/// nesting depth zero, so multiline specs stay in one region).
fn spec_end(masked: &str, base: usize) -> usize {
    let mut depth = 0_usize;
    let mut pos = base + "@spec".len();
    while pos < masked.len() {
        let chr = masked[pos..].chars().next().unwrap_or(' ');
        match chr {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '\n' if depth == 0 => return pos,
            _ => {}
        }
        pos += chr.len_utf8();
    }
    masked.len()
}

/// `(byte offset, name)` of every `%Name{` struct literal in the region.
fn structs_in(region: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut search = 0_usize;
    while search < region.len() {
        let Some(rel) = region[search..].find('%') else {
            break;
        };
        let base = search + rel;
        search = base + 1;
        let name: String = region[base + 1..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        let mut chars = name.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_uppercase()) {
            continue;
        }
        if !region[base + 1 + name.len()..]
            .trim_start()
            .starts_with('{')
        {
            continue;
        }
        out.push((base, name));
    }
    out
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
    fn clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "@spec foo(integer) :: integer\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "@spec foo(%Foo{}) :: integer\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_two_structs_in_one_spec() {
        let src = "defmodule Offender do\n  @spec f(a_struct :: %AStruct{}, my_struct :: %MyApp.MyStruct{}) :: any\n  def f(_, _) do\n    \"oops\"\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].column, Some(23));
        assert_eq!(out[1].column, Some(48));
    }
}
