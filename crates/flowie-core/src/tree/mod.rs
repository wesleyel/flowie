//! In-memory outline tree with a change journal (for undo and persistence).
//!
//! The tree has two layers:
//! - **Records**: every node row as it is stored (`Tree::record`, `Tree::records`). Edits
//!   change records, and only records are journaled and saved.
//! - **Derived indexes**: children lists, reachability, links and mirrors, computed from the
//!   records alone and never written back. Two trees holding the same records derive the
//!   same indexes.
//!
//! What the outline shows is the reachable part: a node is reachable when its parent chain
//! leads to the root through records that exist. `contains`, `get`, `nodes`, `children` and
//! everything built on them see only reachable nodes; `node` reads any record.

mod id;
mod node;
mod ops;

use std::collections::{BTreeSet, HashMap, HashSet};

pub use id::{virtual_id, NodeId, ROOT};
pub use node::{now_millis, set_clock, system_millis, BlockKind, Clock, Node};

/// One recorded change: the node before and after (None = absent).
#[derive(Debug, Clone)]
pub struct Change {
    pub id: NodeId,
    pub before: Option<Node>,
    pub after: Option<Node>,
}

#[derive(Debug, Default)]
pub struct Tree {
    /// The records layer: every node, reachable or not.
    nodes: HashMap<NodeId, Node>,
    /// The derived layer.
    index: Index,
    /// Changes since the last `take_journal` (undo grouping).
    journal: Vec<Change>,
    /// Node ids changed since the last `take_dirty` (persistence).
    dirty: HashSet<NodeId>,
    /// Bumped on every mutation; lets views cache derived data.
    revision: u64,
    /// Virtual row id -> real node id (rows inside mirrors).
    virtuals: HashMap<NodeId, NodeId>,
}

/// What is derived from the records: kept in sync by every mutation so lookups never scan
/// the tree, and rebuilt from scratch (`Tree::reindex`) after bulk changes. Never saved.
#[derive(Debug, Default)]
struct Index {
    /// Parent -> the records under it, in sibling order (rank, then id).
    children: HashMap<NodeId, Vec<NodeId>>,
    /// Records the root cannot reach: their parent chain hits a missing record.
    hidden: HashSet<NodeId>,
    /// Reverse indexes over all records: target -> nodes linking to it (`flowie:ID`),
    /// source -> mirrors of it. Readers keep the reachable ones.
    links: HashMap<NodeId, BTreeSet<NodeId>>,
    mirrors: HashMap<NodeId, BTreeSet<NodeId>>,
    /// Starred nodes in starred-list order: (star key, id).
    stars: BTreeSet<(String, NodeId)>,
}

/// Internal link targets (`flowie:ID`) in a node's text and note.
fn link_targets(node: &Node) -> HashSet<NodeId> {
    let mut out = HashSet::new();
    for text in [&node.text, &node.note] {
        // Cheap prefilter: almost no bullet contains a link.
        if !text.contains("flowie:") {
            continue;
        }
        for run in crate::markup::parse(text) {
            if let crate::markup::Kind::Link { url } = run.kind {
                if let Some(id) = url.strip_prefix("flowie:").and_then(|s| s.parse().ok()) {
                    out.insert(id);
                }
            }
        }
    }
    out
}

