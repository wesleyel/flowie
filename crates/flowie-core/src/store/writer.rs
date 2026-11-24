//! Background writer for batched outline changes.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

use super::imports::save_import_state;
use super::{write_batch, write_import_batch, ImportState};
use crate::tree::{Node, NodeId};

enum Msg {
    Write { upserts: Vec<Node>, deletes: Vec<NodeId> },
    Trash { root: NodeId, label: String, nodes: Vec<Node> },
    Untrash(i64),
    UntrashRoot(NodeId),
    Meta(String, String),
    ImportState(ImportState),
    ImportBatch { upserts: Vec<Node>, deletes: Vec<NodeId>, state: ImportState },
    Blob(crate::images::BlobInfo),
    Seen { root: NodeId, at: i64 },
    Flush(Sender<()>),
}

/// Handle to the background writer. Dropping it flushes and joins the thread.
pub struct Writer {
    tx: Option<Sender<Msg>>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    pub fn spawn(conn: Connection) -> Writer {
        let (tx, rx) = mpsc::channel();
        let thread =
            std::thread::Builder::new().name("flowie-store".into()).spawn(move || writer_loop(conn, rx)).expect("spawn store thread");
        Writer { tx: Some(tx), thread: Some(thread) }
    }

    pub fn write(&self, upserts: Vec<Node>, deletes: Vec<NodeId>) {
        if upserts.is_empty() && deletes.is_empty() {
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Write { upserts, deletes });
        }
    }

    /// Keep a copy of a deleted subtree (root first) in the trash.
    pub fn trash(&self, root: NodeId, label: &str, nodes: Vec<Node>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Trash { root, label: label.to_string(), nodes });
        }
    }

    /// Remove a trash entry (after restoring it or deleting it for good).
    pub fn untrash(&self, seq: i64) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Untrash(seq));
        }
    }

    /// Remove every trash entry for `root` (deleting it for good).
    pub fn untrash_root(&self, root: NodeId) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::UntrashRoot(root));
        }
    }

    pub fn set_meta(&self, key: &str, value: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Meta(key.to_string(), value.to_string()));
        }
    }

    /// Record an imported root.
    pub fn import_state(&self, state: ImportState) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::ImportState(state));
        }
    }

    pub fn import_batch(&self, upserts: Vec<Node>, deletes: Vec<NodeId>, state: ImportState) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::ImportBatch { upserts, deletes, state });
        }
    }

    /// Note an image file's size and type (written before any bullet refers to it).
    pub fn blob(&self, info: crate::images::BlobInfo) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Blob(info));
        }
    }

    /// The user has looked at an imported root.
    pub fn seen(&self, root: NodeId, at: i64) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Seen { root, at });
        }
    }

    /// Block until everything sent so far is on disk.
    pub fn flush(&self) {
        if let Some(tx) = &self.tx {
            let (ack_tx, ack_rx) = mpsc::channel();
            if tx.send(Msg::Flush(ack_tx)).is_ok() {
                let _ = ack_rx.recv_timeout(Duration::from_secs(5));
            }
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn writer_loop(mut conn: Connection, rx: Receiver<Msg>) {
    let mut pending_up: std::collections::HashMap<NodeId, Node> = Default::default();
    let mut pending_del: std::collections::HashSet<NodeId> = Default::default();
    let mut pending_meta: Vec<(String, String)> = Vec::new();
    let mut waiters: Vec<Sender<()>> = Vec::new();
    let mut deadline: Option<Instant> = None;
    loop {
        let msg = match deadline {
            Some(d) => match rx.recv_timeout(d.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    // Closing: a few more tries, then give up.
                    for _ in 0..5 {
                        if commit(&mut conn, &mut pending_up, &mut pending_del, &mut pending_meta) {
                            break;
                        }
                        std::thread::sleep(RETRY);
                    }
                    return;
                }
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            },
        };
        match msg {
            Some(Msg::Write { upserts, deletes }) => {
                for n in upserts {
                    pending_del.remove(&n.id);
                    pending_up.insert(n.id, n);
                }
                for id in deletes {
                    pending_up.remove(&id);
                    pending_del.insert(id);
                }
                deadline.get_or_insert(Instant::now() + Duration::from_millis(200));
            }
            Some(Msg::Trash { root, label, nodes }) => {
                // Make sure earlier writes land first, then record the trash entry.
                if !commit(&mut conn, &mut pending_up, &mut pending_del, &mut pending_meta) {
                    deadline.get_or_insert(Instant::now() + RETRY);
                }
                let payload = serde_json::to_string(&nodes).unwrap_or_default();
                if let Err(e) = conn.execute(
                    "INSERT INTO trash (root, deleted_at, label, payload) VALUES (?1, ?2, ?3, ?4)",
                    params![root, crate::tree::now_millis(), label, payload],
                ) {
                    eprintln!("flowie: trash write failed: {e}");
                }
            }
            Some(Msg::Untrash(seq)) => {
                let _ = conn.execute("DELETE FROM trash WHERE seq = ?1", params![seq]);
            }
            Some(Msg::UntrashRoot(root)) => {
                let _ = conn.execute("DELETE FROM trash WHERE root = ?1", params![root]);
            }
            Some(Msg::ImportState(state)) => {
                if let Err(error) = save_import_state(&mut conn, &state) {
                    eprintln!("flowie: saving import state failed: {error}");
                }
            }
            Some(Msg::ImportBatch { upserts, deletes, state }) => {
                if !commit(&mut conn, &mut pending_up, &mut pending_del, &mut pending_meta) {
                    deadline.get_or_insert(Instant::now() + RETRY);
                }
                if let Err(error) = write_import_batch(&mut conn, &upserts, &deletes, &state) {
                    eprintln!("flowie: saving package change failed: {error}");
                }
            }
            Some(Msg::Blob(b)) => {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO blobs (name, mime, width, height, size) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![b.name, b.mime, b.width, b.height, b.size as i64],
                );
            }
            Some(Msg::Seen { root, at }) => {
                let _ = conn.execute("UPDATE imports SET seen_at = ?2 WHERE root = ?1", params![root, at]);
            }
            Some(Msg::Meta(k, v)) => {
                pending_meta.retain(|(key, _)| *key != k);
                pending_meta.push((k, v));
                deadline.get_or_insert(Instant::now() + Duration::from_millis(200));
            }
            Some(Msg::Flush(ack)) => {
                waiters.push(ack);
                deadline = Some(Instant::now());
            }
            None => {}
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            // A failed batch stays pending and is tried again shortly.
            let written = commit(&mut conn, &mut pending_up, &mut pending_del, &mut pending_meta);
            deadline = (!written).then(|| Instant::now() + RETRY);
            for w in waiters.drain(..) {
                let _ = w.send(());
            }
        }
    }
}

