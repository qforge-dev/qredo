use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3019`: at most `max_blank_lines` (default 1) consecutive blank lines.
/// Blank lines inside heredocs/strings do not count.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let max_blank = helpers::param_usize(params, "max_blank_lines", 1);
    let source = prepared.source();
    let interior = string_interior_rows(source, prepared.facts());
    // Collect blank-line numbers: whitespace-only raw rows outside string
    // literals. Comment-only rows are not blank; the final empty segment
    // after a trailing `\n` is not a line.
    let mut blank_lines: Vec<usize> = Vec::new();
    let rows: Vec<&str> = source.split('\n').collect();
    let row_count = rows.len();
    for (idx, line) in rows.iter().enumerate() {
        let line_no = idx + 1;
        if line_no == row_count && source.ends_with('\n') {
            continue;
        }
        if line.trim().is_empty() && !interior.contains(&line_no) {
            blank_lines.push(line_no);
        }
    }
    let mut findings = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut run_len = 0_usize;
    let mut prev: Option<usize> = None;
    for line in blank_lines {
        match prev {
            Some(p) if line == p + 1 => {
                run_len += 1;
            }
            _ => {
                flush_run(run_start, run_len, &mut findings, max_blank);
                run_start = Some(line);
                run_len = 1;
            }
        }
        prev = Some(line);
    }
    flush_run(run_start, run_len, &mut findings, max_blank);
    findings
}

/// 1-based rows strictly inside multi-line string-like spans: blank source
/// rows in this set are literal contents, not blank code lines.
/// Single-line literals contribute nothing. Row math mirrors the retired
/// tree walk over `string`/`charlist`/`sigil` nodes exactly.
fn string_interior_rows(
    source: &str,
    facts: &crate::facts::Facts,
) -> std::collections::BTreeSet<usize> {
    let mut starts = vec![0_usize];
    starts.extend(source.match_indices('\n').map(|(byte, _)| byte + 1));
    let mut rows = std::collections::BTreeSet::new();
    for (start, end) in &facts.string_ranges {
        let first = starts.partition_point(|slot| *slot <= *start as usize);
        let last = starts.partition_point(|slot| *slot <= *end as usize);
        if last > first + 1 {
            rows.extend((first + 1)..last);
        }
    }
    rows
}

/// Report every blank line beyond the allowance within one run.
fn flush_run(start: Option<usize>, len: usize, findings: &mut Vec<Finding>, max_blank: usize) {
    if len > max_blank
        && let Some(first) = start
    {
        for line in (first + max_blank)..(first + len) {
            findings.push(Finding::no_trigger(
                line,
                format!("There should be no more than {max_blank} consecutive blank lines."),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn allows_single_blank_line() {
        let src = "defmodule M do\n  def a, do: 1\n\n  def b, do: 2\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &defaults()).is_empty());
    }

    #[test]
    fn reports_double_blank_line() {
        let src = "defmodule M do\n  def a, do: 1\n\n\n  def b, do: 2\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &defaults());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn respects_max_blank_lines_param() {
        let src = "defmodule M do\n  def a, do: 1\n\n\n\n  def b, do: 2\nend\n";
        let mut params = BTreeMap::new();
        params.insert("max_blank_lines".to_owned(), "4".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
        params.insert("max_blank_lines".to_owned(), "1".to_owned());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params).len(),
            2
        );
    }

    #[test]
    fn ignores_blank_lines_inside_strings() {
        let src = "defmodule M do\n  def b do\n    foo = \"\n    a\n\n\n    b\n    \"\n\n    2\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &defaults()).is_empty());
    }

    #[test]
    fn ignores_blank_lines_inside_heredocs() {
        let src = "defmodule M do\n  def a do\n    \"\"\"\n    intro\n\n\n    ---\n    \"\"\"\n  end\n\n  def b, do: 2\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &defaults()).is_empty());
    }

    #[test]
    fn comment_lines_are_not_blank() {
        let src = "defmodule M do\n  def a, do: 1\n\n  # comment\n\n  def b, do: 2\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &defaults()).is_empty());
    }
}
