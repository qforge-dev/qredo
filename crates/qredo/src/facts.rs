//! Shared single-walk syntax facts (schema v0).
//!
//! One [`Facts::extract`] walk per file feeds every check whose queries are
//! flat scans over calls, replacing one full tree walk per check. The audit
//! behind v0 lives in the consumer list on [`Facts`]: remote-call heads
//! (`forbidden_function`, `mix_env`), `def` ranges (`mix_env`), plain call
//! heads plus `quote` skip ranges (`wrong_name`). Checks needing deeper
//! shapes (statement dataflow, module bodies, argument contents) keep their
//! tree walks until their fields land as explicit prerequisites.

/// One file's flat syntax inventory from a single walk.
///
/// This struct shape is facts schema v0 (part of the unit identity in
/// `INPUTS.md`): additive changes need a version bump there, not silent
/// extension, since persisted receipts key on the schema.
///
/// Consumers: `calls`/`def_ranges`/`quote_ranges` feed the flat call scans
/// (`forbidden_function`, `mix_env`, `wrong_name`); `scopes`/`line_scopes`
/// feed `Scopes::at`; `modules`/`defs` feed scope bonuses and module
/// discovery; `aliases`/`alias_directives` feed alias consumers
/// (`forbidden_module`, `module_deps`); `module_bodies` feeds `ModuleDoc`;
/// `ArgFact::named`/`keys` feed `logger_metadata`; `string_ranges` feeds
/// `redundant_blank_lines`; `var_idents`/`bind_regions` feed the
/// unused-variables inventory. Checks needing deeper shapes keep their tree
/// walks until their fields land as explicit prerequisites; the remaining
/// walks are declared with reasons in the tests module, pinned by the
/// residual ratchet test there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// True when the parsed tree contains error nodes. Consumers with an
    /// error gate (like the old `collect_with`) report nothing then;
    /// kernels scanning recovery trees ignore this bit.
    pub has_error: bool,
    /// Every `call` node, in walk order (consumers sort findings).
    pub calls: Vec<CallFact>,
    /// `(start, end)` byte spans of `def`/`defp`/`defmacro` calls.
    pub def_ranges: Vec<(u32, u32)>,
    /// `(start, end)` byte spans of argument-carrying `quote` calls whose
    /// subtrees consumers skip (mirrors `wrong_name`).
    pub quote_ranges: Vec<(u32, u32)>,
    /// Dotted scope names by id (`0` is the top-level `""`).
    pub scopes: Vec<String>,
    /// Scope id per 1-based source line; replicates `Scopes::at` exactly.
    pub line_scopes: Vec<u32>,
    /// Every `defmodule` call with its resolved dotted name.
    pub modules: Vec<ModuleFact>,
    /// Every `def`-family call with its parent scope, name and arity.
    pub defs: Vec<DefFact>,
    /// Every `alias` node span, in walk order.
    pub aliases: Vec<(u32, u32)>,
    /// Every `alias`-directive call with its decomposed targets.
    pub alias_directives: Vec<AliasDirectiveFact>,
    /// Every `@attribute` unary span for skip-region queries.
    pub attr_regions: Vec<(u32, u32)>,
    /// Unused-variable candidate identifiers with their vote kinds.
    pub var_idents: Vec<VarIdent>,
    /// Binding regions: stab-clause patterns, `=`/`<-` operands,
    /// definition heads and `test` contexts. Votes per identifier
    /// equal containing-region count.
    pub bind_regions: Vec<(u32, u32)>,
    /// Every string-like literal span (`string`, `charlist`, `sigil`,
    /// including heredocs) for literal-row queries.
    pub string_ranges: Vec<(u32, u32)>,
    /// Per-module direct body statements for module-level checks.
    pub module_bodies: Vec<ModuleBodyFact>,
}

/// One `defmodule` call: resolved name plus position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFact {
    /// Dotted name (`"<Unknown Module Name>"` when unresolvable).
    pub name: String,
    /// First `alias` argument span (`None` for atom/string heads).
    pub alias: Option<(u32, u32)>,
    /// 1-based line of the call start.
    pub line: u32,
    pub start: u32,
    pub end: u32,
}

/// Rule modules walking trees must stay exactly the declared residual
/// set: migrating a check closes its row here, and any new tree walk
/// fails loudly instead of landing silently.
#[cfg(test)]
const RESIDUAL_TREE_WALKS: &[(&str, &str)] = &[
    (
        "warning/unused_return.rs",
        "discard verdicts climb the shared tree from facts-found candidates; needs statement-role facts for full migration",
    ),
    (
        "warning/unused_enum.rs",
        "own discard engine climbs recovered nodes; discovery shared via unused_return candidates",
    ),
    (
        "warning/unused_file.rs",
        "own discard engine climbs recovered nodes; discovery shared via unused_return candidates",
    ),
    (
        "warning/unused_keyword.rs",
        "own discard engine climbs recovered nodes; discovery shared via unused_return candidates",
    ),
    (
        "warning/unused_list.rs",
        "own discard engine climbs recovered nodes; discovery shared via unused_return candidates",
    ),
    (
        "warning/unused_map.rs",
        "own discard engine climbs recovered nodes; discovery shared via unused_return candidates",
    ),
    (
        "warning/unused_op.rs",
        "own discard engine climbs recovered nodes; discovery shared via unused_return candidates",
    ),
    (
        "warning/unused_path.rs",
        "own discard engine climbs recovered nodes; discovery shared via unused_return candidates",
    ),
    (
        "warning/unused_regex.rs",
        "own discard engine climbs recovered nodes; discovery shared via unused_return candidates",
    ),
];
/// One unused-variable candidate identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarIdent {
    pub start: u32,
    pub end: u32,
    /// Vote kind by name shape.
    pub kind: VarKind,
}

/// Vote kind of one unused-variable name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    /// Bare `_`.
    Anonymous,
    /// `_name`.
    Meaningful,
}

/// One `alias`-directive call: decomposed single and grouped targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasDirectiveFact {
    /// 1-based line of the call start.
    pub line: u32,
    pub start: u32,
    pub end: u32,
    /// Head identifier span (`alias`, `import`, `require`, `use`).
    pub head_start: u32,
    pub head_end: u32,
    /// True for directives with options (`alias A.B, warn: false`).
    /// `module_deps` skips such directives entirely; other consumers
    /// ignore this bit.
    pub has_top_comma: bool,
    /// Sole top-level alias target span, if the only named argument is
    /// an alias (comma rule applied by consumers, not here).
    pub single: Option<(u32, u32)>,
    /// Named-children kinds of the `arguments` node, in order.
    pub arg_kinds: Vec<NodeKind>,
    /// Named-children kinds of the first top-level `dot`, if any.
    pub dot_kids: Vec<NodeKind>,
    /// First top-level `Base.{Parts}` decomposition, if valid.
    pub grouped_top: Option<GroupedAlias>,
    /// Every `Base.{Parts}` decomposition in the directive subtree.
    pub grouped_all: Vec<GroupedAlias>,
}

