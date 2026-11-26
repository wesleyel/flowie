//! A [`RecordStore`] over a tree in memory: the outbox, bases and cursor kept beside it.
//! What the workspace does over the database, without one, for tests and for clients that
//! keep their own storage.

use std::collections::{BTreeSet, HashMap};

use flowie_core::tree::{NodeId, Remote, Tree};

use crate::engine::RecordStore;
use crate::wire::NodeRecord;

#[derive(Debug)]
pub struct MemoryStore {
    pub tree: Tree,
    outbox: BTreeSet<NodeId>,
    bases: HashMap<NodeId, (String, NodeRecord)>,
    cursor: Option<String>,
    /// Text that lost a concurrent edit: (node, field, value).
    pub lost: Vec<(NodeId, String, String)>,
}

impl MemoryStore {
    pub fn new(tree: Tree) -> Self {
        let mut store = MemoryStore { tree, outbox: BTreeSet::new(), bases: HashMap::new(), cursor: None, lost: Vec::new() };
        store.collect();
        store
    }

    /// Move what the tree changed into the outbox (the writer's job in the app).
    pub fn collect(&mut self) {
        self.outbox.extend(self.tree.take_dirty().unsent.into_iter().map(|(id, _)| id));
    }

    pub fn has_unsent(&self) -> bool {
        !self.outbox.is_empty()
    }
}

/// This tree's version of a record: the node, its tombstone, or nothing.
pub fn local_record(tree: &Tree, id: NodeId) -> Option<NodeRecord> {
    match tree.record(id) {
        Some(node) => Some(NodeRecord::from_node(node)),
        None => tree.removed_at(id).map(|at| NodeRecord::tombstone(id, at)),
    }
}

/// Put merged records into a tree, skipping those it already has as they are.
pub fn apply_to_tree(tree: &mut Tree, records: Vec<NodeRecord>) -> Vec<NodeId> {
    let remote: Vec<Remote> = records
        .into_iter()
        .filter(|r| local_record(tree, r.id).as_ref() != Some(r))
        .filter_map(|r| match r.to_node(tree.record(r.id)) {
            Some(node) => Some(Remote::Put(node)),
            None => r.removed.map(|at| Remote::Remove(r.id, at)),
        })
        .collect();
    tree.apply_remote(remote)
}

impl RecordStore for MemoryStore {
    fn unsent(&self, limit: usize) -> Vec<NodeId> {
        self.outbox.iter().copied().take(limit).collect()
    }

    fn local(&self, id: NodeId) -> Option<NodeRecord> {
        local_record(&self.tree, id)
    }

    fn apply(&mut self, records: Vec<NodeRecord>) {
        apply_to_tree(&mut self.tree, records);
        self.collect();
    }

    fn clear_unsent(&mut self, id: NodeId) {
        self.outbox.remove(&id);
    }

    fn base(&self, id: NodeId) -> Option<(String, NodeRecord)> {
        self.bases.get(&id).cloned()
    }

    fn set_base(&mut self, id: NodeId, etag: String, record: NodeRecord) {
        self.bases.insert(id, (etag, record));
    }

    fn cursor(&self) -> Option<String> {
        self.cursor.clone()
    }

    fn set_cursor(&mut self, cursor: String) {
        self.cursor = Some(cursor);
    }

    fn keep_lost(&mut self, id: NodeId, field: &str, value: String) {
        self.lost.push((id, field.to_string(), value));
    }
}
