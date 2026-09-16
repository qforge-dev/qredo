use crate::Finding;

/// `EX5015`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let facts = prepared.facts();
    let mut findings = Vec::new();
    let lines: Vec<&str> = source.split('\n').collect();
    let starts = line_starts(source);
    for call in &facts.calls {
        if let Some((message, trigger)) = forbidden_call(call, source) {
            let (line, column) = line_column(&lines, &starts, call.start as usize);
            findings.push(Finding::with_trigger(line, Some(column), message, trigger));
        }
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

/// `(message, trigger)` when `call` is a forbidden command-spawning call.
fn forbidden_call(call: &crate::facts::CallFact, source: &str) -> Option<(String, String)> {
    let Some(crate::facts::HeadFact::Remote {
        mod_start,
        mod_end,
        mod_kind,
        fun_start,
        fun_end,
    }) = &call.head
    else {
        return None;
    };
    if *mod_kind != crate::facts::ModKind::Atom {
        return None;
    }
    let (Some(left), Some(right)) = (
        slice(source, *mod_start, *mod_end),
        slice(source, *fun_start, *fun_end),
    ) else {
        return None;
    };
    let args: Vec<&crate::facts::ArgFact> = call.args.iter().filter(|arg| arg.code).collect();
    match (left, right) {
        (":os", "cmd") => match args.len() {
            1 => Some((
                "Prefer System.cmd/2,3 over :os.cmd/1 to prevent command injection.".to_owned(),
                ":os.cmd".to_owned(),
            )),
            2 => Some((
                "Prefer System.cmd/2,3 over :os.cmd/2 to prevent command injection.".to_owned(),
                ":os.cmd".to_owned(),
            )),
            _ => None,
        },
        (":erlang", "open_port") => {
            if args.len() == 2 && is_spawn_tuple(args[0], source) {
                Some((
                    "Prefer :erlang.open_port/2 with `:spawn_executable` over :erlang.open_port/2 with `:spawn` to prevent command injection."
                        .to_owned(),
                    ":erlang.open_port".to_owned(),
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Whether the argument is a two-element `{:spawn, ...}` tuple literal.
fn is_spawn_tuple(arg: &crate::facts::ArgFact, source: &str) -> bool {
    if arg.kind != crate::facts::NodeKind::Tuple || arg.kids.len() != 2 {
        return false;
    }
    let first = &arg.kids[0];
    first.kind == crate::facts::NodeKind::Atom
        && slice(source, first.start, first.end) == Some(":spawn")
}

/// 1-based `(line, column)` with character-based columns.
fn line_column(lines: &[&str], starts: &[usize], byte: usize) -> (usize, usize) {
    let row = starts
        .partition_point(|start| *start <= byte)
        .saturating_sub(1);
    let text = lines.get(row).copied().unwrap_or("");
    let start = starts.get(row).copied().unwrap_or(0);
    let column = text
        .get(..byte.saturating_sub(start).min(text.len()))
        .map_or(1, |prefix| prefix.chars().count() + 1);
    (row + 1, column)
}

/// Byte offsets where each 1-based line starts (`starts[0]` is zero).
fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    starts.extend(source.match_indices('\n').map(|(byte, _)| byte + 1));
    starts
}

/// Source slice for fact spans; `None` on invalid boundaries.
fn slice(source: &str, start: u32, end: u32) -> Option<&str> {
    source.get(start as usize..end as usize)
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
            check_prepared(&crate::batch::Prepared::lazy("System.cmd(\"ls\", [])\n")).len(),
            0
        );
    }
    #[test]
    fn safe_apis_are_clean() {
        let source = "defmodule CredoSampleModule do\n  def run_with_system_cmd2(executable, arguments) do\n    System.cmd(executable, arguments)\n  end\n\n  def run_with_erlang_open_port(executable, arguments) do\n    :erlang.open_port({:spawn_executable, executable}, args: arguments)\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(source)).is_empty());
    }
    #[test]
    fn reports_spawn_port() {
        let source = "defmodule CredoSampleModule do\n  def run_with_erlang_open_port(command_line) do\n    :erlang.open_port({:spawn, command_line}, [])\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(source));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
        assert_eq!(findings[0].column, Some(5));
    }

    #[test]
    fn broken_source_stays_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("def foo( do\n")).is_empty());
    }
}
