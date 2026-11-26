//! SQLite persistence. The whole outline is loaded once at launch; after that
//! the UI never touches the database: changes go to a background writer
//! thread that batches them into one transaction per ~200 ms.

mod blobs;
mod imports;
mod trash;
#[cfg(feature = "writer-thread")]
mod writer;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::tree::{Node, NodeId};

pub use crate::imports::{ImportRecord, ImportState};
pub use blobs::{collect_unused_images, drop_blobs, image_names_in_use, load_blobs};
pub use imports::{load_bases, load_imports, write_import_batch};
pub use trash::{in_trash, list_trash, TrashEntry};
#[cfg(feature = "writer-thread")]
pub use writer::Writer;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS nodes (
    id        BLOB    PRIMARY KEY,
    parent    BLOB    NOT NULL,
    rank      TEXT    NOT NULL,   -- fractional index (crate::rank), BINARY order
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
    seen_at     INTEGER,
    source_ids  TEXT    NOT NULL DEFAULT '[]'
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

/// The database format, kept in `PRAGMA user_version`. An older database is
/// moved aside, not converted.
const FORMAT: i64 = 4;

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
    conn.busy_timeout(Duration::from_secs(5))?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let has_nodes: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'nodes'", [], |r| r.get(0))?;
    if version > FORMAT {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if has_nodes > 0 && version == FORMAT {
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        return Ok(conn);
    }
    if has_nodes > 0 && version < FORMAT {
        // Fold the WAL into the file so the three files move as one.
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        drop(conn);
        let aside = set_aside(path).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        eprintln!("flowie: {} has an older format; moved it to {}", path.display(), aside.display());
        conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
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
    write_nodes_tx(&tx, upserts, deletes, meta)?;
    tx.commit()
}

fn write_nodes_tx(tx: &rusqlite::Transaction<'_>, upserts: &[Node], deletes: &[NodeId], meta: &[(String, String)]) -> rusqlite::Result<()> {
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
            &n.rank,
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
    for (key, value) in meta {
        m.execute(params![key, value])?;
    }
    Ok(())
}
