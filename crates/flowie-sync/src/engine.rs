//! The sync engine: a state machine with no IO (docs/roadmap/per-node-sync.md §4).
//!
//! Each platform drives it from its own transport: a desktop thread over HTTP, CloudKit's
//! sync engine callbacks on Apple platforms, `fetch` in the browser, or [`MemoryServer`] in
//! tests. The engine asks the [`RecordStore`] for what to send, merges what comes back, and
//! hands the store the merged records; the store saves them.
//!
//! Backends only store records, so a save replaces the backend's record whole. What the
//! engine sends is therefore never just this device's version: it is that merged with the
//! backend's version this device last saw (its *base*), which the conditional save checks
//! is still current. Merging only ever adds, so a save never drops a write the backend had,
//! even when this device has since removed the record and kept nothing but its tombstone.
//!
//! [`MemoryServer`]: crate::memory::MemoryServer

use std::collections::HashMap;

use flowie_core::tree::NodeId;

use crate::merge::merge;
use crate::wire::{Envelope, NodeRecord};

/// What the engine needs from the local outline: its records, its outbox, and where pulling
/// left off. The workspace implements it over the tree and the database.
pub trait RecordStore {
    /// Records changed here and not saved to the backend yet, at most `limit` of them.
    fn unsent(&self, limit: usize) -> Vec<NodeId>;
    /// This device's version of a record: the node, its tombstone, or nothing.
    fn local(&self, id: NodeId) -> Option<NodeRecord>;
    /// Take records merged with this device's version.
    fn apply(&mut self, records: Vec<NodeRecord>);
    /// Nothing about `id` is left to send.
    fn clear_unsent(&mut self, id: NodeId);
    /// The backend's version of a record this device last saw, and its etag.
    fn base(&self, id: NodeId) -> Option<(String, NodeRecord)>;
    fn set_base(&mut self, id: NodeId, etag: String, record: NodeRecord);
    /// Where the last pull left off.
    fn cursor(&self) -> Option<String>;
    fn set_cursor(&mut self, cursor: String);
}

/// How the backend answered one save.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum SaveResult {
    /// Saved; the record's new version.
    Saved { id: String, etag: String },
    /// The backend has a newer version than the one the save started from: here it is.
    Conflict { server: Envelope },
    /// Not saved, for another reason; it is tried again later.
    Failed { id: String, error: String },
}

/// Records fetched since a cursor.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ChangeBatch {
    pub records: Vec<Envelope>,
    /// Where to continue from.
    pub cursor: String,
    /// More records are waiting after this batch.
    pub more: bool,
}

/// The sync engine for one outline and one backend.
#[derive(Debug, Default)]
pub struct Engine {
    /// Records sent and not answered yet, as they were sent.
    in_flight: HashMap<NodeId, NodeRecord>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next records to save, at most `limit`, each with the version it was last seen at.
    pub fn next_saves(&mut self, store: &dyn RecordStore, limit: usize) -> Vec<Envelope> {
        let mut out = Vec::new();
        for id in store.unsent(limit + self.in_flight.len()) {
            if out.len() == limit {
                break;
            }
            if self.in_flight.contains_key(&id) {
                continue;
            }
            let local = store.local(id);
            let (etag, record) = match (store.base(id), local) {
                (Some((etag, base)), Some(local)) => (Some(etag), merge(&local, &base)),
                (Some((etag, base)), None) => (Some(etag), base),
                (None, Some(local)) => (None, local),
                (None, None) => continue,
            };
            out.push(Envelope::node(&record, etag));
            self.in_flight.insert(id, record);
        }
        out
    }

    /// The backend answered saves.
    pub fn on_saved(&mut self, store: &mut dyn RecordStore, results: Vec<SaveResult>) {
        for result in results {
            match result {
                SaveResult::Saved { id, etag } => {
                    let Ok(id) = id.parse::<NodeId>() else { continue };
                    let Some(sent) = self.in_flight.remove(&id) else { continue };
                    // Done, unless something was written here since it was sent.
                    let covered = store.local(id).is_none_or(|local| merge(&local, &sent) == sent);
                    if covered {
                        store.clear_unsent(id);
                    }
                    store.set_base(id, etag, sent);
                }
                SaveResult::Conflict { server } => {
                    let Ok(id) = server.id.parse::<NodeId>() else { continue };
                    self.in_flight.remove(&id);
                    // Merge the newer version in; what is newer here stays unsent and goes
                    // out again, this time from the backend's version.
                    self.take(store, &server);
                }
                SaveResult::Failed { id, .. } => {
                    if let Ok(id) = id.parse::<NodeId>() {
                        self.in_flight.remove(&id);
                    }
                }
            }
        }
    }

