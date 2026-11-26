//! Flowie's document layer: an open outline (tree, undo history, database writer) and the
//! path every edit takes to be recorded and saved. No UI code, and no GPUI.

use std::collections::HashMap;
use std::path::Path;

use flowie_core::images::{BlobInfo, ImageDir};
use flowie_core::store::{self, ImportRecord, ImportState, Writer};
use flowie_core::{Caret, History, HistoryKind, Node, NodeId, Tree};

/// An open outline and what saving it needs.
pub struct Workspace {
    pub tree: Tree,
    pub history: History,
    /// The background database writer (none without a database).
    pub writer: Option<Writer>,
    /// Roots imported from share packages, by root id.
    pub imports: HashMap<NodeId, ImportRecord>,
    /// What is known about each image file, by name.
    pub blobs: HashMap<String, BlobInfo>,
    /// Image files, beside the database (none without one).
    pub image_dir: Option<ImageDir>,
    /// Preferences that sync (`store::PREFS`), by key.
    pub prefs: HashMap<String, String>,
}

impl Workspace {
    /// An outline kept only in memory, with nothing to save it to.
    pub fn new(tree: Tree) -> Workspace {
        Workspace {
            tree,
            history: History::new(),
            writer: None,
            imports: HashMap::new(),
            blobs: HashMap::new(),
            image_dir: None,
            prefs: HashMap::new(),
        }
    }

    /// Open the database at `db`, with its image files in `images`, and start its writer.
    /// `on_save_status` hears about write errors (`Some(message)`) and recovery (`None`).
    /// Returns the workspace and the database's meta entries (this device's state).
    pub fn open<F>(db: &Path, images: ImageDir, on_save_status: F) -> Result<(Workspace, Vec<(String, String)>), String>
    where
        F: Fn(Option<String>) + Send + 'static,
    {
        let loaded = store::open(db).and_then(|conn| {
            Ok((store::load(&conn)?, store::load_imports(&conn)?, store::load_blobs(&conn)?, store::load_prefs(&conn)?, conn))
        });
        let ((nodes, meta), imports, blobs, prefs, conn) = loaded.map_err(|e| e.to_string())?;
        collect_images(db);
        let writer = Writer::spawn_with_error_hook(conn, on_save_status);
        let ws = Workspace {
            tree: Tree::from_nodes(nodes),
            history: History::new(),
            writer: Some(writer),
            imports: imports.into_iter().map(|r| (r.root, r)).collect(),
            blobs: blobs.into_iter().map(|b| (b.name.clone(), b)).collect(),
            image_dir: Some(images),
            prefs: prefs.into_iter().collect(),
        };
        Ok((ws, meta))
    }

    /// Send the tree's unsaved changes to the writer.
    pub fn persist(&mut self) {
        let (up, del) = self.tree.take_dirty();
        if let Some(w) = &self.writer {
            w.write(up, del);
        }
    }

    /// Save state of this device only (`store::LOCAL_META`).
    pub fn set_meta(&self, key: &str, value: &str) {
        if let Some(w) = &self.writer {
            w.set_meta(key, value);
        }
    }

    pub fn pref(&self, key: &str) -> Option<&str> {
        self.prefs.get(key).map(String::as_str)
    }

    /// Change a preference that syncs (`store::PREFS`) and save it.
    pub fn set_pref(&mut self, key: &str, value: &str) {
        self.prefs.insert(key.to_string(), value.to_string());
        if let Some(w) = &self.writer {
            w.set_pref(key, value);
        }
    }

    /// Star `id` at the end of the starred list, or unstar it. Stars are saved
    /// but are not undo steps (and undo leaves them as they are).
    pub fn set_starred(&mut self, id: NodeId, starred: bool) {
        let mark = self.tree.journal_mark();
        if starred {
            if !self.tree.is_starred(id) {
                self.tree.star_at(id, usize::MAX);
            }
        } else {
            self.tree.unstar(id);
        }
        self.tree.forget_since(mark);
        self.persist();
    }

    /// Persist, then try to save everything sent so far.
    pub fn flush(&mut self) -> Result<(), String> {
        self.persist();
        self.writer.as_ref().map_or(Ok(()), |writer| writer.flush())
    }

    pub fn set_import_state(&mut self, state: ImportState) {
        for root in &state.roots {
            self.imports.remove(root);
        }
        self.imports.extend(state.records.iter().map(|record| (record.root, record.clone())));
    }

