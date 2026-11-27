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
#[cfg(not(feature = "writer-thread"))]
mod inline_writer;
mod migrate;
#[cfg(feature = "writer-thread")]
mod writer;

use std::path::Path;

use rusqlite::{params, Connection};

use crate::hlc::{Clocks, Hlc};
use crate::tree::{Dirty, Node, NodeId};

pub use crate::imports::{ImportRecord, ImportState};
pub use blobs::{collect_unused_images, drop_blobs, image_names_in_use, load_blobs};
pub use imports::{load_bases, load_imports, write_import_batch};
#[cfg(not(feature = "writer-thread"))]
pub use inline_writer::Writer;
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
    deleted_at INTEGER,                     -- moved to the Trash then (ms); NULL = in the outline
    clocks     BLOB    NOT NULL DEFAULT x'', -- crate::hlc::Clocks; empty = derived from created/modified
    moved_from BLOB    NOT NULL DEFAULT x'', -- parent before the last move; empty = the parent
    extra      TEXT    NOT NULL DEFAULT '',  -- fields from a newer Flowie, as JSON; '' = none
    local_seq  INTEGER NOT NULL DEFAULT 0    -- order of writes here, for other processes to catch up
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS nodes_parent ON nodes(parent);
CREATE INDEX IF NOT EXISTS nodes_local_seq ON nodes(local_seq);
CREATE TABLE IF NOT EXISTS outbox (     -- records changed here and not sent yet (sync)
    id         BLOB    PRIMARY KEY,
    dirty_mask INTEGER NOT NULL         -- which synced fields changed (crate::hlc::Reg bits)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS purged (     -- records removed for good, and when (crate::hlc::Hlc)
    id BLOB PRIMARY KEY,
    at BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS sync_state ( -- per sync backend: where pulling left off, and the account
    remote  TEXT PRIMARY KEY,
    cursor  BLOB,
    account TEXT
);
CREATE TABLE IF NOT EXISTS bases (      -- the sync backend's version of each record last seen here
    id     BLOB PRIMARY KEY,
    etag   TEXT NOT NULL,
    record TEXT NOT NULL                -- flowie_sync::NodeRecord as JSON
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS lost (       -- text another device's edit won over (conflict copies)
    id    BLOB    NOT NULL,             -- the node
    field TEXT    NOT NULL,             -- 'text' or 'note'
    value TEXT    NOT NULL,             -- what lost
    at    INTEGER NOT NULL              -- when it was noticed (ms)
);
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
    value TEXT NOT NULL,
    at    BLOB NOT NULL DEFAULT x''   -- when it was last written (crate::hlc::Hlc)
);
";

/// Keys of the `meta` table: state of this device only, never synced (window,
/// scroll, the page shown, view options, device and tutorial ids). A key that
/// should follow the user to other devices goes in `PREFS` instead; user
/// content (like stars) goes on the nodes.
pub const LOCAL_META: &[&str] =
    &["actor.app", "device", "hide_completed", "scroll", "sync.held", "sync.prefs", "sync.prefs_base", "tutorial", "window", "zoom"];

/// Keys of the `prefs` table: preferences that sync. Sync sends the whole table
/// as one `Prefs` record, last writer wins.
pub const PREFS: &[&str] = &["saved_searches"];

/// The 1.0 format, the oldest one Flowie upgrades: an older database is moved
/// aside, not converted. See the module docs.
const BASELINE: i64 = 5;

/// Upgrade steps: entry `i` takes a database from format `BASELINE + i` to
/// `BASELINE + i + 1`. Only ever appended to; see the module docs.
const MIGRATIONS: &[Migration] = &[sync_columns];

/// 5 → 6: what per-node sync keeps (wiki: Per-Node-Sync §3.2). Existing nodes get
/// empty clocks and `moved_from`, which load as clocks derived from `created`/`modified` and
/// as the parent.
fn sync_columns(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        "ALTER TABLE nodes ADD COLUMN clocks BLOB NOT NULL DEFAULT x'';
         ALTER TABLE nodes ADD COLUMN moved_from BLOB NOT NULL DEFAULT x'';
         ALTER TABLE nodes ADD COLUMN extra TEXT NOT NULL DEFAULT '';
         ALTER TABLE nodes ADD COLUMN local_seq INTEGER NOT NULL DEFAULT 0;
         CREATE INDEX nodes_parent ON nodes(parent);
         CREATE INDEX nodes_local_seq ON nodes(local_seq);
         CREATE TABLE outbox (id BLOB PRIMARY KEY, dirty_mask INTEGER NOT NULL) WITHOUT ROWID;
         CREATE TABLE purged (id BLOB PRIMARY KEY, at BLOB NOT NULL) WITHOUT ROWID;
         CREATE TABLE sync_state (remote TEXT PRIMARY KEY, cursor BLOB, account TEXT);
         CREATE TABLE bases (id BLOB PRIMARY KEY, etag TEXT NOT NULL, record TEXT NOT NULL) WITHOUT ROWID;
         CREATE TABLE lost (id BLOB NOT NULL, field TEXT NOT NULL, value TEXT NOT NULL, at INTEGER NOT NULL);
         ALTER TABLE prefs ADD COLUMN at BLOB NOT NULL DEFAULT x'';",
    )
}

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
        "SELECT id, parent, rank, text, note, completed, collapsed, created, modified, kind, mirror, image, star, deleted_at,
                clocks, moved_from, extra FROM nodes",
    )?;
    let nodes = stmt
        .query_map([], |r| {
            let parent: NodeId = r.get(1)?;
            let (created, modified): (i64, i64) = (r.get(7)?, r.get(8)?);
            let clocks: Vec<u8> = r.get(14)?;
            let moved_from: Vec<u8> = r.get(15)?;
            Ok(Node {
                id: r.get(0)?,
                parent,
                rank: r.get(2)?,
                text: r.get(3)?,
                note: r.get(4)?,
                completed: r.get::<_, i64>(5)? != 0,
                collapsed: r.get::<_, i64>(6)? != 0,
                created,
                modified,
                kind: crate::tree::BlockKind::from_u8(r.get::<_, i64>(9)? as u8),
                mirror: r.get(10)?,
                image: r.get(11)?,
                star: r.get(12)?,
                deleted_at: r.get(13)?,
                sync: crate::tree::NodeSync {
                    clocks: Clocks::from_bytes(&clocks).unwrap_or_else(|| Clocks::derived(created, modified)),
                    moved_from: NodeId::from_bytes(&moved_from).unwrap_or(parent),
                    extra: r.get(16)?,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT key, value FROM meta")?;
    let meta = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((nodes, meta))
}

/// The synced preferences (`PREFS`), each with when it was last written (the oldest clock
/// for one saved before Flowie kept clocks).
pub fn load_prefs(conn: &Connection) -> rusqlite::Result<Vec<(String, String, Hlc)>> {
    let mut stmt = conn.prepare("SELECT key, value, at FROM prefs")?;
    let prefs = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, Hlc::from_bytes(&r.get::<_, Vec<u8>>(2)?).unwrap_or_default())))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(prefs)
}

pub fn write_pref(conn: &Connection, key: &str, value: &str, at: Hlc) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO prefs (key, value, at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value=?2, at=?3",
        params![key, value, at.to_bytes().to_vec()],
    )?;
    Ok(())
}

