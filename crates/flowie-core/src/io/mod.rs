//! Import and export: indented text / Markdown and OPML (including
//! Workflowy's OPML exports). Everything goes through `Item`, a detached
//! subtree with optional Flowie OPML identity.

mod opml;
mod text;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::markup::{rewrite_links, LinkEdit};
use crate::tree::{BlockKind, Node, NodeId, Tree, ROOT};

pub use opml::{parse_opml, to_opml};
pub use text::{image_path, parse_text, to_markdown, to_plain_text};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    #[serde(skip)]
    pub id: Option<NodeId>,
    #[serde(skip)]
    pub mirror_of: Option<NodeId>,
    pub text: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub kind: BlockKind,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub children: Vec<Item>,
    /// An image: its stored name, or (from Markdown) the path it was written with.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
}

impl Item {
    pub fn new(text: &str) -> Item {
        Item { text: text.to_string(), ..Default::default() }
    }

    pub fn count(items: &[Item]) -> usize {
        items.iter().map(|i| 1 + Item::count(&i.children)).sum()
    }
}

impl Tree {
    /// Detached copies of `ids` and their subtrees. Mirrors are resolved to
    /// their source's content.
    pub fn to_items(&self, ids: &[NodeId]) -> Vec<Item> {
        ids.iter().filter(|id| self.contains(**id)).map(|&id| self.item(id, 0)).collect()
    }

    fn item(&self, id: NodeId, guard: usize) -> Item {
        let n = self.node(id);
        let src = self.source(id);
        let s = self.node(src);
        let children = if guard > 200 { Vec::new() } else { self.children(src).iter().map(|&c| self.item(c, guard + 1)).collect() };
        Item {
            text: s.text.clone(),
            note: s.note.clone(),
            completed: s.completed,
            kind: n.kind,
            collapsed: n.collapsed,
            children,
            image: s.image.clone(),
            ..Default::default()
        }
    }

    pub fn to_opml_items(&self, ids: &[NodeId]) -> Vec<Item> {
        fn build(tree: &Tree, id: NodeId) -> Item {
            let node = tree.node(id);
            let shown = tree.node(tree.source(id));
            Item {
                id: Some(node.id),
                mirror_of: (node.mirror != ROOT).then_some(node.mirror),
                text: shown.text.clone(),
                note: shown.note.clone(),
                completed: shown.completed,
                kind: node.kind,
                collapsed: node.collapsed,
                children: tree.children(id).iter().map(|&child| build(tree, child)).collect(),
                image: String::new(),
            }
        }
        let selected: HashSet<_> = ids.iter().map(|&id| self.resolve(id)).collect();
        let mut emitted = HashSet::new();
        ids.iter()
            .map(|&id| self.resolve(id))
            .filter(|&id| {
                id != ROOT && self.contains(id) && emitted.insert(id) && !self.ancestors(id).iter().any(|parent| selected.contains(parent))
            })
            .map(|id| build(self, id))
            .collect()
    }

    /// Insert items at `index` under `parent`; returns the new top-level ids.
    pub fn insert_items(&mut self, parent: NodeId, index: usize, items: &[Item]) -> Vec<NodeId> {
        let mut roots = Vec::new();
        let parent = self.source(parent);
        // (parent in the tree, rank, item), parents before children.
        let mut stack: Vec<_> =
            items.iter().zip(self.ranks_at(parent, index, items.len())).rev().map(|(item, rank)| (parent, rank, item)).collect();
        let mut imported = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut ambiguous = HashSet::new();
        let mut pending: Vec<_> = items.iter().collect();
        while let Some(item) = pending.pop() {
            if let Some(id) = item.id.filter(|id| *id != ROOT && !id.is_virtual()) {
                if !seen_ids.insert(id) {
                    ambiguous.insert(id);
                }
            }
            pending.extend(&item.children);
        }
        let mut mapped = HashMap::new();
        while let Some((under, rank, item)) = stack.pop() {
            let original = item.id.filter(|id| *id != ROOT && !id.is_virtual() && !ambiguous.contains(id));
            let id = original.filter(|id| !self.contains(*id)).unwrap_or_else(|| self.alloc_id());
            if let Some(original) = original {
                mapped.insert(original, id);
            }
            let mut node = Node::new(id, under, rank);
            node.text = item.text.clone();
            node.note = item.note.clone();
            node.completed = item.completed;
            node.kind = item.kind;
            node.collapsed = item.collapsed && (!item.children.is_empty() || item.mirror_of.is_some());
            if item.kind == BlockKind::Image {
                node.image = item.image.clone();
            }
            self.put(node);
            if under == parent {
                roots.push(id);
            }
            imported.push((id, item.mirror_of));
            // `id` is new, so its children are the only ones.
            stack.extend(
                item.children
                    .iter()
                    .zip(crate::rank::n_between(None, None, item.children.len()))
                    .rev()
                    .map(|(child, rank)| (id, rank, child)),
            );
        }
        let mirrors: HashSet<_> = imported.iter().filter(|(_, source)| source.is_some()).map(|(id, _)| *id).collect();
        let remap = |target: NodeId| mapped.get(&target).copied().unwrap_or(target);
        for (id, mirror_of) in imported {
            let source = mirror_of.filter(|source| !ambiguous.contains(source)).map(remap).filter(|&source| {
                source != ROOT
                    && source != id
                    && !source.is_virtual()
                    && !mirrors.contains(&source)
                    && self.contains(source)
                    && self.node(source).mirror == ROOT
                    && !self.has_children(id)
                    && !self.is_ancestor(source, id)
            });
            if source.is_none() && mapped.is_empty() && ambiguous.is_empty() {
                continue;
            }
            let mut node = self.node(id).clone();
            if let Some(source) = source {
                node.mirror = source;
                node.text.clear();
                node.note.clear();
                node.image.clear();
                node.kind = self.node(source).kind;
            } else {
                let relink = |text: &str| {
                    rewrite_links(text, |url| {
                        let Some(target) = url.strip_prefix("flowie:").and_then(|value| value.parse().ok()) else { return LinkEdit::Keep };
                        if ambiguous.contains(&target) {
                            return LinkEdit::Unlink;
                        }
                        match mapped.get(&target) {
                            Some(&new) if target != new => LinkEdit::Url(format!("flowie:{new}")),
                            _ => LinkEdit::Keep,
                        }
                    })
                };
                if node.kind != BlockKind::Code {
                    node.text = relink(&node.text);
                }
                node.note = relink(&node.note);
            }
            self.put(node);
        }
        roots
    }
}

/// Guess the format from the content.
pub fn parse_any(content: &str) -> Vec<Item> {
    let head = content.trim_start();
    if head.starts_with("<?xml") || head.starts_with("<opml") {
        parse_opml(content)
    } else {
        parse_text(content)
    }
}
