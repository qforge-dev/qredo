use crate::Finding;

/// `EX5024`: the result of a `Tuple` function call has to be used.
///
/// A call is unused unless its value flows somewhere: call arguments,
/// assignments, pipe inputs, conditions, or tail position of its `def`
/// (`do`/`rescue`/`catch`). This mirrors
/// `Credo.Check.Warning.UnusedFunctionReturnHelper` over the parse tree.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    crate::warning::unused_return::check_module(prepared, "Tuple")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = elem(a, 0)\n")).is_empty());
    }
    #[test]
    fn reports() {
        let src = "defmodule M do\n  def f(a) do\n    Tuple.to_list(a)\n    a\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
    }
    #[test]
    fn reports_unused_statement_in_def() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    x = parameter1 + parameter2\n\n    Tuple.delete_at(parameter1, x)\n\n    parameter1\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (5, Some(5)));
    }
}
