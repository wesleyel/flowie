//! The workspace as a sync store (docs/roadmap/per-node-sync.md §6): what the engine needs to
//! send and merge, kept in memory and saved by the writer in order with the records, so a
//! restart picks up where sync left off.

use std::collections::{BTreeSet, HashMap};

use flowie_core::store::{SyncRows, SyncWrite};
use flowie_core::NodeId;
use flowie_sync::store::{apply_to_tree, local_record};
use flowie_sync::{NodeRecord, RecordStore};

use crate::Workspace;

/// Sync's state for one outline.
#[derive(Debug, Default)]
pub struct SyncState {
    /// Records changed here and not saved to the backend yet.
    outbox: BTreeSet<NodeId>,
    /// The backend's version of each record last seen, with its etag.
    bases: HashMap<NodeId, (String, NodeRecord)>,
    /// Where pulling left off, by backend.
    cursors: HashMap<String, String>,
    /// The backend syncing now (`Workspace::set_sync_remote`).
    remote: String,
}

impl SyncState {
    pub(crate) fn load(rows: SyncRows) -> Self {
        SyncState {
            outbox: rows.outbox.into_iter().collect(),
            bases: rows.bases.into_iter().filter_map(|(id, etag, json)| Some((id, (etag, serde_json::from_str(&json).ok()?)))).collect(),
            cursors: rows.cursors.into_iter().collect(),
            remote: String::new(),
        }
    }

    /// Records the tree changed that sync has to send.
    pub(crate) fn note_unsent(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.outbox.extend(ids);
    }

    pub fn has_unsent(&self) -> bool {
        !self.outbox.is_empty()
    }
}

impl Workspace {
    /// Sync with the backend called `remote` from now on: pulling continues where it last
    /// left off on that backend.
    pub fn set_sync_remote(&mut self, remote: &str) {
        self.sync.remote = remote.to_string();
    }

    fn save_sync(&self, write: SyncWrite) {
        if let Some(writer) = &self.writer {
            writer.sync(write);
        }
    }
}

impl RecordStore for Workspace {
    fn unsent(&self, limit: usize) -> Vec<NodeId> {
        self.sync.outbox.iter().copied().take(limit).collect()
    }

    fn local(&self, id: NodeId) -> Option<NodeRecord> {
        local_record(&self.tree, id)
    }

    fn apply(&mut self, records: Vec<NodeRecord>) {
        apply_to_tree(&mut self.tree, records);
        self.persist();
    }

    fn clear_unsent(&mut self, id: NodeId) {
        if self.sync.outbox.remove(&id) {
            self.save_sync(SyncWrite::Sent(id));
        }
    }

    fn base(&self, id: NodeId) -> Option<(String, NodeRecord)> {
        self.sync.bases.get(&id).cloned()
    }

    fn set_base(&mut self, id: NodeId, etag: String, record: NodeRecord) {
        let json = serde_json::to_string(&record).expect("a record is plain data");
        self.save_sync(SyncWrite::Base(id, etag.clone(), json));
        self.sync.bases.insert(id, (etag, record));
    }

    fn cursor(&self) -> Option<String> {
        self.sync.cursors.get(&self.sync.remote).cloned()
    }

    fn set_cursor(&mut self, cursor: String) {
        self.save_sync(SyncWrite::Cursor { remote: self.sync.remote.clone(), cursor: cursor.clone() });
        self.sync.cursors.insert(self.sync.remote.clone(), cursor);
    }
}

#[cfg(test)]
mod tests {
    use flowie_core::images::ImageDir;
    use flowie_core::ROOT;
    use flowie_sync::{Engine, MemoryServer, RecordStore};

    use crate::Workspace;

    fn open(db: &std::path::Path) -> Workspace {
        let (mut ws, _) = Workspace::open(db, ImageDir::beside(db), |_| {}).unwrap();
        ws.set_sync_remote("memory");
        ws
    }

    /// Push everything, then pull everything.
    fn sync(ws: &mut Workspace, engine: &mut Engine, server: &mut MemoryServer) {
        ws.persist();
        loop {
            let saves = engine.next_saves(ws, 50);
            if saves.is_empty() {
                break;
            }
            let results = server.modify(saves);
            engine.on_saved(ws, results);
        }
        loop {
            let batch = server.changes(ws.cursor().as_deref(), 50);
            let more = batch.more;
            engine.on_fetched(ws, batch);
            if !more {
                break;
            }
        }
    }

    #[test]
    fn two_outlines_sync_and_pick_up_where_they_left_off_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (db_a, db_b) = (dir.path().join("a.sqlite"), dir.path().join("b.sqlite"));
        let mut server = MemoryServer::new();
        let (mut a, mut b) = (open(&db_a), open(&db_b));
        let (mut ea, mut eb) = (Engine::new(), Engine::new());

        let x = a.tree.create_at(ROOT, 0, "from a");
        a.commit_with_imports(None, None, None);
        sync(&mut a, &mut ea, &mut server);
        assert!(!a.sync.has_unsent());
        sync(&mut b, &mut eb, &mut server);
        assert_eq!(b.tree.node(x).text, "from a");
        assert!(!b.history.can_undo(), "what sync brings in is no undo step");

        // An edit made just before quitting is still sent after the restart.
        a.tree.set_text(x, "edited before quitting");
        a.commit_with_imports(None, None, None);
        a.flush().unwrap();
        drop(a);
        let mut a = open(&db_a);
        assert!(a.sync.has_unsent());
        sync(&mut a, &mut Engine::new(), &mut server);
        assert!(!a.sync.has_unsent());

        // b, restarted too, pulls on from where it was: only the new version comes.
        b.flush().unwrap();
        drop(b);
        let mut b = open(&db_b);
        let since = b.cursor();
        assert!(since.is_some());
        assert_eq!(server.changes(since.as_deref(), 50).records.len(), 1);
        sync(&mut b, &mut Engine::new(), &mut server);
        assert_eq!(b.tree.node(x).text, "edited before quitting");
        assert!(b.base(x).is_some(), "the backend's version is remembered");

        // What came in is saved: it is there after another restart.
        b.flush().unwrap();
        drop(b);
        let b = open(&db_b);
        assert_eq!(b.tree.node(x).text, "edited before quitting");
        assert!(!b.sync.has_unsent(), "what came in isn't sent back");
    }
}
