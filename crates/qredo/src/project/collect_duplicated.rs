//! Project-level `DuplicatedCode` (EX2002).
//!
//! Ports the upstream duplicate detection over an Elixir-quoted-AST model
//! built from the pinned tree-sitter grammar: mass counts tuple nodes with
//! metadata excluded, structural identity decides duplication, and per-file
//! issues name the other files. Encounter order is kept (upstream chunk
//! streaming is nondeterministic across chunks, but corpora stay in one).
//!
//! Known approximations (none exercised by the corpus): string interpolation
//! is folded into the string value; sigils, maps, structs, bitstrings, `fn`
//! and unknown constructs use generic tagged shapes; multi-line constructs
//! take CST start lines where metadata lines could differ; unknown
//! `compare_to`-style params do not exist here (only thresholds and excluded
//! macros); an empty project yields no findings.

use sha2::Digest as _;
use std::collections::{BTreeMap, HashMap};

use super::{ProjectFile, ProjectIssue};
use crate::helpers;

/// Default thresholds mirroring the upstream parameter defaults.
const DEFAULT_MASS_THRESHOLD: usize = 40;
const DEFAULT_NODES_THRESHOLD: usize = 2;
/// Prune pass always uses the default threshold, mirroring upstream.
const PRUNE_MASS_THRESHOLD: usize = 40;

/// Quoted-AST model (metadata excluded). Calls, vars and aliases are plain
/// tuples like upstream (`{form, meta, args}` minus meta); pairs model
/// 2-tuples and keyword entries.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    /// Atom by raw name.
    Atom(String),
    /// String value with standard escapes interpreted.
    Str(String),
    /// Number literal, raw text (`1` and `1.0` differ).
    Num(String),
    /// Any tuple.
    Tuple(Vec<Node>),
    /// Proper list.
    List(Vec<Node>),
}

/// Model node with an optional source line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    term: Term,
    line: Option<usize>,
}

/// One recorded candidate: owning file, node and mass.
struct Candidate {
    file: usize,
    node: Node,
    mass: usize,
}

/// Run the check over a file set.
pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    let nones = vec![None; files.len()];
    run_with_trees(files, &nones, params)
}

/// Run the check reusing prepare-phase trees: `trees` aligns with
/// `files`, `None` entries parse normally. The pipeline shares one
/// parse per file instead of paying a fresh parse per check.
pub(crate) fn run_with_trees(
    files: &[ProjectFile],
    trees: &[Option<&tree_sitter::Tree>],
    params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    let mass_threshold = helpers::param_usize(params, "mass_threshold", DEFAULT_MASS_THRESHOLD);
    let nodes_threshold = helpers::param_usize(params, "nodes_threshold", DEFAULT_NODES_THRESHOLD);
    let excluded = excluded_macros(params);
    let mut hashes = collect_candidates_with_trees(files, trees, mass_threshold);
    prune_candidates(&mut hashes);
    emit_issues(files, &hashes, nodes_threshold, &excluded)
}

/// Per-file candidates reusing shared trees where available.
fn collect_candidates_with_trees(
    files: &[ProjectFile],
    trees: &[Option<&tree_sitter::Tree>],
    mass_threshold: usize,
) -> HashMap<String, Vec<Candidate>> {
    let mut hashes: HashMap<String, Vec<Candidate>> = HashMap::new();
    for (index, file) in files.iter().enumerate() {
        let root = match trees.get(index).copied().flatten() {
            Some(tree) => model_tree(tree, &file.source),
            None => model_file(&file.source),
        };
        let Some(root) = root else {
            continue;
        };
        for node in preorder(&root) {
            let mass = mass(&node);
            if mass < mass_threshold {
                continue;
            }
            let hash = digest(&node);
            hashes.entry(hash).or_default().push(Candidate {
                file: index,
                node,
                mass,
            });
        }
    }
    hashes
}

/// Keep only multi-node hashes, then prune subhashes of each survivor.
fn prune_candidates(hashes: &mut HashMap<String, Vec<Candidate>>) {
    hashes.retain(|_, items| items.len() > 1);
    let mut doomed: Vec<String> = Vec::new();
    let keys: Vec<String> = hashes.keys().cloned().collect();
    for key in &keys {
        let Some(first) = hashes.get(key).and_then(|items| items.first()) else {
            continue;
        };
        let mut sub: HashMap<String, Vec<Candidate>> = HashMap::new();
        collect_hashes(&first.node, &mut sub, PRUNE_MASS_THRESHOLD);
        for subkey in sub.keys() {
            if subkey != key {
                doomed.push(subkey.clone());
            }
        }
    }
    for key in doomed {
        hashes.remove(&key);
    }
}

