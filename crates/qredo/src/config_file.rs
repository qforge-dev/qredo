//! Static `.credo.exs` configuration reader.
//!
//! Parses the data-literal subset of a Credo config file (via the pinned
//! tree-sitter grammar, never by executing Elixir) into per-check kernel
//! params using the [`params.md`](compatibility/params.md) value encoding.
//! Anything executable or dynamic (calls, attributes, interpolation,
//! `__DIR__`, pins, anonymous functions) is an explicit [`UnsupportedConfig`],
//! never silent defaults.
//!
//! Sole exception: `System.get_env/1,2` with literal arguments. Each call is
//! resolved once against the process environment at config-load time and the
//! observation is recorded in [`CredoConfig::env_snapshot`]:
//!
//! - `System.get_env("FOO")` resolves to the value of `FOO`; an unset `FOO`
//!   fails closed, naming the variable.
//! - `System.get_env("FOO", default)` resolves to the value of `FOO`, or to
//!   the literal `default` (any other static data term, including a nested
//!   `System.get_env` call) when `FOO` is unset. A `nil` default on a miss
//!   fails closed like any other `nil` param.
//! - A non-literal variable name, any other arity, and every other call
//!   (`Mix.env`, `System.fetch_env`, `System.get_env!`, `__DIR__`, …) stay
//!   fail-closed.
//!
//! Snapshots are load-time views, not live bindings: later environment
//! changes do not alter an already-parsed config, and any future config
//! fingerprint must mix the snapshot or reuse across changed environments
//! becomes unsound.

use std::collections::BTreeMap;

/// One selected config block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredoConfig {
    /// Selected config name (e.g. `"default"`).
    pub name: String,
    /// Config-level `files.included` globs (plain strings only here).
    pub files_included: Vec<String>,
    /// Config-level `files.excluded` entries (strings or regex sources).
    pub files_excluded: Vec<FileEntry>,
    /// Enabled and disabled checks in file order.
    pub checks: Vec<CheckEntry>,
    /// Load-time environment snapshot: every `System.get_env` variable
    /// consulted while parsing, mapped to its observed value (`None` when
    /// unset and a literal default applied). Empty when the config uses no
    /// `System.get_env` call. There is no config-fingerprint consumer in
    /// this crate yet; any future fingerprint MUST mix each
    /// `(name, present?, value)` tuple alongside source bytes and tool
    /// identity, or reuse across changed environments becomes unsound.
    pub env_snapshot: BTreeMap<String, Option<String>>,
}

/// A config-level file entry: plain glob string or regex source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEntry {
    Glob(String),
    Regex(String),
}

/// One `{CheckModule, params}` entry with its enabled flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckEntry {
    /// Check module, e.g. `"Credo.Check.Warning.IoInspect"`.
    pub module: String,
    /// From the `enabled` (`true`) or `disabled` (`false`) list.
    pub enabled: bool,
    /// Kernel string params (atoms bare, lists/tuples/regexes as compact
    /// JSON per the params encoding contract; per-check `files:` maps are
    /// flattened to `files.included`/`files.excluded`).
    pub params: BTreeMap<String, String>,
}

/// A config construct outside the static data subset.
#[derive(Debug, PartialEq, Eq)]
pub struct UnsupportedConfig(pub String);

/// Parse the named config block from `.credo.exs` source.
///
/// `System.get_env/1,2` calls with literal arguments resolve against the
/// process environment once per call site at load time; see
/// [`CredoConfig::env_snapshot`]. Everything else executable stays
/// fail-closed.
///
/// # Errors
/// Returns [`UnsupportedConfig`] for non-map sources, missing config names,
/// executable/dynamic constructs, or malformed check entries.
pub fn parse_config(source: &str, config_name: &str) -> Result<CredoConfig, UnsupportedConfig> {
    parse_config_with_lookup(source, config_name, &|name| std::env::var(name).ok())
}

/// Shared parse over an injectable environment reader.
///
/// Production parsing passes the process environment; tests pass a fixed
/// table so parallel tests never mutate process-global state (workspace
/// `unsafe_code = "forbid"` rules out `std::env::set_var` in tests, and env
/// mutation would race under parallel execution anyway).
fn parse_config_with_lookup(
    source: &str,
    config_name: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<CredoConfig, UnsupportedConfig> {
    let mut env = Env {
        lookup,
        snapshot: BTreeMap::new(),
    };
    let tree = parse_tree(source)?;
    let root = single_map_child(&tree.root_node(), source)?;
    let pairs = map_pairs(&root, source)?;
    let configs = find_pair(&pairs, "configs", "top-level map")?;
    let mut selected = None;
    let mut available = Vec::new();
    for item in list_items(&configs, source, "configs")? {
        let block = expect_kind(&item, "map", "config block")?;
        let pairs = map_pairs(&block, source)?;
        let name = find_pair(&pairs, "name", "config block")?;
        let name = as_string(&name, source)?;
        available.push(name.clone());
        if name == config_name {
            selected = Some(parse_block(&block, &pairs, source, config_name, &mut env)?);
        }
    }
    selected
        .map(|mut config| {
            config.env_snapshot = env.snapshot;
            config
        })
        .ok_or_else(|| {
            UnsupportedConfig(format!(
                "config \"{config_name}\" not found (available: {})",
                available.join(", ")
            ))
        })
}

/// Load-time environment reader for the `System.get_env` carve-out.
///
/// Each variable is read at most once per parse (first observation wins),
/// so the snapshot is a single consistent view even if another thread
/// mutates the process environment mid-parse.
struct Env<'a> {
    lookup: &'a dyn Fn(&str) -> Option<String>,
    snapshot: BTreeMap<String, Option<String>>,
}