/// One `Base.{Part, ...}` decomposition with spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedAlias {
    pub base_start: u32,
    pub base_end: u32,
    pub parts: Vec<(u32, u32)>,
}

/// Direct body statements of one `defmodule` for module-level checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleBodyFact {
    /// Index into [`Facts::modules`].
    pub module: u32,
    /// True for `do`-block statements; false for the `do:` one-liner
    /// fallback value. Consumers mirroring do-block-only scans
    /// (`has_direct_defexception`) skip fallback bodies.
    pub from_block: bool,
    /// Direct statements (`do` block or `do:` one-liner value).
    pub stmts: Vec<BodyStmtFact>,
}

/// One direct module-body statement, preclassified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyStmtFact {
    /// Headed call with its head span and first alias argument span.
    Call {
        head_start: u32,
        head_end: u32,
        arg_alias: Option<(u32, u32)>,
    },
    /// `@name` attribute with its value class.
    Attr {
        name_start: u32,
        name_end: u32,
        value: AttrValue,
    },
    /// Anything else (never matches module queries).
    Other,
}

/// Value class of a `@name` attribute call argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrValue {
    /// Empty (possibly multiline) string: asks for `@moduledoc false`.
    EmptyString,
    /// Anything else, including a missing argument.
    Other,
}

/// One `def`-family call: parent scope, head name and arity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefFact {
    /// Dotted parent scope name (`""` at top level).
    pub scope: String,
    /// Head name (`None` for exotic heads; parent scope is kept then).
    pub name: Option<String>,
    /// Argument count by `Parameters.count` rules.
    pub arity: u32,
    /// 1-based line of the call start.
    pub line: u32,
    pub start: u32,
    pub end: u32,
}

/// One `call` node: spans plus decomposed head and top-level arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFact {
    /// 1-based line of the call start.
    pub line: u32,
    /// Byte span of the whole call.
    pub start: u32,
    pub end: u32,
    /// Decomposed head; `None` when exotic (never matches queries).
    pub head: Option<HeadFact>,
    /// Direct children of the `arguments` node.
    pub args: Vec<ArgFact>,
    /// True when piped into (`x |> f()`): direct parent is a `|>` whose
    /// right side contains this call (mirrors `piped_into`).
    pub piped: bool,
}

/// A call head today's decompositions accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadFact {
    /// Bare identifier head (`use`, `quote`, `def`, ...).
    Plain { start: u32, end: u32 },
    /// `Left.fun` with an alias/atom module and identifier function.
    Remote {
        mod_start: u32,
        mod_end: u32,
        mod_kind: ModKind,
        fun_start: u32,
        fun_end: u32,
    },
}

/// Module side of a remote head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKind {
    Alias,
    Atom,
    /// Any other dot-target module side (checked textually by consumers
    /// that accept it, like unquote detection).
    Other,
}

/// One direct child of an `arguments` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgFact {
    /// Tree-sitter named status (comments count as named).
    pub named: bool,
    /// Named non-comment status (`code_children` membership).
    pub code: bool,
    /// True for an `alias` child.
    pub is_alias: bool,
    /// Container shape for argument-structure queries.
    pub kind: NodeKind,
    pub start: u32,
    pub end: u32,
    /// Code children (named non-comment) with their kinds and spans.
    pub kids: Vec<ChildFact>,
    /// Validated keyword pairs: this argument's own pairs when it is a
    /// `keywords` node, plus pairs of `keywords` items when it is a list.
    pub keys: Vec<KeyPair>,
}

/// A code child of an argument: kind, span and pair count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildFact {
    pub kind: NodeKind,
    pub start: u32,
    pub end: u32,
    /// Number of `pair`-kind code children (struct-field counting).
    pub pairs: u32,
}

/// One validated keyword pair: key span plus value span when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPair {
    pub key_start: u32,
    pub key_end: u32,
    pub value: Option<(u32, u32)>,
}

/// Container shapes argument queries distinguish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    List,
    Keywords,
    Tuple,
    Atom,
    Alias,
    Pair,
    Other,
}

/// Map a tree-sitter kind to the shapes queries distinguish.
fn node_kind(kind: &str) -> NodeKind {
    match kind {
        "list" => NodeKind::List,
        "keywords" => NodeKind::Keywords,
        "tuple" => NodeKind::Tuple,
        "atom" => NodeKind::Atom,
        "alias" => NodeKind::Alias,
        "pair" => NodeKind::Pair,
        _ => NodeKind::Other,
    }
}

impl Facts {
    /// Empty facts for unavailable grammars; kernels report nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            has_error: false,
            calls: Vec::new(),
            def_ranges: Vec::new(),
            quote_ranges: Vec::new(),
            scopes: vec![String::new()],
            line_scopes: Vec::new(),
            modules: Vec::new(),
            defs: Vec::new(),
            aliases: Vec::new(),
            alias_directives: Vec::new(),
            attr_regions: Vec::new(),
            var_idents: Vec::new(),
            bind_regions: Vec::new(),
            string_ranges: Vec::new(),
            module_bodies: Vec::new(),
        }
    }
}

/// One walk over `tree` collecting every v0 consumer's inventory.
///
/// Mirrors each consumer's traversal exactly: all `call` nodes anywhere
/// (including captures and recovery trees — no validity gate, like the
/// kernels), head decomposition copied from `mix_env`/`forbidden_function`,
/// `quote` detection copied from `wrong_name`, scope/module/def tracking
/// copied from `scope` (a `defmodule` opens a scope and descends, a
/// `def`-family call records without descending for scope purposes, every
/// other named node records the enclosing scope).
#[must_use]
pub fn extract(tree: &tree_sitter::Tree, source: &str) -> Facts {
    let mut facts = Facts::empty();
    facts.has_error = tree.root_node().has_error();
    let mut path: Vec<String> = Vec::new();
    let mut scopes = ScopeTable::new();
    let mut stack = vec![Work::Visit(tree.root_node(), 0)];
    while let Some(item) = stack.pop() {
        match item {
            Work::PopPath(len) => {
                path.truncate(len);
            }
            Work::Visit(node, scope) => {
                let mut walker = Walker {
                    facts: &mut facts,
                    scopes: &mut scopes,
                    path: &mut path,
                    stack: &mut stack,
                };
                if node.kind() == "call" {
                    walker.visit_call(node, source, scope);
                } else {
                    walker.visit_other(node, source, scope);
                }
            }
        }
    }
    facts.line_scopes = scopes.finish(source);
    facts
}

