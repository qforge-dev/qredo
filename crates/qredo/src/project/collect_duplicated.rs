//! EX2002: exact structural interning over a flat tree-sitter normalization.
//! Each descriptor is interned once, mass is computed bottom-up, and pruning
//! propagates over the identity DAG once. Runtime work never invokes Elixir.
use std::collections::{BTreeMap, HashMap, HashSet};

use super::{ProjectFile, ProjectIssue};
#[path = "duplicated/literals.rs"]
mod literals;
#[path = "duplicated/lower.rs"]
mod lower;
#[path = "duplicated/model.rs"]
mod model;
pub(crate) use model::Summary;
use model::{Id, Shape};

#[cfg(test)]
#[path = "duplicated/tests.rs"]
mod tests;

pub(crate) const RULE: &str = "Credo.Check.Design.DuplicatedCode";

pub(crate) fn summarize(tree: &tree_sitter::Tree, source: &str) -> Summary {
    lower::lower(tree, source)
}

pub(crate) fn run(files: &[ProjectFile], params: &BTreeMap<String, String>) -> Vec<ProjectIssue> {
    run_with_trees(files, &[], params)
}

pub(crate) fn run_with_trees(
    files: &[ProjectFile],
    trees: &[Option<&tree_sitter::Tree>],
    params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    let summaries: Vec<Summary> = crate::batch::parallel_map(files, |index, file| {
        match trees.get(index).copied().flatten() {
            Some(tree) => summarize(tree, &file.source),
            None => crate::ts_parser::parse(&file.source)
                .map_or_else(Summary::default, |tree| summarize(&tree, &file.source)),
        }
    })
    .into_iter()
    .map(|(_, summary)| summary)
    .collect();
    let names: Vec<&str> = files.iter().map(|file| file.filename.as_str()).collect();
    let summaries: Vec<&Summary> = summaries.iter().collect();
    run_summaries(&names, &summaries, params)
}

struct Identity {
    children: Vec<Id>,
    mass: usize,
    atom: Option<String>,
    excluded: Option<String>,
    do_macro: Option<String>,
    attribute: bool,
    call: bool,
}

#[derive(Clone, Copy)]
struct Occurrence {
    file: usize,
    line: Option<usize>,
}

#[derive(Default)]
struct Index {
    shapes: HashMap<Shape, Id>,
    identities: Vec<Identity>,
    groups: Vec<Vec<Occurrence>>,
}

impl Index {
    fn intern(&mut self, shape: Shape) -> Id {
        if let Some(id) = self.shapes.get(&shape) {
            return *id;
        }
        let children = shape.children(|id| self.identities[id].atom.is_some());
        let mass = shape.tuple_mass()
            + children
                .iter()
                .map(|id| self.identities[*id].mass)
                .sum::<usize>();
        let excluded = match &shape {
            Shape::Call {
                head,
                args: Some(_),
            } => self.identities[*head].atom.clone(),
            Shape::List(items) if items.len() == 1 => self.identities[items[0]].do_macro.clone(),
            _ => None,
        };
        let do_macro = match &shape {
            Shape::Pair(key, value)
                if self.identities[*key].atom.as_deref() == Some("do")
                    && self.identities[*value].call =>
            {
                self.identities[*value].excluded.clone()
            }
            _ => None,
        };
        let attribute = matches!(&shape, Shape::Call { head, .. } if self.identities[*head].atom.as_deref() == Some("@"));
        let atom = match &shape {
            Shape::Atom(name) => Some(name.clone()),
            _ => None,
        };
        let id = self.identities.len();
        let call = matches!(&shape, Shape::Call { args: Some(_), .. });
        self.identities.push(Identity {
            children,
            mass,
            atom,
            excluded,
            do_macro,
            attribute,
            call,
        });
        self.groups.push(Vec::new());
        self.shapes.insert(shape, id);
        id
    }

    fn collect(&mut self, summary: &Summary, file: usize, threshold: usize) {
        let mut ids = Vec::with_capacity(summary.nodes.len());
        for node in &summary.nodes {
            ids.push(self.intern(node.shape.remap(&ids)));
        }
        let mut stack: Vec<Id> = summary.root.into_iter().collect();
        while let Some(local) = stack.pop() {
            let node = &summary.nodes[local];
            let id = ids[local];
            if self.identities[id].mass >= threshold {
                self.groups[id].push(Occurrence {
                    file,
                    line: node.line,
                });
            }
            let children = node.shape.children(|id| summary.atom_name(id).is_some());
            stack.extend(children.into_iter().rev());
        }
    }

    fn pruned(&self) -> Vec<bool> {
        let mut covered = vec![false; self.identities.len()];
        for id in (0..self.identities.len()).rev() {
            if covered[id] || self.groups[id].len() > 1 {
                for child in &self.identities[id].children {
                    covered[*child] = true;
                }
            }
        }
        covered
            .into_iter()
            .enumerate()
            .map(|(id, covered)| covered && self.identities[id].mass >= 40)
            .collect()
    }
}

/// Rebuild global identities from portable file summaries. Files remain in
/// discovery order. Within each upstream 30-file chunk occurrence order is
/// reverse prewalk; chunks use discovery order rather than task completion.
pub(crate) fn run_summaries(
    names: &[&str],
    summaries: &[&Summary],
    params: &BTreeMap<String, String>,
) -> Vec<ProjectIssue> {
    let mass = crate::helpers::param_usize(params, "mass_threshold", 40);
    let nodes = crate::helpers::param_usize(params, "nodes_threshold", 2);
    let excluded: Vec<String> = params
        .get("excluded_macros")
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim_start_matches(':').to_owned())
        .collect();
    let mut index = Index::default();
    for (file, summary) in summaries.iter().enumerate() {
        index.collect(summary, file, mass);
    }
    let pruned = index.pruned();
    let mut issues = Vec::new();
    for (id, group) in index.groups.iter_mut().enumerate() {
        let identity = &index.identities[id];
        if pruned[id]
            || group.len() < nodes.max(2)
            || identity.attribute
            || identity
                .excluded
                .as_ref()
                .is_some_and(|name| excluded.contains(name))
        {
            continue;
        }
        group.reverse();
        group.sort_by_key(|node| node.file / 30);
        emit_group(names, group, identity.mass, &mut issues);
    }
    issues
}

fn emit_group(names: &[&str], group: &[Occurrence], mass: usize, issues: &mut Vec<ProjectIssue>) {
    let locations: Vec<String> = group
        .iter()
        .map(|other| {
            format!(
                "{}:{}",
                names[other.file],
                other.line.map_or_else(String::new, |line| line.to_string())
            )
        })
        .collect();
    let mut emitted = HashSet::new();
    for (position, occurrence) in group.iter().enumerate() {
        if !emitted.insert(occurrence.file) {
            continue;
        }
        let peers: Vec<&str> = locations
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != position)
            .map(|(_, location)| location.as_str())
            .collect();
        #[allow(
            clippy::cast_precision_loss,
            reason = "occurrence counts fit exactly in f64"
        )]
        let severity = group.len() as f64;
        issues.push(ProjectIssue {
            file: occurrence.file,
            line: occurrence.line,
            column: None,
            trigger: "no_trigger".to_owned(),
            message: format!(
                "Duplicate code found in {} (mass: {}).",
                peers.join(", "),
                mass
            ),
            severity: Some(severity),
        });
    }
}