impl Env<'_> {
    /// Read one variable, recording the observation for the fingerprint.
    fn get(&mut self, name: &str) -> Option<String> {
        if let Some(known) = self.snapshot.get(name) {
            return known.clone();
        }
        let value = (self.lookup)(name);
        self.snapshot.insert(name.to_owned(), value.clone());
        value
    }
}

/// Parse one selected config block's pairs.
fn parse_block(
    _block: &tree_sitter::Node<'_>,
    pairs: &[(String, tree_sitter::Node<'_>)],
    source: &str,
    config_name: &str,
    env: &mut Env<'_>,
) -> Result<CredoConfig, UnsupportedConfig> {
    let mut files_included = Vec::new();
    let mut files_excluded = Vec::new();
    let mut checks = Vec::new();
    for (key, value) in pairs {
        match key.as_str() {
            "files" => {
                let files = expect_kind(value, "map", "files map")?;
                let pairs = map_pairs(&files, source)?;
                if let Some(included) = lookup(&pairs, "included") {
                    for item in list_items(&included, source, "files.included")? {
                        files_included.push(as_glob(&item, source)?);
                    }
                }
                if let Some(excluded) = lookup(&pairs, "excluded") {
                    for item in list_items(&excluded, source, "files.excluded")? {
                        files_excluded.push(as_file_entry(&item, source)?);
                    }
                }
            }
            "checks" => checks = parse_checks(value, source, env)?,
            "requires" | "plugins" => {
                reject_executable_key(key, value, source)?;
            }
            _ => {}
        }
    }
    Ok(CredoConfig {
        name: config_name.to_owned(),
        files_included,
        files_excluded,
        checks,
        env_snapshot: BTreeMap::new(),
    })
}

/// Executable config keys must be absent or empty.
fn reject_executable_key(
    key: &str,
    value: &tree_sitter::Node<'_>,
    source: &str,
) -> Result<(), UnsupportedConfig> {
    let empty = match value.kind() {
        "list" => list_items(value, source, key)?.is_empty(),
        _ => false,
    };
    if empty {
        Ok(())
    } else {
        Err(UnsupportedConfig(format!(
            "executable config key `{key}` needs native execution"
        )))
    }
}

/// Parse the `checks:` value (enabled/disabled map or bare list).
fn parse_checks(
    node: &tree_sitter::Node<'_>,
    source: &str,
    env: &mut Env<'_>,
) -> Result<Vec<CheckEntry>, UnsupportedConfig> {
    if node.kind() == "list" {
        let mut out = Vec::new();
        for item in list_items(node, source, "checks")? {
            out.push(parse_check_entry(&item, source, true, env)?);
        }
        return Ok(out);
    }
    let map = expect_kind(node, "map", "checks map")?;
    let pairs = map_pairs(&map, source)?;
    let mut out = Vec::new();
    for key in ["enabled", "disabled"] {
        let Some(list) = lookup(&pairs, key) else {
            continue;
        };
        for item in list_items(&list, source, key)? {
            out.push(parse_check_entry(&item, source, key == "enabled", env)?);
        }
    }
    for (key, _) in &pairs {
        if key != "enabled" && key != "disabled" {
            return Err(UnsupportedConfig(format!("unknown checks key `{key}`")));
        }
    }
    Ok(out)
}

/// Parse one `{CheckModule, params}` tuple.
fn parse_check_entry(
    node: &tree_sitter::Node<'_>,
    source: &str,
    listed_enabled: bool,
    env: &mut Env<'_>,
) -> Result<CheckEntry, UnsupportedConfig> {
    let tuple = expect_kind(node, "tuple", "check entry")?;
    let items = named_non_comment(&tuple);
    match items.as_slice() {
        [module_node] => Ok(CheckEntry {
            module: check_module_name(module_node, source)?,
            enabled: listed_enabled,
            params: BTreeMap::new(),
        }),
        [_, _] => parse_check_pair(&items, source, listed_enabled, env),
        _ => Err(UnsupportedConfig(
            "check entry must be {Module} or {Module, params}".to_owned(),
        )),
    }
}

/// Parse a two-element `{CheckModule, params}` tuple.
fn parse_check_pair(
    items: &[tree_sitter::Node<'_>],
    source: &str,
    listed_enabled: bool,
    env: &mut Env<'_>,
) -> Result<CheckEntry, UnsupportedConfig> {
    let module = check_module_name(&items[0], source)?;
    let params_node = &items[1];
    if params_node.kind() == "boolean" && text_of(params_node, source)? == "false" {
        return Ok(CheckEntry {
            module,
            enabled: false,
            params: BTreeMap::new(),
        });
    }
    let list = expect_kind(params_node, "list", "check params")?;
    let mut params = BTreeMap::new();
    for item in list_items(&list, source, "check params")? {
        // Single-line keyword lists nest pairs under one `keywords` node.
        if item.kind() == "keywords" {
            let mut inner = item.walk();
            for pair in item
                .children(&mut inner)
                .filter(|child| child.kind() == "pair")
            {
                let (key, value) = pair_key_value(&pair, source)?;
                insert_param(&mut params, &key, &value, source, env)?;
            }
            continue;
        }
        let pair = expect_kind(&item, "pair", "param entry")?;
        let (key, value) = pair_key_value(&pair, source)?;
        insert_param(&mut params, &key, &value, source, env)?;
    }
    Ok(CheckEntry {
        module,
        enabled: listed_enabled,
        params,
    })
}

/// Parse and validate Elixir source with the pinned grammar.
fn parse_tree(source: &str) -> Result<tree_sitter::Tree, UnsupportedConfig> {
    let tree = crate::ts_parser::parse(source)
        .ok_or_else(|| UnsupportedConfig("config source has syntax errors".to_owned()))?;
    if tree.root_node().has_error() {
        return Err(UnsupportedConfig(
            "config source has syntax errors".to_owned(),
        ));
    }
    Ok(tree)
}

/// Source slice of a node.
fn text_of<'src>(
    node: &tree_sitter::Node<'_>,
    source: &'src str,
) -> Result<&'src str, UnsupportedConfig> {
    node.utf8_text(source.as_bytes())
        .map_err(|_| UnsupportedConfig("config source is not valid UTF-8 ranges".to_owned()))
}

/// Named children excluding comments.
fn named_non_comment<'tree>(node: &tree_sitter::Node<'tree>) -> Vec<tree_sitter::Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
        .collect()
}

