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
    id        BLOB    PRIMARY KEY,
    parent    BLOB    NOT NULL,
    rank      REAL    NOT NULL,
    text      TEXT    NOT NULL DEFAULT '',
    note      TEXT    NOT NULL DEFAULT '',
    completed INTEGER NOT NULL DEFAULT 0,
    collapsed INTEGER NOT NULL DEFAULT 0,
    created   INTEGER NOT NULL,
    modified  INTEGER NOT NULL,
    kind      INTEGER NOT NULL DEFAULT 0,
    mirror    BLOB    NOT NULL,
    image     TEXT    NOT NULL DEFAULT ''
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS blobs (   -- image files in the image folder
    name    TEXT    PRIMARY KEY,     -- <sha256>.<ext>
    mime    TEXT    NOT NULL,
    width   INTEGER NOT NULL,
    height  INTEGER NOT NULL,
    size    INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS trash (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    root       BLOB    NOT NULL,
    deleted_at INTEGER NOT NULL,
    label      TEXT    NOT NULL,
    payload    TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS imports (
    root        BLOB    PRIMARY KEY,
    package_id  TEXT    NOT NULL,
    title       TEXT    NOT NULL,
    author_id   TEXT    NOT NULL,
    author_name TEXT    NOT NULL,
    exported_at INTEGER NOT NULL,
    imported_at INTEGER NOT NULL,
    seen_at     INTEGER
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS import_bases (   -- each imported bullet as the package had it
    node      BLOB PRIMARY KEY,
    root      BLOB NOT NULL,                -- the imported root it came with
    base_hash TEXT NOT NULL                 -- package::content_hash at the last import
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// The database format, kept in `PRAGMA user_version`. Version 2 has UUID
/// node ids, version 3 images; a database from before is moved aside, not converted.
const FORMAT: i64 = 3;

impl rusqlite::ToSql for NodeId {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::from(self.to_bytes().to_vec()))
    }
}

impl rusqlite::types::FromSql for NodeId {
    fn column_result(v: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let b = v.as_blob()?;
        NodeId::from_bytes(b).ok_or(rusqlite::types::FromSqlError::InvalidBlobSize { expected_size: 16, blob_size: b.len() })
    }
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut conn = Connection::open(path)?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let has_nodes: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'nodes'", [], |r| r.get(0))?;
    if has_nodes > 0 && version < FORMAT {
        // Fold the WAL into the file so the three files move as one.
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        drop(conn);
        let aside = set_aside(path).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        eprintln!("flowie: {} has an older format; moved it to {}", path.display(), aside.display());
        conn = Connection::open(path)?;
    }
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "user_version", FORMAT)?;
    Ok(conn)
}

/// Rename an old-format database (and its WAL and shared-memory files) to
/// `<name>.old-<millis>`, where a newer Flowie leaves it alone.
fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".old-{}", crate::tree::now_millis()));
    let aside = PathBuf::from(aside);
    let mut moved = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let mut from = path.as_os_str().to_owned();
        from.push(suffix);
        let mut to = aside.as_os_str().to_owned();
        to.push(suffix);
        if Path::new(&from).exists() {
            if let Err(error) = std::fs::rename(&from, &to) {
                for (original, moved_to) in moved.iter().rev() {
                    let _ = std::fs::rename(moved_to, original);
                }
                return Err(error);
            }
            moved.push((PathBuf::from(from), PathBuf::from(to)));
        }
    }
    Ok(aside)
}