/// Incremental per-line scope table: record lines arrive in
/// non-decreasing order along the document-order walk, so each line
/// resolves inline to the last record at or before it — exactly what
/// the old record scan computed, without storing 1.2M tuples.
struct ScopeTable {
    table: Vec<u32>,
    current: u32,
}

impl ScopeTable {
    fn new() -> Self {
        Self {
            table: Vec::new(),
            current: 0,
        }
    }

    fn record(&mut self, line: usize, id: u32) {
        while self.table.len() < line {
            let current = self.current;
            self.table.push(current);
        }
        if let Some(slot) = self.table.get_mut(line - 1) {
            *slot = id;
        }
        self.current = id;
    }

    fn finish(mut self, source: &str) -> Vec<u32> {
        let lines = source.split('\n').count();
        while self.table.len() < lines {
            self.table.push(self.current);
        }
        self.table
    }
}

/// Pending walk items: node visits plus path truncations on module exit.
enum Work<'tree> {
    Visit(tree_sitter::Node<'tree>, usize),
    PopPath(usize),
}

/// Mutable single-walk state: inventory, scope table and module path.
struct Walker<'a, 'tree> {
    facts: &'a mut Facts,
    scopes: &'a mut ScopeTable,
    path: &'a mut Vec<String>,
    stack: &'a mut Vec<Work<'tree>>,
}

/// Push a node's children with one scope id (reversed for document order).
fn push_children<'tree>(
    stack: &mut Vec<Work<'tree>>,
    node: &tree_sitter::Node<'tree>,
    scope: usize,
) {
    let mut cursor = node.walk();
    let mut children: Vec<tree_sitter::Node<'tree>> = node.children(&mut cursor).collect();
    while let Some(child) = children.pop() {
        stack.push(Work::Visit(child, scope));
    }
}

impl<'tree> Walker<'_, 'tree> {
    /// Handle one `call` node: inventory plus scope/module/def tracking.
    fn visit_call(&mut self, node: tree_sitter::Node<'tree>, source: &str, scope: usize) {
        push_call(self.facts, &node, source);
        record_binding_regions(self.facts, &node, source);
        let head_text = first_identifier_text(&node, source);
        match head_text {
            Some("defmodule") => self.visit_defmodule(node, source),
            Some("def" | "defp" | "defmacro") => self.visit_def(node, source),
            Some("alias" | "import" | "require" | "use") => {
                push_alias_directive(self.facts, &node);
                self.record(node, scope);
                push_children(self.stack, &node, scope);
            }
            _ => {
                self.record(node, scope);
                push_children(self.stack, &node, scope);
            }
        }
    }

    /// Handle one non-call node: literal/alias/variable inventory,
    /// binding regions and scope records.
    fn visit_other(&mut self, node: tree_sitter::Node<'tree>, source: &str, scope: usize) {
        match node.kind() {
            "string" | "charlist" | "sigil" => {
                self.facts
                    .string_ranges
                    .push((clamp(node.start_byte()), clamp(node.end_byte())));
            }
            "alias" => {
                self.facts
                    .aliases
                    .push((clamp(node.start_byte()), clamp(node.end_byte())));
            }
            "identifier" => {
                record_var_ident(self.facts, &node, source);
            }
            "stab_clause" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.facts
                        .bind_regions
                        .push((clamp(left.start_byte()), clamp(left.end_byte())));
                }
            }
            "binary_operator" => {
                if is_assign_op(&node, source) {
                    self.facts
                        .bind_regions
                        .push((clamp(node.start_byte()), clamp(node.end_byte())));
                }
            }
            _ => {}
        }
        if node.kind() == "unary_operator" && is_attr(&node, source) {
            self.facts
                .attr_regions
                .push((clamp(node.start_byte()), clamp(node.end_byte())));
        }
        self.record(node, scope);
        push_children(self.stack, &node, scope);
    }

    /// Record one named node visit with its scope id.
    fn record(&mut self, node: tree_sitter::Node<'tree>, scope: usize) {
        if node.is_named() {
            self.scopes
                .record(node.start_position().row + 1, clamp(scope));
        }
    }

    /// Open a module scope: resolve the name, record, descend.
    fn visit_defmodule(&mut self, node: tree_sitter::Node<'tree>, source: &str) {
        let mut full = self.path.clone();
        let name = module_head(&node, source);
        full.push(name.clone());
        self.facts.scopes.push(full.join("."));
        let id = self.facts.scopes.len() - 1;
        self.scopes.record(node.start_position().row + 1, clamp(id));
        self.facts.modules.push(ModuleFact {
            name: full.join("."),
            alias: module_alias_arg(&node),
            line: clamp(node.start_position().row + 1),
            start: clamp(node.start_byte()),
            end: clamp(node.end_byte()),
        });
        let module_idx = clamp(self.facts.modules.len() - 1);
        let (from_block, stmts) = module_body_stmts(&node, source);
        self.facts.module_bodies.push(ModuleBodyFact {
            module: module_idx,
            from_block,
            stmts,
        });
        *self.path = full;
        let restore = self.path.len() - 1;
        self.stack.push(Work::PopPath(restore));
        push_children(self.stack, &node, id);
    }

    /// Record a definition scope with its name and arity.
    fn visit_def(&mut self, node: tree_sitter::Node<'tree>, source: &str) {
        let parent = self.path.join(".");
        let name = def_head_name(&node, source);
        let arity = def_arity(&node, source);
        self.facts.scopes.push(join_scope(&parent, name.as_deref()));
        let id = self.facts.scopes.len() - 1;
        self.scopes.record(node.start_position().row + 1, clamp(id));
        self.facts.defs.push(DefFact {
            scope: parent,
            name,
            arity,
            line: clamp(node.start_position().row + 1),
            start: clamp(node.start_byte()),
            end: clamp(node.end_byte()),
        });
        // Bodies stay visible to the inventory walk (calls inside defs
        // matter), recording them with the def id: every line they could
        // claim resolves to this def record first, so scope answers are
        // unchanged. This keeps the walk uniform.
        push_children(self.stack, &node, id);
    }
}

/// Dotted scope name from a parent scope and optional head name.
pub(crate) fn join_scope(parent: &str, name: Option<&str>) -> String {
    match (parent.is_empty(), name) {
        (true, Some(name)) => name.to_owned(),
        (true, None) => String::new(),
        (false, Some(name)) => format!("{parent}.{name}"),
        (false, None) => parent.to_owned(),
    }
}