/// Copy the database at `db` to `dest` as it is now (what the writer has saved), without
/// stopping the writer: a consistent snapshot, even mid-write. Fails if `dest` exists.
pub fn snapshot(db: &Path, dest: &Path) -> rusqlite::Result<()> {
    let conn = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.execute("VACUUM INTO ?1", params![dest.to_string_lossy()])?;
    Ok(())
}

/// Give the file back the space that deleted rows leave behind, when there is
/// enough of it to be worth rewriting the file: at least 1 MB and a quarter of
/// the pages. SQLite otherwise keeps freed pages for reuse and the file never
/// shrinks. Returns whether it ran. Not inside a transaction.
pub fn reclaim_free_pages(conn: &Connection) -> rusqlite::Result<bool> {
    let pragma = |name: &str| conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get::<_, i64>(0));
    let (free, total, page) = (pragma("freelist_count")?, pragma("page_count")?, pragma("page_size")?);
    if free * page < 1 << 20 || free * 4 < total {
        return Ok(false);
    }
    conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(true)
}

/// What sync keeps between runs: the records waiting to be sent, the backend's version of
/// each record last seen (etag, `flowie_sync::NodeRecord` JSON), and where pulling left off
/// on each backend.
#[derive(Debug, Default)]
pub struct SyncRows {
    pub outbox: Vec<NodeId>,
    pub bases: Vec<(NodeId, String, String)>,
    pub cursors: Vec<(String, String)>,
    /// Conflict copies (`load_lost`).
    pub lost: Vec<(NodeId, String, String, i64)>,
}

