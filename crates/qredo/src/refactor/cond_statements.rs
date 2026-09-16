use crate::Finding;

/// `EX4005`: `cond` with too few branches.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut findings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("cond do") {
            let mut branches = 0_usize;
            for l in &lines[idx + 1..(idx + 20).min(lines.len())] {
                let t = l.trim_start();
                if t.starts_with("end") {
                    break;
                }
                if t.contains("->") {
                    // Exclude `true ->`.
                    if !t.starts_with("true ->") {
                        branches += 1;
                    }
                }
            }
            if branches < 2 {
                let col = line.find("cond").unwrap_or(0) + 1;
                findings.push(Finding::with_trigger(
                    idx + 1,
                    Some(col),
                    "Cond statements should contain at least two conditions besides `true`, consider using `if` instead.",
                    "cond".to_owned(),
                ));
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rich_cond_is_clean() {
        let src = "cond do\n  x -> 1\n  y -> 2\n  true -> 3\nend\n";
        assert!(check_prepared(&crate::batch::Prepared::lazy(src)).is_empty());
    }
    #[test]
    fn reports_thin_cond() {
        let src = "cond do\n  x -> 1\n  true -> 2\nend\n";
        assert_eq!(check_prepared(&crate::batch::Prepared::lazy(src)).len(), 1);
    }
}
