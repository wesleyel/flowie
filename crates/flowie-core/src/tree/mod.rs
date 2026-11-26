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
//! leads to the root through records that exist and are not in the Trash. Deleting a subtree
//! marks only its root (`Node::deleted_at`); the nodes below keep their records unchanged and
//! are unreachable because their ancestor is. `contains`, `get`, `nodes`, `children` and
//! everything built on them see only reachable nodes; `node` reads any record.
//!
//! Records from several devices can disagree in ways one device never produces, and the
//! derived layer settles them the same way everywhere, without writing anything back
//! (docs/roadmap/per-node-sync.md §3.4, checked by `tests/sync_model.rs`):
//! - **Cycles**: concurrent moves can put A under B and B under A. The view undoes the latest
//!   move in the cycle and shows that node at the top level (`Tree::parent` says where a node
//!   shows; its record keeps the parent it was given).
//! - **Revival**: a deleted subtree comes back when something not deleted was created under it
//!   after the deletion (by a device that hadn't seen it), rather than hide that new content.
//! - **Waiting**: a record whose parent isn't there waits for it, hidden, on load as well as
//!   later, and shows under it once it arrives.

mod change;
mod id;
mod node;
mod ops;

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::hlc::{Clocks, Hlc, HlcClock, Reg, ALL_REGS, REMOVED};

pub use change::{Change, Edit, Field, Value};
pub use id::{virtual_id, NodeId, ROOT};
pub use node::{now_millis, set_clock, system_millis, BlockKind, Clock, Node, NodeSync};

/// How long a deleted subtree stays in the Trash before it is deleted for good.
pub const TRASH_DAYS: i64 = 30;

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
    /// Stamps every local write (`Node::sync`); see `Tree::set_actor`.
    clock: HlcClock,
    /// Records removed for good, with when: they are never shown again unless written to
    /// later, and a record whose parent is here shows at the top level (per-node-sync.md §3.4).
    purged: HashMap<NodeId, Hlc>,
    /// For the outbox: which synced fields of each record changed since the last
    /// `take_dirty` (`hlc::Reg::bit`s, or `hlc::REMOVED`).
    unsent: HashMap<NodeId, u16>,
}

/// What changed since the last [`Tree::take_dirty`], for saving.
#[derive(Debug, Default)]
pub struct Dirty {
    /// Records to write as they are now.
    pub upserts: Vec<Node>,
    /// Records removed for good, with the clock of their removal.
    pub removed: Vec<(NodeId, Hlc)>,
    /// What sync has to send: the changed fields of each record (`hlc::Reg::bit`s), or
    /// `hlc::REMOVED`.
    pub unsent: Vec<(NodeId, u16)>,
}

/// A record as another device has it, merged with this one's (`flowie-sync`).
#[derive(Debug, Clone, PartialEq)]
pub enum Remote {
    /// The node exists, as it is now.
    Put(Node),
    /// The node was removed for good then.
    Remove(NodeId, Hlc),
}

impl Dirty {
    pub fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.removed.is_empty() && self.unsent.is_empty()
    }
}

/// What is derived from the records: kept in sync by every mutation so lookups never scan
/// the tree, and rebuilt from scratch (`Tree::reindex`) after bulk changes. Never saved.
#[derive(Debug, Default)]
struct Index {
    /// Parent -> every record under it, in the Trash or not, in sibling order (rank, then id).
    children: HashMap<NodeId, Vec<NodeId>>,
    /// Parent -> the records under it that are not deleted, in the same order: what
    /// `Tree::children` returns.
    live: HashMap<NodeId, Vec<NodeId>>,
    /// Records the root cannot reach: deleted subtree roots, everything below them, and
    /// records whose parent chain hits a missing record.
    hidden: HashSet<NodeId>,
    /// Records the view shows somewhere other than under their parent: the latest move in a
    /// parent cycle, undone to the top level (`Tree::undo_cycles`). Usually empty.
    moved: HashMap<NodeId, NodeId>,
    /// Deleted records whose deletion doesn't stand: something not deleted was created under
    /// them afterwards (`Tree::revivals`). Usually empty.
    revived: HashSet<NodeId>,
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

    /// Build from loaded nodes (any order). Nothing is rewritten: records whose parent is
    /// missing wait for it, and parent cycles are settled in the view (see the module docs).
    pub fn from_nodes(nodes: impl IntoIterator<Item = Node>) -> Self {
        let mut tree = Tree::new();
        for n in nodes {
            if n.id != ROOT {
                tree.nodes.insert(n.id, n);
            }
        }
        let latest = tree.nodes.values().map(|n| n.sync.clocks.max()).max().unwrap_or_default();
        tree.clock.observe(latest);
        tree.reindex();
        if tree.repair_ranks() {
            tree.reindex();
        }
        let all: Vec<Node> = tree.nodes.values().cloned().collect();
        for n in &all {
            tree.index(n, true);
        }
        tree
    }

    /// Who writes through this tree: its process's actor id (per-node-sync.md §3.1), stamped
    /// in the clock of every field it writes. 0 (the default) is for trees that never sync.
    pub fn set_actor(&mut self, actor: u64) {
        self.clock.set_actor(actor);
    }