    /// Record a structural command as one undo step and save it, together with the
    /// import state it changes (before, after) and the subtrees it moves to the Trash.
    pub fn commit_with_imports_and_trash(
        &mut self,
        before: Option<Caret>,
        after: Option<Caret>,
        mut imports: Option<(ImportState, ImportState)>,
        trash: Vec<(NodeId, String, Vec<Node>)>,
    ) {
        if let Some((previous, next)) = imports.as_mut() {
            previous.trash_removed = trash.clone();
            next.trash_added = trash.clone();
        }
        self.history.commit_with_imports(&mut self.tree, HistoryKind::Structure, before, after, imports.clone());
        let (upserts, deletes) = self.tree.take_dirty();
        if let Some((_, state)) = imports {
            self.set_import_state(state.clone());
            if let Some(writer) = &self.writer {
                writer.import_batch(upserts, deletes, state);
            }
        } else if let Some(writer) = &self.writer {
            writer.write(upserts, deletes);
            for (root, label, nodes) in trash {
                writer.trash(root, &label, nodes);
            }
        }
    }

    /// Record typing in `id` (merged into the previous typing step when recent) and save it.
    pub fn commit_typing(&mut self, id: NodeId, before: Option<Caret>, after: Option<Caret>) {
        self.history.commit(&mut self.tree, HistoryKind::Typing(id), before, after);
        self.persist();
    }

    /// Undo the last step and save the result. `None` when there is nothing to undo,
    /// otherwise the caret from before that step.
    pub fn undo(&mut self) -> Option<Option<Caret>> {
        let imports = self.history.undo_imports().cloned();
        let caret = self.history.undo(&mut self.tree)?;
        self.save_history_step(imports);
        Some(caret)
    }

    /// Redo the last undone step and save the result. `None` when there is nothing to redo,
    /// otherwise the caret from after that step.
    pub fn redo(&mut self) -> Option<Option<Caret>> {
        let imports = self.history.redo_imports().cloned();
        let caret = self.history.redo(&mut self.tree)?;
        self.save_history_step(imports);
        Some(caret)
    }

    fn save_history_step(&mut self, imports: Option<ImportState>) {
        if let Some(state) = imports {
            self.set_import_state(state.clone());
            let (upserts, deletes) = self.tree.take_dirty();
            if let Some(writer) = &self.writer {
                writer.import_batch(upserts, deletes, state);
            }
        } else {
            self.persist();
        }
    }
}

/// Delete image files no bullet (or trashed bullet) uses any more. Runs once at launch,
/// off the calling thread, before this session adds any.
fn collect_images(db: &Path) {
    let db = db.to_path_buf();
    std::thread::Builder::new()
        .name("flowie-images".into())
        .spawn(move || {
            if let Ok(removed) = store::collect_unused_images(&db, std::time::Duration::from_secs(3600)) {
                if removed.is_empty() {
                    return;
                }
                eprintln!("flowie: removed {} unused image files", removed.len());
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowie_core::ROOT;

    #[test]
    fn commit_saves_and_undo_reverts() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite");
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        let id = ws.tree.create_at(ROOT, 0, "hello");
        ws.commit_with_imports_and_trash(None, None, None, Vec::new());
        ws.flush().unwrap();
        drop(ws);

        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert_eq!(ws.tree.node(id).text, "hello");
        ws.tree.set_text(id, "hello world");
        ws.commit_typing(id, None, None);
        assert_eq!(ws.undo(), Some(None));
        assert_eq!(ws.tree.node(id).text, "hello");
        assert_eq!(ws.redo(), Some(None));
        ws.flush().unwrap();
        drop(ws);

        let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert_eq!(ws.tree.node(id).text, "hello world");
    }

    #[test]
    fn stars_save_without_undo_steps_and_prefs_reload() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite");
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        let a = ws.tree.create_at(ROOT, 0, "a");
        let b = ws.tree.create_at(ROOT, 1, "b");
        ws.commit_with_imports_and_trash(None, None, None, Vec::new());
        ws.set_starred(b, true);
        ws.set_starred(a, true);
        ws.set_starred(b, true); // already starred: stays in place
        ws.set_pref("saved_searches", "is:todo");
        // The only undo step is creating the bullets.
        assert!(ws.undo().is_some());
        assert!(ws.undo().is_none());
        assert!(ws.redo().is_some());
        assert_eq!(ws.tree.starred().collect::<Vec<_>>(), vec![b, a]);
        ws.flush().unwrap();
        drop(ws);

        let (mut ws, meta) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert_eq!(ws.tree.starred().collect::<Vec<_>>(), vec![b, a]);
        assert_eq!(ws.pref("saved_searches"), Some("is:todo"));
        assert!(meta.is_empty());
        ws.set_starred(b, false);
        ws.flush().unwrap();
        drop(ws);

        let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert_eq!(ws.tree.starred().collect::<Vec<_>>(), vec![a]);
    }
}
