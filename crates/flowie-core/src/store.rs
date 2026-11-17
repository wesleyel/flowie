//! SQLite persistence. The whole outline is loaded once at launch; after that
//! the UI never touches the database: changes go to a background writer
//! thread that batches them into one transaction per ~200 ms.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

use crate::tree::{Node, NodeId};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS nodes (
    id        INTEGER PRIMARY KEY,
    parent    INTEGER NOT NULL,
    rank      REAL    NOT NULL,
    text      TEXT    NOT NULL DEFAULT '',
    note      TEXT    NOT NULL DEFAULT '',
    completed INTEGER NOT NULL DEFAULT 0,
    collapsed INTEGER NOT NULL DEFAULT 0,
    created   INTEGER NOT NULL,
    modified  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn load(conn: &Connection) -> rusqlite::Result<(Vec<Node>, Vec<(String, String)>)> {
    let mut stmt = conn.prepare(
        "SELECT id, parent, rank, text, note, completed, collapsed, created, modified FROM nodes",
    )?;
    let nodes = stmt
        .query_map([], |r| {
            Ok(Node {
                id: r.get::<_, i64>(0)? as NodeId,
                parent: r.get::<_, i64>(1)? as NodeId,
                rank: r.get(2)?,
                text: r.get(3)?,
                note: r.get(4)?,
                completed: r.get::<_, i64>(5)? != 0,
                collapsed: r.get::<_, i64>(6)? != 0,
                created: r.get(7)?,
                modified: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT key, value FROM meta")?;
    let meta = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((nodes, meta))
}

pub fn write_batch(
    conn: &mut Connection,
    upserts: &[Node],
    deletes: &[NodeId],
    meta: &[(String, String)],
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut up = tx.prepare_cached(
            "INSERT INTO nodes (id, parent, rank, text, note, completed, collapsed, created, modified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET parent=?2, rank=?3, text=?4, note=?5,
               completed=?6, collapsed=?7, modified=?9",
        )?;
        for n in upserts {
            up.execute(params![
                n.id as i64,
                n.parent as i64,
                n.rank,
                n.text,
                n.note,
                n.completed as i64,
                n.collapsed as i64,
                n.created,
                n.modified
            ])?;
        }
        let mut del = tx.prepare_cached("DELETE FROM nodes WHERE id = ?1")?;
        for id in deletes {
            del.execute(params![*id as i64])?;
        }
        let mut m = tx.prepare_cached(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=?2",
        )?;
        for (k, v) in meta {
            m.execute(params![k, v])?;
        }
    }
    tx.commit()
}

enum Msg {
    Write { upserts: Vec<Node>, deletes: Vec<NodeId> },
    Meta(String, String),
    Flush(Sender<()>),
}

/// Handle to the background writer. Dropping it flushes and joins the thread.
pub struct Writer {
    tx: Option<Sender<Msg>>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    pub fn spawn(path: PathBuf) -> Writer {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("flowie-store".into())
            .spawn(move || writer_loop(path, rx))
            .expect("spawn store thread");
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

    pub fn set_meta(&self, key: &str, value: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Meta(key.to_string(), value.to_string()));
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

fn writer_loop(path: PathBuf, rx: Receiver<Msg>) {
    let mut conn = match open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("flowie: cannot open database {}: {e}", path.display());
            return;
        }
    };
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
                    commit(&mut conn, &mut pending_up, &mut pending_del, &mut pending_meta);
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
            Some(Msg::Flush(ack)) => {
                waiters.push(ack);
                deadline = Some(Instant::now());
            }
            None => {}
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            commit(&mut conn, &mut pending_up, &mut pending_del, &mut pending_meta);
            deadline = None;
            for w in waiters.drain(..) {
                let _ = w.send(());
            }
        }
    }
}

fn commit(
    conn: &mut Connection,
    up: &mut std::collections::HashMap<NodeId, Node>,
    del: &mut std::collections::HashSet<NodeId>,
    meta: &mut Vec<(String, String)>,
) {
    if up.is_empty() && del.is_empty() && meta.is_empty() {
        return;
    }
    let upserts: Vec<Node> = up.drain().map(|(_, n)| n).collect();
    let deletes: Vec<NodeId> = del.drain().collect();
    if let Err(e) = write_batch(conn, &upserts, &deletes, meta) {
        eprintln!("flowie: database write failed: {e}");
    }
    meta.clear();
}
