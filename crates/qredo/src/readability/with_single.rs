use crate::Finding;

/// `EX3033`: `with` with a single `<-` and `else` should be `case`.
///
/// A `with` flags when it has exactly one `<-` clause at head depth and
/// its own `else` branch (block or one-line `else:`), mirroring the
/// native `do_block?`/`else_block?` plus single-`<-` count. Inner
/// `if`/`case` branches sit deeper and never count.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if with_head(line) && scan_with(&lines, idx).is_some() {
            let col = line.find("with").unwrap_or(0) + 1;
            findings.push(Finding::with_trigger(
                idx + 1,
                Some(col),
                "`with` contains only one <- clause and an `else` branch, consider using `case` instead",
                "with".to_owned(),
            ));
        }
    }
    findings
}

/// True when the trimmed line opens a `with` head.
fn with_head(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("with ") || trimmed.starts_with("with(")
}

/// Whether one `with` (head at `first`) has a single `<-` and its own
/// `else`: block depth separates the with body (1) from nested blocks
/// (2+), bracket depth skips comprehensions and one-line `if` values.
fn scan_with(lines: &[&str], first: usize) -> Option<()> {
    let mut state = State {
        in_head: true,
        ..State::default()
    };
    let mut line_idx = first;
    // Byte offset just past the `with` keyword.
    let mut line_pos = lines[first].find("with").unwrap_or(0) + "with".len();
    loop {
        let line = lines.get(line_idx).copied()?;
        let bytes = line.as_bytes();
        // Clamp stale offsets (never split a char: keywords are ASCII).
        line_pos = line_pos.min(bytes.len());
        while line_pos < bytes.len() {
            let rest = &line[line_pos..];
            if state.in_head && state.block == 0 && state.brackets == 0 && rest.starts_with("<-") {
                state.arrows += 1;
                line_pos += 2;
                continue;
            }
            match bytes[line_pos] {
                b'(' | b'[' | b'{' => {
                    state.brackets += 1;
                    line_pos += 1;
                    continue;
                }
                b')' | b']' | b'}' => {
                    state.brackets = state.brackets.saturating_sub(1);
                    line_pos += 1;
                    continue;
                }
                _ => {}
            }
            let Some(word) = word_at(&bytes[line_pos..]) else {
                line_pos += 1;
                continue;
            };
            match visit_word(&mut state, word, bytes, line_pos) {
                Step::Continue => {}
                Step::Report => return (state.arrows == 1).then_some(()),
                Step::Done => return None,
            }
            line_pos += word.len();
        }
        line_idx += 1;
        line_pos = 0;
        if line_idx >= lines.len() || line_idx > first + 500 {
            return None;
        }
    }
}

/// Mutable scan state: head/body mode plus nesting depths.
#[derive(Default)]
struct State {
    arrows: usize,
    block: usize,
    brackets: usize,
    in_head: bool,
    one_line: bool,
}

/// Outcome of one word token.
enum Step {
    Continue,
    Report,
    Done,
}

/// Fold one word token into the scan state.
fn visit_word(state: &mut State, word: &str, bytes: &[u8], pos: usize) -> Step {
    match word {
        "do" if !follows_colon(bytes, pos + word.len()) => {
            if state.in_head && state.block == 0 && state.brackets == 0 {
                state.in_head = false;
            }
            state.block += 1;
        }
        // One-line heads (`with a <- b, do: c`) never open a block.
        "do" if state.in_head && state.block == 0 && state.brackets == 0 => {
            state.in_head = false;
            state.one_line = true;
        }
        "fn" => {
            state.block += 1;
        }
        "else" => {
            // Block `else` never takes a colon; one-line `else:` always
            // does. Inner one-line `if` values sit at the same block
            // depth but carry the other form.
            let colon = follows_colon(bytes, pos + word.len());
            let own = !state.in_head
                && state.brackets == 0
                && ((state.block == 1 && !state.one_line && !colon)
                    || (state.one_line && state.block == 0 && colon));
            if own {
                return Step::Report;
            }
        }
        "end" => {
            if state.block == 0 {
                // Enclosing block (or a one-line with) ends first.
                return Step::Done;
            }
            state.block -= 1;
            if state.block == 0 && !state.one_line {
                return Step::Done;
            }
        }
        _ => {}
    }
    Step::Continue
}

/// Identifier word starting at `bytes`, if any (maximal run, so `endo`
/// never reads as `end`).
fn word_at(bytes: &[u8]) -> Option<&str> {
    let len = bytes
        .iter()
        .take_while(|byte| {
            byte.is_ascii_alphanumeric() || **byte == b'_' || **byte == b'?' || **byte == b'!'
        })
        .count();
    if len == 0 {
        return None;
    }
    std::str::from_utf8(&bytes[..len]).ok()
}

/// True when a `:` (skipping whitespace) follows `pos`.
fn follows_colon(bytes: &[u8], mut pos: usize) -> bool {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    bytes.get(pos) == Some(&b':')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multi_clause_is_clean() {
        let src = "with {:ok, a} <- foo(),\n     {:ok, b} <- bar(), do: {a, b}\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_single_with_else() {
        let src = "with {:ok, a} <- foo() do\n  a\nelse\n  _ -> :error\nend\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 1);
    }
    #[test]
    fn nested_else_does_not_count() {
        // Native reference: 0 issues; the `else` belongs to the inner
        // `if`, and the one-line `do:`/`else:` pair reports.
        let nested = "defmodule M do\n  def f(x) do\n    with {:ok, y} <- foo(x) do\n      if y, do: :a, else: :b\n    end\n  end\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(nested)).is_empty());
        let one_line = "defmodule M do\n  def f(x) do\n    with {:ok, y} <- foo(x), do: y, else: (_ -> :e)\n  end\nend\n";
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(one_line)).len(),
            1
        );
    }
}
