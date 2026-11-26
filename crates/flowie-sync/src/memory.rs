//! A backend in memory, with the semantics every real one follows (per-node-sync.md §4, §5.4):
//! conditional saves that answer a stale version with the stored record, and an ordered change
//! feed to pull from. Tests drive engines against it; it stores envelopes without reading them.

use std::collections::HashMap;

use crate::engine::{ChangeBatch, SaveResult};
use crate::wire::Envelope;

#[derive(Debug, Default)]
pub struct MemoryServer {
    /// id -> (record, version, sequence number of its last save).
    records: HashMap<String, (Envelope, u64)>,
    seq: u64,
    /// Image files by name.
    pub blobs: HashMap<String, Vec<u8>>,
}

impl MemoryServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Save records, each only if it starts from the stored version (none for a new one).
    pub fn modify(&mut self, saves: Vec<Envelope>) -> Vec<SaveResult> {
        saves
            .into_iter()
            .map(|mut save| {
                let stored = self.records.get(&save.id);
                let current = stored.map(|(e, _)| e.etag.clone().unwrap_or_default());
                if save.etag != current {
                    return match stored {
                        Some((server, _)) => SaveResult::Conflict { server: server.clone() },
                        // A version for a record the server doesn't have: save it as new.
                        None => self.store(save),
                    };
                }
                save.etag = None;
                self.store(save)
            })
            .collect()
    }

    fn store(&mut self, mut save: Envelope) -> SaveResult {
        self.seq += 1;
        let etag = self.seq.to_string();
        save.etag = Some(etag.clone());
        let id = save.id.clone();
        self.records.insert(id.clone(), (save, self.seq));
        SaveResult::Saved { id, etag }
    }

    /// Records saved after `cursor` (none: everything), oldest first, at most `limit`.
    pub fn changes(&self, cursor: Option<&str>, limit: usize) -> ChangeBatch {
        let since: u64 = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
        let mut newer: Vec<&(Envelope, u64)> = self.records.values().filter(|(_, seq)| *seq > since).collect();
        newer.sort_by_key(|(_, seq)| *seq);
        let more = newer.len() > limit;
        newer.truncate(limit);
        let cursor = newer.last().map_or(since, |(_, seq)| *seq).to_string();
        ChangeBatch { records: newer.into_iter().map(|(e, _)| e.clone()).collect(), cursor, more }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}
