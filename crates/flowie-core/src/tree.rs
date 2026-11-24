//! In-memory outline tree with a change journal (for undo and persistence).

use std::collections::{BTreeSet, HashMap, HashSet};

/// A node's id: a random (version 4) UUID, the same in every outline the
/// node reaches (share packages keep it). Shown as 32 lowercase hex digits;
/// `ShortIds` gives the shortest unambiguous prefix for display.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct NodeId(pub u128);

/// The invisible root of every outline (the nil UUID). Its children are the top-level bullets.
pub const ROOT: NodeId = NodeId(0);

/// Bits 76–79 of a UUID hold its version.
const VERSION_SHIFT: u32 = 76;
const VERSION_MASK: u128 = 0xF << VERSION_SHIFT;
/// Bits 62–63 hold the variant (`10` for RFC 9562 UUIDs).
const VARIANT_MASK: u128 = 0b11 << 62;
const VARIANT_RFC: u128 = 0b10 << 62;
/// Version 8 ("custom") marks *virtual* ids: the rows shown inside a mirror.
/// The tree resolves them to the real node they display (see `add_virtuals`).
const VIRTUAL_VERSION: u128 = 8 << VERSION_SHIFT;

impl NodeId {
    /// A new random id.
    pub fn random() -> NodeId {
        NodeId(uuid::Uuid::new_v4().as_u128())
    }

    /// Whether this is a virtual id (a row inside a mirror), not a stored node.
    #[inline]
    pub fn is_virtual(self) -> bool {
        self.0 & VERSION_MASK == VIRTUAL_VERSION
    }

    /// The bytes stored in the database (big-endian, as UUIDs are written).
    pub fn to_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }

    pub fn from_bytes(b: &[u8]) -> Option<NodeId> {
        Some(NodeId(u128::from_be_bytes(b.try_into().ok()?)))
    }

    /// A short number for UI element ids (not unique; fine for hashing).
    pub fn as_usize(self) -> usize {
        (self.0 as u64 ^ (self.0 >> 64) as u64) as usize
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl std::fmt::Debug for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

/// Parses the full id: 32 hex digits, with or without the UUID hyphens.
impl std::str::FromStr for NodeId {
    type Err = ();

    fn from_str(s: &str) -> Result<NodeId, ()> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(());
        }
        u128::from_str_radix(&hex, 16).map(NodeId).map_err(|_| ())
    }
}

impl serde::Serialize for NodeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for NodeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<NodeId, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(|_| serde::de::Error::custom("expected a node id (32 hex digits)"))
    }
}

/// A stable virtual id for `child` shown under the row `parent_key`.
pub fn virtual_id(parent_key: NodeId, child: NodeId) -> NodeId {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (parent_key, child).hash(&mut h);
    let lo = h.finish();
    (child, parent_key).hash(&mut h);
    let hi = h.finish();
    let bits = ((hi as u128) << 64) | lo as u128;
    NodeId((bits & !VERSION_MASK & !VARIANT_MASK) | VIRTUAL_VERSION | VARIANT_RFC)
}

/// How a node is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, serde::Serialize, serde::Deserialize)]
pub enum BlockKind {
    #[default]
    Bullet,
    /// Plain paragraph without a bullet.
    Paragraph,
    H1,
    H2,
    H3,
    /// Checkbox; uses `completed` as its checked state.
    Todo,
    Quote,
    /// Code block: monospace, no markup parsing.
    Code,
    /// Numbered list item (number = position among siblings).
    Numbered,
    /// An image (`Node::image`); the text is its caption.
    Image,
}

impl BlockKind {
    /// List kinds carry over to the row Enter creates next to them.
    pub fn is_list(self) -> bool {
        matches!(self, BlockKind::Todo | BlockKind::Numbered)
    }

    /// The kind a new row next to a row of this kind starts with.
    pub fn list_kind(self) -> BlockKind {
        if self.is_list() {
            self
        } else {
            BlockKind::Bullet
        }
    }

    pub const ALL: [BlockKind; 10] = [
        BlockKind::Bullet,
        BlockKind::Paragraph,
        BlockKind::H1,
        BlockKind::H2,
        BlockKind::H3,
        BlockKind::Todo,
        BlockKind::Quote,
        BlockKind::Code,
        BlockKind::Numbered,
        BlockKind::Image,
    ];

    pub fn to_u8(self) -> u8 {
        BlockKind::ALL.iter().position(|k| *k == self).unwrap_or(0) as u8
    }

    pub fn from_u8(v: u8) -> BlockKind {
        BlockKind::ALL.get(v as usize).copied().unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        match self {
            BlockKind::Bullet => "bullet",
            BlockKind::Paragraph => "paragraph",
            BlockKind::H1 => "h1",
            BlockKind::H2 => "h2",
            BlockKind::H3 => "h3",
            BlockKind::Todo => "todo",
            BlockKind::Quote => "quote",
            BlockKind::Code => "code",
            BlockKind::Numbered => "numbered",
            BlockKind::Image => "image",
        }
    }

