//! Imported roots, merge bases, and transactional package updates.

use rusqlite::{params, Connection};

use super::write_nodes_tx;
use crate::imports::{ImportRecord, ImportState};
use crate::tree::{Node, NodeId};

pub fn write_import_batch(conn: &mut Connection, upserts: &[Node], deletes: &[NodeId], state: &ImportState) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    write_nodes_tx(&tx, upserts, deletes, &[])?;
    save_import_state_tx(&tx, state)?;
    tx.commit()
}

pub fn load_imports(conn: &Connection) -> rusqlite::Result<Vec<ImportRecord>> {
    let mut stmt =
        conn.prepare("SELECT root, package_id, title, author_id, author_name, exported_at, imported_at, seen_at, source_ids FROM imports")?;
    let rows = stmt.query_map([], |r| {
        let source_ids: String = r.get(8)?;
        Ok(ImportRecord {
            root: r.get(0)?,
            package_id: r.get(1)?,
            title: r.get(2)?,
            author_id: r.get(3)?,
            author_name: r.get(4)?,
            exported_at: r.get(5)?,
            imported_at: r.get(6)?,
            seen_at: r.get(7)?,
            source_ids: serde_json::from_str(&source_ids).unwrap_or_default(),
        })
    })?;
    rows.collect()
}

#[cfg_attr(not(feature = "writer-thread"), allow(dead_code))]
pub(super) fn save_import_state(conn: &mut Connection, state: &ImportState) -> rusqlite::Result<()> {
    let transaction = conn.transaction()?;
    save_import_state_tx(&transaction, state)?;
    transaction.commit()
}

fn save_import_state_tx(tx: &rusqlite::Transaction<'_>, state: &ImportState) -> rusqlite::Result<()> {
    for root in &state.roots {
        tx.execute("DELETE FROM import_bases WHERE root=?1", [root])?;
        tx.execute("DELETE FROM imports WHERE root=?1", [root])?;
    }
    for (node, root, hash) in &state.bases {
        tx.execute("INSERT OR REPLACE INTO import_bases(node,root,base_hash) VALUES (?1,?2,?3)", params![node, root, hash])?;
    }
    for record in &state.records {
        tx.execute("INSERT INTO imports(root,package_id,title,author_id,author_name,exported_at,imported_at,seen_at,source_ids) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![record.root,record.package_id,record.title,record.author_id,record.author_name,record.exported_at,record.imported_at,record.seen_at,serde_json::to_string(&record.source_ids).unwrap_or_default()])?;
    }
    Ok(())
}

pub fn load_bases(conn: &Connection, roots: &[NodeId]) -> rusqlite::Result<Vec<(NodeId, NodeId, String)>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare("SELECT node, base_hash FROM import_bases WHERE root = ?1")?;
    for root in roots {
        for row in stmt.query_map(params![root], |r| Ok((r.get::<_, NodeId>(0)?, r.get::<_, String>(1)?)))? {
            let (node, hash) = row?;
            out.push((node, *root, hash));
        }
    }
    Ok(out)
}