/// The single top-level map of a config file.
fn single_map_child<'tree>(
    root: &tree_sitter::Node<'tree>,
    source: &str,
) -> Result<tree_sitter::Node<'tree>, UnsupportedConfig> {
    let _ = source;
    let mut maps = named_non_comment(root)
        .into_iter()
        .filter(|child| child.kind() == "map");
    match (maps.next(), maps.next()) {
        (Some(map), None) => Ok(map),
        _ => Err(UnsupportedConfig(
            "config source must be a single top-level map".to_owned(),
        )),
    }
}

/// Keyword pairs of a map's content.
fn map_pairs<'tree>(
    map: &tree_sitter::Node<'tree>,
    source: &str,
) -> Result<Vec<(String, tree_sitter::Node<'tree>)>, UnsupportedConfig> {
    let mut cursor = map.walk();
    let contents: Vec<tree_sitter::Node<'_>> = map
        .children(&mut cursor)
        .filter(|child| child.kind() == "map_content")
        .collect();
    if contents.len() != 1 {
        return Err(UnsupportedConfig(
            "config maps hold one content block".to_owned(),
        ));
    }
    let mut out = Vec::new();
    let mut cursor = contents[0].walk();
    for child in contents[0]
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment")
    {
        if child.kind() == "keywords" {
            let mut inner = child.walk();
            for pair in child
                .children(&mut inner)
                .filter(|child| child.kind() == "pair")
            {
                let (key, value) = pair_key_value(&pair, source)?;
                out.push((key, value));
            }
        } else {
            return Err(UnsupportedConfig(
                "config maps hold keyword entries".to_owned(),
            ));
        }
    }
    Ok(out)
}

/// Insert one param, flattening per-check `files:` maps to the dotted
/// `files.included`/`files.excluded` keys the selection layer reads.
fn insert_param(
    params: &mut BTreeMap<String, String>,
    key: &str,
    value: &tree_sitter::Node<'_>,
    source: &str,
    env: &mut Env<'_>,
) -> Result<(), UnsupportedConfig> {
    if key == "files" && value.kind() == "map" {
        let map = map_pairs(value, source)?;
        for entry_key in ["included", "excluded"] {
            if let Some(list) = lookup(&map, entry_key) {
                let mut globs = Vec::new();
                for item in list_items(&list, source, "files map")? {
                    match item.kind() {
                        "string" => globs.push(string_raw(&item, source)?),
                        _ => {
                            return Err(UnsupportedConfig(format!(
                                "per-check files.{entry_key} supports only plain strings"
                            )));
                        }
                    }
                }
                params.insert(format!("files.{entry_key}"), globs.join(","));
            }
        }
        return Ok(());
    }
    params.insert(
        key.to_owned(),
        kernel_string(&encode_term(value, source, env)?),
    );
    Ok(())
}
/// Module name of a check entry; custom modules need native execution.
fn check_module_name(
    node: &tree_sitter::Node<'_>,
    source: &str,
) -> Result<String, UnsupportedConfig> {
    if node.kind() != "alias" {
        return Err(UnsupportedConfig(
            "check entry head must be a module alias".to_owned(),
        ));
    }
    let name = text_of(node, source)?.to_owned();
    if name.starts_with("Credo.Check.") {
        Ok(name)
    } else {
        Err(UnsupportedConfig(format!(
            "custom check module `{name}` needs native execution"
        )))
    }
}

/// Key and value of a `key: value` pair.
fn pair_key_value<'tree>(
    pair: &tree_sitter::Node<'tree>,
    source: &str,
) -> Result<(String, tree_sitter::Node<'tree>), UnsupportedConfig> {
    let mut cursor = pair.walk();
    let mut parts = pair
        .children(&mut cursor)
        .filter(|child| child.is_named() && child.kind() != "comment");
    let key = parts
        .next()
        .ok_or_else(|| UnsupportedConfig("config pair without a key".to_owned()))?;
    let value = parts
        .next()
        .ok_or_else(|| UnsupportedConfig("config pair without a value".to_owned()))?;
    if key.kind() != "keyword" {
        return Err(UnsupportedConfig("config keys must be atoms".to_owned()));
    }
    let name = text_of(&key, source)?
        .trim_end()
        .strip_suffix(':')
        .unwrap_or_default()
        .to_owned();
    Ok((name, value))
}

