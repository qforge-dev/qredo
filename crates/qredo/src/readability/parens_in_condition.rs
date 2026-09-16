use crate::Finding;

/// `EX3013`: conditions of `if`/`unless` should not wrap in parens.
///
/// Mirrors the pinned token walk: an `if`/`unless` identifier directly
/// followed by `(` is an issue unless the parenthesized group continues with
/// an operator (comparison, boolean, arithmetic, `in`, ...) or the group is a
/// call holding its own `do:` (`if(foo, do: ...)`). A piped `|> if(` call is
/// also clean.
pub(crate) fn check_prepared(prepared: &crate::batch::Prepared<'_>) -> Vec<Finding> {
    let masked = prepared.masked();
    let tokens = tokenize(masked);
    let mut findings = Vec::new();
    for (n, tok) in tokens.iter().enumerate() {
        let (kw, paren_form) = match &tok.kind {
            Kind::Kw(kw) => (kw.clone(), false),
            Kind::ParenKw(kw) => (kw.clone(), true),
            _ => continue,
        };
        let next = tokens.get(n + 1).map(|t| &t.kind);
        if next != Some(&Kind::Open) {
            continue;
        }
        let prev = if n == 0 {
            None
        } else {
            Some(&tokens[n - 1].kind)
        };
        if paren_form && prev == Some(&Kind::Arrow) {
            continue;
        }
        let found = if paren_form {
            !paren_children_have_do(&tokens[n + 2..])
        } else {
            closes_like_wrapped(&tokens, n)
        };
        if found {
            findings.push(Finding::with_trigger(
                tok.line,
                Some(tok.col),
                format!("The condition of `{kw}` should not be wrapped in parentheses."),
                kw,
            ));
        }
    }
    findings.sort_by_key(|f| (f.line, f.column.unwrap_or(0)));
    findings
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum Kind {
    Kw(String),
    ParenKw(String),
    Open,
    Close,
    Comma,
    Do,
    Arrow,
    CompOp,
    OrOp,
    AndOp,
    InOp,
    MultOp,
    DualOp,
    RelOp,
    Other,
}

struct Token {
    kind: Kind,
    line: usize,
    col: usize,
}

/// Minimal Elixir-ish tokenizer over masked source. Strings and comments are
/// already blanked, so only code shapes matter; byte columns are preserved.
fn tokenize(masked: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    for (idx, line) in masked.split('\n').enumerate() {
        Lexer {
            line,
            line_no: idx + 1,
            tokens: &mut tokens,
        }
        .scan_line();
    }
    tokens
}

struct Lexer<'a> {
    line: &'a str,
    line_no: usize,
    tokens: &'a mut Vec<Token>,
}

impl Lexer<'_> {
    fn scan_line(&mut self) {
        let mut i = 0_usize;
        while i < self.line.len() {
            let c = self.line[i..].chars().next().unwrap_or(' ');
            if c.is_whitespace() {
                i += c.len_utf8();
            } else if c.is_ascii_alphabetic() || c == '_' {
                i = self.scan_word(i);
            } else if c.is_ascii_digit() {
                i = self.scan_number(i);
            } else {
                i = self.scan_symbol(i, c);
            }
        }
    }

    fn scan_word(&mut self, start: usize) -> usize {
        let line = self.line;
        let mut j = start;
        while let Some(w) = line[j..].chars().next() {
            if !(w.is_alphanumeric() || w == '_') {
                break;
            }
            j += w.len_utf8();
        }
        let mut end = j;
        if line[j..]
            .chars()
            .next()
            .is_some_and(|m| m == '?' || m == '!')
        {
            end += line[j..].chars().next().map_or(0, char::len_utf8);
        }
        let word = line[start..end].to_owned();
        self.word_kind(start, word, end)
    }

    /// Classify a word token; returns the next byte offset.
    fn word_kind(&mut self, start: usize, word: String, end: usize) -> usize {
        let col = start + 1;
        let preceded = self.line[..start].chars().next_back();
        let atom_like = preceded.is_some_and(|p| p == ':' || p == '.' || p == '@');
        if !atom_like && (word == "if" || word == "unless") {
            let paren = self.line[end..].starts_with('(');
            self.tokens.push(Token {
                kind: if paren {
                    Kind::ParenKw(word)
                } else {
                    Kind::Kw(word)
                },
                line: self.line_no,
                col,
            });
            return end;
        }
        if word == "do" {
            // `do:` keywords count as `do` for the paren-children check.
            let mut stop = end;
            if self.line[end..].starts_with(':') && !self.line[end..].starts_with("::") {
                stop += 1;
            }
            self.tokens.push(Token {
                kind: Kind::Do,
                line: self.line_no,
                col,
            });
            return stop;
        }
        let kind = match word.as_str() {
            "or" => Kind::OrOp,
            "and" => Kind::AndOp,
            "in" => Kind::InOp,
            _ => Kind::Other,
        };
        self.tokens.push(Token {
            kind,
            line: self.line_no,
            col,
        });
        end
    }

    fn scan_number(&mut self, start: usize) -> usize {
        let line = self.line;
        let mut j = start;
        while let Some(w) = line[j..].chars().next() {
            if !(w.is_alphanumeric() || w == '_' || w == '.') {
                break;
            }
            j += w.len_utf8();
        }
        self.tokens.push(Token {
            kind: Kind::Other,
            line: self.line_no,
            col: start + 1,
        });
        j
    }

    /// Scan one symbol at byte offset `i`; returns the next byte offset.
    fn scan_symbol(&mut self, i: usize, c: char) -> usize {
        let line = self.line;
        let rest = &line[i..];
        let two: String = rest.chars().take(2).collect();
        let three: String = rest.chars().take(3).collect();
        if three == "===" || three == "!==" {
            self.push(Kind::CompOp, i);
            return i + 3;
        }
        if ["|>>", "<<<", ">>>", "&&&", "|||", "<<~", "~>>", "<|>"].contains(&three.as_str()) {
            self.push(Kind::RelOp, i);
            return i + 3;
        }
        if two == "|>" {
            self.push(Kind::Arrow, i);
            return i + 2;
        }
        if ["==", "!=", "=~", "<=", ">="].contains(&two.as_str()) {
            self.push(Kind::CompOp, i);
            return i + 2;
        }
        if two == "||" {
            self.push(Kind::OrOp, i);
            return i + 2;
        }
        if two == "&&" {
            self.push(Kind::AndOp, i);
            return i + 2;
        }
        if two == "++" || two == "--" {
            self.push(Kind::DualOp, i);
            return i + 2;
        }
        if ["<-", "->", "::", "=>"].contains(&two.as_str()) {
            return i + 2;
        }
        match c {
            '(' => self.push(Kind::Open, i),
            ')' => self.push(Kind::Close, i),
            ',' => self.push(Kind::Comma, i),
            '<' | '>' => self.push(Kind::CompOp, i),
            '+' | '-' => self.push(Kind::DualOp, i),
            '*' | '/' => self.push(Kind::MultOp, i),
            _ => {}
        }
        i + c.len_utf8()
    }

    fn push(&mut self, kind: Kind, i: usize) {
        self.tokens.push(Token {
            kind,
            line: self.line_no,
            col: i + 1,
        });
    }
}

