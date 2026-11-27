//! Imported roots, merge bases, and transactional package updates.

use std::collections::HashSet;

use rusqlite::{params, Connection};

use super::write_nodes_tx;
use crate::imports::{ImportRecord, ImportState};
use crate::tree::{Dirty, NodeId};

pub fn write_import_batch(conn: &mut Connection, dirty: &Dirty, state: &ImportState) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    write_nodes_tx(&tx, dirty, &[])?;
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
    let mut kept_bases = HashSet::with_capacity(state.bases.len());
    for (node, root, hash) in &state.bases {
        kept_bases.insert((*root, *node));
        tx.execute(
            "INSERT INTO import_bases(node,root,base_hash) VALUES (?1,?2,?3)
             ON CONFLICT(node) DO UPDATE SET root = excluded.root, base_hash = excluded.base_hash",
            params![node, root, hash],
        )?;
    }
    let mut kept_roots = HashSet::with_capacity(state.records.len());
    for record in &state.records {
        kept_roots.insert(record.root);
        tx.execute(
            "INSERT INTO imports(root,package_id,title,author_id,author_name,exported_at,imported_at,seen_at,source_ids)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(root) DO UPDATE SET package_id = excluded.package_id, title = excluded.title,
               author_id = excluded.author_id, author_name = excluded.author_name,
               exported_at = excluded.exported_at, imported_at = excluded.imported_at,
               seen_at = excluded.seen_at, source_ids = excluded.source_ids",
            params![
                record.root,
                record.package_id,
                record.title,
                record.author_id,
                record.author_name,
                record.exported_at,
                record.imported_at,
                record.seen_at,
                serde_json::to_string(&record.source_ids).unwrap_or_default()
            ],
        )?;
    }
    for root in &state.roots {
        let existing: Vec<NodeId> = {
            let mut stmt = tx.prepare_cached("SELECT node FROM import_bases WHERE root = ?1")?;
            let rows = stmt.query_map([root], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for node in existing {
            if !kept_bases.contains(&(*root, node)) {
                tx.execute("DELETE FROM import_bases WHERE node = ?1", [node])?;
            }
        }
        if !kept_roots.contains(root) {
            tx.execute("DELETE FROM imports WHERE root = ?1", [root])?;
        }
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
