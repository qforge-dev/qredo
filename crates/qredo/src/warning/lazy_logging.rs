use crate::Finding;
use std::collections::BTreeMap;

/// `EX5007`
pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    let source = prepared.source();
    let ignored = ignored_levels(params);
    let masked = prepared.masked();
    let scan = Scan {
        imported: has_logger_import(masked),
        ignored: &ignored,
    };
    let mut findings = Vec::new();
    for (idx, (raw_line, mask_line)) in source.split('\n').zip(masked.split('\n')).enumerate() {
        check_line(raw_line, mask_line, idx + 1, &scan, &mut findings);
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

/// Levels in `[:debug, :info, :warn, :error]` order; default ignores all but debug.
fn levels() -> [&'static str; 4] {
    ["debug", "info", "warn", "error"]
}

/// Parse the `ignore` list param (compact JSON array of atom names).
fn ignored_levels(params: &BTreeMap<String, String>) -> Vec<String> {
    let Some(raw) = params.get("ignore") else {
        return vec!["error".to_owned(), "warn".to_owned(), "info".to_owned()];
    };
    if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw) {
        return items
            .iter()
            .filter_map(|item| item.as_str())
            .map(|name| name.strip_prefix(':').unwrap_or(name).to_owned())
            .collect();
    }
    vec![raw.strip_prefix(':').unwrap_or(raw).to_owned()]
}

/// Whether the file imports `Logger` (enables bare `debug`/`info`/etc. calls).
fn has_logger_import(masked: &str) -> bool {
    for line in masked.split('\n') {
        let mut search = 0_usize;
        while search < line.len() {
            let Some(rel) = line[search..].find("import") else {
                break;
            };
            let base = search + rel;
            search = base + 1;
            if word_before(line, base) || !call_after(line, base + "import".len()) {
                continue;
            }
            let rest = line[base + "import".len()..].trim_start();
            if rest.starts_with("Logger")
                && rest["Logger".len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '?' || c == '!'))
                && !rest["Logger".len()..].starts_with('.')
            {
                return true;
            }
        }
    }
    false
}

/// Scan context: Logger import flag plus ignored levels.
struct Scan<'a> {
    imported: bool,
    ignored: &'a [String],
}

fn check_line(raw: &str, mask: &str, line_no: usize, scan: &Scan<'_>, findings: &mut Vec<Finding>) {
    for level in levels() {
        if scan.ignored.iter().any(|name| name == level) {
            continue;
        }
        let remote = format!("Logger.{level}");
        let mut search = 0_usize;
        while search < mask.len() {
            let Some(rel) = mask[search..].find(&remote) else {
                break;
            };
            let base = search + rel;
            search = base + 1;
            if word_before(mask, base) || !call_after(mask, base + remote.len()) {
                continue;
            }
            if eager_string_arg(raw, base + remote.len()) {
                findings.push(Finding::with_trigger(
                    line_no,
                    Some(col_of(mask, base)),
                    "Prefer lazy Logger calls.",
                    remote.clone(),
                ));
            }
        }
        if scan.imported {
            check_bare(raw, mask, line_no, level, findings);
        }
    }
}

/// Bare `debug "..."` calls (only valid after `import Logger`).
fn check_bare(raw: &str, mask: &str, line_no: usize, level: &str, findings: &mut Vec<Finding>) {
    let mut search = 0_usize;
    while search < mask.len() {
        let Some(rel) = mask[search..].find(level) else {
            break;
        };
        let base = search + rel;
        search = base + 1;
        if word_before(mask, base)
            || dot_before(mask, base)
            || !call_after(mask, base + level.len())
        {
            continue;
        }
        if eager_string_arg(raw, base + level.len()) {
            findings.push(Finding::with_trigger(
                line_no,
                Some(col_of(mask, base)),
                "Prefer lazy Logger calls.",
                level.to_owned(),
            ));
        }
    }
}

/// Char before `base` must not continue a name (`xdebug`, `Logger.debug`...).
fn word_before(line: &str, base: usize) -> bool {
    if base == 0 {
        return false;
    }
    line[..base]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '@' || c == ':')
}

/// Whether `base` is directly preceded by `.` (a remote call's function name).
fn dot_before(line: &str, base: usize) -> bool {
    base > 0 && line[..base].ends_with('.')
}

/// The char after the name must not continue it (`debugger`, `debug?`, ...).
fn call_after(line: &str, end: usize) -> bool {
    if end >= line.len() {
        return false;
    }
    line[end..]
        .chars()
        .next()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '?' || c == '!'))
}

/// Whether the first argument (in the raw line) is an eager interpolated string.
fn eager_string_arg(raw: &str, end: usize) -> bool {
    let mut rest = raw[end..].trim_start();
    if rest.starts_with('(') {
        rest = rest[1..].trim_start();
    }
    if let Some(body) = rest.strip_prefix("\"\"\"") {
        return heredoc_interpolated(body);
    }
    if !rest.starts_with('"') {
        return false;
    }
    string_interpolated(&rest[1..], '"')
}

/// Interpolated `#{` before the closing `"` on this line.
fn string_interpolated(rest: &str, closer: char) -> bool {
    let mut chars = rest.chars();
    while let Some(chr) = chars.next() {
        if chr == '\\' {
            chars.next();
        } else if chr == '#' && chars.clone().next() == Some('{') {
            return true;
        } else if chr == closer {
            return false;
        }
    }
    false
}

/// Interpolated `#{` before the closing `"""` (or end of line).
fn heredoc_interpolated(rest: &str) -> bool {
    if let Some(end) = rest.find("\"\"\"") {
        return string_interpolated(&rest[..end], '"');
    }
    string_interpolated(rest, '"')
}

/// Column (1-based, characters) of the byte offset (which must be a boundary).
fn col_of(line: &str, byte_pos: usize) -> usize {
    line[..byte_pos].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean() {
        assert!(
            check_prepared(
                &crate::batch::Prepared::lazy("Logger.info(\"hi\")\n"),
                &BTreeMap::new()
            )
            .is_empty()
        );
    }
    #[test]
    fn reports_interpolation() {
        assert_eq!(
            check_prepared(
                &crate::batch::Prepared::lazy("Logger.debug(\"hi #{x}\")\n"),
                &BTreeMap::new()
            )
            .len(),
            1
        );
    }
    #[test]
    fn ignored_level_is_clean() {
        let src = "defmodule CredoSampleModule do\n  def some_function(parameter1, parameter2) do\n    Logger.debug \"Ok #{inspect 1}\"\n  end\nend\n";
        let mut params = BTreeMap::new();
        params.insert("ignore".to_owned(), "[\":debug\"]".to_owned());
        assert!(check_prepared(&crate::batch::Prepared::lazy(src), &params).is_empty());
    }
}
