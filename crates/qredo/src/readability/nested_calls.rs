use crate::{Finding, helpers};
use std::collections::BTreeMap;

/// `EX3012`: prefer pipelines over nested calls with `min_pipeline_length`.
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let min_len = helpers::param_usize(params, "min_pipeline_length", 2);
    // Upstream drops `:<<>>` binaries whole: calls inside string
    // interpolation are invisible to nesting (masking keeps them visible
    // for other checks).
    let masked = helpers::mask_interpolation(prepared.masked());
    let lines: Vec<&str> = masked.split('\n').collect();
    let starts = line_starts(&masked);
    let mut calls = find_calls(&masked);
    calls.sort_by_key(|call| call.head_start);
    let lengths: Vec<usize> = calls
        .iter()
        .map(|call| 1 + pipe_len(first_arg(call)))
        .collect();
    let parents = parent_indexes(&calls);
    let pipe_target: Vec<bool> = calls
        .iter()
        .map(|call| is_pipe_target(&masked, call.head_start))
        .collect();
    // Calls whose first argument is an `fn` expression can never start
    // a pipeline (upstream `[:fn]` argument type): they report nothing
    // themselves, but their subtrees stay visible (unlike pruned ones).
    let transparent: Vec<bool> = calls
        .iter()
        .map(|call| is_fn_lead(first_arg(call)))
        .collect();
    // Ancestors that never prune their subtrees: pipe targets (their
    // arguments are still visited) and transparent `fn`-led calls.
    let shields: Vec<bool> = pipe_target
        .iter()
        .zip(transparent.iter())
        .map(|(pipe, clear)| *pipe || *clear)
        .collect();
    let mut findings = Vec::new();
    for (idx, call) in calls.iter().enumerate() {
        let line = line_of_idx(&starts, &lines, call.head_start);
        if skip_line(line) {
            continue;
        }
        if transparent[idx] {
            continue;
        }
        // Upstream prunes whole subtrees of calls below the minimum
        // length; shielded ancestors never prune (their arguments
        // are still visited).
        if pruned_under(&lengths, &parents, &shields, min_len, idx) {
            continue;
        }
        if pipe_target[idx] {
            continue;
        }
        if lengths[idx] >= min_len {
            findings.push(Finding::with_trigger(
                line_no(&starts, call.head_start),
                credo_column(line, &call.head),
                "Use a pipeline instead of nested function calls.",
                call.head.clone(),
            ));
        }
    }
    findings.sort_by_key(|finding| (finding.line, finding.column.unwrap_or(0)));
    findings
}

struct RemoteCall {
    head: String,
    head_start: usize,
    close: usize,
    args: String,
    args_start: usize,
    args_end: usize,
}

/// Guards, typespecs and documentation carry no pipeline candidates.
fn skip_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    for word in ["defguard ", "defguardp "] {
        if trimmed.starts_with(word) {
            return true;
        }
    }
    if trimmed.starts_with('@') {
        for attr in [
            "@callback",
            "@macrocallback",
            "@opaque",
            "@spec",
            "@type",
            "@typep",
        ] {
            if trimmed.starts_with(attr)
                && trimmed
                    .as_bytes()
                    .get(attr.len())
                    .is_none_or(|b| !is_name_byte(*b))
            {
                return true;
            }
        }
    }
    false
}

/// Remote (`Mod.fun(...)`), Erlang (`:mod.fun(...)`) and anonymous
/// (`name.(...)`) calls over the whole masked source, so multiline outers
/// keep their arguments.
fn find_calls(text: &str) -> Vec<RemoteCall> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0_usize;
    while idx < bytes.len() {
        if bytes[idx] != b'(' {
            idx += 1;
            continue;
        }
        if let Some((head, head_start)) = call_head(text, idx)
            && let Some(close) = match_paren(text, idx)
        {
            let args = text.get(idx + 1..close).unwrap_or("").to_owned();
            out.push(RemoteCall {
                head,
                head_start,
                close,
                args_start: idx + 1,
                args_end: close,
                args,
            });
        }
        idx += 1;
    }
    out
}