/// Lookup helper over pair lists.
fn lookup<'tree>(
    pairs: &[(String, tree_sitter::Node<'tree>)],
    key: &str,
) -> Option<tree_sitter::Node<'tree>> {
    pairs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, node)| *node)
}

/// Required pair lookup with context.
fn find_pair<'tree>(
    pairs: &[(String, tree_sitter::Node<'tree>)],
    key: &str,
    context: &str,
) -> Result<tree_sitter::Node<'tree>, UnsupportedConfig> {
    lookup(pairs, key).ok_or_else(|| UnsupportedConfig(format!("{context} without `{key}`")))
}

/// Assert a node kind with context.
fn expect_kind<'tree>(
    node: &tree_sitter::Node<'tree>,
    kind: &str,
    context: &str,
) -> Result<tree_sitter::Node<'tree>, UnsupportedConfig> {
    if node.kind() == kind {
        Ok(*node)
    } else {
        Err(UnsupportedConfig(format!(
            "{context} must be {kind}, found {}",
            node.kind()
        )))
    }
}

/// Element nodes of a list.
fn list_items<'tree>(
    node: &tree_sitter::Node<'tree>,
    source: &str,
    context: &str,
) -> Result<Vec<tree_sitter::Node<'tree>>, UnsupportedConfig> {
    let list = expect_kind(node, "list", context)?;
    let _ = source;
    Ok(named_non_comment(&list))
}

/// Plain glob string of a files entry.
fn as_glob(node: &tree_sitter::Node<'_>, source: &str) -> Result<String, UnsupportedConfig> {
    match node.kind() {
        "string" => string_raw(node, source),
        _ => Err(UnsupportedConfig(
            "files.included entries must be plain strings".to_owned(),
        )),
    }
}

/// Glob or regex entry of `files.excluded`.
fn as_file_entry(
    node: &tree_sitter::Node<'_>,
    source: &str,
) -> Result<FileEntry, UnsupportedConfig> {
    match node.kind() {
        "string" => Ok(FileEntry::Glob(string_raw(node, source)?)),
        "sigil" => Ok(FileEntry::Regex(sigil_raw(node, source, "files.excluded")?)),
        _ => Err(UnsupportedConfig(
            "files.excluded entries must be strings or regexes".to_owned(),
        )),
    }
}

/// Plain string contents; interpolation is dynamic.
fn as_string(node: &tree_sitter::Node<'_>, source: &str) -> Result<String, UnsupportedConfig> {
    if node.kind() != "string" {
        return Err(UnsupportedConfig("config name must be a string".to_owned()));
    }
    string_raw(node, source)
}

/// Raw contents of a string or sigil body; interpolation is dynamic.
fn string_raw(node: &tree_sitter::Node<'_>, source: &str) -> Result<String, UnsupportedConfig> {
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "quoted_content" | "escape_sequence" => out.push_str(text_of(&child, source)?),
            "interpolation" => {
                return Err(UnsupportedConfig(
                    "interpolated strings need native execution".to_owned(),
                ));
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Raw source of a `~r` sigil without modifiers.
fn sigil_raw(
    node: &tree_sitter::Node<'_>,
    source: &str,
    context: &str,
) -> Result<String, UnsupportedConfig> {
    let mut cursor = node.walk();
    let mut name = None;
    let mut modifiers = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "sigil_name" => name = Some(text_of(&child, source)?),
            "sigil_modifiers" => modifiers = true,
            _ => {}
        }
    }
    match name {
        Some("r") if !modifiers => string_raw(node, source),
        _ => Err(UnsupportedConfig(format!(
            "{context} supports only modifier-free `~r` sigils"
        ))),
    }
}

/// Kernel string encoding of a config term (mirrors the case-harness
/// `param_string` contract: bare atoms strip one colon, structured
/// values use compact JSON).
fn kernel_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => text.strip_prefix(':').unwrap_or(text).to_owned(),
        structured => serde_json::to_string(structured).unwrap_or_default(),
    }
}

/// Encode an Elixir data term to the params-contract JSON value.
fn encode_term(
    node: &tree_sitter::Node<'_>,
    source: &str,
    env: &mut Env<'_>,
) -> Result<serde_json::Value, UnsupportedConfig> {
    if let Some(scalar) = encode_scalar(node, source)? {
        return Ok(scalar);
    }
    match node.kind() {
        "alias" => {
            let dotted = text_of(node, source)?;
            let name = dotted.strip_prefix("Elixir.").unwrap_or(dotted);
            Ok(serde_json::Value::String(format!(":Elixir.{name}")))
        }
        "list" => {
            let mut items = Vec::new();
            for item in list_items(node, source, "list param")? {
                items.push(encode_term(&item, source, env)?);
            }
            Ok(serde_json::Value::Array(items))
        }
        "tuple" => {
            let mut items = Vec::new();
            for item in named_non_comment(node) {
                items.push(encode_term(&item, source, env)?);
            }
            let mut map = serde_json::Map::new();
            map.insert("tuple".to_owned(), serde_json::Value::Array(items));
            Ok(serde_json::Value::Object(map))
        }
        "sigil" => encode_sigil(node, source),
        "binary_operator" => encode_range(node, source),
        "unary_operator" => encode_signed(node, source, env),
        "call" => match encode_get_env(node, source, env)? {
            Some(value) => Ok(value),
            None => Err(UnsupportedConfig(
                "executable term of kind `call` needs native execution".to_owned(),
            )),
        },
        _ => Err(UnsupportedConfig(format!(
            "executable term of kind `{}` needs native execution",
            node.kind()
        ))),
    }
}

