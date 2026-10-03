//! Literal normalization without evaluating code or expanding macros.
use super::lower::{Builder, children, text};
use super::model::{Id, Shape};
use tree_sitter::Node;

impl Builder<'_> {
    pub(super) fn number(&mut self, node: Node<'_>) -> Id {
        let raw = text(node, self.source);
        let shape = match node.kind() {
            "float" => Shape::Float(
                raw.replace('_', "")
                    .parse::<f64>()
                    .unwrap_or_default()
                    .to_bits(),
            ),
            "char" => {
                let bytes = unescape(&raw[1..]);
                let code = String::from_utf8_lossy(&bytes)
                    .chars()
                    .next()
                    .unwrap_or('\0') as u32;
                Shape::Integer(code.to_string())
            }
            _ => Shape::Integer(integer(raw)),
        };
        self.summary.push(shape, None)
    }

    pub(super) fn quoted_name(&self, node: Node<'_>) -> String {
        let mut bytes = Vec::new();
        for child in children(node) {
            if matches!(child.kind(), "quoted_content" | "escape_sequence") {
                bytes.extend(unescape(text(child, self.source)));
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    pub(super) fn quoted(&mut self, node: Node<'_>) -> Id {
        let line = node.start_position().row + 1;
        let is_sigil = node.kind() == "sigil";
        let is_chars = node.kind() == "charlist";
        let is_atom = matches!(node.kind(), "quoted_atom" | "quoted_keyword");
        let parts = children(node);
        let dynamic = parts.iter().any(|n| n.kind() == "interpolation");
        let (mut segments, content) = self.quoted_segments(node, &parts, dynamic || is_sigil);
        if !dynamic && !is_sigil {
            return if is_atom {
                self.summary.atom(&String::from_utf8_lossy(&content))
            } else if is_chars {
                self.charlist(&String::from_utf8_lossy(&content))
            } else {
                self.summary.push(Shape::Bytes(content), None)
            };
        }
        if segments.is_empty() {
            segments.push(self.summary.push(Shape::Bytes(Vec::new()), None));
        }
        if is_chars {
            let list = self.summary.list(segments);
            return self.remote("Elixir.List", "to_charlist", vec![list], line);
        }
        let binary = self.summary.call("<<>>", segments, line);
        if is_atom {
            let utf8 = self.summary.atom("utf8");
            return self.remote("erlang", "binary_to_atom", vec![binary, utf8], line);
        }
        if is_sigil {
            let name = parts
                .iter()
                .find(|n| n.kind() == "sigil_name")
                .map_or("", |n| text(*n, self.source));
            let modifiers = parts
                .iter()
                .find(|n| n.kind() == "sigil_modifiers")
                .map_or("", |n| text(*n, self.source));
            let modifiers = self.charlist(modifiers);
            return self
                .summary
                .call(&format!("sigil_{name}"), vec![binary, modifiers], line);
        }
        binary
    }

    fn quoted_segments(
        &mut self,
        node: Node<'_>,
        parts: &[Node<'_>],
        flush: bool,
    ) -> (Vec<Id>, Vec<u8>) {
        let delimiter = node
            .child_by_field_name("quoted_end")
            .map_or("", |n| text(n, self.source));
        let indent = node
            .child_by_field_name("quoted_end")
            .filter(|_| delimiter.len() == 3)
            .map(|n| n.start_position().column);
        let mut content = Vec::new();
        let mut touched = false;
        let mut segments = Vec::new();
        for child in parts {
            match child.kind() {
                "quoted_content" | "escape_sequence" => {
                    touched = true;
                    let raw = dedent(text(*child, self.source), *child, node, self.source, indent);
                    content.extend(if node.kind() == "sigil" {
                        raw.into_bytes()
                    } else {
                        unescape(&raw)
                    });
                }
                "interpolation" => {
                    self.flush_segment(
                        (node.kind() == "sigil").then_some(delimiter),
                        &mut content,
                        &mut segments,
                        touched,
                    );
                    touched = false;
                    segments.push(self.interpolation(*child, node.kind() == "charlist"));
                }
                _ => {}
            }
        }
        if flush {
            self.flush_segment(
                (node.kind() == "sigil").then_some(delimiter),
                &mut content,
                &mut segments,
                touched,
            );
        }
        (segments, content)
    }

    fn interpolation(&mut self, child: Node<'_>, charlist: bool) -> Id {
        let expressions = children(child).into_iter().map(|n| self.build(n)).collect();
        let expression = self.summary.block(expressions);
        let line = child.start_position().row + 1;
        let string = self.remote("Elixir.Kernel", "to_string", vec![expression], line);
        if charlist {
            string
        } else {
            let binary = self.summary.variable("binary", line);
            self.summary.call("::", vec![string, binary], line)
        }
    }

    fn flush_segment(
        &mut self,
        sigil_delimiter: Option<&str>,
        content: &mut Vec<u8>,
        segments: &mut Vec<Id>,
        touched: bool,
    ) {
        if let Some(delimiter) = sigil_delimiter {
            *content = sigil_bytes(&String::from_utf8_lossy(content), delimiter);
        }
        if touched {
            segments.push(
                self.summary
                    .push(Shape::Bytes(std::mem::take(content)), None),
            );
        }
    }

    fn charlist(&mut self, value: &str) -> Id {
        let chars = value
            .chars()
            .map(|c| {
                self.summary
                    .push(Shape::Integer((c as u32).to_string()), None)
            })
            .collect();
        self.summary.list(chars)
    }
}

fn sigil_bytes(raw: &str, delimiter: &str) -> Vec<u8> {
    raw.replace(&format!("\\{delimiter}"), delimiter)
        .into_bytes()
}

fn dedent(
    raw: &str,
    child: Node<'_>,
    parent: Node<'_>,
    source: &str,
    indent: Option<usize>,
) -> String {
    let Some(indent) = indent else {
        return raw.replace("\r\n", "\n");
    };
    let start = parent
        .child_by_field_name("quoted_start")
        .map_or(parent.start_byte(), |n| n.end_byte());
    let mut out = String::new();
    let mut column = child.start_position().column;
    for (offset, ch) in raw.char_indices() {
        let pos = child.start_byte() + offset;
        if ch == '\r' && source.as_bytes().get(pos + 1) == Some(&b'\n') {
            continue;
        }
        if ch == '\n' {
            if pos > start + usize::from(source.as_bytes().get(start) == Some(&b'\r')) {
                out.push(ch);
            }
            column = 0;
        } else {
            if column >= indent || !matches!(ch, ' ' | '\t') {
                out.push(ch);
            }
            column += 1;
        }
    }
    out
}

fn integer(raw: &str) -> String {
    let clean = raw.replace('_', "");
    let (radix, digits) = if let Some(s) = clean.strip_prefix("0x") {
        (16, s)
    } else if let Some(s) = clean.strip_prefix("0b") {
        (2, s)
    } else if let Some(s) = clean.strip_prefix("0o") {
        (8, s)
    } else {
        let digits = clean.trim_start_matches('0');
        return if digits.is_empty() { "0" } else { digits }.to_owned();
    };
    // Decimal limbs support arbitrarily large Elixir integer literals.
    let mut decimal = vec![0_u32];
    for ch in digits.chars() {
        let mut carry = ch.to_digit(radix).unwrap_or(0);
        for digit in &mut decimal {
            let value = *digit * radix + carry;
            *digit = value % 10;
            carry = value / 10;
        }
        while carry > 0 {
            decimal.push(carry % 10);
            carry /= 10;
        }
    }
    decimal
        .into_iter()
        .rev()
        .filter_map(|n| char::from_digit(n, 10))
        .collect()
}

#[allow(
    clippy::too_many_lines,
    reason = "Elixir escape table and bounded hex decoding"
)]
fn unescape(raw: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            push_char(&mut out, ch);
            continue;
        }
        let Some(ch) = chars.next() else {
            out.push(b'\\');
            break;
        };
        match ch {
            '\n' => {}
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
            }
            '0' => out.push(0),
            'a' => out.push(7),
            'b' => out.push(8),
            'd' => out.push(127),
            'e' => out.push(27),
            'f' => out.push(12),
            'n' => out.push(10),
            'r' => out.push(13),
            's' => out.push(32),
            't' => out.push(9),
            'v' => out.push(11),
            'x' | 'u' => {
                let braced = chars.peek() == Some(&'{');
                if braced {
                    chars.next();
                }
                let mut value = 0;
                let limit = if braced {
                    8
                } else if ch == 'x' {
                    2
                } else {
                    4
                };
                for _ in 0..limit {
                    let Some(digit) = chars.peek().and_then(|c| c.to_digit(16)) else {
                        break;
                    };
                    chars.next();
                    value = value * 16 + digit;
                }
                if braced && chars.peek() == Some(&'}') {
                    chars.next();
                }
                if ch == 'x' && !braced {
                    out.push(u8::try_from(value).unwrap_or(0));
                } else if let Some(c) = char::from_u32(value) {
                    push_char(&mut out, c);
                }
            }
            _ => push_char(&mut out, ch),
        }
    }
    out
}

fn push_char(out: &mut Vec<u8>, ch: char) {
    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
}