/// First-child identifier text of a call, if headed that way.
fn first_identifier_text<'src>(
    node: &tree_sitter::Node<'_>,
    source: &'src str,
) -> Option<&'src str> {
    let mut cursor = node.walk();
    node.children(&mut cursor).next().and_then(|head| {
        (head.kind() == "identifier")
            .then(|| source.get(head.start_byte()..head.end_byte()))
            .flatten()
    })
}

/// Dotted module head of a `defmodule` call (copied from `scope`).
fn module_head(node: &tree_sitter::Node<'_>, source: &str) -> String {
    let mut cursor = node.walk();
    let head = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")
        .and_then(|arguments| {
            let mut inner = arguments.walk();
            arguments
                .children(&mut inner)
                .find(tree_sitter::Node::is_named)
        });
    let Some(head) = head else {
        return "<Unknown Module Name>".to_owned();
    };
    match head.kind() {
        "alias" => text(source, &head).unwrap_or_default().to_owned(),
        "atom" => {
            let name = text(source, &head)
                .unwrap_or_default()
                .trim_start_matches(':');
            name.strip_prefix("Elixir.").unwrap_or(name).to_owned()
        }
        "string" => string_value(&head, source),
        _ => "<Unknown Module Name>".to_owned(),
    }
}

/// Function name of a `def`-family call (copied from `scope`).
fn def_head_name(node: &tree_sitter::Node<'_>, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    let argument = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")
        .and_then(|arguments| {
            let mut inner = arguments.walk();
            arguments
                .children(&mut inner)
                .find(tree_sitter::Node::is_named)
        })?;
    match argument.kind() {
        "identifier" => text(source, &argument).map(str::to_owned),
        "call" => head_name(&argument, source),
        "binary_operator" => {
            let mut cursor = argument.walk();
            let operator = argument
                .children(&mut cursor)
                .find(|child| !child.is_named())
                .and_then(|child| text(source, &child));
            if operator == Some("when") {
                let mut cursor = argument.walk();
                argument
                    .children(&mut cursor)
                    .find(tree_sitter::Node::is_named)
                    .and_then(|left| head_name(&left, source))
            } else {
                operator.map(str::to_owned)
            }
        }
        "unary_operator" => operator_text(&argument, source),
        _ => None,
    }
}

/// Head identifier text of a call or bare identifier (copied from `scope`).
fn head_name(node: &tree_sitter::Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => text(source, node).map(str::to_owned),
        "call" => {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .next()
                .filter(|head| head.kind() == "identifier")
                .and_then(|head| text(source, &head))
                .map(str::to_owned)
        }
        _ => None,
    }
}

/// Operator symbol of a binary/unary call (copied from `scope`).
fn operator_text(node: &tree_sitter::Node<'_>, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| !child.is_named())
        .and_then(|child| text(source, &child))
        .map(str::to_owned)
}

/// Interpreted string contents (copied from `scope`).
fn string_value(node: &tree_sitter::Node<'_>, source: &str) -> String {
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "quoted_content" {
            out.push_str(text(source, &child).unwrap_or(""));
        }
    }
    out
}

/// Arity of a `def`-family call by `Parameters.count` rules: the head
/// expression is the first named argument; bare names take none.
fn def_arity(node: &tree_sitter::Node<'_>, source: &str) -> u32 {
    let mut cursor = node.walk();
    let argument = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")
        .and_then(|arguments| {
            let mut inner = arguments.walk();
            arguments
                .children(&mut inner)
                .find(tree_sitter::Node::is_named)
        });
    argument.map_or(0, |argument| head_arity(&argument, source))
}

/// Arity of one head expression (copied from `scope`).
fn head_arity(node: &tree_sitter::Node<'_>, source: &str) -> u32 {
    clamp(head_arity_inner(node, source))
}

/// Arity of one head expression: bare names take none, calls take their
/// argument count, `when` guards unwrap to the guarded head.
fn head_arity_inner(node: &tree_sitter::Node<'_>, source: &str) -> usize {
    match node.kind() {
        "call" => {
            let mut cursor = node.walk();
            let head = node.children(&mut cursor).next();
            let is_when = head.as_ref().is_some_and(|head| {
                head.kind() == "identifier" && text(source, head) == Some("when")
            });
            if is_when {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(tree_sitter::Node::is_named)
                    .map_or(0, |left| head_arity_inner(&left, source))
            } else {
                call_arity(node)
            }
        }
        "binary_operator" => {
            let mut cursor = node.walk();
            let operator = node
                .children(&mut cursor)
                .find(|child| !child.is_named())
                .and_then(|child| text(source, &child));
            if operator == Some("when") {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(tree_sitter::Node::is_named)
                    .map_or(0, |left| head_arity_inner(&left, source))
            } else {
                named_count(node)
            }
        }
        "unary_operator" => named_count(node),
        _ => 0,
    }
}

/// Named children count of a call's arguments (copied from `scope`).
fn call_arity(node: &tree_sitter::Node<'_>) -> usize {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "arguments")
        .map_or_else(
            || named_count(node),
            |arguments| {
                let mut inner = arguments.walk();
                arguments
                    .children(&mut inner)
                    .filter(|child| child.is_named() && child.kind() != "comment")
                    .count()
            },
        )
}

/// Named non-comment children count (copied from `scope`).
fn named_count(node: &tree_sitter::Node<'_>) -> usize {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
        .count()
}

/// Source slice of a node; `None` on invalid boundaries.
fn text<'src>(source: &'src str, node: &tree_sitter::Node<'_>) -> Option<&'src str> {
    node.utf8_text(source.as_bytes()).ok()
}

/// Record one `call` node: head, `def`/`quote` ranges, top-level arguments.
fn push_call(facts: &mut Facts, node: &tree_sitter::Node<'_>, source: &str) {
    let head = head_fact(node);
    if is_def_head(node, source) {
        facts
            .def_ranges
            .push((clamp(node.start_byte()), clamp(node.end_byte())));
    }
    if is_quote_with_args(node, source) {
        facts
            .quote_ranges
            .push((clamp(node.start_byte()), clamp(node.end_byte())));
    }
    facts.calls.push(CallFact {
        line: clamp(node.start_position().row + 1),
        start: clamp(node.start_byte()),
        end: clamp(node.end_byte()),
        head,
        args: call_args(node),
        piped: piped_into(node, source),
    });
}

/// True when the call is the piped stage of a `|>` pipeline.
fn piped_into(node: &tree_sitter::Node<'_>, source: &str) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != "binary_operator" {
        return false;
    }
    let mut cursor = parent.walk();
    let operator = parent.children(&mut cursor).find(|child| !child.is_named());
    let Some(operator) = operator else {
        return false;
    };
    if source.get(operator.start_byte()..operator.end_byte()) != Some("|>") {
        return false;
    }
    let mut cursor = parent.walk();
    let mut named = parent
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named);
    let (Some(_left), Some(right)) = (named.next(), named.last()) else {
        return false;
    };
    right.start_byte() <= node.start_byte()
}