/// True for a `System.get_env` remote-call target (`dot` of alias
/// `System` and identifier `get_env`); anything else keeps the generic
/// fail-closed call reason.
fn is_system_get_env(target: &tree_sitter::Node<'_>, source: &str) -> bool {
    if target.kind() != "dot" {
        return false;
    }
    let receiver = target
        .child_by_field_name("left")
        .filter(|left| left.kind() == "alias")
        .is_some_and(|left| text_of(&left, source).is_ok_and(|text| text == "System"));
    let function = target
        .child_by_field_name("right")
        .filter(|right| right.kind() == "identifier")
        .is_some_and(|right| text_of(&right, source).is_ok_and(|text| text == "get_env"));
    receiver && function
}
/// Resolve the sole admitted executable form, `System.get_env/1,2`.
///
/// Returns `Ok(None)` for any other call so the caller keeps the generic
/// fail-closed reason. Mirrors native `System.get_env(name, default \\ nil)`
/// except that an unset variable without a (non-`nil`) literal default is an
/// explicit [`UnsupportedConfig`] naming the variable instead of `nil`.
fn encode_get_env(
    node: &tree_sitter::Node<'_>,
    source: &str,
    env: &mut Env<'_>,
) -> Result<Option<serde_json::Value>, UnsupportedConfig> {
    let Some(target) = node.child_by_field_name("target") else {
        return Ok(None);
    };
    if !is_system_get_env(&target, source) {
        return Ok(None);
    }
    let Some(arguments) = node
        .children(&mut node.walk())
        .find(|child| child.kind() == "arguments")
    else {
        return Ok(None);
    };
    let args = named_non_comment(&arguments);
    let (name_node, default_node) = match args.as_slice() {
        [name] => (*name, None),
        [name, default] => (*name, Some(*default)),
        _ => {
            return Err(UnsupportedConfig(format!(
                "System.get_env takes 1 or 2 arguments (found {})",
                args.len()
            )));
        }
    };
    if name_node.kind() != "string" {
        return Err(UnsupportedConfig(format!(
            "System.get_env variable name must be a literal string (found {})",
            name_node.kind()
        )));
    }
    let name = string_raw(&name_node, source)?;
    match env.get(&name) {
        Some(value) => Ok(Some(serde_json::Value::String(value))),
        None => match default_node {
            None => Err(UnsupportedConfig(format!(
                "System.get_env(\"{name}\") is unset and has no default"
            ))),
            Some(default) if default.kind() == "nil" => Err(UnsupportedConfig(format!(
                "System.get_env(\"{name}\") is unset and defaults to nil"
            ))),
            Some(default) => Ok(Some(encode_term(&default, source, env)?)),
        },
    }
}

/// Scalar literals (numbers, booleans, atoms, strings); `None` to continue.
fn encode_scalar(
    node: &tree_sitter::Node<'_>,
    source: &str,
) -> Result<Option<serde_json::Value>, UnsupportedConfig> {
    match node.kind() {
        "integer" => {
            let text = text_of(node, source)?;
            text.parse::<i64>().map_or_else(
                |_| {
                    Err(UnsupportedConfig(format!(
                        "unsupported integer literal `{text}`"
                    )))
                },
                |value| Ok(Some(serde_json::Value::Number(value.into()))),
            )
        }
        "float" => {
            let text = text_of(node, source)?;
            text.parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map_or_else(
                    || {
                        Err(UnsupportedConfig(format!(
                            "unsupported float literal `{text}`"
                        )))
                    },
                    |value| Ok(Some(serde_json::Value::Number(value))),
                )
        }
        "boolean" => match text_of(node, source)? {
            "true" => Ok(Some(serde_json::Value::Bool(true))),
            "false" => Ok(Some(serde_json::Value::Bool(false))),
            text => Err(UnsupportedConfig(format!("unexpected boolean `{text}`"))),
        },
        "atom" => Ok(Some(serde_json::Value::String(format!(
            ":{}",
            text_of(node, source)?.trim_start_matches(':')
        )))),
        "nil" => Err(UnsupportedConfig(
            "nil params need native execution".to_owned(),
        )),
        "string" => Ok(Some(serde_json::Value::String(string_raw(node, source)?))),
        _ => Ok(None),
    }
}

