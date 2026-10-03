//! Flat, portable structural descriptors. Child IDs always precede parents.
//! No Elixir runtime, recursive subtree clones, or subtree serialization.
use serde::{Deserialize, Serialize};

pub(super) type Id = usize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum Shape {
    Atom(String),
    Bytes(Vec<u8>),
    Integer(String),
    Float(u64),
    List(Vec<Id>),
    Pair(Id, Id),
    Call { head: Id, args: Option<Vec<Id>> },
}

impl Shape {
    pub(super) fn remap(&self, ids: &[Id]) -> Self {
        match self {
            Self::List(items) => Self::List(items.iter().map(|i| ids[*i]).collect()),
            Self::Pair(a, b) => Self::Pair(ids[*a], ids[*b]),
            Self::Call { head, args } => Self::Call {
                head: ids[*head],
                args: args
                    .as_ref()
                    .map(|args| args.iter().map(|i| ids[*i]).collect()),
            },
            _ => self.clone(),
        }
    }

    /// Macro.prewalk visits call arguments, but not their enclosing list or
    /// an atom head. A non-atom head (remote/anonymous call) is visited.
    pub(super) fn children(&self, atom: impl Fn(Id) -> bool) -> Vec<Id> {
        match self {
            Self::List(items) => items.clone(),
            Self::Pair(a, b) => vec![*a, *b],
            Self::Call { head, args } => {
                let mut children = Vec::new();
                if !atom(*head) {
                    children.push(*head);
                }
                if let Some(args) = args {
                    children.extend(args);
                }
                children
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn tuple_mass(&self) -> usize {
        usize::from(matches!(self, Self::Pair(..) | Self::Call { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Node {
    pub shape: Shape,
    pub line: Option<usize>,
}

/// Content-bound per-file cache payload. Structural identities are rebuilt
/// by exact interning across these local descriptors, never persisted hashes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Summary {
    pub(super) nodes: Vec<Node>,
    pub(super) root: Option<Id>,
}

impl Summary {
    pub(super) fn atom_name(&self, id: Id) -> Option<&str> {
        match &self.nodes[id].shape {
            Shape::Atom(name) => Some(name),
            _ => None,
        }
    }

    pub(super) fn push(&mut self, shape: Shape, line: Option<usize>) -> Id {
        let id = self.nodes.len();
        self.nodes.push(Node { shape, line });
        id
    }

    pub(super) fn atom(&mut self, name: &str) -> Id {
        self.push(Shape::Atom(name.to_owned()), None)
    }

    pub(super) fn list(&mut self, items: Vec<Id>) -> Id {
        let line = items.iter().find_map(|id| self.nodes[*id].line);
        self.push(Shape::List(items), line)
    }

    pub(super) fn pair(&mut self, a: Id, b: Id) -> Id {
        let line = (self.atom_name(a) == Some("do"))
            .then_some(self.nodes[b].line)
            .flatten();
        self.push(Shape::Pair(a, b), line)
    }

    pub(super) fn call(&mut self, name: &str, args: Vec<Id>, line: usize) -> Id {
        let head = self.atom(name);
        self.call_head(head, Some(args), line)
    }

    pub(super) fn call_head(&mut self, head: Id, args: Option<Vec<Id>>, line: usize) -> Id {
        let location = match self.atom_name(head) {
            Some("__block__") => args
                .as_ref()
                .and_then(|args| args.iter().find_map(|id| self.nodes[*id].line)),
            Some(_) => Some(line),
            None => args
                .as_ref()
                .and_then(|args| args.iter().find_map(|id| self.do_line(*id))),
        };
        self.push(Shape::Call { head, args }, location)
    }

    fn do_line(&self, id: Id) -> Option<usize> {
        match &self.nodes[id].shape {
            Shape::Pair(key, value) if self.atom_name(*key) == Some("do") => {
                self.nodes[*value].line
            }
            Shape::List(items) => items.iter().find_map(|id| self.do_line(*id)),
            _ => None,
        }
    }

    pub(super) fn variable(&mut self, name: &str, line: usize) -> Id {
        let head = self.atom(name);
        self.call_head(head, None, line)
    }

    pub(super) fn block(&mut self, items: Vec<Id>) -> Id {
        if items.len() == 1 {
            items[0]
        } else {
            self.call("__block__", items, 0)
        }
    }

    /// Validate disk-loaded local references before remapping them.
    pub(crate) fn valid(&self) -> bool {
        self.root.is_none_or(|root| root < self.nodes.len())
            && self
                .nodes
                .iter()
                .enumerate()
                .all(|(i, node)| match &node.shape {
                    Shape::List(items) => items.iter().all(|child| *child < i),
                    Shape::Pair(a, b) => *a < i && *b < i,
                    Shape::Call { head, args } => {
                        *head < i
                            && args
                                .as_ref()
                                .is_none_or(|args| args.iter().all(|child| *child < i))
                    }
                    _ => true,
                })
    }
}