    pub fn from_name(s: &str) -> Option<BlockKind> {
        BlockKind::ALL.iter().copied().find(|k| k.name() == s)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    #[serde(default)]
    pub kind: BlockKind,
    #[serde(default)]
    /// If non-zero, this node is a mirror: it shows (and edits) that node's
    /// text, note, completion and children.
    pub mirror: NodeId,
    /// The image file (`<sha256>.<ext>` in the image folder), for `BlockKind::Image`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
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
            kind: BlockKind::Bullet,
            mirror: ROOT,
            image: String::new(),
        }
    }
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
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
    /// Changes since the last `take_journal` (undo grouping).
    journal: Vec<Change>,
    /// Node ids changed since the last `take_dirty` (persistence).
    dirty: HashSet<NodeId>,
    /// Bumped on every mutation; lets views cache derived data.
    revision: u64,
    /// Virtual row id -> real node id (rows inside mirrors).
    virtuals: HashMap<NodeId, NodeId>,
    /// Derived reverse indexes, kept in sync by every mutation so lookups
    /// never scan the tree: target -> nodes linking to it (`flowie:ID`),
    /// source -> mirrors of it.
    links: HashMap<NodeId, BTreeSet<NodeId>>,
    mirrors: HashMap<NodeId, BTreeSet<NodeId>>,
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
        let all: Vec<Node> = tree.nodes.values().cloned().collect();
        for n in &all {
            tree.index(n, true);
        }
        tree
    }

    /// Add (`add`) or remove a node's entries in the reverse indexes.
    fn index(&mut self, node: &Node, add: bool) {
        if node.id == ROOT {
            return;
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
            edit(&mut self.links, target);
        }
        if node.mirror != ROOT {
            edit(&mut self.mirrors, node.mirror);
        }
    }

    /// Re-index after `before` became `after` (skips the parse when the
    /// indexed fields did not change, e.g. a move or a fold).
    fn sync_index(&mut self, before: Option<&Node>, after: Option<&Node>) {
        if let (Some(b), Some(a)) = (before, after) {
            if b.text == a.text && b.note == a.note && b.mirror == a.mirror {
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

    /// Nodes whose text or note links to `id`, sorted by id.
    /// Nodes linking to `id`, in outline order.
    pub fn links_to(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.in_outline_order(self.links.get(&id).into_iter().flatten().copied().collect()).into_iter()
    }

    /// Number of distinct nodes linking to `id`.
    pub fn link_count(&self, id: NodeId) -> usize {
        self.links.get(&id).map_or(0, |s| s.len())
    }

    /// Mirrors of `source`, sorted by id.
    pub fn mirrors_of(&self, source: NodeId) -> Vec<NodeId> {
        self.in_outline_order(self.mirrors.get(&source).into_iter().flatten().copied().collect())
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

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&self.resolve(id))
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[&self.resolve(id)]
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&self.resolve(id))
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values().filter(|n| n.id != ROOT)
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.children.get(&self.resolve(id)).map(Vec::as_slice).unwrap_or(&[])
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
            if b.text != node.text || b.note != node.note || b.completed != node.completed || b.kind != node.kind || b.image != node.image {
                node.modified = now_millis();
            }
            if let Some(list) = self.children.get_mut(&b.parent) {
                list.retain(|c| *c != node.id);
            }
        }
        self.insert_sorted(&node);
        self.children.entry(node.id).or_default();
        self.sync_index(before.as_ref(), Some(&node));
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
        let id = self.resolve(id);
        let Some(before) = self.nodes.remove(&id) else { return };
        if let Some(list) = self.children.get_mut(&before.parent) {
            list.retain(|c| *c != id);
        }
        self.children.remove(&id);
        self.sync_index(Some(&before), None);
        self.journal.push(Change { id, before: Some(before), after: None });
        self.dirty.insert(id);
        self.revision += 1;
    }

    /// Apply a node state without journaling (used by undo/redo, which journal themselves).
    fn apply_state(&mut self, id: NodeId, state: Option<Node>) {
        let old = self.nodes.remove(&id);
        self.sync_index(old.as_ref(), state.as_ref());
        if let Some(old) = old {
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

    /// Nodes removed since the last `take_journal` (as they were).
    pub fn journal_removed(&self) -> Vec<Node> {
        self.journal.iter().filter(|c| c.after.is_none()).filter_map(|c| c.before.clone()).collect()
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
        changes.iter().rev().map(|c| Change { id: c.id, before: c.after.clone(), after: c.before.clone() }).collect()
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
