use crate::Finding;

/// `EX5008`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let masked = prepared.masked();
    let mut findings = Vec::new();
    scan_named(masked, "System.cmd", &mut |start, _| {
        push_if_leaky(masked, source, start, "System.cmd", &mut findings);
    });
    scan_named(masked, ":erlang.open_port", &mut |start, _| {
        push_if_leaky(masked, source, start, ":erlang.open_port", &mut findings);
    });
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

/// Call `fun` for every call-shaped occurrence of `name` in the masked text.
fn scan_named(masked: &str, name: &str, fun: &mut impl FnMut(usize, usize)) {
    let mut search = 0_usize;
    while search < masked.len() {
        let Some(rel) = masked[search..].find(name) else {
            break;
        };
        let base = search + rel;
        search = base + 1;
        if !before_ok(masked, base, name) || !after_ok(masked, base + name.len()) {
            continue;
        }
        fun(base, base + name.len());
    }
}

/// The char before the call must not continue another name or remote path.
fn before_ok(masked: &str, base: usize, _name: &str) -> bool {
    if base == 0 {
        return true;
    }
    !masked[..base]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ':')
}

/// The call name must not continue into a longer name.
fn after_ok(masked: &str, end: usize) -> bool {
    if end >= masked.len() {
        return true;
    }
    !masked[end..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

/// Push an issue when the call leaks the environment.
fn push_if_leaky(
    masked: &str,
    source: &str,
    start: usize,
    trigger: &str,
    findings: &mut Vec<Finding>,
) {
    let end = start + trigger.len();
    let Some(args) = call_args(masked, end) else {
        return;
    };
    let leaky = match trigger {
        "System.cmd" => args.len() == 2 || (args.len() == 3 && !has_top_env(masked, args[2])),
        _ => args.len() == 2 && !has_top_env(masked, args[1]),
    };
    if !leaky {
        return;
    }
    // Third/non-list options never leak per the call shapes above.
    if trigger == "System.cmd" && args.len() == 3 && !is_list_arg(masked, args[2]) {
        return;
    }
    if trigger != "System.cmd" && !is_list_arg(masked, args[1]) {
        return;
    }
    let (line, column) = line_col(source, start);
    findings.push(Finding::with_trigger(
        line,
        Some(column),
        format!("When using {trigger}, clear or overwrite sensitive environment variables."),
        trigger.to_owned(),
    ));
}

/// Byte ranges of the call arguments after the name ending at `end`.
fn call_args(masked: &str, end: usize) -> Option<Vec<(usize, usize)>> {
    let after = masked[end..].trim_start();
    if after.starts_with('(') {
        let open = end + (masked[end..].len() - after.len());
        return paren_args(masked, open);
    }
    None
}

/// Byte ranges of top-level comma-separated arguments in `(...)`.
fn paren_args(masked: &str, open: usize) -> Option<Vec<(usize, usize)>> {
    let mut args = Vec::new();
    let mut depth = 0_usize;
    let mut arg_start = 0_usize;
    for (rel, chr) in masked[open..].char_indices() {
        let idx = open + rel;
        match chr {
            '(' | '[' | '{' => {
                if depth == 0 {
                    arg_start = idx + 1;
                }
                depth += 1;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    args.push((arg_start, idx));
                    return Some(args);
                }
            }
            ',' if depth == 1 => {
                args.push((arg_start, idx));
                arg_start = idx + 1;
            }
            _ => {}
        }
    }
    None
}

/// Whether the argument range holds a list literal or bare trailing
/// keywords (both parse as lists, the only shapes taking `:env`).
fn is_list_arg(masked: &str, arg: (usize, usize)) -> bool {
    let text = masked[arg.0..arg.1].trim();
    if text.starts_with('[') {
        return true;
    }
    !text.is_empty() && split_top(text).iter().all(|item| is_keyword_item(item))
}

/// Split top-level comma-separated items.
fn split_top(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (idx, chr) in text.char_indices() {
        match chr {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&text[start..idx]);
                start = idx + chr.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// Whether the item looks like a keyword entry (`name: value`).
fn is_keyword_item(item: &str) -> bool {
    let item = item.trim_start();
    let len = item
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
        .map(char::len_utf8)
        .sum::<usize>();
    if len == 0 || !item[len..].starts_with(':') {
        return false;
    }
    !item[..len]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
}

/// Whether the argument range sets a top-level `:env` option.
fn has_top_env(masked: &str, arg: (usize, usize)) -> bool {
    let text = &masked[arg.0..arg.1];
    let mut depth = 0_usize;
    let mut idx = 0_usize;
    while idx < text.len() {
        if text[idx..].starts_with("env:") && depth == 0 && env_before_ok(text, idx) {
            return true;
        }
        let chr = text[idx..].chars().next().unwrap_or(' ');
        match chr {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        idx += chr.len_utf8();
    }
    false
}

/// The char before `env:` must not continue another name (`my_env:`).
fn env_before_ok(text: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    !text[..idx]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// 1-based `(line, column-in-chars)` of the byte offset (must be a boundary).
fn line_col(source: &str, byte_idx: usize) -> (usize, usize) {
    let upto = &source[..byte_idx];
    let line = upto.matches('\n').count() + 1;
    let column = upto
        .rsplit('\n')
        .next()
        .map_or(1, |last| last.chars().count() + 1);
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("x = 1\n")).is_empty());
    }
    #[test]
    fn system_cmd_with_env_is_clean() {
        let src = "defmodule M do\n  def f(e, a) do\n    System.cmd(e, a, env: %{\"K\" => nil})\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_system_cmd_without_env() {
        let src = "defmodule CredoSampleModule do\n  def run_with_system_cmd2(executable, arguments) do\n    System.cmd(executable, arguments)\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
        assert_eq!(out[0].column, Some(5));
    }
}
