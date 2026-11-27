//! Sync driven by a platform's own sync engine, CloudKit's CKSyncEngine on Apple platforms
//! (wiki: Per-Node-Sync §5.2): that engine decides when to send and fetch and calls in
//! here, and this answers with records as the platform's adapter carries them, an envelope
//! (JSON) and an opaque etag. The iOS app (through flowie-ffi) and the desktop (through
//! flowie-cloudkit) share it.

use flowie_sync::{ChangeBatch, Engine, Envelope, RecordStore, SaveResult};

use crate::Workspace;

/// A record as a native engine carries it: its name, the version it was last seen at (for
/// CloudKit, the record's archived system fields), and the envelope as JSON, without its etag.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Wire {
    pub id: String,
    #[serde(default)]
    pub etag: Option<String>,
    pub envelope: String,
}

impl Wire {
    pub fn from_envelope(mut envelope: Envelope) -> Wire {
        let etag = envelope.etag.take();
        Wire { id: envelope.id.clone(), etag, envelope: serde_json::to_string(&envelope).expect("an envelope is plain data") }
    }

    /// The envelope, with the etag put back; none for one that doesn't parse.
    pub fn to_envelope(&self) -> Option<Envelope> {
        let mut envelope: Envelope = serde_json::from_str(&self.envelope).ok()?;
        envelope.etag = self.etag.clone();
        Some(envelope)
    }
}

/// How the native engine answered one save.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum NativeSaved {
    Saved {
        id: String,
        etag: String,
    },
    /// The backend has a newer version (CloudKit: serverRecordChanged): here it is.
    Conflict {
        server: Wire,
    },
    /// Not saved for another reason; it is sent again later.
    Failed {
        id: String,
        error: String,
    },
}

/// The sync engine for one outline while a native engine drives it.
#[derive(Debug, Default)]
pub struct NativeSync {
    engine: Engine,
}

impl NativeSync {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ids with something to send, at most `limit`: what the native engine should list as
    /// pending saves. The synced preferences are the record `flowie_sync::PREFS_ID`.
    pub fn unsent_ids(ws: &Workspace, limit: usize) -> Vec<String> {
        let mut ids: Vec<String> = ws.unsent(limit).into_iter().map(|id| id.to_string()).collect();
        if ws.prefs_unsent() && ids.len() < limit {
            ids.push(flowie_sync::PREFS_ID.to_string());
        }
        ids
    }

    /// The next records to save, at most `limit`, each from the version last seen. Each stays
    /// in flight until [`on_saved`](Self::on_saved) answers it.
    pub fn next_saves(&mut self, ws: &mut Workspace, limit: usize) -> Vec<Wire> {
        self.engine.next_saves(ws, limit).into_iter().map(Wire::from_envelope).collect()
    }

    /// The native engine answered saves.
    pub fn on_saved(&mut self, ws: &mut Workspace, outcomes: Vec<NativeSaved>) {
        let results = outcomes
            .into_iter()
            .filter_map(|outcome| match outcome {
                NativeSaved::Saved { id, etag } => Some(SaveResult::Saved { id, etag }),
                NativeSaved::Conflict { server } => server.to_envelope().map(|server| SaveResult::Conflict { server }),
                NativeSaved::Failed { id, error } => Some(SaveResult::Failed { id, error }),
            })
            .collect();
        self.engine.on_saved(ws, results);
    }

    /// Records the native engine fetched. It keeps its own place in the backend's changes, so
    /// `cursor` only marks that this backend has been fetched from (see
    /// [`Workspace::use_remote`]). Returns whether the outline changed.
    pub fn on_fetched(&mut self, ws: &mut Workspace, records: &[Wire], cursor: &str) -> bool {
        let revision = ws.tree.revision();
        let records = records.iter().filter_map(Wire::to_envelope).collect();
        self.engine.on_fetched(ws, ChangeBatch { records, cursor: cursor.to_string(), more: false });
        ws.tree.revision() != revision
    }
}