/// Sigil values: regex sources, word lists, and plain strings.
fn encode_sigil(
    node: &tree_sitter::Node<'_>,
    source: &str,
) -> Result<serde_json::Value, UnsupportedConfig> {
    let mut cursor = node.walk();
    let mut name = None;
    let mut modifiers = String::new();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "sigil_name" => name = Some(text_of(&child, source)?),
            "sigil_modifiers" => modifiers.push_str(text_of(&child, source)?),
            _ => {}
        }
    }
    let raw = string_raw(node, source)?;
    match name {
        Some("r") if modifiers.is_empty() => {
            let mut map = serde_json::Map::new();
            map.insert("regex".to_owned(), serde_json::Value::String(raw));
            Ok(serde_json::Value::Object(map))
        }
        Some("w" | "W") => {
            let words: Vec<&str> = raw.split_whitespace().collect();
            let items = if modifiers.contains('a') {
                words
                    .iter()
                    .map(|word| serde_json::Value::String(format!(":{word}")))
                    .collect()
            } else if modifiers.contains('c') {
                return Err(UnsupportedConfig(
                    "charlist word sigils need native execution".to_owned(),
                ));
            } else {
                words
                    .iter()
                    .map(|word| serde_json::Value::String((*word).to_owned()))
                    .collect()
            };
            Ok(serde_json::Value::Array(items))
        }
        Some("s" | "S") if !modifiers.contains('a') && !modifiers.contains('c') => {
            Ok(serde_json::Value::String(raw))
        }
        _ => Err(UnsupportedConfig(
            "only ~r/~w/~s sigils without dynamic modifiers parse statically".to_owned(),
        )),
    }
}

/// Integer ranges of shape `lo..hi`.
fn encode_range(
    node: &tree_sitter::Node<'_>,
    source: &str,
) -> Result<serde_json::Value, UnsupportedConfig> {
    let mut cursor = node.walk();
    let operator = node
        .children(&mut cursor)
        .find(|child| !child.is_named())
        .map(|child| text_of(&child, source).unwrap_or_default());
    if operator != Some("..") {
        return Err(UnsupportedConfig(
            "only `..` ranges parse statically".to_owned(),
        ));
    }
    let mut ends = Vec::new();
    let mut cursor = node.walk();
    for child in node
        .children(&mut cursor)
        .filter(tree_sitter::Node::is_named)
    {
        let text = text_of(&child, source)?;
        ends.push(
            text.parse::<i64>()
                .map_err(|_| UnsupportedConfig(format!("unsupported range end `{text}`")))?,
        );
    }
    if ends.len() != 2 {
        return Err(UnsupportedConfig("ranges hold two integer ends".to_owned()));
    }
    Ok(serde_json::json!({"range": ends}))
}