/// `if (...)`: step token by token from the opening paren; a wrapped
/// condition ends with `)` directly before `do` or `,`.
fn closes_like_wrapped(tokens: &[Token], kw: usize) -> bool {
    let mut cur = kw + 1;
    let mut prev: Option<&Kind> = if kw == 0 {
        None
    } else {
        Some(&tokens[kw - 1].kind)
    };
    while cur < tokens.len() {
        let kind = &tokens[cur].kind;
        let next = tokens.get(cur + 1).map(|t| &t.kind);
        if *kind == Kind::Do && prev == Some(&Kind::Close) {
            return true;
        }
        if *kind == Kind::Close && next.is_some_and(is_continuing_op) {
            return false;
        }
        if *kind == Kind::Comma && prev == Some(&Kind::Close) {
            return true;
        }
        if matches!(kind, Kind::OrOp | Kind::AndOp | Kind::CompOp) && next == Some(&Kind::Open) {
            return false;
        }
        prev = Some(kind);
        cur += 1;
    }
    false
}

fn is_continuing_op(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::CompOp
            | Kind::OrOp
            | Kind::AndOp
            | Kind::InOp
            | Kind::MultOp
            | Kind::DualOp
            | Kind::RelOp
    )
}

/// `if(...)`: collect top-level children of the paren group; a `do` among
/// them means the parens belong to a call, not a wrapped condition.
fn paren_children_have_do(after_open: &[Token]) -> bool {
    let mut depth = 0_i32;
    for tok in after_open {
        match tok.kind {
            Kind::Open => depth += 1,
            Kind::Close => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            Kind::Do if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_condition() {
        assert!(check_prepared(&crate::batch::Prepared::lazy("if x, do: y\n")).is_empty());
    }
    #[test]
    fn reports_parens() {
        assert_eq!(
            check_prepared(&crate::batch::Prepared::lazy("if (x), do: y\n")).len(),
            1
        );
    }
    #[test]
    fn piped_call_and_inner_parens_are_clean() {
        // `if(` with `do:` inside its own parens is a call, not a wrapped condition.
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "if(valid?(u), do: [:a]) ++ unless(admin?(u), do: [:r])\n"
            ))
            .is_empty()
        );
        // Piping into `if(do: ...)` is a call.
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "boolean |> if(do: :ok, else: :error)\n"
            ))
            .is_empty()
        );
        // Parenthesized subexpression followed by an operator is not wrapping.
        assert!(
            check_prepared(&crate::batch::Prepared::lazy(
                "if (a + b) / 100 > t(), do: :h, else: :l\n"
            ))
            .is_empty()
        );
    }
    #[test]
    fn no_space_paren_reports() {
        let out = check_prepared(&crate::batch::Prepared::lazy(
            "defmodule M do\n  def r(a) do\n    if( allowed? ) do\n      true\n    end\n  end\nend\n",
        ));
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].line, out[0].column), (3, Some(5)));
    }
}
