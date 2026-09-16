use crate::Finding;

/// `EX3015`: do not pipe into anonymous functions.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let mut search = 0_usize;
        while let Some(pos) = line[search..].find("|>") {
            let base = search + pos;
            let rest = line[base + 2..].trim_start();
            if rest.starts_with("(fn ") || rest.starts_with("(fn(") {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(base + 1),
                    "Avoid piping into anonymous function calls.",
                    "|>",
                ));
            }
            search = base + 2;
            if search >= line.len() {
                break;
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_is_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x |> foo()\n")).is_empty());
    }
    #[test]
    fn bare_fn_value_is_clean() {
        // Upstream only flags `|> (fn ... end).()` calls, not a bare `fn` value.
        assert!(check_prepared(&crate::batch::Prepared::lazy("x |> fn y -> y end\n")).is_empty());
    }

    #[test]
    fn reports_called_anon_literal() {
        let src =
            "defmodule M do\n  def f(v) do\n    v\n    |> (fn x -> x * 2 end).()\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 4);
        assert_eq!(out[0].column, Some(5));
    }
}