    /// Stamp a local write that takes a record from `before` (None: new) to `node`: the
    /// fields it changes get a new clock, and a move remembers the parent it left.
    fn stamp(&mut self, before: Option<&Node>, node: &mut Node) {
        let regs = match before {
            None => Reg::ALL.to_vec(),
            Some(b) => change::changed_regs(b, node),
        };
        if regs.is_empty() {
            return;
        }
        let at = self.clock.tick(now_millis());
        let unsent = self.unsent.entry(node.id).or_default();
        match before {
            None => {
                node.sync.clocks = Clocks::new(at);
                node.sync.moved_from = node.parent;
                *unsent = ALL_REGS;
                self.purged.remove(&node.id);
            }
            Some(b) => {
                for reg in regs {
                    node.sync.clocks.set(reg, at);
                    *unsent = (*unsent & !REMOVED) | reg.bit();
                }
                if b.parent != node.parent {
                    node.sync.moved_from = b.parent;
                }
            }
        }
    }

    /// Where the view shows `node`: under its parent, unless that closes a cycle.
    fn view_parent(&self, node: &Node) -> NodeId {
        if let Some(&parent) = self.index.moved.get(&node.id) {
            return parent;
        }
        if !self.nodes.contains_key(&node.parent) && self.purged.contains_key(&node.parent) {
            // The parent was removed for good: rather than hide this record with it, show it
            // at the top level.
            return ROOT;
        }
        node.parent
    }

    /// The records removed for good that this tree knows of (loaded from the database), with
    /// when they were removed. Records under them show at the top level.
    pub fn set_purged(&mut self, purged: impl IntoIterator<Item = (NodeId, Hlc)>) {
        self.purged = purged.into_iter().collect();
        if let Some(latest) = self.purged.values().max() {
            self.clock.observe(*latest);
        }
        self.reindex();
    }