/// Dotted, Erlang or anonymous head ending right before `(` at `open`.
/// A preceding keyword/word on the same line (`case`, `if`, `and`, `do:`)
/// no longer absorbs the head: only the last space-separated segment counts.
fn call_head(text: &str, open: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut start = open;
    while start > 0 && is_head_byte(bytes[start - 1]) && bytes[start - 1] != b'\n' {
        start -= 1;
    }
    let raw = text.get(start..open)?.trim_end();
    if raw.is_empty() {
        return None;
    }
    let trimmed = raw.trim_start();
    let candidate = trimmed
        .split(' ')
        .filter(|part| !part.is_empty())
        .next_back()
        .unwrap_or(trimmed);
    if candidate.is_empty() {
        return None;
    }
    // Anonymous `name.(...)`: trigger is the bare name.
    if candidate.ends_with('.') {
        let base = candidate.trim_end_matches('.');
        if is_local_name(base)
            && let Some(head_start) = text.get(..open)?.rfind(base)
        {
            return Some((base.to_owned(), head_start));
        }
        return None;
    }
    if !is_remote_head(candidate) {
        return None;
    }
    let head_start = text.get(..open)?.rfind(candidate)?;
    // A directly adjacent colon is either an Erlang marker or a `key:`/`do:`
    // separator: both still report the head; `::` stays out.
    if head_start > 0
        && bytes[head_start - 1] == b':'
        && head_start >= 2
        && bytes[head_start - 2] == b':'
    {
        return None;
    }
    Some((candidate.to_owned(), head_start))
}

fn is_head_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'?' | b'!' | b' ')
}

fn is_remote_head(head: &str) -> bool {
    let head = head.trim_end();
    let name = head.rsplit('.').next().unwrap_or("");
    if name.is_empty() || !head.contains('.') {
        return false;
    }
    let mut name_chars = name.chars();
    if name_chars
        .next()
        .is_none_or(|c| !(c.is_ascii_lowercase() || c == '_'))
    {
        return false;
    }
    head.split('.').all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
    })
}

/// Whether the call is the direct target of a `|>` (checked over the whole
/// source, so `|>` at the end of the previous line still counts).
fn is_pipe_target(text: &str, head_start: usize) -> bool {
    let before = text.get(..head_start).unwrap_or("").trim_end();
    before.ends_with("|>")
}

/// Index of the smallest enclosing call for each call, if any.
fn parent_indexes(calls: &[RemoteCall]) -> Vec<Option<usize>> {
    let mut parents = vec![None; calls.len()];
    for (idx, call) in calls.iter().enumerate() {
        let mut best: Option<(usize, usize)> = None;
        for (other, parent) in calls.iter().enumerate() {
            if other == idx {
                continue;
            }
            if parent.args_start <= call.head_start && call.close <= parent.args_end {
                let width = parent.args_end - parent.args_start;
                if best.is_none_or(|(_, best_width)| width < best_width) {
                    best = Some((other, width));
                }
            }
        }
        parents[idx] = best.map(|(other, _)| other);
    }
    parents
}

/// Whether an ancestor with pipeline length below minimum prunes this call.
/// Pipe-target ancestors never prune: upstream still visits their arguments.
fn pruned_under(
    lengths: &[usize],
    parents: &[Option<usize>],
    shields: &[bool],
    min_len: usize,
    idx: usize,
) -> bool {
    let mut current = parents[idx];
    while let Some(parent) = current {
        if !shields[parent] && lengths[parent] < min_len {
            return true;
        }
        current = parents[parent];
    }
    false
}

/// First top-level argument of the call.
fn first_arg(call: &RemoteCall) -> &str {
    split_args(&call.args).into_iter().next().unwrap_or("")
}

