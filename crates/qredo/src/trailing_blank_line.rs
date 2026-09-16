use crate::{Finding, Trigger};

pub(crate) fn check(source: &str) -> Vec<Finding> {
    let last_line = source.rsplit('\n').next().unwrap_or_default();
    if last_line.chars().all(is_credo_whitespace) {
        return vec![];
    }
    vec![Finding {
        line: source.bytes().filter(|byte| *byte == b'\n').count() + 1,
        column: None,
        message: "There should be a final \\n at the end of each file.".to_owned(),
        trigger: Trigger::NoTrigger,
        severity: None,
    }]
}

// Pinned Elixir String.Break uses Unicode White_Space, including nonbreaking
// spaces. Keep the set explicit so Rust Unicode upgrades cannot change results.
fn is_credo_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}
