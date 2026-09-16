use crate::{Finding, Trigger};

/// `EX5001`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        if !line.trim_start().starts_with('@') {
            continue;
        }
        // `@name -> ...` reads the attribute in a `case`/`cond` pattern;
        // only definitions (`@name value`) can carry a forbidden call.
        if is_pattern_clause(line) {
            continue;
        }
        if let Some(hit) = first_forbidden(line) {
            let attr = attribute_name(line, hit.app);
            findings.push(Finding {
                line: idx + 1,
                column: Some(char_col(line, hit.app)),
                message: format!(
                    "Module attribute @{} makes use of unsafe Application configuration call {}",
                    attr, hit.call
                ),
                trigger: Trigger::Text(hit.trigger),
                severity: None,
            });
        }
    }
    findings
}

/// A forbidden `Application` call on one line: byte index of `Application`,
/// the call label for the message and the issue trigger.
struct Hit {
    app: usize,
    call: String,
    trigger: String,
}

/// First forbidden `Application.<fun>` call on the line, if any.
fn first_forbidden(line: &str) -> Option<Hit> {
    let mut search = 0_usize;
    while let Some(rel) = line[search..].find("Application") {
        let base = search + rel;
        if before_ok(line, base)
            && let Some(hit) = match_fun(line, base)
        {
            return Some(hit);
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
    !(prev.is_alphanumeric() || prev == '_' || prev == '.' || prev == ':')
}

fn match_fun(line: &str, base: usize) -> Option<Hit> {
    let after_dot = line[base + "Application".len()..].strip_prefix('.')?;
    // `fetch_env!` must win over the `fetch_env` prefix.
    for fun in ["fetch_env!", "fetch_env", "get_all_env", "get_env"] {
        if let Some(rest) = after_dot.strip_prefix(fun)
            && fun_end_ok(rest, fun)
        {
            let tail = rest.trim_start();
            if tail.starts_with('(') {
                return hit_for(base, fun, rest);
            }
            // A bare `Application.fun` still quotes as a (possibly 0-arg)
            // call upstream (`no_parens: true`).
            return hit_for_bare(base, fun, tail);
        }
    }
    None
}

/// True when `fun` ends at a name boundary (`!` only extends `fetch_env`).
/// A bare name at end of line is a `no_parens` call upstream.
fn fun_end_ok(rest: &str, fun: &str) -> bool {
    match rest.chars().next() {
        Some(next) => {
            if next.is_alphanumeric() || next == '_' || next == '?' {
                return false;
            }
            if next == '!' && !fun.ends_with('!') {
                return false;
            }
            true
        }
        None => true,
    }
}

fn hit_for(base: usize, fun: &str, rest: &str) -> Option<Hit> {
    match fun {
        "fetch_env" => Some(Hit {
            app: base,
            call: "Application.fetch_env/2".to_owned(),
            trigger: "Application.fetch_env".to_owned(),
        }),
        "fetch_env!" => Some(Hit {
            app: base,
            call: "Application.fetch_env!/2".to_owned(),
            trigger: "Application.fetch_env".to_owned(),
        }),
        "get_all_env" => Some(Hit {
            app: base,
            call: "Application.get_all_env/1".to_owned(),
            trigger: "Application.get_all_env".to_owned(),
        }),
        "get_env" => {
            let arity = arg_count(rest)?;
            Some(Hit {
                app: base,
                call: format!("Application.get_env/{arity}"),
                trigger: "Application.get_env".to_owned(),
            })
        }
        _ => None,
    }
}

/// Call label for a no-parentheses call (`Application.get_env :a, :b`).
fn hit_for_bare(base: usize, fun: &str, tail: &str) -> Option<Hit> {
    let (call, trigger) = match fun {
        "fetch_env" => (
            "Application.fetch_env/2".to_owned(),
            "Application.fetch_env",
        ),
        "fetch_env!" => (
            "Application.fetch_env!/2".to_owned(),
            "Application.fetch_env",
        ),
        "get_all_env" => (
            "Application.get_all_env/1".to_owned(),
            "Application.get_all_env",
        ),
        "get_env" => (
            format!("Application.get_env/{}", bare_arity(tail)),
            "Application.get_env",
        ),
        _ => return None,
    };
    Some(Hit {
        app: base,
        call,
        trigger: trigger.to_owned(),
    })
}

/// Argument count of a no-parentheses call tail (top-level comma segments).
fn bare_arity(tail: &str) -> usize {
    let mut depth = 0_usize;
    let mut commas = 0_usize;
    let mut len = 0_usize;
    for char in tail.chars() {
        match char {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => commas += 1,
            ';' if depth == 0 => break,
            c if depth == 0 && !c.is_whitespace() => len += 1,
            _ => {}
        }
    }
    if len == 0 { 0 } else { commas + 1 }
}

/// Top-level argument count of the call starting at `rest` (at its `(`).
fn arg_count(rest: &str) -> Option<usize> {
    let open = rest.find('(')?;
    let mut depth = 0_usize;
    let mut commas = 0_usize;
    let mut len = 0_usize;
    for char in rest[open..].chars() {
        match char {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(if len == 0 { 0 } else { commas + 1 });
                }
            }
            ',' if depth == 1 => commas += 1,
            c if depth == 1 && !c.is_whitespace() => len += 1,
            _ => {}
        }
    }
    None
}

