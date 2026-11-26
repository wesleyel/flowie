//! Image metadata and garbage collection for one database.

use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection};

use super::open;
use crate::tree::Node;

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

pub fn collect_unused_images(path: &Path, min_age: Duration) -> rusqlite::Result<Vec<String>> {
    let conn = open(path)?;
    let used = image_names_in_use(&conn)?;
    let removed = crate::images::ImageDir::beside(path).collect(&used, min_age);
    drop_blobs(&conn, &removed)?;
    Ok(removed)
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

#[cfg(all(test, feature = "writer-thread"))]
mod tests {
    use super::*;
    use crate::images::ImageDir;
    use crate::store::Writer;
    use crate::{BlockKind, NodeId, ROOT};

    #[test]
    fn collection_keeps_images_in_live_and_trashed_bullets() {
        let work = std::env::temp_dir().join(format!("flowie-image-gc-{}", NodeId::random()));
        let db = work.join("flowie.sqlite");
        let writer = Writer::spawn(open(&db).unwrap());
        let images = ImageDir::beside(&db);
        let live = images.put(b"GIF89a\x05\x00\x07\x00").unwrap();
        let trashed = images.put(b"GIF89a\x06\x00\x07\x00").unwrap();
        let unused = images.put(b"GIF89a\x07\x00\x07\x00").unwrap();
        for blob in [&live, &trashed, &unused] {
            writer.blob(blob.clone());
        }
        let image_node = |name: &str| {
            let mut node = Node::new(NodeId::random(), ROOT, "a0".into());
            node.kind = BlockKind::Image;
            node.image = name.to_string();
            node
        };
        writer.write(vec![image_node(&live.name)], Vec::new());
        let deleted = image_node(&trashed.name);
        writer.trash(deleted.id, "image", vec![deleted]);
        writer.flush().unwrap();
        drop(writer);

        assert_eq!(collect_unused_images(&db, Duration::ZERO).unwrap(), std::slice::from_ref(&unused.name));
        assert!(images.has(&live.name));
        assert!(images.has(&trashed.name));
        assert!(!images.has(&unused.name));
        let conn = open(&db).unwrap();
        let blobs = load_blobs(&conn).unwrap();
        assert_eq!(blobs.len(), 2);
        assert!(blobs.contains(&live) && blobs.contains(&trashed));
        drop(conn);
        std::fs::remove_dir_all(work).unwrap();
    }
}
