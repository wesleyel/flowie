//! Flowie's document layer: an open outline (tree, undo history, database writer) and the
//! path every edit takes to be recorded and saved. No UI code, and no GPUI.

use std::collections::HashMap;
use std::path::Path;

#[cfg(feature = "native")]
pub use driver::{SyncDriver, SyncPhase};
use flowie_core::images::{BlobInfo, ImageDir};
use flowie_core::store::{self, ImportRecord, ImportState, Writer};
use flowie_core::{Caret, History, HistoryKind, NodeId, Tree};
pub use outcome::Outcome;
pub use sync::{LostCopy, SyncState};

pub mod capture;
#[cfg(feature = "native")]
mod driver;
pub mod find;
pub mod keys;
mod outcome;
mod sync;
pub mod utf16;

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
    /// When each preference was last written (for sync).
    prefs_at: HashMap<String, flowie_core::hlc::Hlc>,
    /// What sync has left to send and has seen of the backend.
    pub sync: SyncState,
    /// What the edit being tracked changed so far (`Workspace::tracked`).
    tracker: Option<outcome::Tracker>,
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
            prefs_at: HashMap::new(),
            sync: SyncState::default(),
            tracker: None,
        }
    }

    /// Open the database at `db`, with its image files in `images`, and start its writer.
    /// `on_save_status` hears about write errors (`Some(message)`) and recovery (`None`).
    /// Returns the workspace and the database's meta entries (this device's state). On an
    /// error the database file is left as it was (see `store::open`).
    pub fn open<F>(db: &Path, images: ImageDir, on_save_status: F) -> Result<(Workspace, Vec<(String, String)>), store::OpenError>
    where
        F: Fn(Option<String>) + Send + 'static,
    {
        let conn = store::open(db)?;
        let ((nodes, meta), imports, blobs, prefs, purged) = (
            store::load(&conn)?,
            store::load_imports(&conn)?,
            store::load_blobs(&conn)?,
            store::load_prefs(&conn)?,
            store::load_purged(&conn)?,
        );
        let sync = store::load_sync(&conn)?;
        collect_images(db);
        let writer = Writer::spawn_with_error_hook(conn, on_save_status);
        let mut tree = Tree::from_nodes(nodes);
        tree.set_purged(purged);
        let mut ws = Workspace {
            tree,
            history: History::new(),
            writer: Some(writer),
            imports: imports.into_iter().map(|r| (r.root, r)).collect(),
            blobs: blobs.into_iter().map(|b| (b.name.clone(), b)).collect(),
            image_dir: Some(images),
            prefs: prefs.iter().map(|(k, v, _)| (k.clone(), v.clone())).collect(),
            prefs_at: prefs.into_iter().map(|(k, _, at)| (k, at)).collect(),
            sync: SyncState::load(sync),
            tracker: None,
        };
        let actor = meta.iter().find(|(k, _)| k == "actor.app").and_then(|(_, v)| v.parse::<u64>().ok()).filter(|a| *a != 0);
        let actor = actor.unwrap_or_else(|| {
            let actor = new_actor();
            ws.set_meta("actor.app", &actor.to_string());
            actor
        });
        ws.tree.set_actor(actor);
        ws.sync.load_meta(&meta);
        ws.recover_held(&meta);
        ws.purge_expired();
        Ok((ws, meta))
    }

    /// The tree's unsaved changes, noting what sync has to send.
    fn take_dirty(&mut self) -> flowie_core::tree::Dirty {
        let dirty = self.tree.take_dirty();
        self.sync.note_unsent(dirty.unsent.iter().map(|(id, _)| *id));
        dirty
    }

    /// Send the tree's unsaved changes to the writer.
    pub fn persist(&mut self) {
        let dirty = self.take_dirty();
        if let Some(w) = &self.writer {
            w.write(dirty);
        }
    }

    /// Note an image file stored in `image_dir`, and save what is known about it.
    pub fn record_blob(&mut self, info: BlobInfo) {
        if !self.blobs.contains_key(&info.name) {
            if let Some(w) = &self.writer {
                w.blob(info.clone());
            }
            self.blobs.insert(info.name.clone(), info);
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
        let at = self.tree.tick();
        self.prefs.insert(key.to_string(), value.to_string());
        self.prefs_at.insert(key.to_string(), at);
        if let Some(w) = &self.writer {
            w.set_pref(key, value, at);
        }
        self.note_prefs_unsent();
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
    /// import state it changes (before, after). Moving to and from the Trash is part of the
    /// command's tree changes (`Tree::trash`, `Tree::untrash`).
    pub fn commit_with_imports(&mut self, before: Option<Caret>, after: Option<Caret>, imports: Option<(ImportState, ImportState)>) {
        self.track_journal();
        self.history.commit_with_imports(&mut self.tree, HistoryKind::Structure, before, after, imports.clone());
        let dirty = self.take_dirty();
        if let Some((_, state)) = imports {
            self.set_import_state(state.clone());
            if let Some(writer) = &self.writer {
                writer.import_batch(dirty, state);
            }
        } else if let Some(writer) = &self.writer {
            writer.write(dirty);
        }
    }

    /// Delete the Trash entry `root` for good. Not an undo step: undo and redo forget the
    /// removed nodes too, so they can't bring back part of it.
    pub fn purge(&mut self, root: NodeId) {
        let ids = self.tree.purge(root);
        self.forget_purged(ids);
    }

    /// Delete for good what has been in the Trash longer than `tree::TRASH_DAYS`. Returns
    /// how many nodes went.
    pub fn purge_expired(&mut self) -> usize {
        let ids = self.tree.purge_expired(flowie_core::tree::now_millis());
        let n = ids.len();
        self.forget_purged(ids);
        n
    }

    fn forget_purged(&mut self, ids: Vec<NodeId>) {
        if !ids.is_empty() {
            self.history.forget(&ids);
            self.persist();
            // Deleted rows leave free pages that SQLite keeps; give them back.
            if let Some(writer) = &self.writer {
                writer.reclaim_free_pages();
            }
        }
    }

    /// Record typing in `id` (merged into the previous typing step when recent) and save it.
    pub fn commit_typing(&mut self, id: NodeId, before: Option<Caret>, after: Option<Caret>) {
        self.track_journal();
        self.history.commit(&mut self.tree, HistoryKind::Typing(id), before, after);
        self.persist();
    }

    /// Undo the last step and save the result. `None` when there is nothing to undo,
    /// otherwise the caret from before that step.
    pub fn undo(&mut self) -> Option<Option<Caret>> {
        let imports = self.history.undo_imports().cloned();
        if let Some(tracker) = &mut self.tracker {
            tracker.note(self.history.undo_changes());
        }
        let caret = self.history.undo(&mut self.tree)?;
        self.save_history_step(imports);
        Some(caret)
    }

    /// Redo the last undone step and save the result. `None` when there is nothing to redo,
    /// otherwise the caret from after that step.
    pub fn redo(&mut self) -> Option<Option<Caret>> {
        let imports = self.history.redo_imports().cloned();
        if let Some(tracker) = &mut self.tracker {
            tracker.note(self.history.redo_changes());
        }
        let caret = self.history.redo(&mut self.tree)?;
        self.save_history_step(imports);
        Some(caret)
    }

    fn save_history_step(&mut self, imports: Option<ImportState>) {
        if let Some(state) = imports {
            self.set_import_state(state.clone());
            let dirty = self.take_dirty();
            if let Some(writer) = &self.writer {
                writer.import_batch(dirty, state);
            }
        } else {
            self.persist();
        }
    }
}

/// Delete image files no bullet (in the outline or the Trash) uses any more. Runs once at launch,
/// off the calling thread, before this session adds any.
#[cfg(not(feature = "native"))]
fn collect_images(_db: &Path) {}

#[cfg(feature = "native")]
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

/// A new actor id for this process's writes (per-node-sync.md §3.1): random, never 0, which
/// stands for clocks derived from data saved before Flowie kept them.
fn new_actor() -> u64 {
    let bytes = flowie_core::NodeId::random().to_bytes();
    u64::from_le_bytes(bytes[..8].try_into().expect("16 bytes")).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowie_core::{Node, ROOT};

    #[test]
    fn commit_saves_and_undo_reverts() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite");
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        let id = ws.tree.create_at(ROOT, 0, "hello");
        ws.commit_with_imports(None, None, None);
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
        ws.commit_with_imports(None, None, None);
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
        // Stars and prefs aren't kept in meta: only the database's min_reader, this process's
        // actor id, and that the preference waits to be sent (like the outbox, whether or not
        // sync is set up).
        assert!(meta.iter().all(|(key, _)| matches!(key.as_str(), "actor.app" | "min_reader" | "sync.prefs")), "{meta:?}");
        ws.set_starred(b, false);
        ws.flush().unwrap();
        drop(ws);

        let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert_eq!(ws.tree.starred().collect::<Vec<_>>(), vec![a]);
    }

    /// Text of the reachable outline, one line per node, indented.
    fn outline(tree: &Tree) -> String {
        let mut out = String::new();
        let mut stack: Vec<(NodeId, usize)> = tree.children(ROOT).iter().rev().map(|&c| (c, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            out.push_str(&format!("{}{}\n", "  ".repeat(depth), tree.node(id).text));
            stack.extend(tree.children(id).iter().rev().map(|&c| (c, depth + 1)));
        }
        out
    }

    #[test]
    fn soft_delete_undo_restore_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite");
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        let a = ws.tree.create_at(ROOT, 0, "a");
        let b = ws.tree.create_at(a, 0, "b");
        let c = ws.tree.create_at(b, 0, "c");
        let d = ws.tree.create_at(ROOT, 1, "d");
        ws.commit_with_imports(None, None, None);
        ws.set_starred(c, true);
        let whole = outline(&ws.tree);
        // The records' content; undo and redo are new writes, so their clocks move on.
        let records = |ws: &Workspace| {
            let mut all: Vec<_> = ws.tree.records().map(|n| Node { sync: Default::default(), ..n.clone() }).collect();
            all.sort_by_key(|n| n.id);
            all
        };
        let before = records(&ws);
        let c_before = ws.tree.node(c).clone();

        // Deleting marks only the subtree's root; the nodes below keep their records.
        ws.tree.trash(a);
        ws.commit_with_imports(None, None, None);
        assert_eq!(outline(&ws.tree), "d\n");
        assert!(!ws.tree.contains(b) && !ws.tree.contains(c));
        assert_eq!(ws.tree.node(c), &c_before, "clocks and all");
        assert!(ws.tree.node(a).deleted_at.is_some() && ws.tree.node(b).deleted_at.is_none());
        assert_eq!(ws.tree.trashed(), [a]);
        assert_eq!(ws.tree.trashed_subtree(a), [a, b, c]);
        assert_eq!(ws.tree.starred().count(), 0);
        assert_eq!(ws.tree.len(), 1);

        // Undo and redo are ordinary field changes.
        assert!(ws.undo().is_some());
        assert_eq!(outline(&ws.tree), whole);
        assert_eq!(records(&ws), before);
        assert!(ws.redo().is_some());
        assert_eq!(outline(&ws.tree), "d\n");

        // The Trash survives a restart, and so does what sync keeps.
        let mut saved: Vec<Node> = ws.tree.records().cloned().collect();
        saved.sort_by_key(|n| n.id);
        ws.flush().unwrap();
        drop(ws);
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        let mut loaded: Vec<Node> = ws.tree.records().cloned().collect();
        loaded.sort_by_key(|n| n.id);
        assert_eq!(loaded, saved);
        assert_eq!(outline(&ws.tree), "d\n");
        assert_eq!(ws.tree.trashed(), [a]);
        assert_eq!(ws.tree.records().count(), 4);

        // Restoring clears the mark: everything is back as it was, and saved that way.
        assert!(ws.tree.untrash(a));
        ws.commit_with_imports(None, None, None);
        assert_eq!(outline(&ws.tree), whole);
        assert_eq!(records(&ws), before);
        assert_eq!(ws.tree.starred().collect::<Vec<_>>(), [c]);
        assert!(ws.undo().is_some(), "restoring is an undo step");
        assert!(!ws.tree.contains(a));
        assert!(ws.redo().is_some());
        ws.flush().unwrap();
        drop(ws);
        let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert_eq!(outline(&ws.tree), whole);
        assert_eq!(records(&ws), before);
        assert!(ws.tree.trashed().is_empty());
        let _ = d;
    }

    #[test]
    fn purging_deletes_for_good_and_undo_forgets_it() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite");
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        let a = ws.tree.create_at(ROOT, 0, "a");
        let b = ws.tree.create_at(a, 0, "b");
        let c = ws.tree.create_at(b, 0, "c");
        let keep = ws.tree.create_at(ROOT, 1, "keep");
        ws.commit_with_imports(None, None, None);
        // Delete `c`, then `a` around it: two Trash entries, newest first.
        ws.tree.trash(c);
        ws.commit_with_imports(None, None, None);
        ws.tree.trash(a);
        ws.commit_with_imports(None, None, None);
        // `c` is part of `a`'s entry, but doesn't come back with it.
        assert_eq!(ws.tree.trashed(), [a]);
        assert_eq!(ws.tree.trashed_subtree(a), [a, b]);

        // Deleting `a` for good takes everything below it.
        ws.purge(a);
        assert!(!ws.tree.has_record(a) && !ws.tree.has_record(b) && !ws.tree.has_record(c));
        assert!(ws.tree.trashed().is_empty());
        // Undo can't bring back part of it: the steps that only touched those are gone, and
        // the next undo is creating the bullets (now just `keep`).
        assert!(ws.undo().is_some());
        assert!(!ws.tree.has_record(a) && !ws.tree.has_record(c));
        assert!(!ws.tree.has_record(keep));
        assert!(ws.undo().is_none());
        assert!(ws.redo().is_some());
        ws.flush().unwrap();
        drop(ws);
        let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert_eq!(outline(&ws.tree), "keep\n");
        assert_eq!(ws.tree.records().count(), 1);
    }

    #[test]
    fn restoring_an_entry_lists_the_ones_deleted_inside_it() {
        let mut ws = Workspace::new(Tree::new());
        let a = ws.tree.create_at(ROOT, 0, "a");
        let b = ws.tree.create_at(a, 0, "b");
        ws.commit_with_imports(None, None, None);
        ws.tree.trash(b);
        ws.tree.trash(a);
        ws.commit_with_imports(None, None, None);
        assert_eq!(ws.tree.trashed(), [a]);
        assert!(ws.tree.untrash(a));
        assert_eq!(ws.tree.trashed(), [b]);
        assert!(ws.tree.untrash(b));
        assert_eq!(outline(&ws.tree), "a\n  b\n");
    }

    #[test]
    fn the_trash_empties_itself_after_the_retention_period() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite");
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        let old = ws.tree.create_at(ROOT, 0, "old");
        let below = ws.tree.create_at(old, 0, "below");
        let recent = ws.tree.create_at(ROOT, 1, "recent");
        ws.commit_with_imports(None, None, None);
        ws.tree.trash(old);
        ws.tree.trash(recent);
        // Back-date the deletion, and the nodes with it: a node created after its subtree
        // was deleted would bring that subtree back.
        let deleted_at = flowie_core::tree::now_millis() - flowie_core::tree::TRASH_DAYS * 86_400_000 - 1;
        for id in [old, below] {
            let mut node = ws.tree.node(id).clone();
            node.created = deleted_at - 1;
            if id == old {
                node.deleted_at = Some(deleted_at);
            }
            ws.tree.put(node);
        }
        // Back-dating `below` after `old` settles it again: `old` no longer comes back.
        assert_eq!(ws.tree.trashed(), [recent, old]);
        ws.commit_with_imports(None, None, None);
        ws.flush().unwrap();
        drop(ws);

        // Expired entries go at launch.
        let (mut ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert!(!ws.tree.has_record(old) && !ws.tree.has_record(below));
        assert_eq!(ws.tree.trashed(), [recent]);
        assert_eq!(ws.purge_expired(), 0);
        ws.flush().unwrap();
        drop(ws);
        let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).unwrap();
        assert_eq!(ws.tree.records().count(), 1);
    }
}