/// Attribute name of the nearest `@name` before the call.
/// True when the `@`-led line is a pattern clause (`@name -> ...`)
/// rather than an attribute definition.
fn is_pattern_clause(line: &str) -> bool {
    let trimmed = line.trim_start();
    let after_at = &trimmed[1..];
    let name_len = after_at
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
        .map(char::len_utf8)
        .sum::<usize>();
    if name_len == 0 {
        return false;
    }
    match after_at[name_len..].trim_start().strip_prefix("->") {
        Some(_) => true,
        None => after_at[name_len..].trim_start().starts_with("when "),
    }
}

fn attribute_name(line: &str, app: usize) -> String {
    let before = &line[..app];
    let at = before.rfind('@').unwrap_or(0);
    before[at + 1..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '?' || *c == '!')
        .collect()
}

/// 1-based column for a byte index at an ASCII token.
fn char_col(line: &str, base: usize) -> usize {
    line[..base].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Trigger;
    #[test]
    fn clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("@foo 1\n")).is_empty());
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "@foo Application.get_env(:a, :b)\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn case_clauses_matching_attributes_are_not_definitions() {
        // Native reference: 0 issues; `@missing -> ...` reads the
        // attribute in a pattern, while `@foo Application...` defines it
        // with a forbidden call (1 issue).
        let src = "defmodule M do\n  @missing :unset\n  def f(x) do\n    case x do\n      @missing -> Application.get_env(:app, :key)\n      v -> v\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn ignores_compile_env() {
        let src = "defmodule CredoSampleModule do\n  @config_1 Application.compile_env!(:my_app, :key)\n  @config_2 Application.compile_env(:my_app, :key, :default)\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_bare_get_env_without_parens() {
        let src = "defmodule T do\n  @c Application.get_env :a, :b\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Module attribute @c makes use of unsafe Application configuration call Application.get_env/2"
        );
    }
    #[test]
    fn reports_bare_call_without_arguments() {
        let src = "defmodule T do\n  @c Application.get_env\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Module attribute @c makes use of unsafe Application configuration call Application.get_env/0"
        );
    }
    #[test]
    fn reports_fetch_env_with_call_message() {
        let src = "defmodule CredoSampleModule do\n  @config_1 Application.fetch_env(:my_app, :key)\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src)),
            vec![Finding {
                line: 2,
                column: Some(13),
                message: "Module attribute @config_1 makes use of unsafe Application configuration call Application.fetch_env/2"
                    .to_owned(),
                trigger: Trigger::Text("Application.fetch_env".to_owned()),
                        severity: None,
}]
        );
    }
}