/// Whether text leads with an `fn` expression: anything from bare
/// `fn ->` to multi-clause heads. Upstream types every `fn` first
/// argument `[:fn]`, so no `fn`-led argument ever starts a pipeline.
fn is_fn_lead(text: &str) -> bool {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("fn") else {
        return false;
    };
    !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

/// Pipeline length contributed by the first argument: another nested call
/// with arguments adds one level. Paren-less `from x in y` counts as one;
/// `key: value` keywords, field access (`a.b`) and operator rests count zero.
/// An `fn` with arguments cannot start a pipeline (upstream
/// anonymous-function rule); an arg-less `fn ->` counts one.
fn pipe_len(arg: &str) -> usize {
    let trimmed = strip_parens(arg.trim());
    // Erlang `:mod.fun(...)` counts like its dotted form.
    let trimmed = trimmed.strip_prefix(':').unwrap_or(trimmed);
    if trimmed.is_empty() || keyword_value(trimmed).is_some() {
        return 0;
    }
    if is_fn_lead(trimmed) {
        return 0;
    }
    if let Some((head, args)) = split_call(trimmed) {
        if is_excluded_head(head) || args.trim().is_empty() {
            return 0;
        }
        return 1 + pipe_len(first_arg_in(&args));
    }
    if let Some((head, rest)) = split_space_inner(trimmed) {
        if is_excluded_head(head) || rest.trim().is_empty() {
            return 0;
        }
        return 1;
    }
    0
}

fn first_arg_in(args: &str) -> &str {
    split_args(args).into_iter().next().unwrap_or("")
}

/// Heads that cannot start a pipeline (`fn`, operators, sentinels).
fn is_excluded_head(head: &str) -> bool {
    matches!(
        head,
        "fn" | "for" | "with" | "not" | "and" | "or" | "in" | "unquote"
    )
}

/// Leading call (`name(...)` or `Mod.name(...)`) consuming the whole text.
/// Trailing field access (`Repo.get!(...).field`) is not a plain call.
fn split_call(text: &str) -> Option<(&str, String)> {
    let mut head_end = 0_usize;
    for (idx, char) in text.char_indices() {
        if char.is_alphanumeric() || char == '_' || char == '.' || char == '?' || char == '!' {
            head_end = idx + char.len_utf8();
        } else {
            break;
        }
    }
    let head = text.get(..head_end)?;
    if head.is_empty() || head.starts_with('.') || head.ends_with('.') || head.contains("..") {
        return None;
    }
    let open = text[head_end..].find('(')? + head_end;
    if !text
        .get(head_end..open)
        .is_some_and(|gap| gap.trim().is_empty())
    {
        return None;
    }
    let close = match_paren(text, open)?;
    if !text
        .get(close + 1..)
        .is_some_and(|rest| rest.trim().is_empty())
    {
        return None;
    }
    Some((head, text.get(open + 1..close).unwrap_or("").to_owned()))
}

/// Paren-less call (`from log in Log`): head plus a non-operator rest.
fn split_space_inner(text: &str) -> Option<(&str, &str)> {
    let mut head_end = 0_usize;
    for (idx, char) in text.char_indices() {
        if char.is_alphanumeric() || char == '_' || char == '.' || char == '?' || char == '!' {
            head_end = idx + char.len_utf8();
        } else {
            break;
        }
    }
    let head = text.get(..head_end)?;
    if head.is_empty() || head.starts_with('.') || head.ends_with('.') || head.contains("..") {
        return None;
    }
    let rest = text.get(head_end..)?.trim_start();
    if rest.is_empty() {
        return None;
    }
    let first = rest.chars().next().unwrap_or(' ');
    if matches!(
        first,
        '(' | '[' | '{' | '"' | '\'' | ':' | ',' | ';' | ')' | ']' | '}'
    ) || is_operator_start(first)
    {
        return None;
    }
    Some((head, rest))
}

fn is_operator_start(char: char) -> bool {
    matches!(
        char,
        '<' | '>' | '=' | '|' | '+' | '-' | '*' | '/' | '&' | '~' | '^' | '!' | '?' | '.'
    )
}

/// Leading `key:` in `key: value` keywords.
fn keyword_value(text: &str) -> Option<&str> {
    let mut idx = 0_usize;
    for (byte, char) in text.char_indices() {
        if char.is_alphanumeric() || char == '_' || char == '?' || char == '!' {
            idx = byte + char.len_utf8();
        } else {
            break;
        }
    }
    if idx == 0 {
        return None;
    }
    let rest = text.get(idx..)?;
    if !rest.starts_with(':') || rest[1..].starts_with(':') {
        return None;
    }
    let after = rest[1..].trim_start();
    if after.is_empty() {
        return None;
    }
    Some(after)
}

fn is_local_name(name: &str) -> bool {
    let mut chars = name.chars();
    if chars
        .next()
        .is_none_or(|c| !(c.is_ascii_lowercase() || c == '_'))
    {
        return false;
    }
    name.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '?' || c == '!')
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    for (byte, char) in text.char_indices() {
        if char == '\n' {
            starts.push(byte + 1);
        }
    }
    starts
}

fn line_no(starts: &[usize], pos: usize) -> usize {
    starts.partition_point(|start| *start <= pos).max(1)
}

fn line_of_idx<'a>(starts: &[usize], lines: &[&'a str], pos: usize) -> &'a str {
    let line_no = line_no(starts, pos);
    lines.get(line_no - 1).copied().unwrap_or("")
}

/// Strip fully enclosing parentheses (`(f(x))` -> `f(x)`).
fn strip_parens(text: &str) -> &str {
    let mut current = text;
    while current.starts_with('(') {
        let Some(close) = match_paren(current, 0) else {
            break;
        };
        if close + 1 != current.len() {
            break;
        }
        current = current.get(1..close).unwrap_or("").trim();
    }
    current
}

fn split_args(args: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (idx, byte) in args.bytes().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(args.get(start..idx).unwrap_or(""));
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(args.get(start..).unwrap_or(""));
    parts
}

