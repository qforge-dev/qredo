use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// One alias statement: single `alias Foo.Bar` or multi `alias Foo.{A, B}`.
enum Statement {
    Single {
        line: usize,
        column: usize,
        module: String,
    },
    Multi {
        start_line: usize,
        end_line: usize,
        column: usize,
        base: String,
        inners: Vec<Inner>,
    },
}

struct Inner {
    line: usize,
    column: usize,
    short: String,
    full: String,
}

/// `EX3002`: aliases within a group should be alphabetically ordered.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let ascii = helpers::param_str(params, "sort_method", "alpha") == "ascii";
    let statements = parse_statements(prepared.masked());
    let mut findings = Vec::new();
    for group in group_statements(&statements) {
        for statement in &group {
            if let Statement::Multi { inners, .. } = statement {
                check_inner_order(inners, ascii, &mut findings);
            }
        }
        check_group_order(&group, ascii, &mut findings);
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

/// Sort key: compare name plus line span for stable mismatch detection.
fn order_key(statement: &Statement, ascii: bool) -> (String, usize, usize) {
    match statement {
        Statement::Single { line, module, .. } => (compare_name(module, ascii), *line, *line),
        Statement::Multi {
            start_line,
            end_line,
            base,
            inners,
            ..
        } => {
            let first = inners
                .first()
                .map_or_else(|| base.clone(), |inner| inner.full.clone());
            (compare_name(&first, ascii), *start_line, *end_line)
        }
    }
}

fn compare_name(value: &str, ascii: bool) -> String {
    let lowered = if ascii {
        value.to_owned()
    } else {
        value.to_lowercase()
    };
    let no_braces: String = lowered.chars().filter(|c| *c != '{' && *c != '}').collect();
    match no_braces.find(',') {
        Some(pos) => no_braces[..pos].to_owned(),
        None => no_braces,
    }
}

/// First position where the group order differs from the sorted order.
fn check_group_order(group: &[&Statement], ascii: bool, findings: &mut Vec<Finding>) {
    if group.len() < 2 {
        return;
    }
    let mut sorted: Vec<&Statement> = group.to_vec();
    sorted.sort_by_key(|statement| order_key(statement, ascii));
    let actual: Vec<(String, usize, usize)> = group.iter().map(|s| order_key(s, ascii)).collect();
    let expected: Vec<(String, usize, usize)> =
        sorted.iter().map(|s| order_key(s, ascii)).collect();
    if actual == expected {
        return;
    }
    let mismatch = group
        .iter()
        .zip(sorted.iter())
        .find(|(a, b)| order_key(a, ascii) != order_key(b, ascii));
    if let Some((statement, _)) = mismatch {
        match statement {
            Statement::Single {
                line,
                column,
                module,
            } => findings.push(Finding::with_trigger(
                *line,
                Some(*column),
                format!("The alias `{module}` is not alphabetically ordered among its group."),
                (*module).clone(),
            )),
            Statement::Multi {
                start_line,
                column,
                base,
                ..
            } => findings.push(Finding::with_trigger(
                *start_line,
                Some(*column),
                format!("The alias `{base}` is not alphabetically ordered among its group."),
                (*base).clone(),
            )),
        }
    }
}

/// Inner segments of one multi-alias must be ordered among themselves.
fn check_inner_order(inners: &[Inner], ascii: bool, findings: &mut Vec<Finding>) {
    if inners.len() < 2 {
        return;
    }
    let mut sorted: Vec<&Inner> = inners.iter().collect();
    sorted.sort_by(|a, b| {
        compare_name(&a.full, ascii)
            .cmp(&compare_name(&b.full, ascii))
            .then((a.line, a.column).cmp(&(b.line, b.column)))
    });
    let mismatch = inners
        .iter()
        .zip(sorted.iter())
        .find(|(a, b)| a.full != b.full);
    if let Some((inner, _)) = mismatch {
        findings.push(Finding::with_trigger(
            inner.line,
            Some(inner.column),
            format!(
                "The alias `{}` is not alphabetically ordered among its group.",
                inner.full
            ),
            inner.short.clone(),
        ));
    }
}

/// Consecutive (line-adjacent) alias statements form a group.
fn group_statements(statements: &[Statement]) -> Vec<Vec<&Statement>> {
    let mut groups: Vec<Vec<&Statement>> = Vec::new();
    let mut current: Vec<&Statement> = Vec::new();
    let mut prev_end = 0_usize;
    for statement in statements {
        let (start, end) = match statement {
            Statement::Single { line, .. } => (*line, *line),
            Statement::Multi {
                start_line,
                end_line,
                ..
            } => (*start_line, *end_line),
        };
        if !current.is_empty() && start != prev_end + 1 {
            groups.push(std::mem::take(&mut current));
        }
        prev_end = end;
        current.push(statement);
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// Parse `alias` statements; multi-alias segments may span lines.
fn parse_statements(masked: &str) -> Vec<Statement> {
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut out = Vec::new();
    let mut idx = 0_usize;
    while idx < lines.len() {
        let chars: Vec<char> = lines[idx].chars().collect();
        let Some(alias_end) = match_alias(&chars) else {
            idx += 1;
            continue;
        };
        let rest: String = chars[alias_end..].iter().collect();
        let name_start = alias_end + leading_spaces(&rest);
        let name: String = chars[name_start..].iter().collect::<String>();
        let name: String = name
            .chars()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
            .collect();
        if name.is_empty() {
            idx += 1;
            continue;
        }
        let after: String = chars[name_start + name.chars().count()..].iter().collect();
        if after.trim_start().starts_with('{') {
            let base = name.trim_end_matches('.').to_owned();
            if !base.is_empty()
                && let Some(statement) = parse_multi(&lines, idx, name_start, &base)
            {
                idx = statement_end(&statement);
                out.push(statement);
                continue;
            }
        }
        out.push(Statement::Single {
            line: idx + 1,
            column: name_start + 1,
            module: name,
        });
        idx += 1;
    }
    out
}

fn statement_end(statement: &Statement) -> usize {
    match statement {
        Statement::Single { line, .. } => *line,
        Statement::Multi { end_line, .. } => *end_line,
    }
}

/// Byte-free `alias` keyword match; returns the end offset (chars).
fn match_alias(chars: &[char]) -> Option<usize> {
    let word: Vec<char> = "alias".chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        if chars[i..].starts_with(&word)
            && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_'))
            && chars
                .get(i + word.len())
                .is_some_and(|c| *c == ' ' || *c == '(')
            && chars[..i].iter().all(|c| c.is_whitespace())
        {
            let mut end = i + word.len();
            if chars.get(end) == Some(&'(') {
                end += 1;
            }
            while chars.get(end) == Some(&' ') {
                end += 1;
            }
            return Some(end);
        }
        i += 1;
    }
    None
}

fn leading_spaces(text: &str) -> usize {
    text.chars().take_while(|c| *c == ' ').count()
}

/// Parse `alias Base.{A, B}` starting at `line_idx`; `None` when unclosed.
fn parse_multi(
    lines: &[&str],
    line_idx: usize,
    name_start: usize,
    base: &str,
) -> Option<Statement> {
    let first: Vec<char> = lines[line_idx].chars().collect();
    let after_base = name_start + base.chars().count();
    let mut offset = after_base;
    while offset < first.len() && first[offset] == ' ' {
        offset += 1;
    }
    if first.get(offset) == Some(&'.') {
        offset += 1;
    }
    if first.get(offset) != Some(&'{') {
        return None;
    }
    offset += 1;
    let mut scan = InnerScan::new(line_idx, offset, base);
    let mut idx = line_idx;
    let mut pos = offset;
    while idx < lines.len() {
        let chars: Vec<char> = lines[idx].chars().collect();
        while pos < chars.len() {
            if chars[pos] == '}' {
                return Some(scan.close(idx, line_idx, name_start));
            }
            scan.push(chars[pos], idx, pos);
            pos += 1;
        }
        idx += 1;
        pos = 0;
    }
    None
}

/// Incremental `Base.{A, B}` segment scanner.
struct InnerScan<'a> {
    current: String,
    seg_line: usize,
    seg_col: usize,
    started: bool,
    base: &'a str,
    inners: Vec<Inner>,
}

impl<'a> InnerScan<'a> {
    fn new(line_idx: usize, offset: usize, base: &'a str) -> Self {
        Self {
            current: String::new(),
            seg_line: line_idx,
            seg_col: offset,
            started: false,
            base,
            inners: Vec::new(),
        }
    }

    fn push(&mut self, c: char, idx: usize, pos: usize) {
        if c == ',' {
            self.finish_segment();
        } else if c == ' ' && !self.started {
            self.seg_col += 1;
        } else {
            if !self.started {
                self.seg_line = idx;
                self.seg_col = pos;
                self.started = true;
            }
            self.current.push(c);
        }
    }

    fn finish_segment(&mut self) {
        push_inner(
            &mut self.inners,
            &self.current,
            self.seg_line,
            self.seg_col,
            self.base,
        );
        self.current.clear();
        self.started = false;
    }

    fn close(mut self, idx: usize, line_idx: usize, name_start: usize) -> Statement {
        self.finish_segment();
        Statement::Multi {
            start_line: line_idx + 1,
            end_line: idx + 1,
            column: name_start + 1,
            base: self.base.to_owned(),
            inners: self.inners,
        }
    }
}

fn push_inner(inners: &mut Vec<Inner>, raw: &str, seg_line: usize, seg_col: usize, base: &str) {
    let short = raw.trim().to_owned();
    if short.is_empty() {
        return;
    }
    inners.push(Inner {
        line: seg_line + 1,
        column: seg_col + 1,
        full: format!("{base}.{short}"),
        short,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_aliases_are_clean() {
        let src = "alias Foo.Bar\nalias Foo.Baz\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn reports_unordered_aliases() {
        let src = "alias Foo.Zebra\nalias Foo.Apple\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }

    #[test]
    fn multi_alias_inner_order_is_checked() {
        let src = "defmodule M do\n  alias App.Foo.{Sorter,Command,Filename}\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(18));
    }
}