    /// Every record this tree knows was removed for good, with when.
    pub fn purged(&self) -> impl Iterator<Item = (NodeId, Hlc)> + '_ {
        self.purged.iter().map(|(id, at)| (*id, *at))
    }

    /// When `id` was removed for good, if this tree knows it was.
    pub fn removed_at(&self, id: NodeId) -> Option<Hlc> {
        self.purged.get(&id).copied()
    }

    /// Take records from another device, already merged with this tree's (`flowie-sync`).
    /// Unlike a local edit, this is no undo step and nothing to send: the records are saved
    /// as they are, and this tree's clock only catches up with theirs. The view is derived
    /// again once for the whole batch. Returns the ids whose record changed.
    pub fn apply_remote(&mut self, records: impl IntoIterator<Item = Remote>) -> Vec<NodeId> {
        let mut changed = Vec::new();
        // A tombstone for a record this tree never had still changes what shows: records
        // waiting for it go to the top level.
        let mut new_tombstone = false;
        for record in records {
            match record {
                Remote::Put(node) => {
                    if node.id == ROOT {
                        continue;
                    }
                    self.clock.observe(node.sync.clocks.max());
                    let id = node.id;
                    let old = self.nodes.get(&id).cloned();
                    if old.as_ref() == Some(&node) {
                        continue;
                    }
                    self.purged.remove(&id);
                    self.sync_index(old.as_ref(), Some(&node));
                    self.nodes.insert(id, node);
                    self.dirty.insert(id);
                    changed.push(id);
                }
                Remote::Remove(id, at) => {
                    self.clock.observe(at);
                    let at = self.purged.get(&id).map_or(at, |known| (*known).max(at));
                    new_tombstone |= self.purged.insert(id, at).is_none();
                    if let Some(old) = self.nodes.remove(&id) {
                        self.sync_index(Some(&old), None);
                        self.dirty.insert(id);
                        changed.push(id);
                    }
                }
            }
        }
        if !changed.is_empty() || new_tombstone {
            self.reindex();
            self.revision += 1;
        }
        changed
    }

    /// Remove `id`'s record for good, remembering when (for sync, and for the records under
    /// it). Indexes are the caller's to update.
    fn tombstone(&mut self, id: NodeId) -> Option<Node> {
        let node = self.nodes.remove(&id)?;
        let at = self.clock.tick(now_millis());
        self.purged.insert(id, at);
        self.unsent.insert(id, REMOVED);
        self.dirty.insert(id);
        Some(node)
    }

    /// Whether `node` is the root of a deletion that stands (deleted, and not revived).
    fn is_deleted(&self, node: &Node) -> bool {
        node.deleted_at.is_some() && !self.index.revived.contains(&node.id)
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
                    let parent = self.view_parent(node);
                    path.push(self.children(parent).iter().position(|c| *c == n).unwrap_or(usize::MAX));
                    n = parent;
                }
                path.reverse();
                path
            });
        }
        ids
    }

    /// Defensive: give fresh keys to sibling lists holding a malformed rank
    /// (bad data; the editor never writes one), keeping their order. True if any changed.
    fn repair_ranks(&mut self) -> bool {
        let broken: Vec<NodeId> = self
            .index
            .children
            .iter()
            .filter(|(_, list)| list.iter().any(|c| !crate::rank::is_valid(&self.nodes[c].rank)))
            .map(|(parent, _)| *parent)
            .collect();
        let repaired = !broken.is_empty();
        for parent in broken {
            let list = self.index.children[&parent].clone();
            for (id, rank) in list.iter().zip(crate::rank::n_between(None, None, list.len())) {
                self.nodes.get_mut(id).unwrap().rank = rank;
                self.dirty.insert(*id);
            }
        }
        repaired
    }

    /// The cycles in the records' parents, each settled by undoing its latest move: that
    /// node shows under the parent the move took it from (`NodeSync::moved_from`), or at the
    /// top level when that parent is gone or the node closes a cycle there too. Settles one
    /// cycle at a time, finding them in id order, so every device holding the same records
    /// settles them the same way. Returns node -> where it shows instead.
    fn undo_cycles(&self) -> HashMap<NodeId, NodeId> {
        let mut moved: HashMap<NodeId, NodeId> = HashMap::new();
        let mut undone: HashMap<NodeId, u32> = HashMap::new();
        let mut ids: Vec<NodeId> = self.nodes.keys().copied().filter(|id| *id != ROOT).collect();
        ids.sort();
        while let Some(cycle) = self.find_cycle(&ids, &moved) {
            let latest = cycle.into_iter().max_by_key(|c| Self::move_order(&self.nodes[c])).unwrap();
            let times = undone.entry(latest).or_default();
            *times += 1;
            let from = self.nodes[&latest].sync.moved_from;
            let back = *times == 1 && from != latest && (from == ROOT || self.nodes.contains_key(&from));
            moved.insert(latest, if back { from } else { ROOT });
        }
        moved
    }

    /// The first parent cycle found walking up from `ids` in order, with `moved` overriding
    /// records' parents. A missing parent ends a walk: that record waits for it.
    fn find_cycle(&self, ids: &[NodeId], moved: &HashMap<NodeId, NodeId>) -> Option<Vec<NodeId>> {
        let mut done: HashSet<NodeId> = HashSet::new();
        for &start in ids {
            let mut path = Vec::new();
            let mut on_path: HashMap<NodeId, usize> = HashMap::new();
            let mut n = start;
            while n != ROOT && !done.contains(&n) {
                let Some(node) = self.nodes.get(&n) else { break };
                if let Some(&i) = on_path.get(&n) {
                    return Some(path.split_off(i));
                }
                on_path.insert(n, path.len());
                path.push(n);
                n = moved.get(&n).copied().unwrap_or(node.parent);
            }
            done.extend(path);
        }
        None
    }

    /// Orders moves to find the latest one in a cycle: by the clock of the position, then by
    /// id, since positions derived from old data can share a clock.
    fn move_order(node: &Node) -> (Hlc, NodeId) {
        (node.sync.clocks.get(Reg::Position), node.id)
    }

    /// Deleted records to show anyway: something below them in `children`, not deleted itself
    /// and not behind a deletion that stands, was created after they were deleted. A device
    /// that hadn't seen the deletion added it, and hiding it would hide new content. Moves and
    /// edits under a deleted record don't count: that content is in the Trash and comes back
    /// with it. Nor does content a nested deletion keeps in the Trash: nothing new would show.
    ///
    /// Computed bottom-up, once per record: each subtree reports the latest creation in it that
    /// no standing deletion hides. A deleted record passes on what is below it if that revives
    /// it (its own creation doesn't count), and nothing otherwise.
    fn revivals(&self, children: &HashMap<NodeId, Vec<NodeId>>) -> HashSet<NodeId> {
        let mut out = HashSet::new();
        // The latest creation each finished subtree reports, its root included.
        let mut newest: HashMap<NodeId, Option<Hlc>> = HashMap::new();
        // Every record hangs below the root or below a record that is missing (the view has
        // no cycles), so walking down from those reaches them all.
        let tops = children.keys().copied().filter(|id| *id == ROOT || !self.nodes.contains_key(id));
        let mut stack: Vec<(NodeId, bool)> = tops.map(|id| (id, false)).collect();
        while let Some((id, done)) = stack.pop() {
            let kids = children.get(&id).into_iter().flatten();
            if !done {
                stack.push((id, true));
                stack.extend(kids.map(|&c| (c, false)));
                continue;
            }
            let below = kids.filter_map(|c| newest.get(c).copied().flatten()).max();
            let Some(node) = self.nodes.get(&id).filter(|_| id != ROOT) else { continue };
            // Clocks, not wall times: a device whose clock was set back can't make a deletion
            // look older than what it deleted.
            let deleted = node.deleted_at.map(|_| node.sync.clocks.get(Reg::Deleted));
            let up = match deleted {
                Some(at) if below.is_some_and(|b| b > at) => {
                    out.insert(id);
                    below
                }
                Some(_) => None,
                None => below.max(Some(node.sync.clocks.created)),
            };
            newest.insert(id, up);
        }
        out
    }

    /// Whether giving `node` its parent would put it below itself.
    fn closes_cycle(&self, node: &Node) -> bool {
        let mut n = node.parent;
        for _ in 0..=self.nodes.len() {
            if n == node.id {
                return true;
            }
            match self.nodes.get(&n) {
                Some(p) if n != ROOT => n = self.view_parent(p),
                _ => return false,
            }
        }
        true
    }

    /// Whether the records put `id` out of the root's reach, given what is already derived
    /// for its parent. The root is always reachable.
    fn derive_hidden(&self, id: NodeId) -> bool {
        let Some(node) = self.nodes.get(&id) else { return true };
        let parent = self.view_parent(node);
        id != ROOT && (self.is_deleted(node) || !self.nodes.contains_key(&parent) || self.index.hidden.contains(&parent))
    }

    /// Rebuild the live children lists and reachability from scratch: walk down from the root.
    fn derive_reach(&mut self) {
        let parents: Vec<NodeId> = self.index.children.keys().copied().collect();
        self.index.live.clear();
        for parent in parents {
            self.relist(parent);
        }
        let mut seen = HashSet::new();
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            if seen.insert(id) {
                stack.extend(self.index.live.get(&id).into_iter().flatten().copied());
            }
        }
        self.index.hidden = self.nodes.keys().copied().filter(|id| !seen.contains(id)).collect();
    }

    /// Re-derive `parent`'s live children from its records.
    fn relist(&mut self, parent: NodeId) {
        let live: Vec<NodeId> = self
            .index
            .children
            .get(&parent)
            .into_iter()
            .flatten()
            .copied()
            .filter(|c| self.nodes.get(c).is_some_and(|n| !self.is_deleted(n)))
            .collect();
        if live.is_empty() {
            self.index.live.remove(&parent);
        } else {
            self.index.live.insert(parent, live);
        }
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
        self.index.live.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Move `id` and its subtree to the Trash. Only `id`'s record changes (`deleted_at`);
    /// the nodes below keep theirs and are hidden because it is. Recorded like any edit.
    pub fn trash(&mut self, id: NodeId) {
        let id = self.resolve(id);
        if id == ROOT || !self.contains(id) {
            return;
        }
        let mut node = self.node(id).clone();
        node.deleted_at = Some(now_millis());
        self.put(node);
    }

    /// Bring a subtree back from the Trash: clear its root's `deleted_at`. If the parent it
    /// shows under is not in the outline any more (in the Trash itself, or gone), it goes to
    /// the end of the top level. Recorded like any edit. False if `id` is not in the Trash
    /// (a revived deletion isn't).
    pub fn untrash(&mut self, id: NodeId) -> bool {
        let id = self.resolve(id);
        let Some(node) = self.nodes.get(&id).filter(|n| self.is_deleted(n)) else { return false };
        let mut node = node.clone();
        node.deleted_at = None;
        if !self.contains(self.view_parent(&node)) {
            node.rank = self.rank_at(ROOT, self.children(ROOT).len());
            node.parent = ROOT;
        }
        self.put(node);
        true
    }

    /// The Trash: roots of deleted subtrees (deletions that stand), most recently deleted
    /// first. One with another standing deletion above it, up to the root or to a missing
    /// record it waits for, isn't listed: it is part of that entry, and becomes an entry of its
    /// own when the outer one is restored.
    pub fn trashed(&self) -> Vec<NodeId> {
        let outermost = |n: &Node| {
            let mut parent = self.view_parent(n);
            while parent != ROOT && self.index.hidden.contains(&parent) {
                let Some(p) = self.nodes.get(&parent) else { break };
                if self.is_deleted(p) {
                    return false;
                }
                parent = self.view_parent(p);
            }
            true
        };
        let mut roots: Vec<(i64, NodeId)> =
            self.nodes.values().filter(|n| self.is_deleted(n) && outermost(n)).filter_map(|n| Some((n.deleted_at?, n.id))).collect();
        roots.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        roots.into_iter().map(|(_, id)| id).collect()
    }

    /// The records of the deleted subtree at `id`: `id` first, then everything below it,
    /// parents before children, but not other deleted subtrees inside it (they stay in the
    /// Trash when it is restored). What restoring it brings back. Empty if `id` is not in
    /// the Trash.
    pub fn trashed_subtree(&self, id: NodeId) -> Vec<NodeId> {
        let id = self.resolve(id);
        if self.nodes.get(&id).is_none_or(|n| !self.is_deleted(n)) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            out.push(n);
            let kids = self.index.children.get(&n).into_iter().flatten().rev();
            stack.extend(kids.filter(|c| self.nodes.get(c).is_some_and(|c| !self.is_deleted(c))));
        }
        out
    }

    /// Delete the subtree at `id` (which must be in the Trash) for good, with every record
    /// below it: they are removed and saved as deletes, but not recorded for undo. Returns
    /// the removed ids, for `History::forget`.
    pub fn purge(&mut self, id: NodeId) -> Vec<NodeId> {
        let id = self.resolve(id);
        if self.nodes.get(&id).is_none_or(|n| !self.is_deleted(n)) {
            return Vec::new();
        }
        let mut ids = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            ids.push(n);
            stack.extend(self.index.children.get(&n).into_iter().flatten().copied());
        }
        for &n in ids.iter().rev() {
            self.remove_record(n);
        }
        let gone: HashSet<NodeId> = ids.iter().copied().collect();
        self.journal.retain(|c| !gone.contains(&c.id));
        self.reindex();
        ids
    }

    /// Delete for good every Trash entry older than `TRASH_DAYS` at `now`. Returns the
    /// removed ids.
    pub fn purge_expired(&mut self, now: i64) -> Vec<NodeId> {
        let cutoff = now - TRASH_DAYS * 86_400_000;
        let expired: Vec<NodeId> =
            self.nodes.values().filter(|n| self.is_deleted(n) && n.deleted_at.is_some_and(|at| at < cutoff)).map(|n| n.id).collect();
        expired.into_iter().flat_map(|id| self.purge(id)).collect()
    }

    pub fn has_children(&self, id: NodeId) -> bool {
        !self.children(id).is_empty()
    }

    /// Where `id` shows: its record's parent, unless the view moved it to break a cycle.
    pub fn parent(&self, id: NodeId) -> NodeId {
        self.nodes.get(&self.resolve(id)).map(|n| self.view_parent(n)).unwrap_or(ROOT)
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
        }
        let full = self.needs_reindex(before.as_ref(), &node);
        self.stamp(before.as_ref(), &mut node);
        if full {
            self.sync_index(before.as_ref(), Some(&node));
            let id = node.id;
            self.journal_put(before.as_ref(), &node);
            self.dirty.insert(id);
            self.nodes.insert(id, node);
            self.reindex();
            self.revision += 1;
            return;
        }
        if let Some(b) = &before {
            let parent = self.view_parent(b);
            if let Some(list) = self.index.children.get_mut(&parent) {
                list.retain(|c| *c != node.id);
            }
        }
        self.insert_sorted(&node);
        self.index.children.entry(node.id).or_default();
        self.sync_index(before.as_ref(), Some(&node));
        // Where the view shows it, before and after (the same override applies to both: a
        // change that could settle a cycle differently took the reindex above).
        let (id, new, parent) = (node.id, before.is_none(), self.view_parent(&node));
        let old_parent = before.as_ref().map(|b| self.view_parent(b)).filter(|p| *p != parent);
        self.journal_put(before.as_ref(), &node);
        self.dirty.insert(id);
        self.nodes.insert(id, node);
        self.relist(parent);
        if let Some(p) = old_parent {
            self.relist(p);
        }
        if new {
            // It may adopt records that were waiting for it.
            self.relist(id);
        }
        self.update_reach(id, new);
        self.revision += 1;
    }

    /// Whether changing a record from `before` to `after` can touch what is settled across
    /// the whole tree (cycles, revivals), which the incremental update in `put` doesn't
    /// handle. A rare path on one device: moves and edits in the outline stay incremental.
    /// `created` is written once by the editor, but a record put as a whole (an import, a
    /// sync) can change it, and with it what revives.
    fn needs_reindex(&self, before: Option<&Node>, after: &Node) -> bool {
        if before.is_some_and(|b| b.created != after.created) {
            return true;
        }
        // Any write to a position in a cycle can change which move is the latest.
        if !self.index.moved.is_empty() && before.is_none_or(|b| b.parent != after.parent || b.rank != after.rank) {
            return true;
        }
        let structural = before.is_none_or(|b| b.parent != after.parent || b.deleted_at != after.deleted_at);
        structural
            && (!self.index.moved.is_empty()
                || !self.index.revived.is_empty()
                || after.deleted_at.is_some()
                || before.is_some_and(|b| b.deleted_at.is_some())
                || self.index.hidden.contains(&after.id)
                || self.index.hidden.contains(&after.parent)
                || !self.nodes.contains_key(&after.parent)
                || self.purged.contains_key(&after.id)
                || self.closes_cycle(after))
    }

    /// Journal the patches that took a record from `before` (None: it is new) to `after`.
    fn journal_put(&mut self, before: Option<&Node>, after: &Node) {
        match before {
            None => self.journal.push(Change { id: after.id, edit: Edit::Create(after.clone()) }),
            Some(before) => self.journal.extend(Change::diff(before, after)),
        }
    }

    fn insert_sorted(&mut self, node: &Node) {
        let parent = self.view_parent(node);
        let nodes = &self.nodes;
        let list = self.index.children.entry(parent).or_default();
        let pos = list
            .iter()
            .position(|c| nodes.get(c).is_none_or(|n| (n.rank.as_str(), *c) > (node.rank.as_str(), node.id)))
            .unwrap_or(list.len());
        list.insert(pos, node.id);
    }

    /// Remove a single node (its children must already be gone or moved).
    pub fn remove(&mut self, id: NodeId) {
        let id = self.resolve(id);
        let Some(before) = self.remove_record(id) else { return };
        let had_children = self.index.children.get(&id).is_some_and(|c| !c.is_empty());
        if had_children || before.deleted_at.is_some() || !self.index.moved.is_empty() || !self.index.revived.is_empty() {
            self.reindex();
        }
        self.journal.push(Change { id, edit: Edit::Remove(before) });
    }

    /// Remove a record and save that, without journaling it. Updates the indexes
    /// incrementally; callers reindex when it may settle cycles or revivals differently.
    fn remove_record(&mut self, id: NodeId) -> Option<Node> {
        let parent = self.nodes.get(&id).map(|n| self.view_parent(n))?;
        let before = self.tombstone(id)?;
        if let Some(list) = self.index.children.get_mut(&parent) {
            list.retain(|c| *c != id);
        }
        self.relist(parent);
        self.index.hidden.remove(&id);
        // Records still under it (callers normally remove them first) lose the root's reach.
        let orphans = self.index.children.remove(&id).unwrap_or_default();
        if !orphans.is_empty() {
            for &c in &orphans {
                self.update_reach(c, false);
            }
            self.index.children.insert(id, orphans);
        }
        self.relist(id);
        self.sync_index(Some(&before), None);
        self.revision += 1;
        Some(before)
    }

    pub fn take_journal(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.journal)
    }

    /// Nodes removed or moved to the Trash since the last `take_journal` (as they were). A
    /// revived deletion deleted again counts too: it was showing.
    pub fn journal_removed(&self) -> Vec<Node> {
        self.journal
            .iter()
            .filter_map(|c| match &c.edit {
                Edit::Remove(node) => Some(node.clone()),
                Edit::Set { before: before @ Value::Deleted(_), after: after @ Value::Deleted(Some(_)) } if before != after => {
                    self.nodes.get(&c.id).map(|n| Node { deleted_at: None, ..n.clone() })
                }
                _ => None,
            })
            .collect()
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
    /// What changed since the last call: records to write, records removed for good, and
    /// what sync has to send.
    pub fn take_dirty(&mut self) -> Dirty {
        let mut dirty = Dirty::default();
        for id in self.dirty.drain() {
            match self.nodes.get(&id) {
                Some(n) if id != ROOT => dirty.upserts.push(n.clone()),
                Some(_) => {}
                None => dirty.removed.extend(self.purged.get(&id).map(|at| (id, *at))),
            }
        }
        dirty.unsent = self.unsent.drain().collect();
        dirty
    }

    /// Undo `changes` (in journal order; the newest is undone first) and return what redoes
    /// them, in the same form.
    ///
    /// A field goes back only while it still holds the value the change set: one changed
    /// since (by another device, say) keeps that newer value, and the other fields of the
    /// node are left alone. Undoing a creation removes the record; undoing a removal brings
    /// it back as it was, unless a record with its id exists again. Changes skipped this way
    /// are left out of the result, so redoing doesn't touch them either. Stars aren't part of
    /// undo: a record removed here takes its current star along, and one brought back has
    /// the star it had.
    pub fn revert(&mut self, changes: &[Change]) -> Vec<Change> {
        let mut inverse = Vec::new();
        for change in changes.iter().rev() {
            let id = change.id;
            let old = self.nodes.get(&id).cloned();
            let undo = match &change.edit {
                Edit::Create(_) => self.tombstone(id).map(Edit::Remove),
                Edit::Remove(node) if old.is_none() => {
                    // Bringing a record back is a new write of all its fields.
                    let mut node = node.clone();
                    let at = self.clock.tick(now_millis());
                    for reg in Reg::ALL {
                        node.sync.clocks.set(reg, at);
                    }
                    self.purged.remove(&id);
                    self.unsent.insert(id, ALL_REGS);
                    self.nodes.insert(id, node.clone());
                    Some(Edit::Create(node))
                }
                Edit::Remove(_) => None,
                Edit::Set { before, after } => match &old {
                    Some(current) if Value::of(current, after.field()) == *after => {
                        let mut node = current.clone();
                        before.apply(&mut node);
                        self.stamp(Some(current), &mut node);
                        self.nodes.insert(id, node);
                        Some(Edit::Set { before: after.clone(), after: before.clone() })
                    }
                    _ => None,
                },
            };
            let Some(undo) = undo else { continue };
            let new = self.nodes.get(&id).cloned();
            self.sync_index(old.as_ref(), new.as_ref());
            self.dirty.insert(id);
            inverse.push(Change { id, edit: undo });
        }
        if !inverse.is_empty() {
            self.reindex();
            self.revision += 1;
        }
        inverse
    }

    /// Rebuild the derived layer from the records: settle cycles, the children index,
    /// revivals and reachability (on load, after bulk state changes, and on the rare edits
    /// `needs_reindex` sends here).
    fn reindex(&mut self) {
        self.index.moved = self.undo_cycles();
        let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for id in self.nodes.keys() {
            children.entry(*id).or_default();
        }
        for node in self.nodes.values() {
            if node.id != ROOT {
                children.entry(self.view_parent(node)).or_default().push(node.id);
            }
        }
        let nodes = &self.nodes;
        for list in children.values_mut() {
            list.sort_by(|a, b| nodes[a].rank.cmp(&nodes[b].rank).then(a.cmp(b)));
        }
        self.index.revived = self.revivals(&children);
        self.index.children = children;
        self.derive_reach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The incrementally kept index must equal a fresh derivation from the records.
    fn assert_derived(t: &Tree) {
        let mut fresh = Tree { nodes: t.nodes.clone(), purged: t.purged.clone(), ..Tree::default() };
        fresh.reindex();
        assert_eq!(t.index.hidden, fresh.index.hidden);
        assert_eq!(t.index.moved, fresh.index.moved);
        assert_eq!(t.index.revived, fresh.index.revived);
        let order =
            |m: &HashMap<NodeId, Vec<NodeId>>| m.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (*k, v.clone())).collect::<Vec<_>>();
        for (mine, theirs) in [(&t.index.children, &fresh.index.children), (&t.index.live, &fresh.index.live)] {
            let (mut a, mut b) = (order(mine), order(theirs));
            a.sort();
            b.sort();
            assert_eq!(a, b);
        }
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
    fn records_below_a_removed_parent_show_at_the_top_until_it_comes_back() {
        let (mut t, [a, b, c, d]) = sample();
        t.take_dirty();
        let mark = t.journal_mark();
        // Remove `b` for good but leave `c` behind: rather than hide `c` with it, the view
        // shows it at the top level. Its record still names `b`.
        t.remove(b);
        assert!(t.contains(c));
        assert_eq!((t.parent(c), t.node(c).parent), (ROOT, b));
        let dirty = t.take_dirty();
        assert!(!dirty.upserts.iter().any(|n| n.id == c), "nothing is written back");
        assert!(matches!(dirty.removed.as_slice(), [(id, _)] if *id == b), "the removal is saved, with its clock");
        assert_eq!(dirty.unsent, [(b, crate::hlc::REMOVED)]);
        // Where `c` sorts among them depends on the rank it had under `b`.
        let mut top = t.children(ROOT).to_vec();
        top.sort();
        let mut expected = vec![a, c, d];
        expected.sort();
        assert_eq!(top, expected);
        assert!(t.descendants(a).is_empty());
        assert_derived(&t);

        // Putting the parent record back (here by rolling back) puts `c` back under it.
        t.rollback_to(mark);
        assert_eq!(t.descendants(a), [b, c]);
        assert!(t.take_dirty().removed.is_empty());
        assert_derived(&t);

        // So does a new record with the removed parent's id.
        t.remove(b);
        let mut again = Node::new(b, d, "a0".into());
        again.text = "b again".into();
        t.put(again);
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
        t.trash(b);
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
        t.trash(b);
        assert_eq!(t.starred().collect::<Vec<_>>(), [d]);
        assert!(!t.is_starred(c) && !t.node(c).star.is_empty(), "the star stays on the record");
        t.rollback_to(mark);
        assert_eq!(t.starred().collect::<Vec<_>>(), [c, d]);
        assert!(t.contains(a));
    }

    #[test]
    fn a_parent_cycle_in_the_records_is_settled_in_the_view_alone() {
        let (x, y, z) = (NodeId::random(), NodeId::random(), NodeId::random());
        // `x` under `y` and `y` under `x`, with `z` below `x`.
        let nodes = vec![Node::new(x, y, "a0".into()), Node::new(y, x, "a0".into()), Node::new(z, x, "a0".into())];
        let mut t = Tree::from_nodes(nodes.clone());
        assert!(t.take_dirty().upserts.is_empty(), "nothing is written back");
        let (latest, other) = (x.max(y), x.min(y));
        assert_eq!(t.parent(latest), ROOT, "the latest move is undone in the view");
        assert_eq!(t.node(latest).parent, other, "its record keeps its parent");
        assert_eq!(t.parent(other), latest);
        assert!(t.contains(x) && t.contains(y) && t.contains(z));
        assert_derived(&t);
        // Loading the records in another order settles it the same way.
        let t2 = Tree::from_nodes(nodes.into_iter().rev());
        assert_eq!(t2.children(ROOT), t.children(ROOT));
        assert_eq!(t2.descendants(latest), t.descendants(latest));
    }

    #[test]
    fn a_record_that_closes_a_cycle_is_settled_until_a_later_move_resolves_it() {
        let (mut t, [a, b, _c, d]) = sample();
        // As a sync could deliver it: `a` moved under `b`, which is under `a`.
        let mut moved = t.node(a).clone();
        moved.parent = b;
        t.put(moved);
        assert!(t.contains(a) && t.contains(b));
        assert_eq!((t.parent(a), t.parent(b)), (ROOT, a), "the latest move, a's, is undone: a goes back where it came from");
        assert!(!t.is_ancestor(a, a) && !t.is_ancestor(b, b));
        assert_derived(&t);
        // Moving `b` elsewhere ends the cycle, and with it the view's override.
        let mut back = t.node(b).clone();
        back.parent = d;
        t.put(back);
        assert!(t.index.moved.is_empty());
        assert_eq!((t.parent(a), t.parent(b)), (b, d));
        assert_derived(&t);
    }

    #[test]
    fn records_waiting_for_their_parent_on_load_are_not_rewritten() {
        let (parent, child, other) = (NodeId::random(), NodeId::random(), NodeId::random());
        let mut t = Tree::from_nodes([Node::new(child, parent, "a0".into()), Node::new(other, ROOT, "a0".into())]);
        assert!(!t.contains(child) && t.has_record(child));
        assert_eq!(t.node(child).parent, parent);
        assert!(t.take_dirty().upserts.is_empty(), "nothing is written back");
        t.put(Node::new(parent, ROOT, "a1".into()));
        assert_eq!(t.children(parent), [child]);
        assert_derived(&t);
    }

    #[test]
    fn something_created_under_a_deleted_subtree_after_the_deletion_brings_it_back() {
        let (mut t, [a, b, c, _d]) = sample();
        t.trash(a);
        assert!(!t.contains(c));
        let deleted_at = t.node(a).deleted_at.unwrap();
        // A device that hadn't seen the deletion added a bullet under `b`.
        let mut new = Node::new(NodeId::random(), b, "a5".into());
        new.created = deleted_at + 1;
        let id = new.id;
        t.put(new);
        let new = id;
        assert!(t.contains(a) && t.contains(b) && t.contains(c) && t.contains(new));
        assert!(t.trashed().is_empty());
        assert_eq!(t.node(a).deleted_at, Some(deleted_at), "the record stays deleted");
        assert!(t.purge_expired(deleted_at + TRASH_DAYS * 86_400_000 + 1).is_empty(), "nothing revived is purged");
        assert_derived(&t);
        // Once the new bullet is deleted too, the deletion stands again.
        t.trash(new);
        assert!(!t.contains(a) && !t.contains(new));
        assert_eq!(t.trashed(), [a]);
        assert_derived(&t);
    }

    #[test]
    fn something_created_before_the_deletion_stays_in_the_trash_with_it() {
        let (a, b, old) = (NodeId::random(), NodeId::random(), NodeId::random());
        let t = Tree::from_nodes([record(a, ROOT, 10, Some(100)), record(b, a, 10, None), record(old, b, 50, None)]);
        assert!(!t.contains(a) && !t.contains(old));
        assert_eq!(t.trashed(), [a]);
        assert_derived(&t);
    }

    #[test]
    fn removing_half_of_a_cycle_leaves_the_other_half_in_view() {
        let (x, y, z) = (NodeId::random(), NodeId::random(), NodeId::random());
        let mut t = Tree::from_nodes([Node::new(x, y, "a0".into()), Node::new(y, x, "a0".into()), Node::new(z, ROOT, "a1".into())]);
        let (top, below) = (x.max(y), x.min(y));
        assert_eq!(t.parent(below), top);
        // Removing the half the view shows at the top level: the other half's parent is gone
        // for good, so it shows at the top level rather than wait for it.
        t.remove(top);
        assert!(t.contains(below));
        assert_eq!(t.parent(below), ROOT);
        assert_derived(&t);
    }

    /// A record as another device could have written it: created at `created` and deleted at
    /// `deleted_at` (both clocks and times), for `Tree::from_nodes`, since a local `put`
    /// stamps the clocks of now.
    fn record(id: NodeId, parent: NodeId, created: i64, deleted_at: Option<i64>) -> Node {
        let mut node = Node { created, deleted_at, ..Node::new(id, parent, "a0".into()) };
        node.sync.clocks = Clocks::derived(created, created);
        if let Some(at) = deleted_at {
            node.sync.clocks.set(Reg::Deleted, Hlc::at_millis(at));
        }
        node
    }

    #[test]
    fn reordering_a_node_the_view_moved_out_of_a_cycle_reorders_the_top_level() {
        let (x, y, d) = (NodeId(3), NodeId(1), NodeId(2));
        // `x` under `y` and `y` under `x`: `x` is the latest move and shows at the top, before `d`.
        let mut t = Tree::from_nodes([Node::new(x, y, "a0".into()), Node::new(y, x, "a0".into()), Node::new(d, ROOT, "a1".into())]);
        assert_eq!(t.children(ROOT), [x, d]);
        let mut later = t.node(x).clone();
        later.rank = "a2".into();
        t.put(later);
        assert_eq!(t.children(ROOT), [d, x]);
        assert_derived(&t);
    }

    #[test]
    fn an_undone_move_goes_back_under_the_parent_it_left() {
        let (mut t, [a, b, c, _d]) = sample();
        // `b` (under `a`) moved under its own child `c`, as another device could have.
        let mut moved = t.node(b).clone();
        moved.parent = c;
        t.put(moved);
        assert_eq!(t.node(b).sync.moved_from, a);
        assert_eq!((t.parent(b), t.parent(c)), (a, b), "b shows back under a, not at the top level");
        assert_eq!(t.node(b).parent, c, "its record keeps the move");
        assert_derived(&t);
    }

    #[test]
    fn content_hidden_by_a_nested_deletion_does_not_bring_back_the_outer_one() {
        // `a` deleted at 100. A device that hadn't seen that added `c` under `b` at 200, then
        // deleted `b` at 300: nothing new would show, so `a` stays deleted.
        let (a, b, c) = (NodeId::random(), NodeId::random(), NodeId::random());
        let t = Tree::from_nodes([record(a, ROOT, 50, Some(100)), record(b, a, 50, Some(300)), record(c, b, 200, None)]);
        assert!(t.index.revived.is_empty());
        assert!(!t.contains(a) && !t.contains(b) && !t.contains(c));
        assert_eq!(t.trashed(), [a]);
        assert_derived(&t);
        // Created after both deletions, `c` brings both back.
        let t = Tree::from_nodes([record(a, ROOT, 50, Some(100)), record(b, a, 50, Some(300)), record(c, b, 400, None)]);
        assert!(t.contains(a) && t.contains(b) && t.contains(c));
        assert!(t.trashed().is_empty());
        assert_derived(&t);
        // A revived deletion passes on what revives it, not its own creation: `c` (200) brings
        // `b` (deleted at 100) back but not `a` (deleted at 300), though the record of `b` says
        // it was created at 350.
        let t = Tree::from_nodes([record(a, ROOT, 50, Some(300)), record(b, a, 350, Some(100)), record(c, b, 200, None)]);
        assert_eq!(t.index.revived, [b].into());
        assert!(!t.contains(a) && !t.contains(b) && !t.contains(c));
        assert_eq!(t.trashed(), [a]);
        assert_derived(&t);
    }

    #[test]
    fn untrash_leaves_revived_and_cycled_records_alone() {
        // A revived deletion isn't in the Trash: there is nothing to restore.
        let (mut t, [a, b, _c, _d]) = sample();
        t.trash(a);
        let deleted_at = t.node(a).deleted_at;
        t.put(record(NodeId::random(), b, deleted_at.unwrap() + 1, None));
        assert!(t.contains(a));
        assert!(!t.untrash(a));
        assert_eq!(t.node(a).deleted_at, deleted_at);

        // A deleted record the view moved out of a cycle is restored where it shows, and its
        // record keeps its parent.
        let (x, y) = (NodeId(2), NodeId(1));
        let mut t = Tree::from_nodes([record(x, y, 1, None), record(y, x, 1, None)]);
        t.trash(x);
        assert_eq!(t.trashed(), [x]);
        assert!(t.untrash(x));
        assert_eq!(t.node(x).parent, y, "the record keeps its parent");
        assert_eq!(t.parent(x), ROOT);
        assert!(t.contains(x) && t.contains(y));
        assert_derived(&t);
    }

    #[test]
    fn revival_follows_the_clocks_not_the_recorded_times() {
        let (a, c) = (NodeId::random(), NodeId::random());
        let mut t = Tree::from_nodes([record(a, ROOT, 10, Some(100)), record(c, a, 200, None)]);
        assert!(t.contains(a) && t.contains(c), "c's creation clock is after a's deletion");
        // The creation time a record shows is just data: changing it changes nothing here.
        let earlier = Node { created: 50, ..t.node(c).clone() };
        t.put(earlier);
        assert!(t.contains(a));
        assert_derived(&t);
    }

    #[test]
    fn deleting_a_revived_record_again_counts_as_a_removal() {
        // `a` was deleted before `b` was created below it (by a device that hadn't seen that).
        let (a, b) = (NodeId::random(), NodeId::random());
        let mut t = Tree::from_nodes([record(a, ROOT, 10, Some(100)), record(b, a, 200, None)]);
        assert!(t.contains(a));
        t.trash(a);
        assert!(!t.contains(a));
        assert_eq!(t.journal_removed().iter().map(|n| n.id).collect::<Vec<_>>(), [a]);
    }

    #[test]
    fn deletions_below_a_missing_ancestor_are_trash_entries() {
        let (gone, p, d, inner, e) = (NodeId::random(), NodeId::random(), NodeId::random(), NodeId::random(), NodeId::random());
        // `d` waits below `p`, which waits for `gone`; `e` waits for `gone` directly. `inner` is
        // part of `d`'s entry.
        let t = Tree::from_nodes([
            record(p, gone, 1, None),
            record(d, p, 1, Some(20)),
            record(inner, d, 1, Some(30)),
            record(e, gone, 1, Some(10)),
        ]);
        assert_eq!(t.trashed(), [d, e]);
    }
}