/// Head decomposition copied from `mix_env`/`forbidden_function`: a bare
/// identifier head, or a `dot` whose module side is an alias/atom and whose
/// function side is an identifier. Anything else yields `None` (skipped).
fn head_fact(node: &tree_sitter::Node<'_>) -> Option<HeadFact> {
    let mut cursor = node.walk();
    let first = node.children(&mut cursor).next()?;
    if first.kind() == "identifier" {
        return Some(HeadFact::Plain {
            start: clamp(first.start_byte()),
            end: clamp(first.end_byte()),
        });
    }
    let mut cursor = node.walk();
    let dot = node
        .children(&mut cursor)
        .find(|child| child.kind() == "dot")?;
    let mut cursor = dot.walk();
    let mut named = dot
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named);
    let left = named.next()?;
    let fun = named.last()?;
    if fun.kind() != "identifier" {
        return None;
    }
    let fun_span = (clamp(fun.start_byte()), clamp(fun.end_byte()));
    let mod_kind = match left.kind() {
        "alias" => ModKind::Alias,
        "atom" => ModKind::Atom,
        _ => ModKind::Other,
    };
    Some(HeadFact::Remote {
        mod_start: clamp(left.start_byte()),
        mod_end: clamp(left.end_byte()),
        mod_kind,
        fun_start: fun_span.0,
        fun_end: fun_span.1,
    })
}

/// True for a `def`-family definition head (mirrors `mix_env::is_def`).
fn is_def_head(node: &tree_sitter::Node<'_>, source: &str) -> bool {
    node.child(0).is_some_and(|head| {
        head.kind() == "identifier"
            && matches!(
                source.get(head.start_byte()..head.end_byte()),
                Some("def" | "defp" | "defmacro")
            )
    })
}

/// True for an argument-carrying `quote` call (mirrors `wrong_name`).
fn is_quote_with_args(node: &tree_sitter::Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    let headed = node.children(&mut cursor).any(|child| {
        child.kind() == "identifier"
            && source.get(child.start_byte()..child.end_byte()) == Some("quote")
    });
    if !headed {
        return false;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| {
        (child.kind() == "arguments" && child.named_child_count() > 0) || child.kind() == "do_block"
    })
}

/// Every direct child of the first `arguments` node.
fn call_args(node: &tree_sitter::Node<'_>) -> Vec<ArgFact> {
    let mut cursor = node.walk();
    let Some(arguments) = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")
    else {
        return Vec::new();
    };
    let mut cursor = arguments.walk();
    arguments
        .children(&mut cursor)
        .map(|child| ArgFact {
            code: child.is_named() && child.kind() != "comment",
            named: child.is_named(),
            is_alias: child.kind() == "alias",
            kind: node_kind(child.kind()),
            start: clamp(child.start_byte()),
            end: clamp(child.end_byte()),
            kids: arg_kids(&child),
            keys: arg_keys(&child),
        })
        .collect()
}

/// Code children (named non-comment) of one argument with kinds and spans.
fn arg_kids(arg: &tree_sitter::Node<'_>) -> Vec<ChildFact> {
    let mut cursor = arg.walk();
    arg.children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
        .map(|child| ChildFact {
            kind: node_kind(child.kind()),
            start: clamp(child.start_byte()),
            end: clamp(child.end_byte()),
            pairs: pair_count(&child),
        })
        .collect()
}

/// Number of `pair`-kind code children (struct-field counting).
fn pair_count(node: &tree_sitter::Node<'_>) -> u32 {
    let mut cursor = node.walk();
    clamp(
        node.children(&mut cursor)
            .filter(|child| child.is_named() && child.kind() != "comment")
            .filter(|child| child.kind() == "pair")
            .count(),
    )
}

