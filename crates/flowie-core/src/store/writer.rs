//! Background writer for batched outline changes.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

use super::imports::save_import_state;
use super::{write_batch, write_import_batch, write_pref, write_sync, ImportState, SyncWrite, LOCAL_META, PREFS};
use crate::hlc::{Hlc, REMOVED};
use crate::tree::{Dirty, Node, NodeId};

enum Msg {
    Write(Dirty),
    Meta(String, String),
    Pref(String, String, Hlc),
    ImportState(ImportState),
    ImportBatch { dirty: Dirty, state: ImportState },
    Blob(crate::images::BlobInfo),
    Seen { root: NodeId, at: i64 },
    Reclaim,
    Sync(SyncWrite),
    Retry,
    Flush(Sender<Result<(), String>>),
}

enum PendingOp {
    Batch {
        dirty: Dirty,
        meta: Vec<(String, String)>,
    },
    Pref(String, String, Hlc),
    ImportState(ImportState),
    ImportBatch {
        dirty: Dirty,
        state: ImportState,
    },
    Blob(crate::images::BlobInfo),
    Seen {
        root: NodeId,
        at: i64,
    },
    /// `store::reclaim_free_pages`, after what was queued before it.
    Reclaim,
    Sync(SyncWrite),
}

/// Handle to the background writer. Dropping it flushes and joins the thread.
pub struct Writer {
    tx: Option<Sender<Msg>>,
    thread: Option<JoinHandle<()>>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl Writer {
    pub fn spawn(conn: Connection) -> Writer {
        Self::spawn_with_error_hook(conn, |_| {})
    }

    /// Spawn the background writer and notify `on_status` whenever the write
    /// error state changes (`Some(message)` on failure, `None` once recovered).
    pub fn spawn_with_error_hook<F>(conn: Connection, on_status: F) -> Writer
    where
        F: Fn(Option<String>) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let last_error = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&last_error);
        let thread = std::thread::Builder::new()
            .name("flowie-store".into())
            .spawn(move || writer_loop(conn, rx, shared, on_status))
            .expect("spawn store thread");
        Writer { tx: Some(tx), thread: Some(thread), last_error }
    }

    /// The last database write error if changes are still waiting to be saved.
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|e| e.clone())
    }

    /// Save what changed in the tree (`Tree::take_dirty`), with what sync has to send.
    pub fn write(&self, dirty: Dirty) {
        if dirty.is_empty() {
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Write(dirty));
        }
    }

    /// Save a `LOCAL_META` key (this device only).
    pub fn set_meta(&self, key: &str, value: &str) {
        debug_assert!(LOCAL_META.contains(&key), "meta key {key:?} is not in store::LOCAL_META");
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Meta(key.to_string(), value.to_string()));
        }
    }

    /// Save a `PREFS` key (a preference that syncs), written at `at`.
    pub fn set_pref(&self, key: &str, value: &str, at: Hlc) {
        debug_assert!(PREFS.contains(&key), "pref key {key:?} is not in store::PREFS");
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Pref(key.to_string(), value.to_string(), at));
        }
    }

    /// Record an imported root.
    pub fn import_state(&self, state: ImportState) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::ImportState(state));
        }
    }

    pub fn import_batch(&self, dirty: Dirty, state: ImportState) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::ImportBatch { dirty, state });
        }
    }

    /// Save a change to what sync keeps, after the record changes sent before it.
    pub fn sync(&self, write: SyncWrite) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Sync(write));
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

    /// After everything sent so far is saved, shrink the file if deletes freed
    /// enough of it (`store::reclaim_free_pages`).
    pub fn reclaim_free_pages(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Reclaim);
        }
    }

    /// Trigger an immediate retry of any queued writes without blocking.
    pub fn retry_now(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Retry);
        }
    }

    /// Try to save everything sent so far. Failed writes stay queued for retry.
    pub fn flush(&self) -> Result<(), String> {
        let sender = self.tx.as_ref().ok_or("Database writer stopped")?;
        let (ack_tx, ack_rx) = mpsc::channel();
        sender.send(Msg::Flush(ack_tx)).map_err(|_| "Database writer stopped")?;
        ack_rx.recv_timeout(Duration::from_secs(5)).map_err(|error| match error {
            RecvTimeoutError::Timeout => "Timed out waiting for the database writer",
            RecvTimeoutError::Disconnected => "Database writer stopped",
        })?
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

fn writer_loop<F>(mut conn: Connection, rx: Receiver<Msg>, last_error: Arc<Mutex<Option<String>>>, on_status: F)
where
    F: Fn(Option<String>) + Send + 'static,
{
    let mut pending_ops: VecDeque<PendingOp> = VecDeque::new();
    let mut pending = Pending::default();
    let mut pending_meta: Vec<(String, String)> = Vec::new();
    let mut waiters = Vec::new();
    let mut deadline: Option<Instant> = None;
    let mut reported: Option<String> = None;
    let mut retrying = false;

    let report = |reported: &mut Option<String>, next: Option<String>| {
        if *reported != next {
            *reported = next.clone();
            if let Ok(mut slot) = last_error.lock() {
                *slot = next.clone();
            }
            on_status(next);
        }
    };

    let freeze_batch = |ops: &mut VecDeque<PendingOp>, pending: &mut Pending, meta: &mut Vec<(String, String)>| {
        if pending.is_empty() && meta.is_empty() {
            return;
        }
        ops.push_back(PendingOp::Batch { dirty: pending.take(), meta: std::mem::take(meta) });
    };

    loop {
        let msg = match deadline {
            Some(d) => match rx.recv_timeout(d.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    // Closing: a few more tries, then give up.
                    for _ in 0..5 {
                        match flush_all(&mut conn, &mut pending_ops, &mut pending, &mut pending_meta) {
                            Ok(()) => {
                                report(&mut reported, None);
                                break;
                            }
                            Err(e) => {
                                eprintln!("flowie: database write failed on shutdown, retrying: {e}");
                                report(&mut reported, Some(e.to_string()));
                                std::thread::sleep(RETRY);
                            }
                        }
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
            Some(Msg::Write(dirty)) => {
                pending.add(dirty);
                deadline.get_or_insert(Instant::now() + Duration::from_millis(200));
            }
            Some(Msg::Meta(k, v)) => {
                pending_meta.retain(|(key, _)| *key != k);
                pending_meta.push((k, v));
                deadline.get_or_insert(Instant::now() + Duration::from_millis(200));
            }
            Some(Msg::Retry) => {
                deadline = Some(Instant::now());
            }
            Some(Msg::Flush(ack)) => {
                waiters.push(ack);
                deadline = Some(Instant::now());
            }
            Some(op_msg) => {
                let op = match op_msg {
                    Msg::Pref(key, value, at) => PendingOp::Pref(key, value, at),
                    Msg::ImportState(state) => PendingOp::ImportState(state),
                    Msg::ImportBatch { dirty, state } => PendingOp::ImportBatch { dirty, state },
                    Msg::Blob(b) => PendingOp::Blob(b),
                    Msg::Seen { root, at } => PendingOp::Seen { root, at },
                    Msg::Reclaim => PendingOp::Reclaim,
                    Msg::Sync(write) => PendingOp::Sync(write),
                    Msg::Write(_) | Msg::Meta(..) | Msg::Retry | Msg::Flush(_) => unreachable!(),
                };
                freeze_batch(&mut pending_ops, &mut pending, &mut pending_meta);
                pending_ops.push_back(op);
                if !retrying {
                    deadline = Some(Instant::now());
                } else {
                    deadline.get_or_insert(Instant::now() + RETRY);
                }
            }
            None => {}
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            // A failed operation or batch stays queued in order and is tried again shortly.
            let result = flush_all(&mut conn, &mut pending_ops, &mut pending, &mut pending_meta).map_err(|error| error.to_string());
            if let Err(error) = &result {
                eprintln!("flowie: database write failed, will retry: {error}");
            }
            retrying = result.is_err();
            report(&mut reported, result.as_ref().err().cloned());
            deadline = retrying.then(|| Instant::now() + RETRY);
            for waiter in waiters.drain(..) {
                let _ = waiter.send(result.clone());
            }
        }
    }
}

/// How long to wait before trying a failed write again.
const RETRY: Duration = Duration::from_secs(1);

fn run_op(conn: &mut Connection, op: &PendingOp) -> rusqlite::Result<()> {
    match op {
        PendingOp::Batch { dirty, meta } => write_batch(conn, dirty, meta),

        PendingOp::Pref(key, value, at) => write_pref(conn, key, value, *at),
        PendingOp::ImportState(state) => save_import_state(conn, state),
        PendingOp::ImportBatch { dirty, state } => write_import_batch(conn, dirty, state),
        PendingOp::Blob(b) => super::blobs::write_blob(conn, b),
        PendingOp::Sync(write) => write_sync(conn, write),
        PendingOp::Seen { root, at } => {
            conn.execute("UPDATE imports SET seen_at = ?2 WHERE root = ?1", params![root, at])?;
            Ok(())
        }
        PendingOp::Reclaim => super::reclaim_free_pages(conn).map(|_| ()),
    }
}

/// Write queued operations in order, then any pending coalesced batch.
/// On failure, the failed item and everything after it stay queued for retry.
fn flush_all(
    conn: &mut Connection,
    ops: &mut VecDeque<PendingOp>,
    pending: &mut Pending,
    meta: &mut Vec<(String, String)>,
) -> rusqlite::Result<()> {
    while let Some(op) = ops.front() {
        run_op(conn, op)?;
        ops.pop_front();
    }
    if pending.is_empty() && meta.is_empty() {
        return Ok(());
    }
    write_batch(conn, &pending.snapshot(), meta)?;
    pending.take();
    meta.clear();
    Ok(())
}

/// Tree changes waiting to be written, merged: the latest state of each record, removals
/// that weren't undone, and the outbox bits of every change.
#[derive(Default)]
struct Pending {
    up: std::collections::HashMap<NodeId, Node>,
    removed: std::collections::HashMap<NodeId, Hlc>,
    unsent: std::collections::HashMap<NodeId, u16>,
}

impl Pending {
    fn add(&mut self, dirty: Dirty) {
        for n in dirty.upserts {
            self.removed.remove(&n.id);
            self.up.insert(n.id, n);
        }
        for (id, at) in dirty.removed {
            self.up.remove(&id);
            self.removed.insert(id, at);
        }
        for (id, mask) in dirty.unsent {
            let bits = self.unsent.entry(id).or_default();
            *bits = merge_mask(*bits, mask);
        }
    }

    fn is_empty(&self) -> bool {
        self.up.is_empty() && self.removed.is_empty() && self.unsent.is_empty()
    }

    fn snapshot(&self) -> Dirty {
        Dirty {
            upserts: self.up.values().cloned().collect(),
            removed: self.removed.iter().map(|(id, at)| (*id, *at)).collect(),
            unsent: self.unsent.iter().map(|(id, m)| (*id, *m)).collect(),
        }
    }

    fn take(&mut self) -> Dirty {
        let dirty = self.snapshot();
        *self = Pending::default();
        dirty
    }
}

/// Outbox bits after `old`, then `new`: a removal replaces everything before it, and a
/// later write clears it.
pub(super) fn merge_mask(old: u16, new: u16) -> u16 {
    if new & REMOVED != 0 {
        REMOVED
    } else {
        (old & !REMOVED) | new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_bits_add_up_until_a_removal_replaces_them() {
        use crate::hlc::Reg;
        let (text, pos) = (Reg::Text.bit(), Reg::Position.bit());
        assert_eq!(merge_mask(text, pos), text | pos);
        assert_eq!(merge_mask(text | pos, REMOVED), REMOVED);
        assert_eq!(merge_mask(REMOVED, text), text, "written again after the removal");
    }
    use crate::store::{load, load_prefs, open};
    use crate::ROOT;

    #[test]
    fn failed_writes_retry_in_order_and_report_error_status() {
        let work = std::env::temp_dir().join(format!("flowie-retry-{}", NodeId::random()));
        let db = work.join("flowie.sqlite");
        let writer_conn = open(&db).unwrap();
        writer_conn.busy_timeout(Duration::from_millis(30)).unwrap();

        let (status_tx, status_rx) = mpsc::channel();
        let writer = Writer::spawn_with_error_hook(writer_conn, move |status| {
            let _ = status_tx.send(status);
        });

        // Hold an exclusive write lock on a second connection so the writer fails.
        let blocker = open(&db).unwrap();
        blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();

        let mut a = Node::new(NodeId::random(), ROOT, "a0".into());
        a.text = "kept after retry".into();
        let mut b = Node::new(NodeId::random(), ROOT, "a1".into());
        b.deleted_at = Some(7);
        writer.write(Dirty { upserts: vec![a.clone(), b.clone()], ..Dirty::default() });
        writer.set_pref("saved_searches", "kept too", Hlc::default());
        let error = writer.flush().unwrap_err();

        let reported_err = status_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(reported_err.as_deref(), Some(error.as_str()));
        assert_eq!(writer.last_error().as_deref(), Some(error.as_str()));

        // Release the lock: queued writes and the pref must now land without being dropped.
        blocker.execute_batch("COMMIT").unwrap();
        drop(blocker);

        writer.flush().unwrap();
        let recovered = status_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(recovered, None);
        assert_eq!(writer.last_error(), None);
        drop(writer);

        let conn = open(&db).unwrap();
        let (mut nodes, _) = load(&conn).unwrap();
        nodes.sort_by_key(|n| n.rank.clone());
        assert_eq!(nodes, [a, b], "including deleted_at");
        assert_eq!(load_prefs(&conn).unwrap(), [("saved_searches".to_string(), "kept too".to_string(), Hlc::default())]);
        drop(conn);
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn deleting_rows_for_good_shrinks_the_file_once_enough_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("flowie.sqlite");
        let size = || std::fs::metadata(&db).unwrap().len() + std::fs::metadata(db.with_extension("sqlite-wal")).map_or(0, |m| m.len());
        let mut conn = open(&db).unwrap();
        let nodes: Vec<Node> = (0..3000)
            .map(|i| {
                let mut n = Node::new(NodeId::random(), ROOT, crate::rank::between(None, None));
                n.text = format!("{i} {}", "words ".repeat(100));
                n
            })
            .collect();
        write_batch(&mut conn, &Dirty { upserts: nodes.clone(), ..Dirty::default() }, &[]).unwrap();
        // Little free space: nothing to do.
        assert!(!crate::store::reclaim_free_pages(&conn).unwrap());
        let removed = nodes.iter().map(|n| (n.id, crate::hlc::Hlc::default())).collect();
        write_batch(&mut conn, &Dirty { removed, ..Dirty::default() }, &[]).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let full = size();
        assert!(crate::store::reclaim_free_pages(&conn).unwrap());
        assert!(size() * 4 < full, "{} → {}", full, size());
    }
}
