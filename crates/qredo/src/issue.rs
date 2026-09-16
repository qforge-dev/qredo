//! Full Credo issue shape for the first vertical slice.
//!
//! Mirrors `Credo.Issue` fields needed by `TrailingBlankLine`: check,
//! category, priority, severity, message, filename, line, column, exit
//! status, trigger sentinel and scope. Scope priority effects beyond the
//! base value remain pending without syntax facts.

/// Complete pipeline issue before output filtering.
///
/// Native cache values serialize this full shape; it is the native
/// diagnostic shape, not the real-Credo JSON shape, so native and
/// real-Credo cache namespaces must stay separated by tool identity.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Issue {
    /// Check module, e.g. `Credo.Check.Readability.TrailingBlankLine`.
    pub check: String,
    pub category: Category,
    /// Integer priority after base + scope effects.
    pub priority: i32,
    /// Severity ratio (`Severity.compute`); `1.0` default.
    pub severity: f64,
    pub message: String,
    pub filename: String,
    pub line_no: Option<usize>,
    pub column: Option<usize>,
    pub exit_status: i32,
    pub trigger: IssueTrigger,
    /// Enclosing scope name, e.g. `Foo.Bar.baz`.
    pub scope: Option<String>,
}

/// Credo category with its default exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Category {
    Consistency,
    Design,
    Readability,
    Refactor,
    Warning,
}

impl Category {
    #[must_use]
    pub fn default_exit_status(self) -> i32 {
        match self {
            Self::Consistency => 1,
            Self::Design => 2,
            Self::Readability => 4,
            Self::Refactor => 8,
            Self::Warning => 16,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Consistency => "consistency",
            Self::Design => "design",
            Self::Readability => "readability",
            Self::Refactor => "refactor",
            Self::Warning => "warning",
        }
    }
}

/// Explicit trigger sentinel preserved separately from text triggers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum IssueTrigger {
    NoTrigger,
    Text(String),
}

/// Base priorities from `Credo.Priority`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasePriority {
    Higher,
    High,
    Normal,
    Low,
    Ignore,
}

impl BasePriority {
    #[must_use]
    pub fn to_integer(self) -> i32 {
        match self {
            Self::Higher => 20,
            Self::High => 10,
            Self::Normal => 1,
            Self::Low => -10,
            Self::Ignore => -100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readability_default_exit_status_is_four() {
        assert_eq!(Category::Readability.default_exit_status(), 4);
    }

    #[test]
    fn low_base_priority_is_negative_ten() {
        assert_eq!(BasePriority::Low.to_integer(), -10);
    }
}