/// Validated keyword pairs of one argument: its own pairs when it is a
/// `keywords` node, plus pairs of `keywords` items when it is a list.
fn arg_keys(arg: &tree_sitter::Node<'_>) -> Vec<KeyPair> {
    match node_kind(arg.kind()) {
        NodeKind::Keywords => keyword_pairs(arg),
        NodeKind::List => {
            let mut out = Vec::new();
            let mut cursor = arg.walk();
            for item in arg
                .children(&mut cursor)
                .filter(|child| child.is_named() && child.kind() != "comment")
            {
                if node_kind(item.kind()) == NodeKind::Keywords {
                    out.extend(keyword_pairs(&item));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Validated `(key, value)` pairs of a `keywords` node; any invalid pair
/// invalidates the whole list, mirroring the reference.
fn keyword_pairs(keywords: &tree_sitter::Node<'_>) -> Vec<KeyPair> {
    let mut out = Vec::new();
    let mut cursor = keywords.walk();
    for pair in keywords
        .children(&mut cursor)
        .filter(|child| child.kind() == "pair")
    {
        let key = pair.child_by_field_name("key");
        let value = pair.child_by_field_name("value");
        match key {
            Some(key) if key.kind() == "keyword" => out.push(KeyPair {
                key_start: clamp(key.start_byte()),
                key_end: clamp(key.end_byte()),
                value: value.map(|value| (clamp(value.start_byte()), clamp(value.end_byte()))),
            }),
            _ => return Vec::new(),
        }
    }
    out
}

/// Direct body statements of a `defmodule` (block or `do:` one-liner).
fn module_body_stmts(node: &tree_sitter::Node<'_>, source: &str) -> (bool, Vec<BodyStmtFact>) {
    let mut cursor = node.walk();
    let body: Vec<BodyStmtFact> = node
        .children(&mut cursor)
        .filter(|child| child.kind() == "do_block")
        .flat_map(|block| {
            let mut inner = block.walk();
            block
                .children(&mut inner)
                .filter(|child| {
                    child.is_named()
                        && !child.kind().ends_with("_block")
                        && !matches!(
                            child.kind(),
                            "do" | "end" | "comment" | "else" | "rescue" | "catch" | "after"
                        )
                })
                .map(|child| classify_body_stmt(&child, source))
                .collect::<Vec<_>>()
        })
        .collect();
    if body.is_empty() {
        (false, pairs_do_values(node, source))
    } else {
        (true, body)
    }
}

/// `do:` value of one-liner definitions, classified like block statements.
fn pairs_do_values(node: &tree_sitter::Node<'_>, source: &str) -> Vec<BodyStmtFact> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.kind() == "arguments")
        .flat_map(|arguments| {
            let mut inner = arguments.walk();
            arguments
                .children(&mut inner)
                .filter(|child| child.kind() == "keywords")
                .collect::<Vec<_>>()
        })
        .flat_map(|keywords| {
            let mut inner = keywords.walk();
            keywords
                .children(&mut inner)
                .filter(|child| child.kind() == "pair")
                .collect::<Vec<_>>()
        })
        .filter(|pair| {
            pair.child(0).is_some_and(|key| {
                key.kind() == "keyword"
                    && source
                        .get(key.start_byte()..key.end_byte())
                        .unwrap_or("")
                        .starts_with("do:")
            })
        })
        .filter_map(|pair| {
            let mut inner = pair.walk();
            pair.children(&mut inner)
                .filter(|child| child.is_named() && child.kind() != "keyword")
                .last()
        })
        .map(|value| classify_body_stmt(&value, source))
        .collect()
}

/// Classify one body statement: headed calls, `@name` attributes, other.
fn classify_body_stmt(node: &tree_sitter::Node<'_>, source: &str) -> BodyStmtFact {
    if node.kind() == "call" {
        let mut cursor = node.walk();
        if let Some(head) = node.children(&mut cursor).next()
            && head.kind() == "identifier"
        {
            return BodyStmtFact::Call {
                head_start: clamp(head.start_byte()),
                head_end: clamp(head.end_byte()),
                arg_alias: module_alias_arg(node),
            };
        }
        return BodyStmtFact::Other;
    }
    if node.kind() == "unary_operator" {
        return attr_stmt(node, source);
    }
    BodyStmtFact::Other
}

/// Classify one `@name` attribute: name span plus value class.
fn attr_stmt(node: &tree_sitter::Node<'_>, source: &str) -> BodyStmtFact {
    let mut cursor = node.walk();
    let mut children = node.children(&mut cursor);
    let is_attr = children
        .next()
        .is_some_and(|op| source.get(op.start_byte()..op.end_byte()) == Some("@"));
    if !is_attr {
        return BodyStmtFact::Other;
    }
    let mut cursor = node.walk();
    let call = node.children(&mut cursor).find(tree_sitter::Node::is_named);
    let Some(call) = call else {
        return BodyStmtFact::Other;
    };
    let mut cursor = call.walk();
    let (name_start, name_end) = match call.children(&mut cursor).next() {
        Some(head) if head.kind() == "identifier" => {
            (clamp(head.start_byte()), clamp(head.end_byte()))
        }
        _ => return BodyStmtFact::Other,
    };
    let mut cursor = call.walk();
    let value = call
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")
        .and_then(|arguments| {
            let mut inner = arguments.walk();
            arguments
                .children(&mut inner)
                .find(tree_sitter::Node::is_named)
        });
    let value = match value {
        Some(value)
            if value.kind() == "string" && string_text(&value, source).trim().is_empty() =>
        {
            AttrValue::EmptyString
        }
        _ => AttrValue::Other,
    };
    BodyStmtFact::Attr {
        name_start,
        name_end,
        value,
    }
}

/// Interior text of a string literal (heredoc content included).
fn string_text(node: &tree_sitter::Node<'_>, source: &str) -> String {
    let mut cursor = node.walk();
    let mut out = String::new();
    for child in node.children(&mut cursor) {
        if child.kind() == "quoted_content" {
            out.push_str(text(source, &child).unwrap_or(""));
        }
    }
    out
}

/// True when a unary node's whole text starts with `@` (an attribute).
fn is_attr(node: &tree_sitter::Node<'_>, source: &str) -> bool {
    source
        .get(node.start_byte()..node.end_byte())
        .is_some_and(|text| text.starts_with('@'))
}

/// Definition-like calls whose head patterns bind variables.
const DEF_NAMES: [&str; 4] = ["def", "defp", "defmacro", "defmacrop"];

/// Record one identifier as an unused-variable candidate unless it can
/// never vote: plain names, `__`-prefixed specials and call targets.
fn record_var_ident(facts: &mut Facts, node: &tree_sitter::Node<'_>, source: &str) {
    let Some(name) = source.get(node.start_byte()..node.end_byte()) else {
        return;
    };
    let kind = if name == "_" {
        VarKind::Anonymous
    } else if name.starts_with("__") || !name.starts_with('_') {
        return;
    } else {
        VarKind::Meaningful
    };
    if is_callee(node) {
        return;
    }
    facts.var_idents.push(VarIdent {
        start: clamp(node.start_byte()),
        end: clamp(node.end_byte()),
        kind,
    });
}

/// Whether the identifier names the called function and cannot bind.
fn is_callee(node: &tree_sitter::Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() == "dot" {
        return true;
    }
    if parent.kind() != "call" {
        return false;
    }
    let mut cursor = parent.walk();
    parent
        .children(&mut cursor)
        .find(|child| child.kind() == "target")
        .is_some_and(|target| {
            target.start_byte() <= node.start_byte() && node.end_byte() <= target.end_byte()
        })
}

/// True when a binary node's operator spells `=` or `<-`.
fn is_assign_op(node: &tree_sitter::Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| !child.is_named())
        .and_then(|operator| source.get(operator.start_byte()..operator.end_byte()))
        .is_some_and(|op| op == "=" || op == "<-")
}

/// Record definition-head and `test`-context binding regions of one call.
fn record_binding_regions(facts: &mut Facts, node: &tree_sitter::Node<'_>, source: &str) {
    let target = call_target(node, source);
    if target.is_some_and(|name| DEF_NAMES.contains(&name))
        && let Some(head) = head_pattern(node)
    {
        facts
            .bind_regions
            .push((clamp(head.start_byte()), clamp(head.end_byte())));
    }
    if target == Some("test")
        && let Some(context) = test_context(node, source)
    {
        facts
            .bind_regions
            .push((clamp(context.start_byte()), clamp(context.end_byte())));
    }
}

/// Target identifier text of a call, if any.
fn call_target<'src>(node: &tree_sitter::Node<'_>, source: &'src str) -> Option<&'src str> {
    let target = node.child_by_field_name("target")?;
    if target.kind() != "identifier" {
        return None;
    }
    source.get(target.start_byte()..target.end_byte())
}

/// First positional argument of a definition call.
fn head_pattern<'tree>(node: &tree_sitter::Node<'tree>) -> Option<tree_sitter::Node<'tree>> {
    let mut cursor = node.walk();
    let arguments = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")?;
    let mut inner = arguments.walk();
    arguments
        .children(&mut inner)
        .find(tree_sitter::Node::is_named)
}

/// Context argument of a `test ... do` block call, if present.
fn test_context<'tree>(
    node: &tree_sitter::Node<'tree>,
    source: &str,
) -> Option<tree_sitter::Node<'tree>> {
    if !test_body_present(node, source) {
        return None;
    }
    let mut cursor = node.walk();
    let arguments = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")?;
    let mut inner = arguments.walk();
    let mut named = arguments
        .children(&mut inner)
        .filter(tree_sitter::Node::is_named);
    named.next()?;
    named.next()
}