/// Signed number literals (`-3`, `+1.5`).
fn encode_signed(
    node: &tree_sitter::Node<'_>,
    source: &str,
    env: &mut Env<'_>,
) -> Result<serde_json::Value, UnsupportedConfig> {
    let mut cursor = node.walk();
    let mut sign = None;
    let mut operand = None;
    for child in node.children(&mut cursor) {
        if child.is_named() {
            if operand.is_some() {
                return Err(UnsupportedConfig(
                    "signed literals hold one operand".to_owned(),
                ));
            }
            operand = Some(child);
        } else if sign.is_none() {
            sign = Some(text_of(&child, source)?);
        }
    }
    let operand =
        operand.ok_or_else(|| UnsupportedConfig("signed literal without operand".to_owned()))?;
    if !matches!(sign, Some("-" | "+")) || !matches!(operand.kind(), "integer" | "float") {
        return Err(UnsupportedConfig(
            "only signed number literals parse statically".to_owned(),
        ));
    }
    let mut value = encode_term(&operand, source, env)?;
    if sign == Some("-") {
        if let Some(number) = value.as_i64() {
            value = serde_json::Value::Number((-number).into());
        } else if let Some(number) = value.as_f64() {
            value = serde_json::Number::from_f64(-number).map_or(value, serde_json::Value::Number);
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "%{\n  configs: [\n    %{\n      name: \"default\",\n      files: %{included: [\"lib/\", \"test/\"]},\n      checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}]}\n    }\n  ]\n}\n";

    /// Fixed-table parse so parallel tests never mutate process-global env.
    /// Unique `QREDO_TEST_GETENV_*` names additionally guard against real
    /// environment collisions.
    fn parse_with_env(
        source: &str,
        vars: &BTreeMap<String, String>,
    ) -> Result<CredoConfig, UnsupportedConfig> {
        parse_config_with_lookup(source, "default", &|name| vars.get(name).cloned())
    }

    fn table(vars: &[(&str, &str)]) -> BTreeMap<String, String> {
        vars.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn param_source(value_src: &str) -> String {
        format!(
            "%{{configs: [%{{name: \"default\", checks: %{{enabled: [{{Credo.Check.Warning.IoInspect, [label: {value_src}]}}]}}}}]}}\n"
        )
    }

    #[test]
    fn getenv_set_var_resolves_and_snapshots() {
        let vars = table(&[("QREDO_TEST_GETENV_SET", "fast")]);
        let source = param_source("System.get_env(\"QREDO_TEST_GETENV_SET\")");
        let config = parse_with_env(&source, &vars).expect("set var resolves");
        assert_eq!(
            config.checks[0].params.get("label").map(String::as_str),
            Some("fast")
        );
        assert_eq!(
            config.env_snapshot.get("QREDO_TEST_GETENV_SET"),
            Some(&Some("fast".to_owned()))
        );
    }

    #[test]
    fn getenv_unset_without_default_fails_closed_naming_var() {
        let source = param_source("System.get_env(\"QREDO_TEST_GETENV_UNSET\")");
        let error = parse_with_env(&source, &table(&[])).expect_err("unset rejects");
        assert!(
            error.0.contains("QREDO_TEST_GETENV_UNSET"),
            "names the variable: {error:?}"
        );
    }

    #[test]
    fn getenv_unset_with_default_uses_default() {
        let vars = table(&[]);
        let source = param_source("System.get_env(\"QREDO_TEST_GETENV_DFLT\", \"dflt\")");
        let config = parse_with_env(&source, &vars).expect("default applies");
        assert_eq!(
            config.checks[0].params.get("label").map(String::as_str),
            Some("dflt")
        );
        assert_eq!(
            config.env_snapshot.get("QREDO_TEST_GETENV_DFLT"),
            Some(&None)
        );
    }

    #[test]
    fn getenv_set_var_wins_over_default() {
        let vars = table(&[("QREDO_TEST_GETENV_WIN", "fast")]);
        let source = param_source("System.get_env(\"QREDO_TEST_GETENV_WIN\", \"dflt\")");
        let config = parse_with_env(&source, &vars).expect("set var wins");
        assert_eq!(
            config.checks[0].params.get("label").map(String::as_str),
            Some("fast")
        );
    }

    #[test]
    fn getenv_nonliteral_name_fails_closed() {
        for value_src in ["System.get_env(name)", "System.get_env(\"A\" <> \"B\")"] {
            let source = param_source(value_src);
            assert!(
                parse_with_env(&source, &table(&[])).is_err(),
                "non-literal name rejected: {value_src}"
            );
        }
    }

    #[test]
    fn getenv_nested_inside_list_param() {
        let vars = table(&[("QREDO_TEST_GETENV_NEST", "fast")]);
        let source = param_source("[System.get_env(\"QREDO_TEST_GETENV_NEST\"), :extra]");
        let config = parse_with_env(&source, &vars).expect("nested resolves");
        assert_eq!(
            config.checks[0].params.get("label").map(String::as_str),
            Some("[\"fast\",\":extra\"]")
        );
        assert_eq!(
            config.env_snapshot.get("QREDO_TEST_GETENV_NEST"),
            Some(&Some("fast".to_owned()))
        );
    }

    #[test]
    fn getenv_wrong_arity_fails_closed() {
        for value_src in ["System.get_env()", "System.get_env(\"A\", \"b\", \"c\")"] {
            let source = param_source(value_src);
            assert!(
                parse_with_env(&source, &table(&[])).is_err(),
                "wrong arity rejected: {value_src}"
            );
        }
    }

    #[test]
    fn getenv_nil_default_on_miss_fails_closed() {
        let source = param_source("System.get_env(\"QREDO_TEST_GETENV_NIL\", nil)");
        let error = parse_with_env(&source, &table(&[])).expect_err("nil miss rejects");
        assert!(
            error.0.contains("QREDO_TEST_GETENV_NIL"),
            "names the variable: {error:?}"
        );
    }

    #[test]
    fn getenv_snapshot_records_every_consulted_var() {
        let vars = table(&[("QREDO_TEST_GETENV_SNAP_SET", "fast")]);
        let source = param_source(
            "[System.get_env(\"QREDO_TEST_GETENV_SNAP_SET\"), System.get_env(\"QREDO_TEST_GETENV_SNAP_MISS\", \"d\")]",
        );
        let config = parse_with_env(&source, &vars).expect("both resolve");
        assert_eq!(
            config.env_snapshot.get("QREDO_TEST_GETENV_SNAP_SET"),
            Some(&Some("fast".to_owned()))
        );
        assert_eq!(
            config.env_snapshot.get("QREDO_TEST_GETENV_SNAP_MISS"),
            Some(&None)
        );
    }

    #[test]
    fn getenv_real_path_unset_fails_closed_naming_var() {
        // No env mutation: a unique surely-absent name exercises the real
        // `std::env` lookup through the public entry point.
        let source = param_source("System.get_env(\"QREDO_TEST_GETENV_ABSENT_QREDO\")");
        let error = parse_config(&source, "default").expect_err("unset rejects");
        assert!(
            error.0.contains("QREDO_TEST_GETENV_ABSENT_QREDO"),
            "names the variable: {error:?}"
        );
    }

    #[test]
    fn getenv_other_calls_stay_fail_closed() {
        for value_src in [
            "Mix.env()",
            "System.fetch_env(\"QREDO_TEST_GETENV_SET\")",
            "System.get_env!(\"QREDO_TEST_GETENV_SET\")",
        ] {
            let vars = table(&[("QREDO_TEST_GETENV_SET", "fast")]);
            let source = param_source(value_src);
            assert!(
                parse_with_env(&source, &vars).is_err(),
                "still rejected: {value_src}"
            );
        }
    }

    #[test]
    fn minimal_config_parses() {
        let config = parse_config(MINIMAL, "default").expect("minimal parses");
        assert_eq!(config.name, "default");
        assert_eq!(
            config.files_included,
            vec!["lib/".to_owned(), "test/".to_owned()]
        );
        assert!(config.files_excluded.is_empty());
        assert_eq!(config.checks.len(), 1);
        assert_eq!(config.checks[0].module, "Credo.Check.Warning.IoInspect");
        assert!(config.checks[0].enabled);
        assert!(config.checks[0].params.is_empty());
    }

    #[test]
    fn scalar_params_encode_like_kernels_expect() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Refactor.CyclomaticComplexity, [max_complexity: 8, burn: true, mode: :strict, label: \"x\"]}]}}]}\n";
        let config = parse_config(source, "default").expect("scalars parse");
        let params = &config.checks[0].params;
        assert_eq!(params.get("max_complexity").map(String::as_str), Some("8"));
        assert_eq!(params.get("burn").map(String::as_str), Some("true"));
        assert_eq!(params.get("mode").map(String::as_str), Some("strict"));
        assert_eq!(params.get("label").map(String::as_str), Some("x"));
    }

    #[test]
    fn structured_params_use_compact_json() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Refactor.Apply, [only: [:foo, \"bar\"], mfa: {Foo, :bar, \"baz\"}, range: 1..3, pattern: ~r/^x+$/]}]}}]}\n";
        let config = parse_config(source, "default").expect("structured parses");
        let params = &config.checks[0].params;
        assert_eq!(
            params.get("only").map(String::as_str),
            Some("[\":foo\",\"bar\"]")
        );
        assert_eq!(
            params.get("mfa").map(String::as_str),
            Some("{\"tuple\":[\":Elixir.Foo\",\":bar\",\"baz\"]}")
        );
        assert_eq!(
            params.get("range").map(String::as_str),
            Some("{\"range\":[1,3]}")
        );
        assert_eq!(
            params.get("pattern").map(String::as_str),
            Some("{\"regex\":\"^x+$\"}")
        );
    }

    #[test]
    fn files_map_flattens_and_regex_excludes_keep_source() {
        let source = "%{configs: [%{name: \"default\", files: %{included: [\"lib/\"], excluded: [~r\"/_build/\", \"tmp/\"]}, checks: %{enabled: []}}]}\n";
        let config = parse_config(source, "default").expect("files parse");
        assert_eq!(config.files_included, vec!["lib/".to_owned()]);
        assert_eq!(
            config.files_excluded,
            vec![
                FileEntry::Regex("/_build/".to_owned()),
                FileEntry::Glob("tmp/".to_owned()),
            ]
        );
    }

    #[test]
    fn disabled_and_false_entries_are_disabled() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, []}], disabled: [{Credo.Check.Refactor.UtcNowTruncate, []}, {Credo.Check.Warning.MixEnv, false}]}}]}\n";
        let config = parse_config(source, "default").expect("disabled parses");
        assert_eq!(config.checks.len(), 3);
        assert!(config.checks[0].enabled);
        assert!(!config.checks[1].enabled);
        assert!(!config.checks[2].enabled);
        assert!(config.checks[2].params.is_empty());
    }

    #[test]
    fn per_check_files_map_flattens() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, [files: %{included: [\"special/\"], excluded: [\"special/old/\"]}]}]}}]}\n";
        let config = parse_config(source, "default").expect("files map flattens");
        let params = &config.checks[0].params;
        assert_eq!(
            params.get("files.included").map(String::as_str),
            Some("special/")
        );
        assert_eq!(
            params.get("files.excluded").map(String::as_str),
            Some("special/old/")
        );
    }

    #[test]
    fn per_check_files_regex_is_explicit() {
        let source = "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, [files: %{included: [~r\"x\"]}]}]}}]}\n";
        assert!(parse_config(source, "default").is_err());
    }

    #[test]
    fn list_form_checks_are_all_enabled() {
        let source =
            "%{configs: [%{name: \"default\", checks: [{Credo.Check.Warning.IoInspect, []}]}]}\n";
        let config = parse_config(source, "default").expect("list form parses");
        assert_eq!(config.checks.len(), 1);
        assert!(config.checks[0].enabled);
    }

    #[test]
    fn missing_config_name_is_explicit() {
        let error = parse_config(MINIMAL, "other").expect_err("missing name errors");
        assert!(error.0.contains("other"), "names the missing config");
    }

    #[test]
    fn executable_constructs_are_explicit() {
        for source in [
            "%{configs: [%{name: \"default\", checks: Mix.env()}]}\n",
            "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, [path: System.fetch_env(\"X\")]}]}}]}\n",
            "%{configs: [%{name: \"default\", checks: %{enabled: [{Credo.Check.Warning.IoInspect, [label: \"a#{1}b\"]}]}}]}\n",
        ] {
            assert!(
                parse_config(source, "default").is_err(),
                "executable rejected: {source}"
            );
        }
    }

    #[test]
    fn labqoat_shaped_excerpt_parses() {
        let source = "%{\n  configs: [\n    %{\n      name: \"default\",\n      files: %{included: [\"lib/\", \"test/\"], excluded: [~r\"/_build/\", ~r\"/deps/\"]},\n      checks: %{\n        enabled: [\n          {Credo.Check.Refactor.CyclomaticComplexity, [max_complexity: 8]},\n          {Credo.Check.Design.TagTODO, [exit_status: 2]}\n        ],\n        disabled: [{Credo.Check.Refactor.UtcNowTruncate, []}]\n      }\n    }\n  ]\n}\n";
        let config = parse_config(source, "default").expect("labqoat shapes parse");
        assert_eq!(config.checks.len(), 3);
        assert_eq!(
            config.checks[0]
                .params
                .get("max_complexity")
                .map(String::as_str),
            Some("8")
        );
        assert_eq!(
            config.checks[1]
                .params
                .get("exit_status")
                .map(String::as_str),
            Some("2")
        );
        assert!(!config.checks[2].enabled);
    }
}
