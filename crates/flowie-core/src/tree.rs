//! In-memory outline tree with a change journal (for undo and persistence).

use std::collections::{HashMap, HashSet};

pub type NodeId = u64;

/// The invisible root of every outline. Its children are the top-level bullets.
pub const ROOT: NodeId = 0;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub parent: NodeId,
    pub rank: f64,
    pub text: String,
    pub note: String,
    pub completed: bool,
    pub collapsed: bool,
    pub created: i64,
    pub modified: i64,
}

impl Node {
    pub fn new(id: NodeId, parent: NodeId, rank: f64) -> Self {
        let now = now_millis();
        Node {
            id,
            parent,
            rank,
            text: String::new(),
            note: String::new(),
            completed: false,
            collapsed: false,
            created: now,
            modified: now,
        }
    }
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One recorded change: the node before and after (None = absent).
#[derive(Debug, Clone)]
pub struct Change {
    pub id: NodeId,
    pub before: Option<Node>,
    pub after: Option<Node>,
}

#[derive(Debug, Default)]
pub struct Tree {
    nodes: HashMap<NodeId, Node>,
    children: HashMap<NodeId, Vec<NodeId>>,
    next_id: NodeId,
    /// Changes since the last `take_journal` (undo grouping).
    journal: Vec<Change>,
    /// Node ids changed since the last `take_dirty` (persistence).
    dirty: HashSet<NodeId>,
    /// Bumped on every mutation; lets views cache derived data.
    revision: u64,
}

impl Tree {
    pub fn new() -> Self {
        let mut tree = Tree { next_id: 1, ..Default::default() };
        tree.nodes.insert(ROOT, Node::new(ROOT, ROOT, 0.0));
        tree.children.insert(ROOT, Vec::new());
        tree
    }

    /// Build from loaded nodes (any order). Orphans are re-attached to the root.
    pub fn from_nodes(nodes: impl IntoIterator<Item = Node>) -> Self {
        let mut tree = Tree::new();
        let mut all: Vec<Node> = nodes.into_iter().filter(|n| n.id != ROOT).collect();
        let ids: HashSet<NodeId> = all.iter().map(|n| n.id).collect();
        for n in &mut all {
            if n.parent != ROOT && !ids.contains(&n.parent) {
                n.parent = ROOT;
            }
            tree.next_id = tree.next_id.max(n.id + 1);
        }
        for n in all {
            tree.children.entry(n.id).or_default();
            tree.children.entry(n.parent).or_default().push(n.id);
            tree.nodes.insert(n.id, n);
        }
        let nodes = &tree.nodes;
        for list in tree.children.values_mut() {
            list.sort_by(|a, b| nodes[a].rank.total_cmp(&nodes[b].rank).then(a.cmp(b)));
        }
        tree.break_cycles();
        tree
    }

    /// Defensive: any node unreachable from the root (a parent cycle in bad data) moves to the root.
    fn break_cycles(&mut self) {
        let mut seen = HashSet::new();
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            if seen.insert(id) {
                stack.extend(self.children(id).iter().copied());
            }
        }
        let lost: Vec<NodeId> = self.nodes.keys().copied().filter(|id| !seen.contains(id)).collect();
        for id in lost {
            let old_parent = self.nodes[&id].parent;
            if let Some(list) = self.children.get_mut(&old_parent) {
                list.retain(|c| *c != id);
            }
            let rank = self.rank_at(ROOT, self.children(ROOT).len());
            let node = self.nodes.get_mut(&id).unwrap();
            node.parent = ROOT;
            node.rank = rank;
            self.children.get_mut(&ROOT).unwrap().push(id);
            self.dirty.insert(id);
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[&id]
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values().filter(|n| n.id != ROOT)
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn has_children(&self, id: NodeId) -> bool {
        !self.children(id).is_empty()
    }

    pub fn parent(&self, id: NodeId) -> NodeId {
        self.nodes.get(&id).map(|n| n.parent).unwrap_or(ROOT)
    }

    pub fn index_in_parent(&self, id: NodeId) -> usize {
        let parent = self.parent(id);
        self.children(parent).iter().position(|c| *c == id).unwrap_or(0)
    }

    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        let i = self.index_in_parent(id);
        (i > 0).then(|| self.children(self.parent(id))[i - 1])
    }

    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let siblings = self.children(self.parent(id));
        let i = self.index_in_parent(id);
        siblings.get(i + 1).copied()
    }

