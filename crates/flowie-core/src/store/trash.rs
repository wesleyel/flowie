//! Persistence for deleted subtrees.

use rusqlite::{params, Connection};

use crate::tree::{Node, NodeId};

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
