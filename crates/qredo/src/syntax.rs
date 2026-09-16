//! Tree-sitter syntax validation for Elixir source.
//!
//! Pinned grammar `tree-sitter-elixir 0.3.5` with runtime `tree-sitter 0.27`.
//! The CST is not Elixir's quoted AST. Recovery trees with `ERROR`/`MISSING`
//! nodes must not turn invalid source into clean lint. Grammar acceptance
//! mismatches are explicit unsupported inputs.

/// True when the grammar accepts `content` without error nodes.
#[must_use]
pub fn is_valid(content: &str) -> bool {
    validate(content) == SyntaxStatus::Valid
}

/// Parse status validated against the pinned grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxStatus {
    Valid,
    Invalid,
}

/// Validate `content` against the pinned grammar.
#[must_use]
pub fn validate(content: &str) -> SyntaxStatus {
    let Some(tree) = crate::ts_parser::parse(content) else {
        return SyntaxStatus::Invalid;
    };
    if tree.root_node().has_error() {
        SyntaxStatus::Invalid
    } else {
        SyntaxStatus::Valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_empty_module() {
        assert!(is_valid("defmodule M do\nend\n"));
    }

    #[test]
    fn rejects_unclosed_def() {
        assert!(!is_valid("def foo( do\n"));
    }

    #[test]
    fn validate_marks_status() {
        assert_eq!(validate("x = 1\n"), SyntaxStatus::Valid);
        assert_eq!(validate("def foo( do\n"), SyntaxStatus::Invalid);
    }

    #[test]
    fn empty_source_is_valid() {
        assert!(is_valid(""));
    }

    #[test]
    fn bare_cr_after_code_is_invalid() {
        // Matches the `line-primitive` boundary: lone CR is not valid Elixir.
        assert!(!is_valid("x = 1\r"));
    }
}