/// `do` body of a `test` call: a `do` block or trailing `do:` keywords.
fn test_body_present(node: &tree_sitter::Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    if node
        .children(&mut cursor)
        .any(|child| child.kind() == "do_block")
    {
        return true;
    }
    let mut cursor = node.walk();
    let Some(arguments) = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")
    else {
        return false;
    };
    let mut inner = arguments.walk();
    let count = arguments
        .children(&mut inner)
        .filter(tree_sitter::Node::is_named)
        .count();
    let Some(last) = arguments_last_named(&arguments, count) else {
        return false;
    };
    last.kind() == "keywords" && keywords_contain(&last, source, "do")
}

/// Last named child of an arguments node by counted index.
fn arguments_last_named<'tree>(
    arguments: &tree_sitter::Node<'tree>,
    count: usize,
) -> Option<tree_sitter::Node<'tree>> {
    let index = u32::try_from(count.saturating_sub(1)).ok()?;
    arguments.named_child(index)
}

/// Whether trailing keywords pass a `want:` option.
fn keywords_contain(parent: &tree_sitter::Node<'_>, source: &str, want: &str) -> bool {
    let mut cursor = parent.walk();
    parent.children(&mut cursor).any(|pair| {
        pair.child_by_field_name("key")
            .is_some_and(|key| keyword_key(&key, source) == want)
    })
}

/// Keyword key without decorations (`do: ` renders with colon and space).
fn keyword_key<'src>(key: &tree_sitter::Node<'_>, source: &'src str) -> &'src str {
    source
        .get(key.start_byte()..key.end_byte())
        .unwrap_or("")
        .trim()
        .trim_matches(':')
}

/// Saturating `usize` narrowing for byte offsets and rows.
fn clamp(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// First `alias` child span of a call's arguments (`None` when absent).
fn module_alias_arg(node: &tree_sitter::Node<'_>) -> Option<(u32, u32)> {
    let mut cursor = node.walk();
    let arguments = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments")?;
    let mut cursor = arguments.walk();
    arguments
        .children(&mut cursor)
        .find(|child| child.kind() == "alias")
        .map(|alias| (clamp(alias.start_byte()), clamp(alias.end_byte())))
}

/// Decompose one `alias`-directive call (mirrors `module_deps` and the
/// `forbidden_module` grouped scan).
fn push_alias_directive(facts: &mut Facts, node: &tree_sitter::Node<'_>) {
    let mut cursor = node.walk();
    let head = node.children(&mut cursor).next();
    let (head_start, head_end) = match head {
        Some(head) if head.kind() == "identifier" => {
            (clamp(head.start_byte()), clamp(head.end_byte()))
        }
        _ => (0, 0),
    };
    let mut cursor = node.walk();
    let arguments = node
        .children(&mut cursor)
        .find(|child| child.kind() == "arguments");
    let (single, arg_kinds, dot_kids, grouped_top, has_top_comma) = match arguments {
        Some(arguments) => (
            single_target(&arguments),
            arg_kinds_of(&arguments),
            dot_kids_of(&arguments),
            grouped_top(&arguments),
            has_top_comma(&arguments),
        ),
        None => (None, Vec::new(), Vec::new(), None, false),
    };
    facts.alias_directives.push(AliasDirectiveFact {
        line: clamp(node.start_position().row + 1),
        start: clamp(node.start_byte()),
        end: clamp(node.end_byte()),
        head_start,
        head_end,
        has_top_comma,
        single,
        arg_kinds,
        dot_kids,
        grouped_top,
        grouped_all: grouped_subtree(node),
    });
}

/// Named-children kinds of an `arguments` node, in order.
fn arg_kinds_of(arguments: &tree_sitter::Node<'_>) -> Vec<NodeKind> {
    let mut cursor = arguments.walk();
    arguments
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named)
        .map(|child| node_kind(child.kind()))
        .collect()
}

/// Named-children kinds of the first top-level `dot`, if any.
fn dot_kids_of(arguments: &tree_sitter::Node<'_>) -> Vec<NodeKind> {
    let mut cursor = arguments.walk();
    let Some(dot) = arguments
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named)
        .find(|child| child.kind() == "dot")
    else {
        return Vec::new();
    };
    let mut cursor = dot.walk();
    dot.children(&mut cursor)
        .filter(tree_sitter::Node::is_named)
        .map(|child| node_kind(child.kind()))
        .collect()
}

/// Sole top-level alias target span, if the only named argument is an
/// alias. Consumers apply their own comma rules via `has_top_comma`.
fn single_target(arguments: &tree_sitter::Node<'_>) -> Option<(u32, u32)> {
    let mut cursor = arguments.walk();
    let mut named = arguments
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named);
    let only = named.next()?;
    if named.next().is_some() || only.kind() != "alias" {
        return None;
    }
    Some((clamp(only.start_byte()), clamp(only.end_byte())))
}

/// True when `arguments` holds a top-level comma (directive with options).
fn has_top_comma(arguments: &tree_sitter::Node<'_>) -> bool {
    let mut cursor = arguments.walk();
    arguments
        .children(&mut cursor)
        .any(|child| !child.is_named() && child.kind() == ",")
}

/// First top-level `Base.{Parts}` decomposition, if valid.
fn grouped_top(arguments: &tree_sitter::Node<'_>) -> Option<GroupedAlias> {
    let mut cursor = arguments.walk();
    let dot = arguments
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named)
        .find(|child| child.kind() == "dot")?;
    decompose_grouped(&dot)
}

/// One `Base.{Part, ...}` decomposition: alias base plus alias parts.
fn decompose_grouped(dot: &tree_sitter::Node<'_>) -> Option<GroupedAlias> {
    let mut cursor = dot.walk();
    let mut named = dot
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named);
    let (Some(base), Some(last)) = (named.next(), named.last()) else {
        return None;
    };
    if base.kind() != "alias" || last.kind() != "tuple" || base.id() == last.id() {
        return None;
    }
    let mut parts = Vec::new();
    let mut cursor = last.walk();
    for part in last
        .children(&mut cursor)
        .filter(|child| child.kind() == "alias")
    {
        parts.push((clamp(part.start_byte()), clamp(part.end_byte())));
    }
    Some(GroupedAlias {
        base_start: clamp(base.start_byte()),
        base_end: clamp(base.end_byte()),
        parts,
    })
}

