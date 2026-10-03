//! Normalize the existing tree-sitter tree directly into flat descriptors.
use super::model::{Id, Shape, Summary};
use tree_sitter::Node;

pub(super) fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

pub(super) fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|n| n.kind() != "comment")
        .collect()
}

pub(super) fn lower(tree: &tree_sitter::Tree, source: &str) -> Summary {
    let mut builder = Builder {
        source,
        summary: Summary::default(),
    };
    if !tree.root_node().has_error() {
        let root = builder.build(tree.root_node());
        builder.summary.root = Some(root);
    }
    builder.summary
}

pub(super) struct Builder<'a> {
    pub source: &'a str,
    pub summary: Summary,
}

impl Builder<'_> {
    #[allow(
        clippy::too_many_lines,
        reason = "exhaustive pinned-grammar expression dispatch"
    )]
    pub(super) fn build(&mut self, node: Node<'_>) -> Id {
        let line = node.start_position().row + 1;
        match node.kind() {
            "operator_identifier" if text(node, self.source) == ".." => {
                self.summary.call("..", vec![], line)
            }
            "identifier" | "operator_identifier" => {
                self.summary.variable(text(node, self.source), line)
            }
            "atom" | "boolean" | "nil" => self
                .summary
                .atom(text(node, self.source).trim_start_matches(':')),
            "keyword" => self
                .summary
                .atom(text(node, self.source).trim_end().trim_end_matches(':')),
            "alias" => {
                let args = text(node, self.source)
                    .split('.')
                    .map(|s| self.summary.atom(s.trim()))
                    .collect();
                self.summary.call("__aliases__", args, line)
            }
            "integer" | "float" | "char" => self.number(node),
            "string" | "charlist" | "quoted_atom" | "quoted_keyword" | "sigil" => self.quoted(node),
            "pair" => {
                let parts = children(node);
                let a = self.build(parts[0]);
                let b = self.build(parts[1]);
                self.summary.pair(a, b)
            }
            "keywords" | "arguments" => {
                let items = self.items(node, false);
                self.summary.list(items)
            }
            "list" => {
                let items = self.items(node, true);
                self.summary.list(items)
            }
            "tuple" => {
                let items = self.items(node, false);
                if items.len() == 2 {
                    self.summary.pair(items[0], items[1])
                } else {
                    self.summary.call("{}", items, line)
                }
            }
            "bitstring" => {
                let items = self.items(node, false);
                self.summary.call("<<>>", items, line)
            }
            "binary_operator" => self.binary(node),
            "unary_operator" => {
                let op = node.child_by_field_name("operator").expect("operator");
                let operand = node
                    .child_by_field_name("operand")
                    .filter(Node::is_named)
                    .or_else(|| children(node).last().copied())
                    .expect("operand");
                let arg = self.build(operand);
                self.summary.call(
                    text(op, self.source),
                    vec![arg],
                    op.start_position().row + 1,
                )
            }
            "call" => self.call(node),
            "dot" => self.dot(node),
            "access_call" => {
                let parts = children(node);
                let args = parts.into_iter().map(|n| self.build(n)).collect();
                self.remote("Elixir.Access", "get", args, line)
            }
            "map" => self.map(node),
            "anonymous_function" => {
                let args = self.items(node, false);
                self.summary.call("fn", args, line)
            }
            "stab_clause" => self.stab(node),
            // Grammar wrappers carry no quoted term of their own.
            _ => self.body(children(node)),
        }
    }

    pub(super) fn items(&mut self, node: Node<'_>, flatten_keywords: bool) -> Vec<Id> {
        let mut out = Vec::new();
        for child in children(node) {
            if flatten_keywords && child.kind() == "keywords" {
                out.extend(self.items(child, false));
            } else {
                out.push(self.build(child));
            }
        }
        out
    }

    fn body(&mut self, nodes: Vec<Node<'_>>) -> Id {
        let clauses = nodes.first().is_some_and(|n| n.kind() == "stab_clause");
        let items = nodes.into_iter().map(|n| self.build(n)).collect();
        if clauses {
            self.summary.list(items)
        } else {
            self.summary.block(items)
        }
    }

    fn binary(&mut self, node: Node<'_>) -> Id {
        let op = node.child_by_field_name("operator").expect("operator");
        let left = node.child_by_field_name("left").expect("left");
        let right = node.child_by_field_name("right").expect("right");
        let name = text(op, self.source);
        let line = op.start_position().row + 1;
        if name == "//" && left.kind() == "binary_operator" {
            let mut args = self.items(left, false);
            args.push(self.build(right));
            return self.summary.call("..//", args, line);
        }
        let mut args = if left.kind() == "arguments" {
            self.items(left, false)
        } else {
            vec![self.build(left)]
        };
        args.push(self.build(right));
        match name {
            "=>" => self.summary.pair(args[0], args[1]),
            "not in" => {
                let inside = self.summary.call("in", args, line);
                self.summary.call("not", vec![inside], line)
            }
            _ => self.summary.call(name, args, line),
        }
    }

    fn call(&mut self, node: Node<'_>) -> Id {
        let target = node.child_by_field_name("target").expect("call target");
        if let Some(range) = self.signed_range(node, target) {
            return range;
        }
        let head = if target.kind() == "identifier" {
            self.summary.atom(text(target, self.source))
        } else {
            self.build(target)
        };
        let mut args = Vec::new();
        for child in children(node).into_iter().skip(1) {
            match child.kind() {
                "arguments" => args.extend(self.items(child, false)),
                "do_block" => {
                    let pairs = self.do_pairs(child);
                    args.push(self.summary.list(pairs));
                }
                _ => args.push(self.build(child)),
            }
        }
        self.summary
            .call_head(head, Some(args), target.start_position().row + 1)
    }

    /// The pinned grammar ambiguously parses `f(x..-1)` as a call to `x`
    /// with `.. - 1` as argument. Recover the range directly from its CST.
    fn signed_range(&mut self, node: Node<'_>, target: Node<'_>) -> Option<Id> {
        if target.kind() != "identifier" {
            return None;
        }
        let parts = children(node);
        let arguments = *parts.get(1)?;
        if parts.len() != 2 || arguments.kind() != "arguments" {
            return None;
        }
        let args = children(arguments);
        if args.len() != 1 {
            return None;
        }
        let expression = args[0];
        let op = expression.child_by_field_name("operator")?;
        let stepped = text(op, self.source) == "//";
        let range = if stepped {
            expression.child_by_field_name("left")?
        } else {
            expression
        };
        let sign = range.child_by_field_name("operator")?;
        if !matches!(text(sign, self.source), "+" | "-") {
            return None;
        }
        let dots = range.child_by_field_name("left")?;
        if dots.kind() != "operator_identifier" || text(dots, self.source) != ".." {
            return None;
        }
        let start = self.build(target);
        let end = self.build(range.child_by_field_name("right")?);
        let end = self.summary.call(
            text(sign, self.source),
            vec![end],
            sign.start_position().row + 1,
        );
        let mut args = vec![start, end];
        if stepped {
            args.push(self.build(expression.child_by_field_name("right")?));
        }
        Some(self.summary.call(
            if stepped { "..//" } else { ".." },
            args,
            dots.start_position().row + 1,
        ))
    }

    fn dot(&mut self, node: Node<'_>) -> Id {
        let parts = children(node);
        let left = self.build(parts[0]);
        let line = node
            .child_by_field_name("operator")
            .unwrap_or(node)
            .start_position()
            .row
            + 1;
        let Some(right) = parts.get(1).copied() else {
            return self.summary.call(".", vec![left], line);
        };
        match right.kind() {
            "alias" => {
                let mut args = match &self.summary.nodes[left].shape {
                    Shape::Call {
                        head,
                        args: Some(args),
                    } if self.summary.atom_name(*head) == Some("__aliases__") => args.clone(),
                    _ => vec![left],
                };
                args.extend(
                    text(right, self.source)
                        .split('.')
                        .map(|s| self.summary.atom(s.trim())),
                );
                self.summary
                    .call("__aliases__", args, node.start_position().row + 1)
            }
            "tuple" => {
                let name = self.summary.atom("{}");
                let head = self.summary.call(".", vec![left, name], line);
                let args = self.items(right, false);
                self.summary.call_head(head, Some(args), line)
            }
            _ => {
                let name = if matches!(right.kind(), "string" | "charlist") {
                    self.quoted_name(right)
                } else {
                    text(right, self.source).to_owned()
                };
                let right = self.summary.atom(&name);
                self.summary.call(".", vec![left, right], line)
            }
        }
    }

    fn map(&mut self, node: Node<'_>) -> Id {
        let mut structure = None;
        let mut args = Vec::new();
        for child in children(node) {
            match child.kind() {
                "struct" => structure = children(child).first().map(|n| self.build(*n)),
                "map_content" => args.extend(self.items(child, true)),
                _ => {}
            }
        }
        let line = node.start_position().row + 1;
        let map = self.summary.call("%{}", args, line);
        structure.map_or(map, |s| self.summary.call("%", vec![s, map], line))
    }

    fn stab(&mut self, node: Node<'_>) -> Id {
        let left = node.child_by_field_name("left");
        let args = match left {
            Some(n) if n.kind() == "arguments" => self.items(n, false),
            Some(n) => vec![self.build(n)],
            None => Vec::new(),
        };
        let left = self.summary.list(args);
        let right = node.child_by_field_name("right");
        let body = if let Some(n) = right {
            self.build(n)
        } else {
            self.summary.atom("nil")
        };
        let line = node
            .child_by_field_name("operator")
            .unwrap_or(node)
            .start_position()
            .row
            + 1;
        self.summary.call("->", vec![left, body], line)
    }

    fn do_pairs(&mut self, node: Node<'_>) -> Vec<Id> {
        let mut pairs = Vec::new();
        let mut body = Vec::new();
        for child in children(node) {
            if child.kind().ends_with("_block") {
                let key = self.summary.atom(child.kind().trim_end_matches("_block"));
                let value = self.body(children(child));
                pairs.push(self.summary.pair(key, value));
            } else {
                body.push(child);
            }
        }
        let key = self.summary.atom("do");
        let value = self.body(body);
        pairs.insert(0, self.summary.pair(key, value));
        pairs
    }

    pub(super) fn remote(&mut self, module: &str, fun: &str, args: Vec<Id>, line: usize) -> Id {
        let module = self.summary.atom(module);
        let fun = self.summary.atom(fun);
        let head = self.summary.call(".", vec![module, fun], line);
        self.summary.call_head(head, Some(args), line)
    }
}