/// One issue per file in each surviving hash.
fn emit_issues(
    files: &[ProjectFile],
    hashes: &HashMap<String, Vec<Candidate>>,
    nodes_threshold: usize,
    excluded: &[String],
) -> Vec<ProjectIssue> {
    let mut issues = Vec::new();
    for items in hashes.values() {
        let mut present: Vec<usize> = Vec::new();
        for item in items {
            if !present.contains(&item.file) {
                present.push(item.file);
            }
        }
        for file in present {
            if let Some(issue) = file_issue(files, items, file, nodes_threshold, excluded) {
                issues.push(issue);
            }
        }
    }
    issues
}

/// Issue for one file in a surviving hash, if it qualifies.
fn file_issue(
    files: &[ProjectFile],
    items: &[Candidate],
    file: usize,
    nodes_threshold: usize,
    excluded: &[String],
) -> Option<ProjectIssue> {
    let this = items.iter().find(|item| item.file == file)?;
    let others: Vec<&Candidate> = items.iter().filter(|item| item.file != file).collect();
    if others.len() + 1 < nodes_threshold {
        return None;
    }
    let line = line_of(&this.node)?;
    if !create_issue(&this.node, excluded) {
        return None;
    }
    let filenames: Vec<String> = others
        .iter()
        .map(|other| {
            let name = files
                .get(other.file)
                .map_or("", |file| file.filename.as_str());
            format!("{}:{}", name, line_of(&other.node).unwrap_or(0))
        })
        .collect();
    #[allow(
        clippy::cast_precision_loss,
        reason = "duplicate node counts never approach 2^53"
    )]
    let other_nodes = others.len() as f64;
    Some(ProjectIssue {
        file,
        line: Some(line),
        column: None,
        trigger: "no_trigger".to_owned(),
        message: format!(
            "Duplicate code found in {} (mass: {}).",
            filenames.join(", "),
            this.mass
        ),
        severity: Some(1.0 + other_nodes),
    })
}

/// Pre-order model walk mirroring `Macro.prewalk` visitation: every tuple
/// and list node is visited, except a call's argument list (calls are
/// destructured into head and argument elements, exactly like upstream).
fn preorder(root: &Node) -> Vec<Node> {
    fn visit(node: &Node, out: &mut Vec<Node>) {
        if let Term::Tuple(items) = &node.term
            && let [head, meta, args] = items.as_slice()
            && is_atom(head)
            && is_list(meta)
        {
            out.push(node.clone());
            visit(head, out);
            if let Term::List(elements) = &args.term {
                for element in elements {
                    visit(element, out);
                }
            }
            return;
        }
        out.push(node.clone());
        if let Term::Tuple(items) | Term::List(items) = &node.term {
            for item in items {
                visit(item, out);
            }
        }
    }
    fn is_atom(node: &Node) -> bool {
        matches!(node.term, Term::Atom(_))
    }
    fn is_list(node: &Node) -> bool {
        matches!(node.term, Term::List(_))
    }
    let mut order = Vec::new();
    visit(root, &mut order);
    order
}

/// Collect candidates of one model subtree at or above `threshold`.
fn collect_hashes(node: &Node, hashes: &mut HashMap<String, Vec<Candidate>>, threshold: usize) {
    for node in preorder(node) {
        let mass = mass(&node);
        if mass < threshold {
            continue;
        }
        let hash = digest(&node);
        hashes.entry(hash).or_default().push(Candidate {
            file: 0,
            node,
            mass,
        });
    }
}