pub fn load(conn: &Connection) -> rusqlite::Result<(Vec<Node>, Vec<(String, String)>)> {
    let mut stmt =
        conn.prepare("SELECT id, parent, rank, text, note, completed, collapsed, created, modified, kind, mirror, image FROM nodes")?;
    let nodes = stmt
        .query_map([], |r| {
            Ok(Node {
                id: r.get(0)?,
                parent: r.get(1)?,
                rank: r.get(2)?,
                text: r.get(3)?,
                note: r.get(4)?,
                completed: r.get::<_, i64>(5)? != 0,
                collapsed: r.get::<_, i64>(6)? != 0,
                created: r.get(7)?,
                modified: r.get(8)?,
                kind: crate::tree::BlockKind::from_u8(r.get::<_, i64>(9)? as u8),
                mirror: r.get(10)?,
                image: r.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT key, value FROM meta")?;
    let meta = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((nodes, meta))
}

pub fn write_batch(conn: &mut Connection, upserts: &[Node], deletes: &[NodeId], meta: &[(String, String)]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut up = tx.prepare_cached(
            "INSERT INTO nodes (id, parent, rank, text, note, completed, collapsed, created, modified, kind, mirror, image)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET parent=?2, rank=?3, text=?4, note=?5,
               completed=?6, collapsed=?7, modified=?9, kind=?10, mirror=?11, image=?12",
        )?;
        for n in upserts {
            up.execute(params![
                n.id,
                n.parent,
                n.rank,
                n.text,
                n.note,
                n.completed as i64,
                n.collapsed as i64,
                n.created,
                n.modified,
                n.kind.to_u8() as i64,
                n.mirror,
                n.image
            ])?;
        }
        let mut del = tx.prepare_cached("DELETE FROM nodes WHERE id = ?1")?;
        for id in deletes {
            del.execute(params![id])?;
        }
        let mut m = tx.prepare_cached("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=?2")?;
        for (k, v) in meta {
            m.execute(params![k, v])?;
        }
    }
    tx.commit()
}

/// A root imported from a share package (one row per root). The bullet
/// shows an unread dot until `seen_at` is set.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRecord {
    pub root: NodeId,
    pub package_id: String,
    pub title: String,
    pub author_id: String,
    pub author_name: String,
    pub exported_at: i64,
    pub imported_at: i64,
    pub seen_at: Option<i64>,
}

pub fn load_imports(conn: &Connection) -> rusqlite::Result<Vec<ImportRecord>> {
    let mut stmt =
        conn.prepare("SELECT root, package_id, title, author_id, author_name, exported_at, imported_at, seen_at FROM imports")?;
    let rows = stmt.query_map([], |r| {
        Ok(ImportRecord {
            root: r.get(0)?,
            package_id: r.get(1)?,
            title: r.get(2)?,
            author_id: r.get(3)?,
            author_name: r.get(4)?,
            exported_at: r.get(5)?,
            imported_at: r.get(6)?,
            seen_at: r.get(7)?,
        })
    })?;
    rows.collect()
}

/// The merge bases of everything imported with `roots`.
pub fn load_bases(conn: &Connection, roots: &[NodeId]) -> rusqlite::Result<std::collections::HashMap<NodeId, String>> {
    let mut out = std::collections::HashMap::new();
    let mut stmt = conn.prepare("SELECT node, base_hash FROM import_bases WHERE root = ?1")?;
    for root in roots {
        for row in stmt.query_map(params![root], |r| Ok((r.get::<_, NodeId>(0)?, r.get::<_, String>(1)?)))? {
            let (node, hash) = row?;
            out.insert(node, hash);
        }
    }
    Ok(out)
}

pub fn load_blobs(conn: &Connection) -> rusqlite::Result<Vec<crate::images::BlobInfo>> {
    let mut stmt = conn.prepare("SELECT name, mime, width, height, size FROM blobs")?;
    let rows = stmt.query_map([], |r| {
        Ok(crate::images::BlobInfo {
            name: r.get(0)?,
            mime: r.get(1)?,
            width: r.get(2)?,
            height: r.get(3)?,
            size: r.get::<_, i64>(4)? as u64,
        })
    })?;
    rows.collect()
}

/// Image files still referred to: by a bullet, or by one in the trash.
pub fn image_names_in_use(conn: &Connection) -> rusqlite::Result<std::collections::HashSet<String>> {
    let mut used: std::collections::HashSet<String> = conn
        .prepare("SELECT DISTINCT image FROM nodes WHERE image != ''")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare("SELECT payload FROM trash WHERE payload LIKE '%\"image\"%'")?;
    for payload in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let nodes: Vec<Node> = serde_json::from_str(&payload?).unwrap_or_default();
        used.extend(nodes.into_iter().map(|n| n.image).filter(|i| !i.is_empty()));
    }
    Ok(used)
}

/// Forget blobs whose files are gone (after `images::ImageDir::collect`).
pub fn drop_blobs(conn: &Connection, names: &[String]) -> rusqlite::Result<()> {
    for n in names {
        conn.execute("DELETE FROM blobs WHERE name = ?1", params![n])?;
    }
    Ok(())
}

/// Whether a bullet with this id sits in the trash (inside any deleted subtree).
pub fn in_trash(conn: &Connection, id: NodeId) -> rusqlite::Result<bool> {
    // Payloads are JSON with ids as hex strings.
    let needle = format!("%\"{id}\"%");
    conn.query_row("SELECT EXISTS (SELECT 1 FROM trash WHERE payload LIKE ?1)", params![needle], |r| r.get(0))
}

/// A deleted subtree kept for restoring.
#[derive(Debug, Clone)]
pub struct TrashEntry {
    pub seq: i64,
    pub root: NodeId,
    pub deleted_at: i64,
    pub label: String,
    /// The root first, then descendants (parents before children).
    pub nodes: Vec<Node>,
}

/// Trash entries, newest first. Entries older than `keep_days` are purged.
pub fn list_trash(conn: &Connection, keep_days: i64) -> rusqlite::Result<Vec<TrashEntry>> {
    let cutoff = crate::tree::now_millis() - keep_days * 86_400_000;
    conn.execute("DELETE FROM trash WHERE deleted_at < ?1", params![cutoff])?;
    let mut stmt = conn.prepare("SELECT seq, root, deleted_at, label, payload FROM trash ORDER BY seq DESC")?;
    let rows = stmt.query_map([], |r| {
        let payload: String = r.get(4)?;
        Ok(TrashEntry {
            seq: r.get(0)?,
            root: r.get(1)?,
            deleted_at: r.get(2)?,
            label: r.get(3)?,
            nodes: serde_json::from_str(&payload).unwrap_or_default(),
        })
    })?;
    rows.collect()
}

enum Msg {
    Write { upserts: Vec<Node>, deletes: Vec<NodeId> },
    Trash { root: NodeId, label: String, nodes: Vec<Node> },
    Untrash(i64),
    Meta(String, String),
    Import(ImportRecord),
    Blob(crate::images::BlobInfo),
    Bases(Vec<(NodeId, NodeId, String)>),
    Seen { root: NodeId, at: i64 },
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
        let thread =
            std::thread::Builder::new().name("flowie-store".into()).spawn(move || writer_loop(path, rx)).expect("spawn store thread");
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

    pub fn set_meta(&self, key: &str, value: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Meta(key.to_string(), value.to_string()));
        }
    }

    /// Record an imported root.
    pub fn import(&self, record: ImportRecord) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Import(record));
        }
    }

    /// Note an image file's size and type (written before any bullet refers to it).
    pub fn blob(&self, info: crate::images::BlobInfo) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Blob(info));
        }
    }

    /// Record merge bases: (bullet, its import root, content hash).
    pub fn bases(&self, rows: Vec<(NodeId, NodeId, String)>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Bases(rows));
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
            Some(Msg::Trash { root, label, nodes }) => {
                // Make sure earlier writes land first, then record the trash entry.
                commit(&mut conn, &mut pending_up, &mut pending_del, &mut pending_meta);
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
            Some(Msg::Import(r)) => {
                if let Err(e) = conn.execute(
                    "INSERT OR REPLACE INTO imports (root, package_id, title, author_id, author_name, exported_at, imported_at, seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![r.root, r.package_id, r.title, r.author_id, r.author_name, r.exported_at, r.imported_at, r.seen_at],
                ) {
                    eprintln!("flowie: import record write failed: {e}");
                }
            }
            Some(Msg::Blob(b)) => {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO blobs (name, mime, width, height, size) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![b.name, b.mime, b.width, b.height, b.size as i64],
                );
            }
            Some(Msg::Bases(rows)) => {
                let tx = conn.transaction();
                if let Ok(tx) = tx {
                    for (node, root, hash) in rows {
                        let _ = tx.execute(
                            "INSERT OR REPLACE INTO import_bases (node, root, base_hash) VALUES (?1, ?2, ?3)",
                            params![node, root, hash],
                        );
                    }
                    let _ = tx.commit();
                }
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
