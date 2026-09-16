use crate::Finding;

/// `EX5023`: the result of a `String` function call has to be used.
///
/// A call is unused unless its value flows somewhere: call arguments,
/// assignments, pipe inputs, conditions, or tail position of its `def`
/// (`do`/`rescue`/`catch`). This mirrors
/// `Credo.Check.Warning.UnusedFunctionReturnHelper` over the parse tree.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    crate::warning::unused_return::check_module(prepared, "String")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = String.upcase(a)\n")).is_empty());
    }
    #[test]
    fn reports() {
        let src = "defmodule M do\n  def f(a) do\n    String.upcase(a)\n    a\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
    }
    #[test]
    fn struct_and_map_values_are_used() {
        let src =
            "defmodule M do\n  def f(x) do\n    %URI{path: String.trim(x)}\n    :ok\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
        let src = "defmodule M do\n  def f(x) do\n    %{a: String.trim(x)}\n    :ok\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_unused_statement_in_def() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    x = parameter1 + parameter2\n\n    String.split(parameter1)\n\n    parameter1\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (5, Some(5)));
    }
    #[test]
    fn cond_conditions_are_used() {
        // Native reference: 0 issues; `->` clauses are transparent and
        // `cond` verifies its arguments like any generic call.
        let src = "defmodule M do\n  def normalize(type) do\n    cond do\n      String.starts_with?(type, [\"a\"]) -> :integer\n      true -> :string\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn dot_function_position_is_used() {
        // Native reference: 0 issues; the `.` operator call verifies its
        // function argument, so `String.trim(x).()` uses the result.
        let src = "defmodule M do\n  def f(x) do\n    String.trim(x).()\n    :ok\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn discarded_cond_still_uses_conditions_and_single_bodies() {
        // Native reference: 0 issues for both; `cond` verifies everything
        // reaching it while multi-statement non-tail bodies falsify inside.
        let conds = "defmodule M do\n  def f(x) do\n    cond do\n      String.trim(x) -> :a\n      true -> :b\n    end\n    :ok\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(conds)).is_empty());
        let body = "defmodule M do\n  def f(x) do\n    cond do\n      x -> String.trim(x)\n    end\n    :ok\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(body)).is_empty());
    }
    #[test]
    fn cond_multi_body_nontail_stays_unused() {
        // Native reference: 1 issue; the inner block falsifies before cond.
        let src = "defmodule M do\n  def f(x) do\n    cond do\n      x ->\n        String.trim(x)\n        :done\n    end\n    :ok\n  end\nend\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 1);
    }
}