pub fn load_sync(conn: &Connection) -> rusqlite::Result<SyncRows> {
    let mut stmt = conn.prepare("SELECT id FROM outbox")?;
    let outbox = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare("SELECT id, etag, record FROM bases")?;
    let bases = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare("SELECT remote, cursor FROM sync_state WHERE cursor IS NOT NULL")?;
    let cursors = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?
        .filter_map(|row| row.map(|(remote, c)| String::from_utf8(c).ok().map(|c| (remote, c))).transpose())
        .collect::<rusqlite::Result<_>>()?;
    Ok(SyncRows { outbox, bases, cursors, lost: load_lost(conn)? })
}

/// Conflict copies (`SyncWrite::Lost`), oldest first: node, field, the text that lost, and
/// when (ms).
pub fn load_lost(conn: &Connection) -> rusqlite::Result<Vec<(NodeId, String, String, i64)>> {
    let mut stmt = conn.prepare("SELECT id, field, value, at FROM lost ORDER BY at, rowid")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    rows.collect()
}

/// One change to what sync keeps (see [`SyncRows`]), saved by the writer in order with the
/// records.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncWrite {
    /// Nothing of the record is left to send.
    Sent(NodeId),
    /// The backend's version of a record: etag and record JSON.
    Base(NodeId, String, String),
    /// Where pulling from a backend left off.
    Cursor { remote: String, cursor: String },
    /// Text in a node's field (`"text"` or `"note"`) that another device's edit won over.
    Lost { id: NodeId, field: String, value: String, at: i64 },
    /// A conflict copy dealt with: restored or dismissed.
    Unlost { id: NodeId, field: String, value: String, at: i64 },
}

pub fn write_sync(conn: &Connection, write: &SyncWrite) -> rusqlite::Result<()> {
    match write {
        SyncWrite::Sent(id) => conn.execute("DELETE FROM outbox WHERE id = ?1", params![id]),
        SyncWrite::Lost { id, field, value, at } => {
            conn.execute("INSERT INTO lost (id, field, value, at) VALUES (?1, ?2, ?3, ?4)", params![id, field, value, at])
        }
        SyncWrite::Unlost { id, field, value, at } => conn.execute(
            "DELETE FROM lost WHERE rowid = (SELECT rowid FROM lost WHERE id = ?1 AND field = ?2 AND value = ?3 AND at = ?4 LIMIT 1)",
            params![id, field, value, at],
        ),
        SyncWrite::Base(id, etag, record) => conn.execute(
            "INSERT INTO bases (id, etag, record) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET etag = ?2, record = ?3",
            params![id, etag, record],
        ),
        SyncWrite::Cursor { remote, cursor } => conn.execute(
            "INSERT INTO sync_state (remote, cursor) VALUES (?1, ?2) ON CONFLICT(remote) DO UPDATE SET cursor = ?2",
            params![remote, cursor.as_bytes()],
        ),
    }
    .map(drop)
}

/// The records removed for good, with when (`Tree::set_purged`).
pub fn load_purged(conn: &Connection) -> rusqlite::Result<Vec<(NodeId, Hlc)>> {
    let mut stmt = conn.prepare("SELECT id, at FROM purged")?;
    let rows = stmt.query_map([], |r| {
        let at: Vec<u8> = r.get(1)?;
        Ok((r.get(0)?, Hlc::from_bytes(&at).unwrap_or_default()))
    })?;
    rows.collect()
}

/// Save tree changes in one transaction: the records, removals, and what sync has to send.
pub fn write_batch(conn: &mut Connection, dirty: &Dirty, meta: &[(String, String)]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    write_nodes_tx(&tx, dirty, meta)?;
    tx.commit()
}