/// How long to wait before trying a failed batch again.
const RETRY: Duration = Duration::from_secs(1);

/// Write the pending changes in one transaction. Returns whether nothing is
/// left pending: on failure the changes stay queued for another try.
fn commit(
    conn: &mut Connection,
    up: &mut std::collections::HashMap<NodeId, Node>,
    del: &mut std::collections::HashSet<NodeId>,
    meta: &mut Vec<(String, String)>,
) -> bool {
    if up.is_empty() && del.is_empty() && meta.is_empty() {
        return true;
    }
    let upserts: Vec<Node> = up.values().cloned().collect();
    let deletes: Vec<NodeId> = del.iter().copied().collect();
    match write_batch(conn, &upserts, &deletes, meta) {
        Ok(()) => {
            up.clear();
            del.clear();
            meta.clear();
            true
        }
        Err(e) => {
            eprintln!("flowie: database write failed, will retry: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{list_trash, open};
    use crate::ROOT;

    #[test]
    fn untrash_root_removes_every_entry_for_that_bullet_only() {
        let work = std::env::temp_dir().join(format!("flowie-untrash-{}", NodeId::random()));
        let db = work.join("flowie.sqlite");
        let writer = Writer::spawn(open(&db).unwrap());
        let (a, b) = (Node::new(NodeId::random(), ROOT, 1.0), Node::new(NodeId::random(), ROOT, 2.0));
        writer.trash(a.id, "a", vec![a.clone()]);
        writer.trash(a.id, "a again", vec![a.clone()]);
        writer.trash(b.id, "b", vec![b.clone()]);
        writer.untrash_root(a.id);
        writer.flush();
        drop(writer);

        let conn = open(&db).unwrap();
        let roots: Vec<NodeId> = list_trash(&conn, 30).unwrap().into_iter().map(|e| e.root).collect();
        assert_eq!(roots, vec![b.id]);
        drop(conn);
        std::fs::remove_dir_all(work).unwrap();
    }
}
