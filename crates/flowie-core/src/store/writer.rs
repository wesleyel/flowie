//! Background writer for batched outline changes.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

use super::imports::save_import_state;
use super::{write_batch, write_import_batch, write_pref, ImportState, LOCAL_META, PREFS};
use crate::tree::{Node, NodeId};

enum Msg {
    Write { upserts: Vec<Node>, deletes: Vec<NodeId> },
    Meta(String, String),
    Pref(String, String),
    ImportState(ImportState),
    ImportBatch { upserts: Vec<Node>, deletes: Vec<NodeId>, state: ImportState },
    Blob(crate::images::BlobInfo),
    Seen { root: NodeId, at: i64 },
    Retry,
    Flush(Sender<Result<(), String>>),
}

enum PendingOp {
    Batch { upserts: Vec<Node>, deletes: Vec<NodeId>, meta: Vec<(String, String)> },
    Pref(String, String),
    ImportState(ImportState),
    ImportBatch { upserts: Vec<Node>, deletes: Vec<NodeId>, state: ImportState },
    Blob(crate::images::BlobInfo),
    Seen { root: NodeId, at: i64 },
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

    pub fn write(&self, upserts: Vec<Node>, deletes: Vec<NodeId>) {
        if upserts.is_empty() && deletes.is_empty() {
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Write { upserts, deletes });
        }
    }

    /// Save a `LOCAL_META` key (this device only).
    pub fn set_meta(&self, key: &str, value: &str) {
        debug_assert!(LOCAL_META.contains(&key), "meta key {key:?} is not in store::LOCAL_META");
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Meta(key.to_string(), value.to_string()));
        }
    }

    /// Save a `PREFS` key (a preference that syncs).
    pub fn set_pref(&self, key: &str, value: &str) {
        debug_assert!(PREFS.contains(&key), "pref key {key:?} is not in store::PREFS");
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Pref(key.to_string(), value.to_string()));
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
    let mut pending_up: std::collections::HashMap<NodeId, Node> = Default::default();
    let mut pending_del: std::collections::HashSet<NodeId> = Default::default();
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

    let freeze_batch = |ops: &mut VecDeque<PendingOp>,
                        up: &mut std::collections::HashMap<NodeId, Node>,
                        del: &mut std::collections::HashSet<NodeId>,
                        meta: &mut Vec<(String, String)>| {
        if up.is_empty() && del.is_empty() && meta.is_empty() {
            return;
        }
        ops.push_back(PendingOp::Batch {
            upserts: up.drain().map(|(_, n)| n).collect(),
            deletes: del.drain().collect(),
            meta: std::mem::take(meta),
        });
    };

    loop {
        let msg = match deadline {
            Some(d) => match rx.recv_timeout(d.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    // Closing: a few more tries, then give up.
                    for _ in 0..5 {
                        match flush_all(&mut conn, &mut pending_ops, &mut pending_up, &mut pending_del, &mut pending_meta) {
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
                    Msg::Pref(key, value) => PendingOp::Pref(key, value),
                    Msg::ImportState(state) => PendingOp::ImportState(state),
                    Msg::ImportBatch { upserts, deletes, state } => PendingOp::ImportBatch { upserts, deletes, state },
                    Msg::Blob(b) => PendingOp::Blob(b),
                    Msg::Seen { root, at } => PendingOp::Seen { root, at },
                    Msg::Write { .. } | Msg::Meta(..) | Msg::Retry | Msg::Flush(_) => unreachable!(),
                };
                freeze_batch(&mut pending_ops, &mut pending_up, &mut pending_del, &mut pending_meta);
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
            let result = flush_all(&mut conn, &mut pending_ops, &mut pending_up, &mut pending_del, &mut pending_meta)
                .map_err(|error| error.to_string());
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
        PendingOp::Batch { upserts, deletes, meta } => write_batch(conn, upserts, deletes, meta),

        PendingOp::Pref(key, value) => write_pref(conn, key, value),
        PendingOp::ImportState(state) => save_import_state(conn, state),
        PendingOp::ImportBatch { upserts, deletes, state } => write_import_batch(conn, upserts, deletes, state),
        PendingOp::Blob(b) => {
            conn.execute(
                "INSERT OR REPLACE INTO blobs (name, mime, width, height, size) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![b.name, b.mime, b.width, b.height, b.size as i64],
            )?;
            Ok(())
        }
        PendingOp::Seen { root, at } => {
            conn.execute("UPDATE imports SET seen_at = ?2 WHERE root = ?1", params![root, at])?;
            Ok(())
        }
    }
}

/// Write queued operations in order, then any pending coalesced batch.
/// On failure, the failed item and everything after it stay queued for retry.
fn flush_all(
    conn: &mut Connection,
    ops: &mut VecDeque<PendingOp>,
    up: &mut std::collections::HashMap<NodeId, Node>,
    del: &mut std::collections::HashSet<NodeId>,
    meta: &mut Vec<(String, String)>,
) -> rusqlite::Result<()> {
    while let Some(op) = ops.front() {
        run_op(conn, op)?;
        ops.pop_front();
    }
    if up.is_empty() && del.is_empty() && meta.is_empty() {
        return Ok(());
    }
    let upserts: Vec<Node> = up.values().cloned().collect();
    let deletes: Vec<NodeId> = del.iter().copied().collect();
    write_batch(conn, &upserts, &deletes, meta)?;
    up.clear();
    del.clear();
    meta.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        writer.write(vec![a.clone(), b.clone()], Vec::new());
        writer.set_pref("saved_searches", "kept too");
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
        assert_eq!(load_prefs(&conn).unwrap(), [("saved_searches".to_string(), "kept too".to_string())]);
        drop(conn);
        std::fs::remove_dir_all(work).unwrap();
    }
}
