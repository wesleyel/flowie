//! The writer for builds without threads (`store` without `writer-thread`: the browser's
//! wasm build). Same calls as the background writer (`writer.rs`), but each saves at once on
//! the calling thread, in its own transaction. A failed save is reported and not retried: a
//! browser's storage that fails stays failing until the page is reloaded.

use std::cell::RefCell;

use rusqlite::{params, Connection};

use super::{
    imports::save_import_state, reclaim_free_pages, write_batch, write_import_batch, write_pref, write_sync, ImportState, SyncWrite,
};
use super::{LOCAL_META, PREFS};
use crate::hlc::Hlc;
use crate::tree::{Dirty, NodeId};

/// Saves to the database as each change comes.
pub struct Writer {
    conn: RefCell<Connection>,
    last_error: RefCell<Option<String>>,
    on_status: Box<dyn Fn(Option<String>)>,
}

impl Writer {
    /// Named like the background writer's, which starts a thread; this one doesn't.
    pub fn spawn(conn: Connection) -> Writer {
        Self::spawn_with_error_hook(conn, |_| {})
    }

    pub fn spawn_with_error_hook<F>(conn: Connection, on_status: F) -> Writer
    where
        F: Fn(Option<String>) + Send + 'static,
    {
        Writer { conn: RefCell::new(conn), last_error: RefCell::new(None), on_status: Box::new(on_status) }
    }

    fn run(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<()>) {
        let result = f(&mut self.conn.borrow_mut()).map_err(|e| e.to_string());
        let error = result.err();
        if *self.last_error.borrow() != error {
            (self.on_status)(error.clone());
            *self.last_error.borrow_mut() = error;
        }
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.borrow().clone()
    }

    pub fn write(&self, dirty: Dirty) {
        if !dirty.is_empty() {
            self.run(|conn| write_batch(conn, &dirty, &[]));
        }
    }

    pub fn set_meta(&self, key: &str, value: &str) {
        debug_assert!(LOCAL_META.contains(&key), "meta key {key:?} is not in store::LOCAL_META");
        let meta = [(key.to_string(), value.to_string())];
        self.run(|conn| write_batch(conn, &Dirty::default(), &meta));
    }

    pub fn set_pref(&self, key: &str, value: &str, at: Hlc) {
        debug_assert!(PREFS.contains(&key), "pref key {key:?} is not in store::PREFS");
        self.run(|conn| write_pref(conn, key, value, at));
    }

    pub fn import_state(&self, state: ImportState) {
        self.run(|conn| save_import_state(conn, &state));
    }

    pub fn import_batch(&self, dirty: Dirty, state: ImportState) {
        self.run(|conn| write_import_batch(conn, &dirty, &state));
    }

    pub fn sync(&self, write: SyncWrite) {
        self.run(|conn| write_sync(conn, &write));
    }

    pub fn blob(&self, b: crate::images::BlobInfo) {
        self.run(|conn| super::blobs::write_blob(conn, &b));
    }

    pub fn seen(&self, root: NodeId, at: i64) {
        self.run(|conn| conn.execute("UPDATE imports SET seen_at = ?2 WHERE root = ?1", params![root, at]).map(drop));
    }

    pub fn reclaim_free_pages(&self) {
        self.run(|conn| reclaim_free_pages(conn).map(drop));
    }

    /// Nothing waits to be retried.
    pub fn retry_now(&self) {}

    /// Everything is saved already; the last error, if the last save failed.
    pub fn flush(&self) -> Result<(), String> {
        self.last_error().map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{load, load_prefs, open};
    use crate::tree::{Node, ROOT};

    #[test]
    fn each_change_is_saved_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("flowie.sqlite");
        let writer = Writer::spawn(open(&db).unwrap());
        let node = Node::new(NodeId::random(), ROOT, crate::rank::between(None, None));
        writer.write(Dirty { upserts: vec![node.clone()], ..Dirty::default() });
        writer.set_pref("saved_searches", "is:todo", Hlc::default());
        writer.set_meta("zoom", "0");
        // Read back on another connection while the writer is still open.
        let conn = open(&db).unwrap();
        let (nodes, meta) = load(&conn).unwrap();
        assert_eq!(nodes, [node]);
        assert!(meta.contains(&("zoom".to_string(), "0".to_string())));
        assert_eq!(load_prefs(&conn).unwrap()[0].1, "is:todo");
        assert!(writer.flush().is_ok());
    }
}