/// Excluded macro atoms from params (corpus atoms arrive colon-marked).
fn excluded_macros(params: &BTreeMap<String, String>) -> Vec<String> {
    let Some(raw) = params.get("excluded_macros") else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<serde_json::Value>>(raw)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| match value {
                    serde_json::Value::String(name) => {
                        Some(name.trim_start_matches(':').to_owned())
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a candidate root reports, mirroring `create_issue?/2`.
fn create_issue(node: &Node, excluded: &[String]) -> bool {
    match &node.term {
        Term::Tuple(items) => match items.as_slice() {
            [head, _, _] => match &head.term {
                Term::Atom(name) if name == "@" => false,
                Term::Atom(name) => !excluded.iter().any(|item| item == name),
                _ => true,
            },
            [key, value] => match &key.term {
                Term::Atom(key) if key == "do" => create_issue(value, excluded),
                _ => true,
            },
            _ => true,
        },
        _ => true,
    }
}

/// Source line for a candidate root, mirroring `line_no_for/1`.
fn line_of(node: &Node) -> Option<usize> {
    match &node.term {
        Term::Tuple(items) => match items.as_slice() {
            [head, _, _] => match &head.term {
                Term::Atom(name) if name == "__block__" => None,
                Term::Atom(_) => node.line,
                _ => None,
            },
            [key, value] => match &key.term {
                Term::Atom(key) if key == "do" => line_of(value),
                _ => None,
            },
            _ => None,
        },
        Term::List(items) => items.iter().find_map(line_of),
        _ => None,
    }
}

/// Tuple count with metadata excluded: every `Tuple` node counts one.
fn mass(node: &Node) -> usize {
    let mut count = 0_usize;
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        match &node.term {
            Term::Tuple(items) => {
                count += 1;
                stack.extend(items.iter());
            }
            Term::List(items) => stack.extend(items.iter()),
            Term::Atom(_) | Term::Str(_) | Term::Num(_) => {}
        }
    }
    count
}

/// Canonical structural serialization (equality classes only, length-framed).
fn digest(node: &Node) -> String {
    use std::fmt::Write as _;
    fn render(node: &Node, out: &mut String) {
        match &node.term {
            Term::Atom(name) => {
                let _ = write!(out, "a{}:{name}", name.len());
            }
            Term::Str(text) => {
                let _ = write!(out, "s{}:{text}", text.len());
            }
            Term::Num(text) => {
                let _ = write!(out, "n{}:{text}", text.len());
            }
            Term::Tuple(items) => {
                let _ = write!(out, "t{}:[", items.len());
                for item in items {
                    render(item, out);
                }
                out.push(']');
            }
            Term::List(items) => {
                let _ = write!(out, "l{}:[", items.len());
                for item in items {
                    render(item, out);
                }
                out.push(']');
            }
        }
    }
    let mut rendered = String::new();
    render(node, &mut rendered);
    sha2::Sha256::digest(rendered.as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Build the model root for one source; `None` on parse failure or error trees.
fn model_file(source: &str) -> Option<Node> {
    let tree = crate::ts_parser::parse(source)?;
    model_tree(&tree, source)
}

/// Build the model root over an already-parsed tree; `None` on error trees.
fn model_tree(tree: &tree_sitter::Tree, source: &str) -> Option<Node> {
    if tree.root_node().has_error() {
        return None;
    }
    let mut statements = Vec::new();
    let mut cursor = tree.root_node().walk();
    for child in tree.root_node().children(&mut cursor) {
        if !child.is_named() || child.kind() == "comment" {
            continue;
        }
        statements.push(build_term(&child, source));
    }
    match statements.len() {
        0 => None,
        1 => statements.into_iter().next(),
        _ => {
            let line = statements.iter().find_map(|node| node.line);
            let block = Node {
                term: Term::Atom("__block__".to_owned()),
                line: None,
            };
            let meta = Node {
                term: Term::List(Vec::new()),
                line: None,
            };
            Some(Node {
                term: Term::Tuple(vec![
                    block,
                    meta,
                    Node {
                        term: Term::List(statements),
                        line: None,
                    },
                ]),
                line,
            })
        }
    }
}

/// Text of a node.
fn text<'src>(node: &tree_sitter::Node<'_>, source: &'src str) -> &'src str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

/// 1-based line of a tree-sitter node.
fn node_line(node: &tree_sitter::Node<'_>) -> usize {
    node.start_position().row + 1
}

/// Atom term for identifier-like text.
fn atom_term(text: &str) -> Node {
    Node {
        term: Term::Atom(text.to_owned()),
        line: None,
    }
}

/// Empty argument/meta list term.
fn empty_list() -> Node {
    Node {
        term: Term::List(Vec::new()),
        line: None,
    }
}

/// Named non-comment children as terms.
fn named_terms(parent: &tree_sitter::Node<'_>, source: &str) -> Vec<Node> {
    let mut cursor = parent.walk();
    parent
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
        .map(|child| build_term(&child, source))
        .collect()
}

/// Build a term for one CST node.
fn build_term(node: &tree_sitter::Node<'_>, source: &str) -> Node {
    let line = node_line(node);
    match node.kind() {
        "call" => build_call(node, source, line),
        "dot" => dot_term(node, source, line),
        "binary_operator" => build_binary(node, source, line),
        "unary_operator" => build_unary(node, source, line),
        "map" => build_map(node, source, line),
        "pair" => build_pair(node, source),
        "tuple" => Node {
            term: Term::Tuple(named_terms(node, source)),
            line: None,
        },
        "arguments" | "keywords" | "list" => Node {
            term: Term::List(named_terms(node, source)),
            line: None,
        },
        "keyword" => Node {
            term: Term::Atom(text(node, source).trim_end_matches(':').to_owned()),
            line: None,
        },
        _ => build_scalar(node, source),
    }
}

/// Leaves and names: identifiers, aliases, atoms, numbers, strings.
fn build_scalar(node: &tree_sitter::Node<'_>, source: &str) -> Node {
    let line = node_line(node);
    match node.kind() {
        "identifier" => Node {
            term: Term::Tuple(vec![
                atom_term(text(node, source)),
                empty_list(),
                atom_term("Elixir"),
            ]),
            line: Some(line),
        },
        "alias" => {
            let segments: Vec<Node> = text(node, source)
                .split('.')
                .map(|segment| Node {
                    term: Term::Atom(segment.to_owned()),
                    line: None,
                })
                .collect();
            Node {
                term: Term::Tuple(vec![
                    atom_term("__aliases__"),
                    empty_list(),
                    Node {
                        term: Term::List(segments),
                        line: None,
                    },
                ]),
                line: Some(line),
            }
        }
        _ => build_literal(node, source),
    }
}

/// Atoms, numbers, strings and charlists.
fn build_literal(node: &tree_sitter::Node<'_>, source: &str) -> Node {
    let line = node_line(node);
    match node.kind() {
        "atom" | "boolean" | "nil" => Node {
            term: Term::Atom(text(node, source).trim_start_matches(':').to_owned()),
            line: None,
        },
        "integer" | "float" => Node {
            term: Term::Num(text(node, source).to_owned()),
            line: None,
        },
        "string" => Node {
            term: Term::Str(string_value(node, source)),
            line: None,
        },
        "charlist" => Node {
            term: Term::List(
                string_value(node, source)
                    .chars()
                    .map(|character| Node {
                        term: Term::Num((character as u32).to_string()),
                        line: None,
                    })
                    .collect(),
            ),
            line: None,
        },
        _ => Node {
            term: Term::Tuple(vec![
                Node {
                    term: Term::Atom(format!("node:{}", node.kind())),
                    line: None,
                },
                empty_list(),
                Node {
                    term: Term::List(named_terms(node, source)),
                    line: None,
                },
            ]),
            line: Some(line),
        },
    }
}

/// Binary operator call with field children falling back to position.
fn build_binary(node: &tree_sitter::Node<'_>, source: &str, line: usize) -> Node {
    let mut cursor = node.walk();
    let mut parts = node
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment");
    let left = parts.next().map(|left| build_term(&left, source));
    let right = parts.next().map(|right| build_term(&right, source));
    let mut elements = vec![atom_term(&operator_text(node, source)), empty_list()];
    if let Some(left) = left {
        elements.push(left);
    }
    if let Some(right) = right {
        elements.push(right);
    }
    Node {
        term: Term::Tuple(elements),
        line: Some(line),
    }
}

/// Unary operator call; the operator is the first anonymous child.
fn build_unary(node: &tree_sitter::Node<'_>, source: &str, line: usize) -> Node {
    let mut cursor = node.walk();
    let op = node
        .children(&mut cursor)
        .find(|child| !child.is_named())
        .map(|child| text(&child, source).to_owned())
        .unwrap_or_default();
    let mut cursor = node.walk();
    let mut operands = Vec::new();
    for part in node
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
    {
        operands.push(build_term(&part, source));
    }
    let mut quoted = vec![atom_term(&op), empty_list()];
    quoted.push(Node {
        term: Term::List(operands),
        line: None,
    });
    Node {
        term: Term::Tuple(quoted),
        line: Some(line),
    }
}

/// Keyword pair with `keyword` keys unwrapped to atoms.
fn build_pair(node: &tree_sitter::Node<'_>, source: &str) -> Node {
    let mut cursor = node.walk();
    let mut parts = node
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment");
    let key = parts.next().map(|key| {
        if key.kind() == "keyword" {
            Node {
                term: Term::Atom(text(&key, source).trim_end_matches(':').to_owned()),
                line: None,
            }
        } else {
            build_term(&key, source)
        }
    });
    let value = parts.next().map(|value| build_term(&value, source));
    let mut elements = Vec::new();
    if let Some(key) = key {
        elements.push(key);
    }
    if let Some(value) = value {
        elements.push(value);
    }
    Node {
        term: Term::Tuple(elements),
        line: None,
    }
}

fn dot_term(node: &tree_sitter::Node<'_>, source: &str, line: usize) -> Node {
    let mut cursor = node.walk();
    let mut parts = node
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment");
    let left = parts.next().map(|left| build_term(&left, source));
    let right = parts.next().map(|right| Node {
        term: Term::Atom(text(&right, source).to_owned()),
        line: None,
    });
    let mut elements = vec![atom_term("."), empty_list()];
    if let Some(left) = left {
        elements.push(left);
    }
    if let Some(right) = right {
        elements.push(right);
    }
    Node {
        term: Term::Tuple(elements),
        line: Some(line),
    }
}

/// Call head term for a `call` target child.
fn head_term(node: &tree_sitter::Node<'_>, source: &str) -> Node {
    match node.kind() {
        "identifier" => atom_term(text(node, source)),
        _ => build_term(node, source),
    }
}

/// Build a `call` node: head plus argument terms with `do` pairs appended.
fn build_call(node: &tree_sitter::Node<'_>, source: &str, line: usize) -> Node {
    let mut cursor = node.walk();
    let mut children = node
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment");
    let head = children.next().map(|head| head_term(&head, source));
    let mut args: Vec<Node> = Vec::new();
    let mut blocks: Vec<Node> = Vec::new();
    for child in children {
        match child.kind() {
            "arguments" => {
                let mut inner = child.walk();
                for argument in child
                    .children(&mut inner)
                    .filter(|child| child.is_named() && child.kind() != "comment")
                {
                    args.push(build_term(&argument, source));
                }
            }
            "do_block" => blocks.extend(do_pairs(&child, source)),
            _ => args.push(build_term(&child, source)),
        }
    }
    if !blocks.is_empty() {
        args.push(Node {
            term: Term::List(blocks),
            line: None,
        });
    }
    let mut elements = Vec::new();
    if let Some(head) = head {
        elements.push(head);
    }
    elements.push(empty_list());
    elements.push(Node {
        term: Term::List(args),
        line: None,
    });
    Node {
        term: Term::Tuple(elements),
        line: Some(line),
    }
}

/// `%{...}` maps as `{:%{}, _, args}` and structs with a `%` head.
fn build_map(node: &tree_sitter::Node<'_>, source: &str, line: usize) -> Node {
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node<'_>> = node
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
        .collect();
    let alias = map_alias(&children, source);
    let (pairs, others) = map_contents(&children, source);
    let mut args = others;
    args.push(Node {
        term: Term::List(pairs),
        line: None,
    });
    let inner = Node {
        term: Term::Tuple(vec![
            atom_term("%{}"),
            empty_list(),
            Node {
                term: Term::List(args),
                line: None,
            },
        ]),
        line: Some(line),
    };
    match alias {
        None => inner,
        Some(alias) => Node {
            term: Term::Tuple(vec![
                atom_term("%"),
                empty_list(),
                Node {
                    term: Term::List(vec![alias, inner]),
                    line: None,
                },
            ]),
            line: Some(line),
        },
    }
}

/// Alias of a `%Struct{...}` map, if present.
fn map_alias(children: &[tree_sitter::Node<'_>], source: &str) -> Option<Node> {
    children
        .iter()
        .find(|child| child.kind() == "struct")
        .and_then(|item| {
            let mut inner = item.walk();
            item.children(&mut inner)
                .find(|child| child.kind() == "alias")
                .map(|alias| build_term(&alias, source))
        })
}

/// Keyword pairs and other content terms of a map's content blocks.
fn map_contents(children: &[tree_sitter::Node<'_>], source: &str) -> (Vec<Node>, Vec<Node>) {
    let mut pairs: Vec<Node> = Vec::new();
    let mut others: Vec<Node> = Vec::new();
    for content in children
        .iter()
        .filter(|child| child.kind() == "map_content")
    {
        let mut inner = content.walk();
        for child in content
            .children(&mut inner)
            .filter(|child| child.is_named() && child.kind() != "comment")
        {
            if child.kind() == "keywords" {
                let mut inner = child.walk();
                for pair in child
                    .children(&mut inner)
                    .filter(|child| child.kind() == "pair")
                {
                    pairs.push(build_term(&pair, source));
                }
            } else {
                others.push(build_term(&child, source));
            }
        }
    }
    (pairs, others)
}
fn do_pairs(node: &tree_sitter::Node<'_>, source: &str) -> Vec<Node> {
    let mut body: Vec<Node> = Vec::new();
    let mut or_else: Vec<Node> = Vec::new();
    let mut cursor = node.walk();
    for child in node
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
    {
        match child.kind() {
            "do" | "end" => {}
            "else_block" => {
                let mut inner = child.walk();
                for statement in child.children(&mut inner).filter(|child| {
                    child.is_named()
                        && child.kind() != "comment"
                        && child.kind() != "else"
                        && child.kind() != "end"
                }) {
                    or_else.push(build_term(&statement, source));
                }
            }
            _ => body.push(build_term(&child, source)),
        }
    }
    let mut pairs = vec![Node {
        term: Term::Tuple(vec![atom_term("do"), block_value(body)]),
        line: None,
    }];
    if !or_else.is_empty() {
        pairs.push(Node {
            term: Term::Tuple(vec![atom_term("else"), block_value(or_else)]),
            line: None,
        });
    }
    pairs
}

/// One term or `__block__` for several statements.
fn block_value(mut statements: Vec<Node>) -> Node {
    if statements.len() == 1 {
        statements.pop().unwrap_or(Node {
            term: Term::List(Vec::new()),
            line: None,
        })
    } else {
        let line = statements.iter().find_map(|node| node.line);
        Node {
            term: Term::Tuple(vec![
                atom_term("__block__"),
                empty_list(),
                Node {
                    term: Term::List(statements),
                    line: None,
                },
            ]),
            line,
        }
    }
}

/// Interpreted string value: quoted contents with standard escapes.
fn string_value(node: &tree_sitter::Node<'_>, source: &str) -> String {
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "quoted_content" {
            out.push_str(&interpret_escapes(text(&child, source)));
        }
    }
    out
}

/// Standard Elixir string escapes (approximation for exotic sequences).
fn interpret_escapes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('a') => out.push('\u{7}'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some('e') => out.push('\u{1b}'),
            Some('d' | 's') => out.push(' '),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Operator text of a `binary_operator` node.
fn operator_text(node: &tree_sitter::Node<'_>, source: &str) -> String {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| !child.is_named())
        .map(|child| text(&child, source).to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(source: &str) -> Node {
        model_file(source).expect("model builds")
    }

    #[test]
    fn oracle_mass_table() {
        // Masses probed against DuplicatedCode.mass/1 on the pinned checkout.
        let cases = [
            ("p1", 1),
            (":foo", 0),
            ("42", 0),
            ("\"str\"", 0),
            ("M1", 1),
            ("p1 + p2", 3),
            ("A.f(p1)", 4),
            ("def foo, do: :ok", 3),
            ("test \"x\" do\n  1\nend", 2),
            ("%{a: 1}", 2),
            ("{a, b}", 3),
            ("[1, 2]", 0),
            ("x = 1", 2),
            ("foo |> bar()", 3),
            ("@mod true", 2),
            ("1.5", 0),
        ];
        for (source, expected) in cases {
            assert_eq!(mass(&model(source)), expected, "{source}");
        }
    }

    #[test]
    fn identical_sources_hash_equal() {
        let first = model("def foo(p1) do\n  p1 + 1\nend\n");
        let second = model("def foo(p1) do\n  p1 + 1\nend\n");
        assert_eq!(digest(&first), digest(&second));
    }

    #[test]
    fn renamed_variables_hash_differently() {
        let first = model("def foo(p1) do\n  p1 + 1\nend\n");
        let second = model("def foo(q9) do\n  q9 + 1\nend\n");
        assert_ne!(digest(&first), digest(&second));
    }

    #[test]
    fn int_and_float_hash_differently() {
        assert_ne!(digest(&model("x = 1\n")), digest(&model("x = 1.0\n")));
    }

    #[test]
    fn shared_trees_match_fresh_parses() {
        // The pipeline shares prepare-phase trees instead of reparsing;
        // both paths must report identically.
        let mut params = BTreeMap::new();
        params.insert("mass_threshold".to_owned(), "3".to_owned());
        let block = "def duplicated(p1, p2) do\n  p1 + p2\nend\n";
        let files = vec![
            ProjectFile {
                filename: "a.ex".to_owned(),
                source: format!("defmodule A do\n{block}end\n"),
            },
            ProjectFile {
                filename: "b.ex".to_owned(),
                source: format!("defmodule B do\n{block}end\n"),
            },
        ];
        let expected = run(&files, &params);
        assert!(!expected.is_empty());
        let prepared: Vec<crate::batch::Prepared<'_>> = files
            .iter()
            .map(|file| crate::batch::Prepared::eager(&file.source))
            .collect();
        let trees: Vec<Option<&tree_sitter::Tree>> =
            prepared.iter().map(|ready| ready.tree()).collect();
        let mut actual = run_with_trees(&files, &trees, &params);
        let mut expected = expected;
        // Hash-map iteration order is nondeterministic; the pipeline
        // sorts issues downstream, so compare as sets.
        let key = |issue: &crate::project::ProjectIssue| {
            format!(
                "{:?}",
                (
                    issue.file,
                    issue.line,
                    issue.trigger.clone(),
                    issue.message.clone()
                )
            )
        };
        actual.sort_by_key(key);
        expected.sort_by_key(key);
        assert_eq!(actual, expected);
    }

    #[test]
    fn corpus_groups_match() {
        let entries = corpus_entries();
        let mut failures = Vec::new();
        for entry in &entries {
            check_corpus_entry(entry, &mut failures);
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// Parsed EX2002 corpus entries.
    fn corpus_entries() -> Vec<serde_json::Value> {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/compatibility/cases/EX2002.json"
        ))
        .expect("corpus readable");
        serde_json::from_str(&text).expect("valid JSON")
    }

    /// One corpus entry compared as a filename-keyed project.
    fn check_corpus_entry(entry: &serde_json::Value, failures: &mut Vec<String>) {
        if !entry["excluded_reason"].is_null() {
            return;
        }
        let sources = entry["sources"].as_array().cloned().unwrap_or_default();
        if sources.is_empty() {
            return;
        }
        let files = entry_files(&sources);
        let params = entry_params(entry);
        let issues = run(&files, &params);
        let mut actual: Vec<String> = issues
            .iter()
            .map(|issue| {
                let filename = files
                    .get(issue.file)
                    .map_or("", |file| file.filename.as_str());
                format!(
                    "{}|{}|{}|{}|{}",
                    filename,
                    issue.line.unwrap_or(0),
                    issue.column.unwrap_or(0),
                    issue.trigger,
                    issue.message
                )
            })
            .collect();
        let mut expected: Vec<String> = entry["findings"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|finding| {
                format!(
                    "{}|{}|{}|{}|{}",
                    finding["filename"].as_str().unwrap_or(""),
                    finding["line"].as_u64().unwrap_or(0),
                    finding["column"].as_u64().unwrap_or(0),
                    finding["trigger"].as_str().unwrap_or(""),
                    finding["message"].as_str().unwrap_or("")
                )
            })
            .collect();
        actual.sort();
        expected.sort();
        if actual != expected {
            failures.push(format!(
                "{}: mismatch\n  actual:   {actual:?}\n  expected: {expected:?}",
                entry["id"]
            ));
        }
    }

    /// Project files for one corpus entry.
    fn entry_files(sources: &[serde_json::Value]) -> Vec<ProjectFile> {
        sources
            .iter()
            .map(|source| ProjectFile {
                filename: source["filename"].as_str().unwrap_or("").to_owned(),
                source: source["source"].as_str().unwrap_or("").to_owned(),
            })
            .collect()
    }

    /// Colon-tolerant params for one corpus entry.
    fn entry_params(entry: &serde_json::Value) -> BTreeMap<String, String> {
        let mut params = BTreeMap::new();
        if let Some(map) = entry["params"].as_object() {
            for (key, value) in map {
                let rendered = match value {
                    serde_json::Value::Bool(flag) => flag.to_string(),
                    serde_json::Value::Number(number) => number.to_string(),
                    serde_json::Value::String(text) => text.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                params.insert(key.clone(), rendered);
            }
        }
        params
    }
}
