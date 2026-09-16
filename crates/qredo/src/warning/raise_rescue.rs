use crate::Finding;

/// `EX5013`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let mut findings = Vec::new();
    // Open `do`/`fn` blocks; each records whether `rescue` was seen.
    let mut stack: Vec<bool> = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        scan_line(line, idx + 1, &mut stack, &mut findings);
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

fn scan_line(line: &str, line_no: usize, stack: &mut Vec<bool>, findings: &mut Vec<Finding>) {
    let mut pos = 0_usize;
    while pos < line.len() {
        let Some((word, start, end)) = next_word(line, pos) else {
            break;
        };
        pos = end;
        match word {
            "do" | "fn" if !colon_follows(line, end) => stack.push(false),
            "end" => {
                stack.pop();
            }
            "rescue" => {
                if let Some(top) = stack.last_mut() {
                    *top = true;
                }
            }
            "raise" => {
                if stack.iter().any(|seen| *seen) {
                    findings.push(Finding::with_trigger(
                        line_no,
                        Some(col_of(line, start)),
                        "Use `reraise` inside a rescue block to preserve the original stacktrace.",
                        "raise".to_owned(),
                    ));
                }
            }
            _ => {}
        }
    }
}

/// Next identifier word with byte offsets, skipping keyword sigils.
fn next_word(line: &str, mut pos: usize) -> Option<(&str, usize, usize)> {
    while pos < line.len() {
        let chr = line[pos..].chars().next()?;
        if !(chr.is_ascii_alphabetic() || chr == '_') {
            pos += chr.len_utf8();
            continue;
        }
        // Atoms, attributes and remote calls are not keywords.
        if pos > 0
            && line[..pos]
                .chars()
                .next_back()
                .is_some_and(|c| c == ':' || c == '@' || c == '.')
        {
            pos += chr.len_utf8();
            continue;
        }
        let mut end = pos + chr.len_utf8();
        while let Some(next) = line[end..].chars().next() {
            if !(next.is_ascii_alphanumeric() || next == '_' || next == '?' || next == '!') {
                break;
            }
            end += next.len_utf8();
        }
        return Some((&line[pos..end], pos, end));
    }
    None
}

/// Whether the word is directly followed by `:` (`do:`/`fn:` keywords).
fn colon_follows(line: &str, end: usize) -> bool {
    line[end..].starts_with(':')
}

/// Column (1-based, characters) of the byte offset (which must be a boundary).
fn col_of(line: &str, byte_pos: usize) -> usize {
    line[..byte_pos].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reraise_is_clean() {
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "try do\n x\nrescue\n e -> reraise e, __STACKTRACE__\nend\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports_raise() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy(
                "try do\n x\nrescue\n e -> raise e\nend\n"
            ))
            .len(),
            1
        );
    }
    #[test]
    fn reports_raise_after_inner_block() {
        let src = "defmodule CredoSampleModule do\n  use ExUnit.Case\n\n  def catcher do\n    try do\n      raise \"oops\"\n    rescue\n      e ->\n        if is_nil(e) do\n          :ok\n        end\n        raise e\n    end\n  end\nend\n";
        let out = check_prepared(&crate::batch::Prepared::lazy(src));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 12);
    }
}
