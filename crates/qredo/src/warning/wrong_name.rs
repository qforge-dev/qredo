use crate::Finding;

/// `EX5030`: `use XCase` outside `*_test.exs` files.
///
/// Without a filename the kernel is conservative (no findings); filename
/// gating and the real scan live behind `check_source` (see `crate::filename`).
/// `quote` subtrees are skipped, mirroring upstream.
pub(crate) fn check(_source: &str) -> Vec<Finding> {
    Vec::new()
}

/// All `use <...Case>` calls outside `quote` blocks, over an
/// already-parsed tree: the pipeline shares its prepare-phase parse
/// instead of reparsing per file.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let facts = prepared.facts();
    let mut findings = Vec::new();
    for call in &facts.calls {
        if facts
            .quote_ranges
            .iter()
            .any(|(start, end)| *start <= call.start && call.end <= *end)
        {
            continue;
        }
        let Some(crate::facts::HeadFact::Plain { start, end }) = &call.head else {
            continue;
        };
        if slice(source, *start, *end) != Some("use") {
            continue;
        }
        let Some(alias) = call.args.iter().find(|arg| arg.is_alias) else {
            continue;
        };
        let Some(module) = slice(source, alias.start, alias.end) else {
            continue;
        };
        if module
            .rsplit('.')
            .next()
            .is_some_and(|last| last.ends_with("Case"))
        {
            let (line, column) = line_col(source, alias.start as usize);
            findings.push(Finding::with_trigger(
                line,
                Some(column),
                format!("Test files that `use {module}` should end with `_test.exs`."),
                module,
            ));
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// 1-based `(line, column)` with character-based columns.
fn line_col(source: &str, byte: usize) -> (usize, usize) {
    let before = source.get(..byte).unwrap_or("");
    let line = before.bytes().filter(|&byte| byte == b'\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

/// Source slice for fact spans; `None` on invalid boundaries.
fn slice(source: &str, start: u32, end: u32) -> Option<&str> {
    source.get(start as usize..end as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Single-source entry: the same lazy parse the old `check_source` built.
    fn check_source(source: &str) -> Vec<Finding> {
        check_prepared(&crate::batch::Prepared::lazy(source))
    }

    #[test]
    fn conservative_empty() {
        assert!(check("x = 1\n").is_empty());
    }
    #[test]
    fn reports_case_use() {
        let findings = check_source("defmodule M do\n  use ExUnit.Case\nend\n");
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (2, Some(7)));
    }
    #[test]
    fn skips_quote_blocks() {
        let src = "defmodule M do\n  quote do\n    use ExUnit.Case\n  end\nend\n";
        assert!(check_source(src).is_empty());
    }
    #[test]
    fn ignores_non_case_use() {
        assert!(check_source("defmodule M do\n  use Foo.Bar\nend\n").is_empty());
    }

    #[test]
    fn broken_source_stays_clean() {
        assert!(check_source("def foo( do\n").is_empty());
    }
}
