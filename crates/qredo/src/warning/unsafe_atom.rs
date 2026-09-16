use crate::Finding;

/// `EX5016`
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let source = prepared.source();
    let facts = prepared.facts();
    let mut findings = Vec::new();
    let lines: Vec<&str> = source.split('\n').collect();
    let starts = line_starts(source);
    let skipped = skipped_regions(source, facts);
    for call in &facts.calls {
        if skipped
            .iter()
            .any(|(start, end)| *start <= call.start && call.end <= *end)
        {
            continue;
        }
        if let Some((message, trigger)) = forbidden_call(call, source) {
            let (line, column) = line_column(&lines, &starts, call.start as usize);
            findings.push(Finding::with_trigger(line, Some(column), message, trigger));
        }
    }
    findings.sort_by(|a, b| (a.line, a.column).cmp(&(b.line, b.column)));
    findings
}

/// Skip regions: `@attribute` spans plus `unquote` call spans. Calls
/// inside them are skipped by the reference, which climbs every ancestor.
fn skipped_regions(source: &str, facts: &crate::facts::Facts) -> Vec<(u32, u32)> {
    let mut regions = facts.attr_regions.clone();
    for call in &facts.calls {
        let unquoted = match &call.head {
            Some(crate::facts::HeadFact::Plain { start, end }) => {
                slice(source, *start, *end) == Some("unquote")
            }
            Some(crate::facts::HeadFact::Remote {
                fun_start, fun_end, ..
            }) => slice(source, *fun_start, *fun_end) == Some("unquote"),
            None => false,
        };
        if unquoted {
            regions.push((call.start, call.end));
        }
    }
    regions
}

/// `(message, trigger)` when `call` unsafely creates atoms at runtime.
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
    let (Some(module), Some(fun)) = (
        slice(source, *mod_start, *mod_end),
        slice(source, *fun_start, *fun_end),
    ) else {
        return None;
    };
    let args: Vec<&crate::facts::ArgFact> = call.args.iter().filter(|arg| arg.code).collect();
    match (*mod_kind, module, fun) {
        (crate::facts::ModKind::Alias, module @ ("String" | "List"), "to_atom") => {
            to_atom_finding(module, &args, call.piped)
        }
        (crate::facts::ModKind::Alias, "Module", "concat") => concat_finding(&args, call.piped),
        (crate::facts::ModKind::Atom, ":erlang", "list_to_atom") => {
            erlang_atom_finding(&args, call.piped, LIST_TO_ATOM_FIX)
        }
        (crate::facts::ModKind::Atom, ":erlang", "binary_to_atom") => {
            erlang_atom_finding(&args, call.piped, BINARY_TO_ATOM_FIX)
        }
        (crate::facts::ModKind::Alias, "Jason", decode)
            if decode == "decode" || decode == "decode!" =>
        {
            jason_finding(decode, &args, source)
        }
        _ => None,
    }
}

/// Atom-constructor reports: (message, trigger, arity).
const LIST_TO_ATOM_FIX: (&str, &str, usize) = (
    "Prefer :erlang.list_to_existing_atom/1 over :erlang.list_to_atom/1 to avoid creating atoms at runtime.",
    ":erlang.list_to_atom",
    1,
);
const BINARY_TO_ATOM_FIX: (&str, &str, usize) = (
    "Prefer :erlang.binary_to_existing_atom/2 over :erlang.binary_to_atom/2 to avoid creating atoms at runtime.",
    ":erlang.binary_to_atom",
    2,
);

/// `(message, trigger)` for `String/List.to_atom` with one effective argument.
fn to_atom_finding(
    module: &str,
    args: &[&crate::facts::ArgFact],
    piped: bool,
) -> Option<(String, String)> {
    if args.len() == 1 || (args.is_empty() && piped) {
        Some((
            format!(
                "Prefer {module}.to_existing_atom/1 over {module}.to_atom/1 to avoid creating atoms at runtime."
            ),
            format!("{module}.to_atom"),
        ))
    } else {
        None
    }
}

/// `(message, trigger)` for `Module.concat` by arity.
fn concat_finding(args: &[&crate::facts::ArgFact], piped: bool) -> Option<(String, String)> {
    match args.len() {
        1 => Some((
            "Prefer Module.safe_concat/1 over Module.concat/1 to avoid creating atoms at runtime."
                .to_owned(),
            "Module.concat".to_owned(),
        )),
        2 => Some((
            "Prefer Module.safe_concat/2 over Module.concat/2 to avoid creating atoms at runtime."
                .to_owned(),
            "Module.concat".to_owned(),
        )),
        _ if args.is_empty() && piped => Some((
            "Prefer Module.safe_concat/1 over Module.concat/1 to avoid creating atoms at runtime."
                .to_owned(),
            "Module.concat".to_owned(),
        )),
        _ => None,
    }
}

/// `(message, trigger)` for `:erlang` atom constructors.
fn erlang_atom_finding(
    args: &[&crate::facts::ArgFact],
    piped: bool,
    fix: (&str, &str, usize),
) -> Option<(String, String)> {
    let (message, trigger, arity) = fix;
    if args.len() == arity || (args.len() + 1 == arity && piped) {
        Some((message.to_owned(), trigger.to_owned()))
    } else {
        None
    }
}

/// `(message, trigger)` for `Jason.decode(!)` with `keys: :atoms`.
fn jason_finding(
    decode: &str,
    args: &[&crate::facts::ArgFact],
    source: &str,
) -> Option<(String, String)> {
    if uses_atoms_keys(args, source) {
        Some((
            format!(
                "Prefer Jason.{decode}(..., keys: :atoms!) over Jason.{decode}(..., keys: :atoms) to avoid creating atoms at runtime."
            ),
            format!("Jason.{decode}"),
        ))
    } else {
        None
    }
}

/// Whether any explicit argument carries a `keys: :atoms` keyword pair.
fn uses_atoms_keys(args: &[&crate::facts::ArgFact], source: &str) -> bool {
    args.iter().any(|arg| {
        arg.keys.iter().any(|pair| {
            let key = source
                .get(pair.key_start as usize..pair.key_end as usize)
                .map(|key| key.trim().trim_matches(':'));
            let value = pair
                .value
                .and_then(|(start, end)| source.get(start as usize..end as usize));
            key == Some("keys") && value == Some(":atoms")
        })
    })
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
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "String.to_existing_atom(x)\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn reports() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("String.to_atom(x)\n")).len(),
            1
        );
    }
    #[test]
    fn reports_module_concat() {
        let source = "defmodule CredoSampleModule do\n  def some_function(parameter) do\n    Module.concat(__MODULE__, parameter)\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(source));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].column, Some(5));
        assert_eq!(
            findings[0].message,
            "Prefer Module.safe_concat/2 over Module.concat/2 to avoid creating atoms at runtime."
        );
    }
    #[test]
    fn reports_jason_keys_atoms() {
        let source = "defmodule CredoSampleModule do\n  def some_function(parameter) do\n    Jason.decode(parameter, keys: :atoms)\n  end\nend\n";
        let findings = check_prepared(&crate::batch::Prepared::lazy(source));
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].message,
            "Prefer Jason.decode(..., keys: :atoms!) over Jason.decode(..., keys: :atoms) to avoid creating atoms at runtime."
        );
    }
    #[test]
    fn module_attributes_are_clean() {
        let source = "defmodule CredoSampleModule do\n  @test_module_attribute String.to_atom(\"foo\")\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(source)).is_empty());
    }

    #[test]
    fn broken_source_stays_clean() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("def foo( do\n")).is_empty());
    }
}
