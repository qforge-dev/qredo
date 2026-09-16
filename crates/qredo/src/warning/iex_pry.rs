use crate::Finding;

/// `EX5005`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        let mut search = 0_usize;
        while search < line.len() {
            let Some(rel) = line[search..].find("IEx.pry") else {
                break;
            };
            let base = search + rel;
            if before_ok(line, base) && after_ok(line, base + "IEx.pry".len()) {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(col_of(line, base)),
                    "There should be no calls to `IEx.pry/0`.",
                    "IEx.pry".to_owned(),
                ));
            }
            search = base + "IEx.pry".len();
        }
    }
    findings
}

/// Column (1-based, characters) of the byte offset (which must be a boundary).
fn col_of(line: &str, byte_pos: usize) -> usize {
    line[..byte_pos].chars().count() + 1
}

/// The char before `IEx` must not continue an alias or attribute path.
fn before_ok(line: &str, base: usize) -> bool {
    if base == 0 {
        return true;
    }
    !line[..base]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// `pry` must not continue into a longer name (`pry?`, `pry_all`, ...).
fn after_ok(line: &str, end: usize) -> bool {
    if end >= line.len() {
        return true;
    }
    !line[end..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = 1\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("IEx.pry()\n")).len(),
            1
        );
    }
    #[test]
    fn reports_two_on_same_line() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    IEx.pry(); IEx.pry()\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].line, 3);
        assert_eq!(out[0].column, Some(5));
        assert_eq!(out[1].line, 3);
        assert_eq!(out[1].column, Some(16));
    }
}