fn match_paren(line: &str, open: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0_usize;
    let mut idx = open;
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'?' || byte == b'!'
}

/// Credo `SourceFile.column/3`: 1-based column of `trigger` when surrounded by
/// whitespace, parens, commas or word boundaries; `None` otherwise.
fn credo_column(line: &str, trigger: &str) -> Option<usize> {
    if trigger.is_empty() {
        return None;
    }
    let lchars: Vec<char> = line.chars().collect();
    let tchars: Vec<char> = trigger.chars().collect();
    if lchars.len() < tchars.len() {
        return None;
    }
    for idx in 0..=lchars.len() - tchars.len() {
        if lchars[idx..idx + tchars.len()] != tchars[..] {
            continue;
        }
        let before_ok = if idx == 0 {
            is_word(tchars[0])
        } else {
            before_ok(lchars[idx - 1], tchars[0])
        };
        let after = idx + tchars.len();
        let after_ok = if after == lchars.len() {
            is_word(tchars[tchars.len() - 1])
        } else {
            after_ok(tchars[tchars.len() - 1], lchars[after])
        };
        if before_ok && after_ok {
            return Some(idx + 1);
        }
    }
    None
}

fn before_ok(prev: char, first: char) -> bool {
    prev.is_whitespace()
        || prev == '('
        || prev == ')'
        || prev == ','
        || is_word(prev) != is_word(first)
}

fn after_ok(last: char, next: char) -> bool {
    next.is_whitespace()
        || next == '('
        || next == ')'
        || next == ','
        || is_word(last) != is_word(next)
}

fn is_word(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_is_clean() {
        let src = "x |> foo() |> bar()\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn reports_deeply_nested() {
        let src = "Foo.bar(Baz.qux(x))\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }

    #[test]
    fn no_arg_inner_call_is_clean() {
        // EX3012.upstream.inner-no-args: `some_list()` takes no arguments.
        let src = "defmodule CredoSampleModule do\n  def some_code do\n    Enum.uniq(some_list())\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn reports_each_nesting_level() {
        // EX3012.upstream.three-nested: outer and middle calls both report.
        let src = "defmodule CredoSampleModule do\n  def some_code do\n    Enum.shuffle(Enum.uniq(Enum.take([1,2,2,3,3], 10)))\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 2);
        assert_eq!((findings[0].line, findings[0].column), (3, Some(5)));
        assert_eq!((findings[1].line, findings[1].column), (3, Some(18)));
    }

    #[test]
    fn min_pipeline_length_raises_bar() {
        let src = "Foo.bar(Baz.qux(x))\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
        let mut params = BTreeMap::new();
        params.insert("min_pipeline_length".to_owned(), "3".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
    #[test]
    fn multiline_outer_reports() {
        let src = "Repo.update_all(\n  from(i in Item, where: i.id == ^id),\n  set: [x: 1]\n)\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn keyword_preceded_outer_reports() {
        let src = "case Integer.parse(to_string(raw)) do\n  {n, \"\"} -> n\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn do_colon_outer_reports() {
        let src = "def f(raw), do: Integer.parse(to_string(raw))\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn parenless_from_inner_counts() {
        let src = "Repo.one(from log in Log, where: log.build_id == ^build_id, select: max(log.seq)) || 0\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn anonymous_outer_reports() {
        let src = "nil -> insert.(Ecto.Changeset.put_change(changeset, :reuse_key, key))\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
    }
    #[test]
    fn when_line_rhs_reports() {
        let src = "{:ok, values} when is_list(values) <- Jason.decode(to_string(json)),\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn erlang_outer_reports() {
        let src = "volume = Base.encode16(:crypto.hash(:sha256, owner), case: :lower)\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).len(),
            1
        );
    }
    #[test]
    fn capture_hides_nested_calls() {
        // Upstream never visits calls inside `&` captures.
        let src = "Enum.map(items, &Map.merge(Map.from_struct(&1), %{}))\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn nested_inside_fn_with_args_reports() {
        // `fn`-led first arguments never report themselves, but their
        // subtrees stay visible: remote nesting inside still reports.
        let src = "def f(x) do\n  foo(fn y -> Mod.bar(Mod2.baz(y)) end)\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].trigger,
            crate::Trigger::Text("Mod.bar".to_owned())
        );
    }

    #[test]
    fn nested_inside_fn_body_reports() {
        // Short outers never suppress inner nesting; only `&` captures
        // hide their contents.
        let src = "def f(build) do\n  Repo.transaction(fn ->\n    Repo.update_all(\n      from(i in B, where: i.id == 1),\n      set: [x: 1]\n    )\n  end)\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(src), &BTreeMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
    }
}
