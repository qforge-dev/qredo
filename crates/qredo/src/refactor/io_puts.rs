use crate::Finding;

/// `EX4011`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        // Byte offsets from `find` on an ASCII needle stay on char boundaries.
        let mut search = 0_usize;
        while search <= line.len() {
            let Some(rel) = line[search..].find("IO.puts") else {
                break;
            };
            let pos = search + rel;
            let prev_ok = line[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| !is_name_char(c) && c != '.' && c != ':' && c != '@');
            let next_ok = line[pos + "IO.puts".len()..]
                .chars()
                .next()
                .is_none_or(|c| !is_name_char(c) && c != '?' && c != '!');
            if prev_ok && next_ok {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(line[..pos].chars().count() + 1),
                    "There should be no calls to `IO.puts/1`.",
                    "IO.puts".to_owned(),
                ));
            }
            search = pos + "IO.puts".len();
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
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
            check_prepared(&crate::batch::Prepared::lazy("IO.puts(x)\n")).len(),
            1
        );
    }
    #[test]
    fn reports_two_on_same_line() {
        let found = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    IO.puts(parameter1); IO.puts(parameter2)\n  end\nend\n",
        ));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].column, Some(5));
        assert_eq!(found[1].column, Some(26));
        assert!(
            found
                .iter()
                .all(|f| f.message == "There should be no calls to `IO.puts/1`.")
        );
    }
}