    /// Ancestors from the root (exclusive) down to the parent of `id`.
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.parent(id);
        while cur != ROOT && self.contains(cur) {
            out.push(cur);
            cur = self.parent(cur);
        }
        out.reverse();
        out
    }

    pub fn is_ancestor(&self, ancestor: NodeId, mut id: NodeId) -> bool {
        while id != ROOT {
            id = self.parent(id);
            if id == ancestor {
                return true;
            }
        }
        false
    }

    /// Depth-first ids of the subtree below `id` (exclusive).
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.children(id).iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            out.push(n);
            stack.extend(self.children(n).iter().rev().copied());
        }
        out
    }

    pub fn alloc_id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// A rank that sorts at position `index` among the current children of `parent`.
    pub fn rank_at(&mut self, parent: NodeId, index: usize) -> f64 {
        self.rank_at_excluding(parent, index, None)
    }

    /// Like `rank_at`, but computed as if `exclude` were not among the children
    /// (`index` counts siblings without it).
    pub fn rank_at_excluding(&mut self, parent: NodeId, index: usize, exclude: Option<NodeId>) -> f64 {
        let list: Vec<NodeId> = self.children(parent).iter().copied().filter(|c| Some(*c) != exclude).collect();
        let index = index.min(list.len());
        let before = index.checked_sub(1).map(|i| self.nodes[&list[i]].rank);
        let after = list.get(index).map(|id| self.nodes[id].rank);
        match (before, after) {
            (None, None) => 1.0,
            (Some(b), None) => b + 1.0,
            (None, Some(a)) => a - 1.0,
            (Some(b), Some(a)) => {
                let mid = (a + b) / 2.0;
                if mid > b && mid < a && (a - b) > 1e-9 {
                    mid
                } else {
                    self.renumber(parent, exclude);
                    index as f64 + 0.5
                }
            }
        }
    }

    /// Respace sibling ranks to 1, 2, 3… (rare; recorded like any other change).
    fn renumber(&mut self, parent: NodeId, exclude: Option<NodeId>) {
        let list: Vec<NodeId> = self.children(parent).iter().copied().filter(|c| Some(*c) != exclude).collect();
        for (i, id) in list.into_iter().enumerate() {
            let mut node = self.nodes[&id].clone();
            node.rank = i as f64 + 1.0;
            self.put(node);
        }
    }

    /// Insert or replace a node, keeping the children index sorted and recording the change.
    pub fn put(&mut self, mut node: Node) {
        assert!(node.id != ROOT, "the root is immutable");
        let before = self.nodes.get(&node.id).cloned();
        if before.as_ref() == Some(&node) {
            return;
        }
        if let Some(b) = &before {
            if b.text != node.text || b.note != node.note || b.completed != node.completed {
                node.modified = now_millis();
            }
            if let Some(list) = self.children.get_mut(&b.parent) {
                list.retain(|c| *c != node.id);
            }
        }
        self.insert_sorted(&node);
        self.children.entry(node.id).or_default();
        self.next_id = self.next_id.max(node.id + 1);
        self.journal.push(Change { id: node.id, before, after: Some(node.clone()) });
        self.dirty.insert(node.id);
        self.nodes.insert(node.id, node);
        self.revision += 1;
    }

    fn insert_sorted(&mut self, node: &Node) {
        let nodes = &self.nodes;
        let list = self.children.entry(node.parent).or_default();
        let pos = list
            .iter()
            .position(|c| {
                let r = nodes.get(c).map(|n| n.rank).unwrap_or(f64::MAX);
                r > node.rank || (r == node.rank && *c > node.id)
            })
            .unwrap_or(list.len());
        list.insert(pos, node.id);
    }

    /// Remove a single node (its children must already be gone or moved).
    pub fn remove(&mut self, id: NodeId) {
        let Some(before) = self.nodes.remove(&id) else { return };
        if let Some(list) = self.children.get_mut(&before.parent) {
            list.retain(|c| *c != id);
        }
        self.children.remove(&id);
        self.journal.push(Change { id, before: Some(before), after: None });
        self.dirty.insert(id);
        self.revision += 1;
    }

    /// Apply a node state without journaling (used by undo/redo, which journal themselves).
    fn apply_state(&mut self, id: NodeId, state: Option<Node>) {
        if let Some(old) = self.nodes.remove(&id) {
            if let Some(list) = self.children.get_mut(&old.parent) {
                list.retain(|c| *c != id);
            }
        }
        match state {
            Some(node) => {
                self.insert_sorted(&node);
                self.children.entry(id).or_default();
                self.nodes.insert(id, node);
            }
            None => {
                self.children.remove(&id);
            }
        }
        self.dirty.insert(id);
        self.revision += 1;
    }

    pub fn take_journal(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.journal)
    }

    pub fn journal_is_empty(&self) -> bool {
        self.journal.is_empty()
    }

    /// Ids changed since last call, split into (present nodes, deleted ids).
    pub fn take_dirty(&mut self) -> (Vec<Node>, Vec<NodeId>) {
        let mut upserts = Vec::new();
        let mut deletes = Vec::new();
        for id in self.dirty.drain() {
            match self.nodes.get(&id) {
                Some(n) if id != ROOT => upserts.push(n.clone()),
                Some(_) => {}
                None => deletes.push(id),
            }
        }
        (upserts, deletes)
    }

    /// Revert a list of changes (newest first), returning the inverse list.
    pub fn revert(&mut self, changes: &[Change]) -> Vec<Change> {
        // Nodes must be restored parents-first; applying all states then fixing
        // the children index is simplest: apply in reverse order.
        for change in changes.iter().rev() {
            self.apply_state(change.id, change.before.clone());
        }
        self.reindex();
        // Reversed so that reverting the inverse replays the original order.
        changes
            .iter()
            .rev()
            .map(|c| Change { id: c.id, before: c.after.clone(), after: c.before.clone() })
            .collect()
    }

    /// Rebuild the children index from parent pointers (after bulk state changes).
    fn reindex(&mut self) {
        let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for id in self.nodes.keys() {
            children.entry(*id).or_default();
        }
        for node in self.nodes.values() {
            if node.id != ROOT {
                children.entry(node.parent).or_default().push(node.id);
            }
        }
        let nodes = &self.nodes;
        for list in children.values_mut() {
            list.sort_by(|a, b| nodes[a].rank.total_cmp(&nodes[b].rank).then(a.cmp(b)));
        }
        self.children = children;
    }
}