/// Every `Base.{Parts}` decomposition in a directive subtree, in walk
/// order (mirrors the `forbidden_module` grouped scan).
fn grouped_subtree(node: &tree_sitter::Node<'_>) -> Vec<GroupedAlias> {
    let mut out = Vec::new();
    let mut stack = vec![*node];
    while let Some(current) = stack.pop() {
        if current.kind() == "dot"
            && let Some(grouped) = decompose_grouped(&current)
        {
            out.push(grouped);
        }
        let mut cursor = current.walk();
        stack.extend(current.children(&mut cursor));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn extract_source(source: &str) -> Facts {
        let tree = crate::ts_parser::parse(source).expect("test source parses");
        extract(&tree, source)
    }

    /// Rule modules walking trees must stay exactly the declared residual
    /// set: migrating a check closes its row here, and any new tree walk
    /// fails loudly instead of landing silently.
    #[test]
    fn residual_tree_walks() {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut walked = BTreeSet::new();
        let mut dirs = vec![manifest.join("src")];
        while let Some(dir) = dirs.pop() {
            let entries = std::fs::read_dir(&dir).expect("src readable");
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let content = std::fs::read_to_string(&path).expect("source readable");
                    if content.contains("tree_sitter") {
                        let relative = path
                            .strip_prefix(manifest.join("src"))
                            .expect("under src")
                            .to_string_lossy()
                            .into_owned();
                        walked.insert(relative);
                    }
                }
            }
        }
        let declared: BTreeSet<String> = RESIDUAL_TREE_WALKS
            .iter()
            .map(|(file, _)| (*file).to_owned())
            .chain(
                [
                    "batch.rs",
                    "config_file.rs",
                    "facts.rs",
                    "project.rs",
                    "project/collect_duplicated.rs",
                    "runner.rs",
                    "ts_parser.rs",
                ]
                .into_iter()
                .map(str::to_owned),
            )
            .collect();
        assert_eq!(walked, declared);
    }

    #[test]
    fn plain_assignment_has_no_calls() {
        let facts = extract_source("x = 1\n");
        assert!(facts.calls.is_empty());
        assert!(facts.def_ranges.is_empty());
        assert!(facts.quote_ranges.is_empty());
    }

    #[test]
    fn records_plain_remote_and_def_shapes() {
        let facts = extract_source("defmodule M do\n  def f, do: Mix.env()\nend\n");
        let mut heads: Vec<String> = Vec::new();
        let source = "defmodule M do\n  def f, do: Mix.env()\nend\n";
        for call in &facts.calls {
            match &call.head {
                Some(HeadFact::Plain { start, end }) => {
                    heads.push(source[*start as usize..*end as usize].to_owned());
                }
                Some(HeadFact::Remote {
                    mod_start,
                    mod_end,
                    fun_start,
                    fun_end,
                    ..
                }) => {
                    heads.push(format!(
                        "{}.{}",
                        &source[*mod_start as usize..*mod_end as usize],
                        &source[*fun_start as usize..*fun_end as usize]
                    ));
                }
                None => heads.push("<other>".to_owned()),
            }
        }
        heads.sort();
        assert_eq!(heads, vec!["Mix.env", "def", "defmodule"]);
        assert_eq!(facts.def_ranges.len(), 1);
        assert!(facts.quote_ranges.is_empty());
    }

    #[test]
    fn quote_with_args_is_a_skip_range() {
        let source = "defmodule M do\n  quote do\n    use ExUnit.Case\n  end\nend\n";
        let facts = extract_source(source);
        assert_eq!(facts.quote_ranges.len(), 1);
        let (start, end) = facts.quote_ranges[0];
        assert_eq!(&source[start as usize..end as usize][..5], "quote");
    }

    #[test]
    fn broken_source_still_extracts_without_panicking() {
        let facts = extract_source("def foo( do\n");
        let _ = facts.calls.len() + facts.def_ranges.len() + facts.quote_ranges.len();
    }

    #[test]
    fn records_var_idents_and_binding_regions() {
        let source = "defmodule M do\n  def f(_a, b) do\n    _c = _a\n  end\nend\n";
        let facts = extract_source(source);
        assert!(!facts.var_idents.is_empty());
        assert!(
            facts
                .var_idents
                .iter()
                .any(|ident| &source[ident.start as usize..ident.end as usize] == "_a")
        );
        assert!(!facts.bind_regions.is_empty());
    }

    #[test]
    fn module_bodies_classify_statements() {
        let source = "defmodule M do\n  @moduledoc \"Docs\"\n  use Foo\n  def f, do: 1\nend\n";
        let facts = extract_source(source);
        assert_eq!(facts.module_bodies.len(), 1);
        assert_eq!(facts.module_bodies[0].module, 0);
        let stmts = &facts.module_bodies[0].stmts;
        assert_eq!(stmts.len(), 3);
        let BodyStmtFact::Attr {
            name_start,
            name_end,
            value,
        } = &stmts[0]
        else {
            panic!("expected moduledoc attr, got {:?}", stmts[0]);
        };
        assert_eq!(
            &source[*name_start as usize..*name_end as usize],
            "moduledoc"
        );
        assert_eq!(*value, AttrValue::Other);
        let BodyStmtFact::Call {
            head_start,
            head_end,
            arg_alias,
        } = &stmts[1]
        else {
            panic!("expected use call, got {:?}", stmts[1]);
        };
        assert_eq!(&source[*head_start as usize..*head_end as usize], "use");
        let (alias_start, alias_end) = arg_alias.expect("use alias");
        assert_eq!(&source[alias_start as usize..alias_end as usize], "Foo");
        assert!(matches!(stmts[2], BodyStmtFact::Call { .. }));
    }

    #[test]
    fn records_alias_inventory_and_grouped_directives() {
        let source = "defmodule M do\n  alias Foo.{Bar, Baz}\n  alias Qux\nend\n";
        let facts = extract_source(source);
        let mut names: Vec<&str> = facts
            .aliases
            .iter()
            .map(|(start, end)| &source[*start as usize..*end as usize])
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["Bar", "Baz", "Foo", "M", "Qux"]);
        assert_eq!(facts.alias_directives.len(), 2);
        let grouped = &facts.alias_directives[0];
        assert!(grouped.single.is_none());
        let top = grouped.grouped_top.as_ref().expect("grouped top");
        assert_eq!(
            &source[top.base_start as usize..top.base_end as usize],
            "Foo"
        );
        assert_eq!(top.parts.len(), 2);
        assert_eq!(grouped.grouped_all.len(), 1);
        let single = &facts.alias_directives[1];
        assert!(single.grouped_top.is_none());
        let span = single.single.expect("single target");
        assert_eq!(&source[span.0 as usize..span.1 as usize], "Qux");
    }
}