fn write_nodes_tx(tx: &rusqlite::Transaction<'_>, dirty: &Dirty, meta: &[(String, String)]) -> rusqlite::Result<()> {
    // Rows written together share one write order, after every earlier one.
    let seq: i64 =
        if dirty.upserts.is_empty() { 0 } else { tx.query_row("SELECT ifnull(MAX(local_seq), 0) + 1 FROM nodes", [], |r| r.get(0))? };
    let mut up = tx.prepare_cached(
        "INSERT INTO nodes (id, parent, rank, text, note, completed, collapsed, created, modified, kind, mirror, image, star, deleted_at,
                            clocks, moved_from, extra, local_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(id) DO UPDATE SET parent=?2, rank=?3, text=?4, note=?5,
           completed=?6, collapsed=?7, modified=?9, kind=?10, mirror=?11, image=?12, star=?13, deleted_at=?14,
           clocks=?15, moved_from=?16, extra=?17, local_seq=?18",
    )?;
    let mut unpurge = tx.prepare_cached("DELETE FROM purged WHERE id = ?1")?;
    for n in &dirty.upserts {
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
            n.deleted_at,
            n.sync.clocks.to_bytes(),
            n.sync.moved_from,
            n.sync.extra,
            seq
        ])?;
        unpurge.execute(params![n.id])?;
    }
    let mut del = tx.prepare_cached("DELETE FROM nodes WHERE id = ?1")?;
    let mut purge = tx.prepare_cached("INSERT OR REPLACE INTO purged (id, at) VALUES (?1, ?2)")?;
    for (id, at) in &dirty.removed {
        del.execute(params![id])?;
        purge.execute(params![id, at.to_bytes().to_vec()])?;
    }
    let mut outbox = tx.prepare_cached(
        "INSERT INTO outbox (id, dirty_mask) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET dirty_mask = CASE WHEN ?2 & ?3 != 0 THEN ?2 ELSE (dirty_mask & ~?3) | ?2 END",
    )?;
    for (id, mask) in &dirty.unsent {
        outbox.execute(params![id, *mask as i64, crate::hlc::REMOVED as i64])?;
    }
    let mut m = tx.prepare_cached("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=?2")?;
    for (key, value) in meta {
        m.execute(params![key, value])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::Clocks;
    use crate::tree::ROOT;

    /// Format 5 (1.0) as 1.0 created it. Never edit: users' databases are in this format.
    const V5_SCHEMA: &str = "
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

    /// Each table's columns and each index's columns, to compare a migrated database with a
    /// new one.
    fn layout(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("SELECT type, name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name").unwrap();
        let objects: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        let mut out = Vec::new();
        for (kind, name) in objects {
            let sql = if kind == "table" {
                "SELECT name || ' ' || type || ' ' || \"notnull\" || ' ' || ifnull(dflt_value, '-') || ' ' || pk FROM pragma_table_info(?1)"
            } else {
                "SELECT name FROM pragma_index_info(?1)"
            };
            let mut cols = conn.prepare(sql).unwrap();
            let cols: Vec<String> = cols.query_map([&name], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
            out.push(format!("{kind} {name}: {}", cols.join(", ")));
        }
        out
    }

    #[test]
    fn conflict_copies_are_kept_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("flowie.sqlite")).unwrap();
        let id = NodeId::random();
        for (value, at) in [("first", 5), ("second", 9)] {
            write_sync(&conn, &SyncWrite::Lost { id, field: "text".into(), value: value.into(), at }).unwrap();
        }
        let lost = load_lost(&conn).unwrap();
        assert_eq!(lost, [(id, "text".into(), "first".into(), 5), (id, "text".into(), "second".into(), 9)]);
        write_sync(&conn, &SyncWrite::Unlost { id, field: "text".into(), value: "first".into(), at: 5 }).unwrap();
        assert_eq!(load_lost(&conn).unwrap(), [(id, "text".into(), "second".into(), 9)]);
    }

    #[test]
    fn a_snapshot_copies_what_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let (db, copy) = (dir.path().join("flowie.sqlite"), dir.path().join("copy.sqlite"));
        let mut conn = open(&db).unwrap();
        let node = Node::new(NodeId::random(), ROOT, crate::rank::between(None, None));
        write_batch(&mut conn, &Dirty { upserts: vec![node.clone()], ..Dirty::default() }, &[]).unwrap();
        snapshot(&db, &copy).unwrap();
        let (nodes, _) = load(&open(&copy).unwrap()).unwrap();
        assert_eq!(nodes, [node]);
        assert!(snapshot(&db, &copy).is_err(), "never over an existing file");
    }

    #[test]
    fn a_1_0_database_upgrades_to_what_sync_keeps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        let (id, child) = (NodeId::random(), NodeId::random());
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(V5_SCHEMA).unwrap();
            for (n, parent, text) in [(id, ROOT, "a"), (child, id, "b")] {
                conn.execute(
                    "INSERT INTO nodes (id, parent, rank, text, created, modified, mirror) VALUES (?1, ?2, 'a0', ?3, 1000, 2000, ?4)",
                    params![n, parent, text, ROOT],
                )
                .unwrap();
            }
            conn.pragma_update(None, "user_version", 5).unwrap();
        }

        let conn = open(&path).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, FORMAT);
        let fresh = open(&dir.path().join("fresh.sqlite")).unwrap();
        assert_eq!(layout(&conn), layout(&fresh), "the upgrade builds what a new database has");

        // Old rows get clocks from their times, and their own parent as the one moved from.
        let (nodes, _) = load(&conn).unwrap();
        let loaded = nodes.iter().find(|n| n.id == child).unwrap();
        assert_eq!(loaded.text, "b");
        assert_eq!(loaded.sync.clocks, Clocks::derived(1000, 2000));
        assert_eq!(loaded.sync.moved_from, id);
        assert_eq!(loaded.sync.extra, "");

        // What sync keeps is saved and read back as it was.
        let mut edited = loaded.clone();
        edited.sync.clocks.set(crate::hlc::Reg::Text, crate::hlc::Hlc { wall: 3000, counter: 2, actor: 9 });
        edited.sync.moved_from = ROOT;
        edited.sync.extra = r#"{"future":1}"#.into();
        let mut conn = conn;
        write_batch(&mut conn, &Dirty { upserts: vec![edited.clone()], ..Dirty::default() }, &[]).unwrap();
        let (nodes, _) = load(&conn).unwrap();
        assert_eq!(nodes.iter().find(|n| n.id == child), Some(&edited));
    }

    #[test]
    fn a_batch_saves_removals_and_the_outbox_with_the_records() {
        use crate::hlc::{Reg, REMOVED};
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(&dir.path().join("flowie.sqlite")).unwrap();
        let (a, b) = (Node::new(NodeId::random(), ROOT, "a0".into()), Node::new(NodeId::random(), ROOT, "a1".into()));
        let gone = Hlc { wall: 50, counter: 1, actor: 7 };
        let outbox = |conn: &Connection| -> Vec<(NodeId, i64)> {
            let mut stmt = conn.prepare("SELECT id, dirty_mask FROM outbox ORDER BY id").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
        };
        let seq = |conn: &Connection, id: NodeId| -> i64 {
            conn.query_row("SELECT local_seq FROM nodes WHERE id = ?1", params![id], |r| r.get(0)).unwrap()
        };

        let first =
            Dirty { upserts: vec![a.clone(), b.clone()], removed: vec![], unsent: vec![(a.id, Reg::Text.bit()), (b.id, Reg::Text.bit())] };
        write_batch(&mut conn, &first, &[]).unwrap();
        assert_eq!(seq(&conn, a.id), seq(&conn, b.id), "one batch, one place in the write order");
        // Later changes add bits; a removal replaces them and leaves a tombstone.
        let second =
            Dirty { upserts: vec![a.clone()], removed: vec![(b.id, gone)], unsent: vec![(a.id, Reg::Position.bit()), (b.id, REMOVED)] };
        write_batch(&mut conn, &second, &[]).unwrap();
        assert!(seq(&conn, a.id) > 1);
        let mut expected = vec![(a.id, (Reg::Text.bit() | Reg::Position.bit()) as i64), (b.id, REMOVED as i64)];
        expected.sort();
        assert_eq!(outbox(&conn), expected);
        assert_eq!(load_purged(&conn).unwrap(), [(b.id, gone)]);
        assert_eq!(load(&conn).unwrap().0.len(), 1);

        // A record written again leaves the tombstones, and its outbox bits start over.
        let back = Dirty { upserts: vec![b.clone()], removed: vec![], unsent: vec![(b.id, crate::hlc::ALL_REGS)] };
        write_batch(&mut conn, &back, &[]).unwrap();
        assert!(load_purged(&conn).unwrap().is_empty());
        assert!(outbox(&conn).contains(&(b.id, crate::hlc::ALL_REGS as i64)));
    }
}
