use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX5026`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let allow_captures = helpers::param_bool(params, "allow_captures", false);
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with('@') {
            continue;
        }
        let mut search = 0_usize;
        while let Some(base) = find_dbg(line, search) {
            let hit = classify(line, &lines, idx, base, allow_captures);
            if hit.report {
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(hit.column),
                    "There should be no calls to `dbg/1`.",
                    hit.trigger,
                ));
            }
            search = base + "dbg".len();
        }
    }
    findings
}

/// A classified `dbg` occurrence: whether to report, at which column, trigger.
struct Hit {
    report: bool,
    column: usize,
    trigger: String,
}

/// Byte index of a `dbg` word at or after `from`.
fn find_dbg(line: &str, from: usize) -> Option<usize> {
    let mut search = from;
    while let Some(rel) = line[search..].find("dbg") {
        let base = search + rel;
        if before_ok(line, base) && after_ok(line, base + "dbg".len()) {
            return Some(base);
        }
        search = base + 1;
    }
    None
}

fn before_ok(line: &str, base: usize) -> bool {
    if base == 0 {
        return true;
    }
    let prev = line[..base].chars().next_back().unwrap_or(' ');
    // `.` passes through so `Kernel.dbg` stays visible; other receivers are
    // rejected during classification.
    !(prev.is_alphanumeric() || prev == '_' || prev == ':' || prev == '@')
}

fn after_ok(line: &str, end: usize) -> bool {
    match line[end..].chars().next() {
        Some(next) => !(next.is_alphanumeric() || next == '_' || next == '?' || next == '!'),
        None => true,
    }
}

/// Classify one `dbg` word: qualified calls, captures, paren/no-paren calls.
fn classify(line: &str, lines: &[&str], idx: usize, base: usize, allow_captures: bool) -> Hit {
    if line[..base].ends_with('.') {
        // Remote call: only `Kernel.dbg` / `Elixir.Kernel.dbg` flag upstream.
        return match qualified(line, base) {
            Some(hit) => hit,
            None => quiet_hit(line, base),
        };
    }
    let end = base + "dbg".len();
    let rest = line[end..].trim_start();
    if rest.starts_with('/') {
        return capture_hit(line, base, rest, allow_captures);
    }
    classify_call(line, lines, idx, base, rest)
}

/// 1-based column of the byte index `at` (callers pass ASCII boundaries).
fn col_of(line: &str, at: usize) -> usize {
    line[..at].chars().count() + 1
}

/// A non-reported occurrence at `base` with the plain `dbg` trigger.
fn quiet_hit(line: &str, base: usize) -> Hit {
    Hit {
        report: false,
        column: col_of(line, base),
        trigger: "dbg".to_owned(),
    }
}

/// Classify a `dbg/arity` capture (`&dbg/1` flags unless allowed).
fn capture_hit(line: &str, base: usize, rest: &str, allow_captures: bool) -> Hit {
    let digits: String = rest[1..].chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return quiet_hit(line, base);
    }
    let captured = line[..base].trim_end().ends_with('&');
    Hit {
        report: captured && !allow_captures,
        column: col_of(line, base),
        trigger: "dbg".to_owned(),
    }
}

/// Classify a plain `dbg` word by what follows it on the line.
fn classify_call(line: &str, lines: &[&str], idx: usize, base: usize, rest: &str) -> Hit {
    if rest.starts_with('(') {
        return Hit {
            report: paren_arity(rest) <= 2,
            column: col_of(line, base),
            trigger: "dbg".to_owned(),
        };
    }
    if rest.is_empty() {
        let piped = line[..base].trim_end().ends_with("|>")
            || (idx > 0 && lines[idx - 1].trim_end().ends_with("|>"));
        return Hit {
            report: piped,
            column: col_of(line, base),
            trigger: "dbg".to_owned(),
        };
    }
    let first = rest.chars().next().unwrap_or(' ');
    if "=,.:;)]}>|&+-*/%^!~<>".contains(first) {
        return quiet_hit(line, base);
    }
    Hit {
        report: top_commas(rest) < 2,
        column: col_of(line, base),
        trigger: "dbg".to_owned(),
    }
}

/// `Kernel.dbg` / `Elixir.Kernel.dbg` remote calls.
fn qualified(line: &str, base: usize) -> Option<Hit> {
    let before = &line[..base];
    for (prefix, trigger) in [
        ("Elixir.Kernel.", "Elixir.Kernel.dbg"),
        ("Kernel.", "Kernel.dbg"),
    ] {
        if let Some(start) = before.strip_suffix(prefix).map(str::len)
            && head_ok(line, start)
        {
            let col = line[..start].chars().count() + 1;
            return Some(Hit {
                report: true,
                column: col,
                trigger: trigger.to_owned(),
            });
        }
    }
    None
}

/// True when a qualifier at `start` is not itself part of a longer path.
fn head_ok(line: &str, start: usize) -> bool {
    if start == 0 {
        return true;
    }
    let prev = line[..start].chars().next_back().unwrap_or(' ');
    !(prev.is_alphanumeric() || prev == '_' || prev == '.')
}

/// Argument count of a paren call starting at `(`; unparseable counts as many.
fn paren_arity(rest: &str) -> usize {
    let mut depth = 0_usize;
    let mut commas = 0_usize;
    let mut len = 0_usize;
    for char in rest.chars() {
        match char {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return if len == 0 { 0 } else { commas + 1 };
                }
            }
            ',' if depth == 1 => commas += 1,
            c if depth >= 1 && !c.is_whitespace() => len += 1,
            _ => {}
        }
    }
    usize::MAX
}

/// Top-level commas in no-paren call arguments.
fn top_commas(rest: &str) -> usize {
    let mut depth = 0_usize;
    let mut commas = 0_usize;
    for char in rest.chars() {
        match char {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
            }
            ',' if depth == 0 => commas += 1,
            _ => {}
        }
    }
    commas
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn params() -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = 1\n"), &params()).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("dbg(x)\n"), &params()).len(),
            1
        );
    }
    #[test]
    fn ignores_dbg_variables_and_attributes() {
        let src = "defmodule CredoSampleModule do\n  @dbg \"this should be found\"\n\n  def some_function(parameter1, parameter2) do\n    dbg = \"variables should also not be a problem\"\n    parameter1 + parameter2\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params()).is_empty());
    }
    #[test]
    fn ignores_three_arg_calls() {
        let src = "defmodule CredoSampleModule do\n  def dbg(my_param1, my_param2, myparam3) do\n    my_param\n  end\n\n  def some_fun(param1, param2, param3) do\n    dbg(param1 + param2, param3 * 4, false)\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params()).is_empty());
    }
    #[test]
    fn reports_two_calls_on_one_line() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    dbg(parameter1) + dbg(parameter2)\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params());
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].column, Some(5));
        assert_eq!(findings[1].column, Some(23));
    }
    #[test]
    fn reports_qualified_dbg() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    Kernel.dbg(parameter1 + parameter2)\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params()),
            vec![Finding::with_trigger(
                3,
                Some(5),
                "There should be no calls to `dbg/1`.",
                "Kernel.dbg",
            )]
        );
    }
    #[test]
    fn reports_capture_without_allow() {
        let src = "defmodule CredoSampleModule do\n  def some_function(params) do\n    params\n    |> tap(&dbg/1)\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &params());
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].line, findings[0].column), (4, Some(13)));
    }
    #[test]
    fn allow_captures_suppresses_capture_only() {
        let src = "defmodule CredoSampleModule do\n  def some_function(params) do\n    params\n    |> tap(&dbg/1)\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &params()).len(),
            1
        );
        let mut allowed = BTreeMap::new();
        allowed.insert("allow_captures".to_owned(), "true".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &allowed).is_empty());
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("dbg(x)\n"), &allowed).len(),
            1
        );
    }
}
