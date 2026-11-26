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
    /// True when the stored payload could not be parsed or contained no nodes.
    pub corrupted: bool,
}

/// Trash entries, newest first. Entries older than `keep_days` are purged.
pub fn list_trash(conn: &Connection, keep_days: i64) -> rusqlite::Result<Vec<TrashEntry>> {
    let cutoff = crate::tree::now_millis() - keep_days * 86_400_000;
    conn.execute("DELETE FROM trash WHERE deleted_at < ?1", params![cutoff])?;
    let mut stmt = conn.prepare("SELECT seq, root, deleted_at, label, payload FROM trash ORDER BY seq DESC")?;
    let rows = stmt.query_map([], |r| {
        let seq: i64 = r.get(0)?;
        let payload: String = r.get(4)?;
        let (nodes, corrupted) = match serde_json::from_str::<Vec<Node>>(&payload) {
            Ok(nodes) if !nodes.is_empty() => (nodes, false),
            Ok(_) => {
                eprintln!("flowie: trash entry {seq} has an empty payload");
                (Vec::new(), true)
            }
            Err(e) => {
                eprintln!("flowie: could not parse trash entry {seq}: {e}");
                (Vec::new(), true)
            }
        };
        Ok(TrashEntry { seq, root: r.get(1)?, deleted_at: r.get(2)?, label: r.get(3)?, nodes, corrupted })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::open;
    use crate::ROOT;

    #[test]
    fn unparseable_or_empty_trash_entries_are_marked_corrupted() {
        let work = std::env::temp_dir().join(format!("flowie-trash-corrupt-{}", NodeId::random()));
        let db = work.join("flowie.sqlite");
        let conn = open(&db).unwrap();
        let now = crate::tree::now_millis();
        let ok_node = Node::new(NodeId::random(), ROOT, "a0".into());
        let ok_payload = serde_json::to_string(&vec![ok_node.clone()]).unwrap();
        let bad_root = NodeId::random();
        let empty_root = NodeId::random();
        conn.execute(
            "INSERT INTO trash (root, deleted_at, label, payload) VALUES (?1, ?2, 'ok', ?3)",
            params![ok_node.id, now, ok_payload],
        )
        .unwrap();
        conn.execute("INSERT INTO trash (root, deleted_at, label, payload) VALUES (?1, ?2, 'bad', '{not-json')", params![bad_root, now])
            .unwrap();
        conn.execute("INSERT INTO trash (root, deleted_at, label, payload) VALUES (?1, ?2, 'empty', '[]')", params![empty_root, now])
            .unwrap();

        let entries = list_trash(&conn, 30).unwrap();
        assert_eq!(entries.len(), 3);
        // Newest first (highest seq first).
        assert_eq!(entries[0].root, empty_root);
        assert!(entries[0].corrupted && entries[0].nodes.is_empty());
        assert_eq!(entries[1].root, bad_root);
        assert!(entries[1].corrupted && entries[1].nodes.is_empty());
        assert_eq!(entries[2].root, ok_node.id);
        assert!(!entries[2].corrupted && entries[2].nodes.len() == 1);

        drop(conn);
        std::fs::remove_dir_all(work).unwrap();
    }
}
