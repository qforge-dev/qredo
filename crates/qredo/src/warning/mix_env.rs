use crate::{Finding, Trigger};

/// `EX5010`: no `Mix.env` calls in application code.
///
/// Only calls inside `def`/`defp`/`defmacro` bodies are reported; module
/// attributes and top-level code are out of scope. Filename-based
/// exclusions (`.exs` files, `excluded_paths`) live at pipeline level:
/// without a filename the kernel reports every in-function call.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let facts = prepared.facts();
    // One query over shared facts replaces the old two-pass walk: every
    // `Mix.env` remote call inside any `def`-family range is reported.
    let mut findings = Vec::new();
    for call in &facts.calls {
        let Some(crate::facts::HeadFact::Remote {
            mod_start,
            mod_end,
            mod_kind,
            fun_start,
            fun_end,
        }) = &call.head
        else {
            continue;
        };
        if *mod_kind != crate::facts::ModKind::Alias {
            continue;
        }
        let (Some(module), Some(fun)) = (
            slice(source, *mod_start, *mod_end),
            slice(source, *fun_start, *fun_end),
        ) else {
            continue;
        };
        if module != "Mix" || fun != "env" {
            continue;
        }
        if !facts
            .def_ranges
            .iter()
            .any(|(start, end)| *start <= call.start && call.start < *end)
        {
            continue;
        }
        let (line, column) = line_col(source, *mod_start as usize);
        findings.push(Finding {
            line,
            column: Some(column),
            message: "There should be no calls to `Mix.env` in application code.".to_owned(),
            trigger: Trigger::Text("Mix.env".to_owned()),
            severity: None,
        });
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

/// 1-based `(line, column)` with the column counted in characters.
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

    /// Single-source entry: the same lazy parse the old `check` built.
    fn check(src: &str) -> Vec<Finding> {
        check_prepared(&crate::batch::Prepared::lazy(src))
    }

    #[test]
    fn clean() {
        assert!(check("x = 1\n").is_empty());
    }
    #[test]
    fn reports() {
        let src = "defmodule M do\n  def f do\n    Mix.env()\n  end\nend\n";
        let findings = check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
    }
    #[test]
    fn ignores_module_attributes() {
        let src = "defmodule CredoSampleModule do\n  @myvar Mix.env() == :test\n\n  def test do\n    @myvar\n  end\nend\n";
        assert!(check(src).is_empty());
    }
    #[test]
    fn reports_capture_and_each_occurrence() {
        let src = "defmodule M do\n  def f(a, b) do\n    Mix.env(); Mix.env()\n  end\nend\n";
        let findings = check(src);
        assert_eq!(findings.len(), 2);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
        assert_eq!((findings[1].line, findings[1].column), (3, Some(16)));
        let src = "defmodule M do\n  def f do\n    &Mix.env/0\n  end\nend\n";
        let findings = check(src);
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(6)));
    }

    #[test]
    fn broken_source_stays_clean() {
        assert!(check("def foo( do\n").is_empty());
    }
}