    /// Records fetched from the backend: merge them in. Returns the ids that changed here: a
    /// version this device already has (its own save, coming back in the change feed) is
    /// skipped.
    pub fn on_fetched(&mut self, store: &mut dyn RecordStore, batch: ChangeBatch) -> Vec<NodeId> {
        let ids = batch.records.iter().filter_map(|e| self.take(store, e)).collect();
        store.set_cursor(batch.cursor);
        ids
    }

    /// Merge one record from the backend into the store and note its version.
    fn take(&mut self, store: &mut dyn RecordStore, envelope: &Envelope) -> Option<NodeId> {
        let remote = envelope.node_record()?;
        let id = remote.id;
        if envelope.etag.is_some() && store.base(id).is_some_and(|(etag, _)| envelope.etag.as_ref() == Some(&etag)) {
            return None;
        }
        let merged = match store.local(id) {
            Some(local) => merge(&local, &remote),
            None => remote.clone(),
        };
        store.apply(vec![merged]);
        if let Some(etag) = envelope.etag.clone() {
            store.set_base(id, etag, remote);
        }
        Some(id)
    }
}

/// A backend that the engine pulls from and pushes to on its own schedule: the self-hosted
/// server over HTTP, or [`crate::MemoryServer`]. (CloudKit drives the engine itself instead.)
pub trait Backend {
    /// Save records conditionally (each from the version it was last seen at).
    fn modify(&mut self, saves: Vec<Envelope>) -> Result<Vec<SaveResult>, String>;
    /// Records saved after `cursor`, oldest first.
    fn changes(&mut self, cursor: Option<&str>, limit: usize) -> Result<ChangeBatch, String>;
    /// How long `changes` may wait on the backend while nothing is new (long polling), for
    /// backends that can.
    fn set_wait(&mut self, _wait: std::time::Duration) {}
    /// Whether the backend has the image file `name` (`<sha256>.<ext>`).
    fn has_blob(&mut self, _name: &str) -> Result<bool, String> {
        Err("this backend keeps no image files".into())
    }
    /// Store an image file under its name, which the backend may check against the bytes.
    fn put_blob(&mut self, _name: &str, _bytes: Vec<u8>) -> Result<(), String> {
        Err("this backend keeps no image files".into())
    }
    /// The image file `name`; `None` if the backend doesn't have it.
    fn get_blob(&mut self, _name: &str) -> Result<Option<Vec<u8>>, String> {
        Err("this backend keeps no image files".into())
    }
}

/// What one round of [`sync_once`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub sent: usize,
    pub received: usize,
    /// Every record is saved and nothing new is waiting on the backend.
    pub settled: bool,
}

/// One round with a pull-style backend: send everything waiting here, then fetch everything
/// new. A transport error ends the round; what was in flight is sent again next time.
pub fn sync_once(engine: &mut Engine, store: &mut dyn RecordStore, backend: &mut dyn Backend, batch: usize) -> Result<SyncReport, String> {
    let mut report = SyncReport::default();
    for _ in 0..1000 {
        let saves = engine.next_saves(store, batch);
        if saves.is_empty() {
            break;
        }
        let ids: Vec<String> = saves.iter().map(|e| e.id.clone()).collect();
        report.sent += saves.len();
        match backend.modify(saves) {
            Ok(results) => engine.on_saved(store, results),
            Err(error) => {
                let failed = ids.into_iter().map(|id| SaveResult::Failed { id, error: error.clone() }).collect();
                engine.on_saved(store, failed);
                return Err(error);
            }
        }
    }
    loop {
        let changes = backend.changes(store.cursor().as_deref(), batch)?;
        let more = changes.more;
        report.received += engine.on_fetched(store, changes).len();
        if !more {
            break;
        }
    }
    report.settled = store.unsent(1).is_empty();
    Ok(report)
}

impl Backend for crate::MemoryServer {
    fn modify(&mut self, saves: Vec<Envelope>) -> Result<Vec<SaveResult>, String> {
        Ok(crate::MemoryServer::modify(self, saves))
    }

    fn changes(&mut self, cursor: Option<&str>, limit: usize) -> Result<ChangeBatch, String> {
        Ok(crate::MemoryServer::changes(self, cursor, limit))
    }

    fn has_blob(&mut self, name: &str) -> Result<bool, String> {
        Ok(self.blobs.contains_key(name))
    }

    fn put_blob(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.blobs.insert(name.to_string(), bytes);
        Ok(())
    }

    fn get_blob(&mut self, name: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.blobs.get(name).cloned())
    }
}