impl Tree {
    pub fn new() -> Self {
        let mut tree = Tree::default();
        tree.nodes.insert(ROOT, Node::new(ROOT, ROOT, String::new()));
        tree.index.children.insert(ROOT, Vec::new());
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
        }
        for n in all {
            tree.index.children.entry(n.id).or_default();
            tree.index.children.entry(n.parent).or_default().push(n.id);
            tree.nodes.insert(n.id, n);
        }
        let nodes = &tree.nodes;
        for list in tree.index.children.values_mut() {
            list.sort_by(|a, b| nodes[a].rank.cmp(&nodes[b].rank).then(a.cmp(b)));
        }
        tree.repair_ranks();
        tree.break_cycles();
        let all: Vec<Node> = tree.nodes.values().cloned().collect();
        for n in &all {
            tree.index(n, true);
        }
        tree.derive_reach();
        tree
    }

    /// Add (`add`) or remove a node's entries in the reverse indexes.
    fn index(&mut self, node: &Node, add: bool) {
        if node.id == ROOT {
            return;
        }
        if !node.star.is_empty() {
            let key = (node.star.clone(), node.id);
            if add {
                self.index.stars.insert(key);
            } else {
                self.index.stars.remove(&key);
            }
        }
        let edit = |map: &mut HashMap<NodeId, BTreeSet<NodeId>>, key: NodeId| {
            if add {
                map.entry(key).or_default().insert(node.id);
            } else if let Some(set) = map.get_mut(&key) {
                set.remove(&node.id);
                if set.is_empty() {
                    map.remove(&key);
                }
            }
        };
        for target in link_targets(node) {
            edit(&mut self.index.links, target);
        }
        if node.mirror != ROOT {
            edit(&mut self.index.mirrors, node.mirror);
        }
    }

    /// Re-index after `before` became `after` (skips the parse when the
    /// indexed fields did not change, e.g. a move or a fold).
    fn sync_index(&mut self, before: Option<&Node>, after: Option<&Node>) {
        if let (Some(b), Some(a)) = (before, after) {
            if b.text == a.text && b.note == a.note && b.mirror == a.mirror {
                if b.star != a.star {
                    self.index.stars.remove(&(b.star.clone(), b.id));
                    if !a.star.is_empty() {
                        self.index.stars.insert((a.star.clone(), a.id));
                    }
                }
                return;
            }
        }
        if let Some(b) = before {
            self.index(b, false);
        }
        if let Some(a) = after {
            self.index(a, true);
        }
    }

    /// Reachable nodes whose text or note links to `id`, in outline order.
    pub fn links_to(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.in_outline_order(self.linkers(id).collect()).into_iter()
    }

    /// Number of distinct reachable nodes linking to `id`.
    pub fn link_count(&self, id: NodeId) -> usize {
        self.linkers(id).count()
    }

    fn linkers(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.index.links.get(&id).into_iter().flatten().copied().filter(|n| self.contains(*n))
    }

    /// Reachable mirrors of `source`, in outline order.
    pub fn mirrors_of(&self, source: NodeId) -> Vec<NodeId> {
        let ids = self.index.mirrors.get(&source).into_iter().flatten().copied().filter(|n| self.contains(*n)).collect();
        self.in_outline_order(ids)
    }

    /// Sort `ids` the way they appear reading the outline top to bottom (ids
    /// are random, so their own order means nothing).
    fn in_outline_order(&self, mut ids: Vec<NodeId>) -> Vec<NodeId> {
        if ids.len() > 1 {
            ids.sort_by_cached_key(|&id| {
                let mut path = Vec::new();
                let mut n = id;
                while n != ROOT {
                    let Some(node) = self.nodes.get(&n) else { break };
                    path.push(self.children(node.parent).iter().position(|c| *c == n).unwrap_or(usize::MAX));
                    n = node.parent;
                }
                path.reverse();
                path
            });
        }
        ids
    }

    /// Defensive: give fresh keys to sibling lists holding a malformed rank
    /// (bad data; the editor never writes one), keeping their order.
    fn repair_ranks(&mut self) {
        let broken: Vec<NodeId> = self
            .index
            .children
            .iter()
            .filter(|(_, list)| list.iter().any(|c| !crate::rank::is_valid(&self.nodes[c].rank)))
            .map(|(parent, _)| *parent)
            .collect();
        for parent in broken {
            let list = self.index.children[&parent].clone();
            for (id, rank) in list.iter().zip(crate::rank::n_between(None, None, list.len())) {
                self.nodes.get_mut(id).unwrap().rank = rank;
                self.dirty.insert(*id);
            }
        }
    }

    /// Defensive: any node unreachable from the root (a parent cycle in bad data) moves to the root.
    fn break_cycles(&mut self) {
        let mut seen = HashSet::new();
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            if seen.insert(id) {
                stack.extend(self.index.children.get(&id).into_iter().flatten().copied());
            }
        }
        let lost: Vec<NodeId> = self.nodes.keys().copied().filter(|id| !seen.contains(id)).collect();
        for id in lost {
            let old_parent = self.nodes[&id].parent;
            if let Some(list) = self.index.children.get_mut(&old_parent) {
                list.retain(|c| *c != id);
            }
            let rank = self.rank_at(ROOT, self.children(ROOT).len());
            let node = self.nodes.get_mut(&id).unwrap();
            node.parent = ROOT;
            node.rank = rank;
            self.index.children.get_mut(&ROOT).unwrap().push(id);
            self.dirty.insert(id);
        }
    }

    /// Whether the records put `id` out of the root's reach, given what is already derived
    /// for its parent. The root is always reachable.
    fn derive_hidden(&self, id: NodeId) -> bool {
        let Some(node) = self.nodes.get(&id) else { return true };
        id != ROOT && (!self.nodes.contains_key(&node.parent) || self.index.hidden.contains(&node.parent))
    }

    /// Rebuild reachability from scratch: walk down from the root.
    fn derive_reach(&mut self) {
        let mut seen = HashSet::new();
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            if seen.insert(id) {
                stack.extend(self.index.children.get(&id).into_iter().flatten().copied());
            }
        }
        self.index.hidden = self.nodes.keys().copied().filter(|id| !seen.contains(id)).collect();
    }

    /// Re-derive the reachability of `id` after its record changed, and of the records below
    /// it wherever that changes theirs. `force` walks below `id` even if its own did not
    /// change (a new record can adopt records that were waiting for their parent).
    fn update_reach(&mut self, id: NodeId, force: bool) {
        let mut stack = vec![(id, force)];
        while let Some((id, force)) = stack.pop() {
            let hide = self.derive_hidden(id);
            let changed = if hide { self.index.hidden.insert(id) } else { self.index.hidden.remove(&id) };
            if changed || force {
                stack.extend(self.index.children.get(&id).into_iter().flatten().map(|&c| (c, false)));
            }
        }
    }

    /// Number of reachable nodes (not counting the root).
    pub fn len(&self) -> usize {
        self.nodes.len() - 1 - self.index.hidden.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The real node behind a (possibly virtual) id.
    #[inline]
    pub fn resolve(&self, id: NodeId) -> NodeId {
        if id.is_virtual() {
            self.virtuals.get(&id).copied().unwrap_or(id)
        } else {
            id
        }
    }

    /// Register virtual ids produced by `flatten` (kept across refreshes so
    /// ids held by the caret stay valid).
    pub fn add_virtuals(&mut self, map: impl IntoIterator<Item = (NodeId, NodeId)>) {
        self.virtuals.extend(map);
        if self.virtuals.len() > 200_000 {
            self.virtuals.retain(|_, real| self.nodes.contains_key(real));
        }
    }

    /// A reachable node.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        let id = self.resolve(id);
        self.nodes.get(&id).filter(|_| !self.index.hidden.contains(&id))
    }

    /// The record of `id`, reachable or not. Panics if there is none.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[&self.resolve(id)]
    }

    /// Whether `id` is a node the root can reach.
    pub fn contains(&self, id: NodeId) -> bool {
        let id = self.resolve(id);
        self.nodes.contains_key(&id) && !self.index.hidden.contains(&id)
    }

    /// Every reachable node except the root, in no particular order.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values().filter(|n| n.id != ROOT && !self.index.hidden.contains(&n.id))
    }

    /// The record of `id`, reachable or not.
    pub fn record(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&self.resolve(id))
    }

    /// Whether there is a record for `id`, reachable or not.
    pub fn has_record(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&self.resolve(id))
    }

    /// Every record except the root, reachable or not, in no particular order.
    pub fn records(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values().filter(|n| n.id != ROOT)
    }

    /// Reachable starred nodes, in starred-list order (star key, then id). A star on a
    /// record the root can't reach stays on the record and counts again once it is back.
    pub fn starred(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.index.stars.iter().map(|(_, id)| *id).filter(|id| !self.index.hidden.contains(id))
    }

    pub fn is_starred(&self, id: NodeId) -> bool {
        self.get(id).is_some_and(|n| !n.star.is_empty())
    }

    /// The children of a reachable node, in order (none for one the root can't reach).
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        let id = self.resolve(id);
        if self.index.hidden.contains(&id) {
            return &[];
        }
        self.index.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn has_children(&self, id: NodeId) -> bool {
        !self.children(id).is_empty()
    }

    pub fn parent(&self, id: NodeId) -> NodeId {
        self.nodes.get(&self.resolve(id)).map(|n| n.parent).unwrap_or(ROOT)
    }

    pub fn index_in_parent(&self, id: NodeId) -> usize {
        let id = self.resolve(id);
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

    pub fn is_ancestor(&self, ancestor: NodeId, id: NodeId) -> bool {
        let ancestor = self.resolve(ancestor);
        let mut id = self.resolve(id);
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

    /// A new id no node in this tree has.
    pub fn alloc_id(&mut self) -> NodeId {
        loop {
            let id = NodeId::random();
            if !self.nodes.contains_key(&id) {
                return id;
            }
        }
    }

    /// A new rank that sorts at position `index` among the current children
    /// of `parent`. Only the neighbours' keys are read; no other node changes.
    pub fn rank_at(&self, parent: NodeId, index: usize) -> String {
        self.rank_at_excluding(parent, index, None)
    }

    /// Like `rank_at`, but computed as if `exclude` were not among the children
    /// (`index` counts siblings without it).
    pub fn rank_at_excluding(&self, parent: NodeId, index: usize, exclude: Option<NodeId>) -> String {
        let (before, after) = self.rank_bounds(parent, index, exclude);
        crate::rank::between(before, after)
    }

    /// `n` new ranks, in order, for a run of nodes inserted at `index` among
    /// `parent`'s children (short, evenly spread keys for pastes and imports).
    pub fn ranks_at(&self, parent: NodeId, index: usize, n: usize) -> Vec<String> {
        let (before, after) = self.rank_bounds(parent, index, None);
        crate::rank::n_between(before, after, n)
    }

    /// The keys a new rank at `index` must fall strictly between. Siblings
    /// sharing one key are ordered by id, so no key fits between them: a new
    /// node there goes after the whole tied run instead.
    fn rank_bounds(&self, parent: NodeId, index: usize, exclude: Option<NodeId>) -> (Option<&str>, Option<&str>) {
        let ranks: Vec<&str> = self.children(parent).iter().filter(|c| Some(**c) != exclude).map(|c| self.nodes[c].rank.as_str()).collect();
        let index = index.min(ranks.len());
        let valid = |r: &&str| crate::rank::is_valid(r);
        let before = ranks[..index].iter().rev().copied().find(valid);
        let after = ranks[index..].iter().copied().filter(valid).find(|r| before.is_none_or(|b| b < *r));
        (before, after)
    }

    /// Insert or replace a node, keeping the children index sorted and recording the change.
    pub fn put(&mut self, mut node: Node) {
        assert!(node.id != ROOT, "the root is immutable");
        let before = self.nodes.get(&node.id).cloned();
        if before.as_ref() == Some(&node) {
            return;
        }
        if let Some(b) = &before {
            if b.text != node.text || b.note != node.note || b.completed != node.completed || b.kind != node.kind || b.image != node.image {
                node.modified = now_millis();
            }
            if let Some(list) = self.index.children.get_mut(&b.parent) {
                list.retain(|c| *c != node.id);
            }
        }
        self.insert_sorted(&node);
        self.index.children.entry(node.id).or_default();
        self.sync_index(before.as_ref(), Some(&node));
        let (id, new) = (node.id, before.is_none());
        self.journal.push(Change { id, before, after: Some(node.clone()) });
        self.dirty.insert(id);
        self.nodes.insert(id, node);
        self.update_reach(id, new);
        self.revision += 1;
    }

    fn insert_sorted(&mut self, node: &Node) {
        let nodes = &self.nodes;
        let list = self.index.children.entry(node.parent).or_default();
        let pos = list
            .iter()
            .position(|c| nodes.get(c).is_none_or(|n| (n.rank.as_str(), *c) > (node.rank.as_str(), node.id)))
            .unwrap_or(list.len());
        list.insert(pos, node.id);
    }

    /// Remove a single node (its children must already be gone or moved).
    pub fn remove(&mut self, id: NodeId) {
        let id = self.resolve(id);
        let Some(before) = self.nodes.remove(&id) else { return };
        if let Some(list) = self.index.children.get_mut(&before.parent) {
            list.retain(|c| *c != id);
        }
        self.index.hidden.remove(&id);
        // Records still under it (callers normally remove them first) lose the root's reach.
        let orphans = self.index.children.remove(&id).unwrap_or_default();
        if !orphans.is_empty() {
            for &c in &orphans {
                self.update_reach(c, false);
            }
            self.index.children.insert(id, orphans);
        }
        self.sync_index(Some(&before), None);
        self.journal.push(Change { id, before: Some(before), after: None });
        self.dirty.insert(id);
        self.revision += 1;
    }

    /// Apply a node state without journaling (used by undo/redo, which journal themselves).
    /// Stars are not part of undo: a node that stays keeps the star it has now.
    fn apply_state(&mut self, id: NodeId, state: Option<Node>) {
        let old = self.nodes.remove(&id);
        let state = match (state, &old) {
            (Some(mut node), Some(old)) => {
                node.star.clone_from(&old.star);
                Some(node)
            }
            (state, _) => state,
        };
        self.sync_index(old.as_ref(), state.as_ref());
        if let Some(old) = old {
            if let Some(list) = self.index.children.get_mut(&old.parent) {
                list.retain(|c| *c != id);
            }
        }
        match state {
            Some(node) => {
                self.insert_sorted(&node);
                self.index.children.entry(id).or_default();
                self.nodes.insert(id, node);
            }
            None => {
                self.index.children.remove(&id);
            }
        }
        self.dirty.insert(id);
        self.revision += 1;
    }

    pub fn take_journal(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.journal)
    }

    /// Nodes removed since the last `take_journal` (as they were).
    pub fn journal_removed(&self) -> Vec<Node> {
        self.journal.iter().filter(|c| c.after.is_none()).filter_map(|c| c.before.clone()).collect()
    }

    pub fn journal_is_empty(&self) -> bool {
        self.journal.is_empty()
    }

    /// Drop the journal entries made since `mark`, keeping the changes (they are
    /// still saved, but not undone). For edits outside undo, such as stars.
    pub fn forget_since(&mut self, mark: usize) {
        self.journal.truncate(mark);
    }

    /// A point in the journal to roll back to with `rollback_to`.
    pub fn journal_mark(&self) -> usize {
        self.journal.len()
    }

    /// Undo every change made since `mark` and drop it from the journal, as if
    /// it never happened (an all-or-nothing group of edits that failed midway).
    pub fn rollback_to(&mut self, mark: usize) {
        if mark >= self.journal.len() {
            return;
        }
        let changes = self.journal.split_off(mark);
        self.revert(&changes);
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
        // Nodes must be restored parents-first; applying all states then rebuilding
        // the derived index is simplest: apply in reverse order.
        // Stars are not undone, so the inverse brings back the star each node has now
        // (a starred node that this removes comes back starred).
        let mut stars: HashMap<NodeId, String> = HashMap::new();
        for change in changes.iter().rev() {
            if let Some(node) = self.nodes.get(&change.id) {
                stars.entry(change.id).or_insert_with(|| node.star.clone());
            }
            self.apply_state(change.id, change.before.clone());
        }
        self.reindex();
        let with_star = |id: NodeId, node: &Option<Node>| {
            node.clone().map(|mut node| {
                if let Some(star) = stars.get(&id) {
                    node.star.clone_from(star);
                }
                node
            })
        };
        // Reversed so that reverting the inverse replays the original order.
        changes.iter().rev().map(|c| Change { id: c.id, before: with_star(c.id, &c.after), after: c.before.clone() }).collect()
    }

    /// Rebuild the children index and reachability from the records (after bulk state changes).
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
            list.sort_by(|a, b| nodes[a].rank.cmp(&nodes[b].rank).then(a.cmp(b)));
        }
        self.index.children = children;
        self.derive_reach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The incrementally kept index must equal a fresh derivation from the records.
    fn assert_derived(t: &Tree) {
        let mut fresh = Tree { nodes: t.nodes.clone(), ..Tree::default() };
        fresh.reindex();
        assert_eq!(t.index.hidden, fresh.index.hidden);
        let order =
            |m: &HashMap<NodeId, Vec<NodeId>>| m.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (*k, v.clone())).collect::<Vec<_>>();
        let (mut a, mut b) = (order(&t.index.children), order(&fresh.index.children));
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    const TARGET: &str = "0000000000000000000000000000abcd";

    fn sample() -> (Tree, [NodeId; 4]) {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "a");
        let b = t.create_at(a, 0, "b");
        let c = t.create_at(b, 0, &format!("c [x](flowie:{TARGET})"));
        let d = t.create_at(ROOT, 1, "d");
        (t, [a, b, c, d])
    }

    #[test]
    fn everything_is_reachable_in_a_whole_tree() {
        let (t, ids) = sample();
        for id in ids {
            assert!(t.contains(id) && t.has_record(id) && t.get(id).is_some());
        }
        assert_eq!(t.len(), 4);
        assert_eq!(t.nodes().count(), t.records().count());
        assert_derived(&t);
    }

    #[test]
    fn records_below_a_missing_parent_are_unreachable_and_come_back_with_it() {
        let (mut t, [a, b, c, d]) = sample();
        t.take_dirty();
        let mark = t.journal_mark();
        // Remove `b` but leave `c` behind: `c` is still a record, but nothing reaches it.
        t.remove(b);
        assert!(!t.contains(c) && t.has_record(c) && t.get(c).is_none());
        assert_eq!(t.node(c).parent, b, "the record is untouched");
        assert!(!t.take_dirty().0.iter().any(|n| n.id == c), "nothing is written back");
        assert!(t.contains(a) && t.contains(d));
        assert_eq!(t.len(), 2);
        assert_eq!(t.records().count(), 3);
        assert!(!t.nodes().any(|n| n.id == c));
        assert!(t.descendants(a).is_empty());
        assert!(t.children(c).is_empty());
        assert_derived(&t);

        // Putting the parent record back (here by rolling back) makes the subtree reachable.
        t.rollback_to(mark);
        assert!(t.contains(b) && t.contains(c));
        assert_eq!(t.descendants(a), [b, c]);
        assert_derived(&t);

        // A new record with the missing parent's id adopts the waiting records.
        t.remove(b);
        let mut again = Node::new(b, d, "a0".into());
        again.text = "b again".into();
        t.put(again);
        assert!(t.contains(c));
        assert_eq!(t.descendants(d), [b, c]);
        assert_derived(&t);
    }

    #[test]
    fn links_and_mirrors_from_unreachable_records_are_not_counted() {
        let (mut t, [a, b, c, d]) = sample();
        let target: NodeId = TARGET.parse().unwrap();
        assert_eq!(t.link_count(target), 1);
        let kept = t.create_mirror(d, a, 1).unwrap();
        let lost = t.create_mirror(d, b, 1).unwrap();
        assert_eq!(t.mirrors_of(d), [lost, kept]);
        t.remove(b);
        assert_eq!(t.link_count(target), 0);
        assert_eq!(t.links_to(target).count(), 0);
        assert!(t.has_record(c) && t.has_record(lost));
        assert_eq!(t.mirrors_of(d), [kept]);
        assert_derived(&t);
    }

    #[test]
    fn stars_on_unreachable_records_are_not_listed() {
        let (mut t, [a, b, c, d]) = sample();
        for (id, star) in [(c, "a0"), (d, "a1")] {
            let mut node = t.node(id).clone();
            node.star = star.into();
            t.put(node);
        }
        assert_eq!(t.starred().collect::<Vec<_>>(), [c, d]);
        let mark = t.journal_mark();
        t.remove(b);
        assert_eq!(t.starred().collect::<Vec<_>>(), [d]);
        assert!(!t.is_starred(c) && !t.node(c).star.is_empty(), "the star stays on the record");
        t.rollback_to(mark);
        assert_eq!(t.starred().collect::<Vec<_>>(), [c, d]);
        assert!(t.contains(a));
    }
}
