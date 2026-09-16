use crate::Finding;
use std::collections::BTreeMap;

/// `EX5029`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let max_fields: usize = params
        .get("max_fields")
        .and_then(|v| v.parse().ok())
        .unwrap_or(31);
    let mut findings = Vec::new();
    let facts = prepared.facts();
    let lines: Vec<&str> = source.split('\n').collect();
    let starts = line_starts(source);
    for call in &facts.calls {
        let Some(crate::facts::HeadFact::Plain { start, end }) = &call.head else {
            continue;
        };
        if slice(source, *start, *end) != Some("defstruct") {
            continue;
        }
        if let Some(count) = field_count(call)
            && count > max_fields
        {
            let (line, column) = line_column(&lines, &starts, call.start as usize);
            findings.push(Finding::with_trigger(
                line,
                Some(column),
                format!("Struct has more than {max_fields} fields ({count} found)."),
                "defstruct".to_owned(),
            ));
        }
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

/// Field count when the single argument is a field list; `None` otherwise
/// (mirrors the reference match on a single list argument).
fn field_count(call: &crate::facts::CallFact) -> Option<usize> {
    let mut items = call.args.iter().filter(|arg| arg.code);
    let only = items.next()?;
    if items.next().is_some() {
        return None;
    }
    match only.kind {
        crate::facts::NodeKind::List => {
            if only.kids.len() == 1 && only.kids[0].kind == crate::facts::NodeKind::Keywords {
                Some(only.kids[0].pairs as usize)
            } else {
                Some(only.kids.len())
            }
        }
        crate::facts::NodeKind::Keywords => Some(
            only.kids
                .iter()
                .filter(|kid| kid.kind == crate::facts::NodeKind::Pair)
                .count(),
        ),
        _ => None,
    }
}

/// 1-based `(line, column)` with character-based columns.
fn line_column(lines: &[&str], starts: &[usize], byte: usize) -> (usize, usize) {
    let row = starts
        .partition_point(|start| *start <= byte)
        .saturating_sub(1);
    let text = lines.get(row).copied().unwrap_or("");
    let start = starts.get(row).copied().unwrap_or(0);
    let column = text
        .get(..byte.saturating_sub(start).min(text.len()))
        .map_or(1, |prefix| prefix.chars().count() + 1);
    (row + 1, column)
}

/// Byte offsets where each 1-based line starts (`starts[0]` is zero).
fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    starts.extend(source.match_indices('\n').map(|(byte, _)| byte + 1));
    starts
}

/// Source slice for fact spans; `None` on invalid boundaries.
fn slice(source: &str, start: u32, end: u32) -> Option<&str> {
    source.get(start as usize..end as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_struct_is_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("defstruct [:a, :b]\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn multiline_large_struct_reports() {
        use std::fmt::Write as _;
        let mut fields = String::from("defmodule MyApp.LargeStruct do\n  defstruct ");
        for n in 1..=32 {
            if n > 1 {
                fields.push_str(",\n            ");
            }
            let _ = write!(fields, "field_{n}: \"field_{n}\"");
        }
        fields.push_str("\nend\n");
        let findings = check_prepared(&crate::batch::Prepared::lazy(&fields), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, Some(3));
        assert_eq!(
            findings[0].message,
            "Struct has more than 31 fields (32 found)."
        );
    }
    #[test]
    fn max_fields_param_is_honored() {
        use std::fmt::Write as _;
        let mut fields =
            String::from("defmodule MyApp.SmallStruct do\n  @moduledoc false\n\n  defstruct ");
        for n in 1..=10 {
            if n > 1 {
                fields.push_str(",\n            ");
            }
            let _ = write!(fields, "field_{n}: \"field_{n}\"");
        }
        fields.push_str("\nend\n");
        let params: BTreeMap<String, String> = [("max_fields".to_owned(), "8".to_owned())]
            .into_iter()
            .collect();
        let findings = check_prepared(&crate::batch::Prepared::lazy(&fields), &params);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Struct has more than 8 fields (10 found)."
        );
    }

    #[test]
    fn broken_source_stays_clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("def foo( do\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
}
