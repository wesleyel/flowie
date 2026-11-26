//! SQLite persistence. The whole outline is loaded once at launch; after that
//! the UI never touches the database: changes go to a background writer
//! thread that batches them into one transaction per ~200 ms.
//!
//! # Formats and migrations
//!
//! The database's format is its `PRAGMA user_version`. `BASELINE` is the 1.0
//! format, and each entry of `MIGRATIONS` upgrades by one, so this build
//! writes `FORMAT = BASELINE + MIGRATIONS.len()`. [`open`] creates a new
//! database straight at `FORMAT` from `SCHEMA`, opens one at `FORMAT` as it
//! is, and upgrades one between `BASELINE` and `FORMAT`: it saves a copy as
//! `<db>.backup-v<format>-<millis>` (keeping the newest three), then runs the
//! missing migrations and sets `user_version` in one transaction. If the
//! backup or any step fails, nothing is changed and [`open`] returns the
//! error. A database older than `BASELINE` (a 1.0 release candidate's) is
//! renamed to `<db>.old-<millis>` and a new one created; one newer than
//! `FORMAT` is refused with [`OpenError::TooNew`] and left alone.
//!
//! To change the format (from 1.0 on, when `BASELINE` no longer changes):
//!
//! 1. Change `SCHEMA` so it creates the new format: a new database never runs
//!    migrations.
//! 2. Append one [`Migration`] to `MIGRATIONS` that turns a database of the
//!    previous format into exactly what the new `SCHEMA` creates (`ALTER
//!    TABLE`, `CREATE TABLE`/`INDEX`, and filling in data for existing rows).
//!    It runs inside the upgrade's transaction: don't commit, don't set
//!    `user_version`, and never edit or reorder earlier entries, which users'
//!    databases have already been through.
//! 3. Test it: build a database at the previous format (copy the old `CREATE`
//!    statements into the test, set `user_version`, add rows), [`open`] it,
//!    and check that the rows came through and that each table has the same
//!    columns (`pragma_table_info`) as a database created from `SCHEMA`. The
//!    framework itself is tested in `store/migrate.rs`.
//! 4. Add a CHANGELOG entry: an upgrade is user-visible, since it leaves a
//!    backup beside the database and older Flowies can't open the result.

mod blobs;
mod imports;
mod migrate;
#[cfg(feature = "writer-thread")]
mod writer;

use std::path::Path;

use rusqlite::{params, Connection};

use crate::tree::{Node, NodeId};

pub use crate::imports::{ImportRecord, ImportState};
pub use blobs::{collect_unused_images, drop_blobs, image_names_in_use, load_blobs};
pub use imports::{load_bases, load_imports, write_import_batch};
pub use migrate::{Migration, OpenError};
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
    image     TEXT    NOT NULL DEFAULT '',
    star      TEXT    NOT NULL DEFAULT '',  -- fractional index in the starred list; '' = not starred
    deleted_at INTEGER                      -- moved to the Trash then (ms); NULL = in the outline
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS blobs (   -- image files in the image folder
    name    TEXT    PRIMARY KEY,     -- <sha256>.<ext>
    mime    TEXT    NOT NULL,
    width   INTEGER NOT NULL,
    height  INTEGER NOT NULL,
    size    INTEGER NOT NULL
) WITHOUT ROWID;

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
CREATE TABLE IF NOT EXISTS meta (   -- this device only (LOCAL_META)
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS prefs (  -- synced preferences (PREFS)
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Keys of the `meta` table: state of this device only, never synced (window,
/// scroll, the page shown, view options, device and tutorial ids). A key that
/// should follow the user to other devices goes in `PREFS` instead; user
/// content (like stars) goes on the nodes.
pub const LOCAL_META: &[&str] = &["device", "hide_completed", "scroll", "tutorial", "window", "zoom"];

/// Keys of the `prefs` table: preferences that sync. Sync sends the whole table
/// as one `Prefs` record, last writer wins.
pub const PREFS: &[&str] = &["saved_searches"];

/// The 1.0 format, the oldest one Flowie upgrades: an older database is moved
/// aside, not converted. See the module docs.
const BASELINE: i64 = 5;

/// Upgrade steps: entry `i` takes a database from format `BASELINE + i` to
/// `BASELINE + i + 1`. Only ever appended to; see the module docs.
const MIGRATIONS: &[Migration] = &[];

/// The format this build writes, kept in `PRAGMA user_version`.
pub const FORMAT: i64 = BASELINE + MIGRATIONS.len() as i64;

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

/// Open the database at `path`, creating or upgrading it to `FORMAT` (see the
/// module docs).
pub fn open(path: &Path) -> Result<Connection, OpenError> {
    migrate::open(path, &migrate::Format { schema: SCHEMA, baseline: BASELINE, migrations: MIGRATIONS })
}

pub fn load(conn: &Connection) -> rusqlite::Result<(Vec<Node>, Vec<(String, String)>)> {
    let mut stmt = conn.prepare(
        "SELECT id, parent, rank, text, note, completed, collapsed, created, modified, kind, mirror, image, star, deleted_at FROM nodes",
    )?;
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
                star: r.get(12)?,
                deleted_at: r.get(13)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT key, value FROM meta")?;
    let meta = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((nodes, meta))
}

/// The synced preferences (`PREFS`).
pub fn load_prefs(conn: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT key, value FROM prefs")?;
    let prefs = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(prefs)
}

pub fn write_pref(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO prefs (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=?2", params![key, value])?;
    Ok(())
}

pub fn write_batch(conn: &mut Connection, upserts: &[Node], deletes: &[NodeId], meta: &[(String, String)]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    write_nodes_tx(&tx, upserts, deletes, meta)?;
    tx.commit()
}

fn write_nodes_tx(tx: &rusqlite::Transaction<'_>, upserts: &[Node], deletes: &[NodeId], meta: &[(String, String)]) -> rusqlite::Result<()> {
    let mut up = tx.prepare_cached(
        "INSERT INTO nodes (id, parent, rank, text, note, completed, collapsed, created, modified, kind, mirror, image, star, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(id) DO UPDATE SET parent=?2, rank=?3, text=?4, note=?5,
           completed=?6, collapsed=?7, modified=?9, kind=?10, mirror=?11, image=?12, star=?13, deleted_at=?14",
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
            n.image,
            n.star,
            n.deleted_at
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
