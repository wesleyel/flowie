//! The workspace as a sync store (wiki: Per-Node-Sync §6): what the engine needs to
//! send and merge, kept in memory and saved by the writer in order with the records, so a
//! restart picks up where sync left off.

use std::collections::{BTreeSet, HashMap};

use flowie_core::store::{SyncRows, SyncWrite};
use flowie_core::NodeId;
use flowie_sync::store::{apply_to_tree, local_record};
use flowie_sync::{merge, NodeRecord, PrefsRecord, RecordStore, Reg};

use crate::Workspace;

/// The `meta` key naming a record held while it was being edited, until it is applied.
const HELD_META: &str = "sync.held";
/// The `meta` keys for the preferences: "1" while they wait to be sent, and the backend's
/// version last seen (`{"etag", "record"}`).
const PREFS_META: &str = "sync.prefs";
const PREFS_BASE_META: &str = "sync.prefs_base";

/// Text that lost a concurrent edit to another device's (a conflict copy), kept until it is
/// restored or dismissed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostCopy {
    /// The node it was in (perhaps deleted since).
    pub id: NodeId,
    /// `"text"` or `"note"`.
    pub field: String,
    pub value: String,
    /// When it was noticed (ms).
    pub at: i64,
}

/// Sync's state for one outline.
#[derive(Debug, Default)]
pub struct SyncState {
    /// Records changed here and not saved to the backend yet.
    outbox: BTreeSet<NodeId>,
    /// The backend's version of each record last seen, with its etag.
    bases: HashMap<NodeId, (String, NodeRecord)>,
    /// Where pulling left off, by backend.
    cursors: HashMap<String, String>,
    /// The backend syncing now (`Workspace::set_sync_remote`).
    remote: String,
    /// The record whose text is being edited (`Workspace::begin_edit`).
    editing: Option<NodeId>,
    /// What came in for it meanwhile, merged, to apply when editing ends.
    held: Option<NodeRecord>,
    /// The synced preferences changed here and wait to be sent.
    prefs_unsent: bool,
    /// The backend's version of the preferences last seen, with its etag.
    prefs_base: Option<(String, PrefsRecord)>,
    /// Counts preference changes that came from another device, for the app to reload them.
    prefs_revision: u64,
    /// Conflict copies, oldest first.
    lost: Vec<LostCopy>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedPrefsBase {
    etag: String,
    record: PrefsRecord,
}

impl SyncState {
    pub(crate) fn load(rows: SyncRows) -> Self {
        SyncState {
            outbox: rows.outbox.into_iter().collect(),
            bases: rows.bases.into_iter().filter_map(|(id, etag, json)| Some((id, (etag, serde_json::from_str(&json).ok()?)))).collect(),
            cursors: rows.cursors.into_iter().collect(),
            lost: rows.lost.into_iter().map(|(id, field, value, at)| LostCopy { id, field, value, at }).collect(),
            ..SyncState::default()
        }
    }

    /// Records the tree changed that sync has to send.
    pub(crate) fn note_unsent(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.outbox.extend(ids);
    }

    pub fn has_unsent(&self) -> bool {
        !self.outbox.is_empty() || self.prefs_unsent
    }

    /// What `meta` keeps of sync's state (the preferences').
    pub(crate) fn load_meta(&mut self, meta: &[(String, String)]) {
        let get = |key: &str| meta.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        self.prefs_unsent = get(PREFS_META) == Some("1");
        self.prefs_base =
            get(PREFS_BASE_META).and_then(|v| serde_json::from_str::<SavedPrefsBase>(v).ok()).map(|saved| (saved.etag, saved.record));
    }

    /// Conflict copies, oldest first.
    pub fn lost(&self) -> &[LostCopy] {
        &self.lost
    }

    /// Changes when preferences come in from another device.
    pub fn prefs_revision(&self) -> u64 {
        self.prefs_revision
    }
}

impl Workspace {
    /// Sync with the backend called `remote` from now on: pulling continues where it last
    /// left off on that backend.
    pub fn set_sync_remote(&mut self, remote: &str) {
        self.sync.remote = remote.to_string();
    }

    /// The record `id` (a mirror's source, not the mirror) is being edited. What another
    /// device changed in it waits until [`Workspace::end_edit`], so the text under the caret,
    /// and an input method's unfinished text, never change while someone types. Ends any
    /// edit already going on; returns what that applied, like `end_edit`.
    pub fn begin_edit(&mut self, id: NodeId) -> Vec<NodeId> {
        let applied = if self.sync.editing == Some(id) { Vec::new() } else { self.end_edit() };
        self.sync.editing = Some(id);
        applied
    }

    /// Editing ended: apply what came in for the record meanwhile, merged with the edits
    /// made here. Returns the ids whose record changed.
    pub fn end_edit(&mut self) -> Vec<NodeId> {
        let Some(id) = self.sync.editing.take() else { return Vec::new() };
        let Some(held) = self.sync.held.take().filter(|r| r.id == id) else { return Vec::new() };
        let merged = match local_record(&self.tree, id) {
            Some(local) => merge(&local, &held),
            None => held,
        };
        let changed = apply_to_tree(&mut self.tree, vec![merged]);
        self.persist();
        self.set_meta(HELD_META, "");
        changed
    }

    /// Delete the subtree at `root` for good without telling the backend: records that never
    /// left this device and shouldn't (a fresh device's untouched tutorial, when it joins an
    /// outline that already has one). Not an undo step.
    pub fn discard_unsent(&mut self, root: NodeId) {
        self.tree.trash(root);
        let ids = self.tree.purge(root);
        self.history.forget(&ids);
        self.persist();
        for id in ids {
            if self.sync.outbox.remove(&id) {
                self.save_sync(SyncWrite::Sent(id));
            }
        }
    }

    /// After a restart: a record held while it was being edited (see `begin_edit`) was never
    /// applied, though its version was noted as seen. Take it from that version.
    pub(crate) fn recover_held(&mut self, meta: &[(String, String)]) {
        let Some(id) = meta.iter().find(|(k, _)| k == HELD_META).and_then(|(_, v)| v.parse::<NodeId>().ok()) else { return };
        if let Some((_, base)) = self.sync.bases.get(&id).cloned() {
            let merged = match local_record(&self.tree, id) {
                Some(local) => merge(&local, &base),
                None => base,
            };
            apply_to_tree(&mut self.tree, vec![merged]);
            self.persist();
        }
        self.set_meta(HELD_META, "");
    }

    /// Forget the conflict copy at `index` (restored by the caller, or dismissed).
    pub fn dismiss_lost(&mut self, index: usize) -> Option<LostCopy> {
        if index >= self.sync.lost.len() {
            return None;
        }
        let copy = self.sync.lost.remove(index);
        self.save_sync(SyncWrite::Unlost { id: copy.id, field: copy.field.clone(), value: copy.value.clone(), at: copy.at });
        Some(copy)
    }

    pub(crate) fn note_prefs_unsent(&mut self) {
        if !self.sync.prefs_unsent {
            self.sync.prefs_unsent = true;
            self.set_meta(PREFS_META, "1");
        }
    }

    fn save_sync(&self, write: SyncWrite) {
        if let Some(writer) = &self.writer {
            writer.sync(write);
        }
    }
}

impl RecordStore for Workspace {
    fn unsent(&self, limit: usize) -> Vec<NodeId> {
        self.sync.outbox.iter().copied().take(limit).collect()
    }

    fn local(&self, id: NodeId) -> Option<NodeRecord> {
        local_record(&self.tree, id)
    }

    fn apply(&mut self, mut records: Vec<NodeRecord>) {
        if let Some(editing) = self.sync.editing {
            if let Some(ix) = records.iter().position(|r| r.id == editing) {
                let record = records.swap_remove(ix);
                self.sync.held = Some(match self.sync.held.take() {
                    Some(held) => merge(&held, &record),
                    None => {
                        self.set_meta(HELD_META, &editing.to_string());
                        record
                    }
                });
            }
        }
        apply_to_tree(&mut self.tree, records);
        self.persist();
    }

    fn clear_unsent(&mut self, id: NodeId) {
        if self.sync.outbox.remove(&id) {
            self.save_sync(SyncWrite::Sent(id));
        }
    }

    fn base(&self, id: NodeId) -> Option<(String, NodeRecord)> {
        self.sync.bases.get(&id).cloned()
    }

    fn set_base(&mut self, id: NodeId, etag: String, record: NodeRecord) {
        let json = serde_json::to_string(&record).expect("a record is plain data");
        self.save_sync(SyncWrite::Base(id, etag.clone(), json));
        self.sync.bases.insert(id, (etag, record));
    }

    fn cursor(&self) -> Option<String> {
        self.sync.cursors.get(&self.sync.remote).cloned()
    }

    fn prefs_unsent(&self) -> bool {
        self.sync.prefs_unsent
    }

    fn local_prefs(&self) -> PrefsRecord {
        let values = flowie_core::store::PREFS
            .iter()
            .filter_map(|&key| {
                Some((key.to_string(), Reg::new(self.prefs.get(key)?.clone(), self.prefs_at.get(key).copied().unwrap_or_default())))
            })
            .collect();
        PrefsRecord { values }
    }

    fn apply_prefs(&mut self, merged: PrefsRecord) {
        for (key, reg) in merged.values {
            let known = flowie_core::store::PREFS.contains(&key.as_str());
            let newer = self.prefs_at.get(&key).is_none_or(|at| reg.at > *at);
            if !known || !newer || self.prefs.get(&key) == Some(&reg.v) && self.prefs_at.get(&key) == Some(&reg.at) {
                continue;
            }
            self.tree.observe(reg.at);
            if let Some(w) = &self.writer {
                w.set_pref(&key, &reg.v, reg.at);
            }
            self.prefs.insert(key.clone(), reg.v);
            self.prefs_at.insert(key, reg.at);
            self.sync.prefs_revision += 1;
        }
    }

    fn clear_prefs_unsent(&mut self) {
        if self.sync.prefs_unsent {
            self.sync.prefs_unsent = false;
            self.set_meta(PREFS_META, "");
        }
    }

    fn prefs_base(&self) -> Option<(String, PrefsRecord)> {
        self.sync.prefs_base.clone()
    }

    fn set_prefs_base(&mut self, etag: String, record: PrefsRecord) {
        let saved = SavedPrefsBase { etag, record };
        self.set_meta(PREFS_BASE_META, &serde_json::to_string(&saved).expect("plain data"));
        self.sync.prefs_base = Some((saved.etag, saved.record));
    }

    fn keep_lost(&mut self, id: NodeId, field: &str, value: String) {
        let at = flowie_core::tree::now_millis();
        self.save_sync(SyncWrite::Lost { id, field: field.to_string(), value: value.clone(), at });
        self.sync.lost.push(LostCopy { id, field: field.to_string(), value, at });
    }

    fn set_cursor(&mut self, cursor: String) {
        self.save_sync(SyncWrite::Cursor { remote: self.sync.remote.clone(), cursor: cursor.clone() });
        self.sync.cursors.insert(self.sync.remote.clone(), cursor);
    }
}

#[cfg(test)]
mod tests {
    use flowie_core::images::ImageDir;
    use flowie_core::{Tree, ROOT};
    use flowie_sync::{Engine, MemoryServer, RecordStore};

    use crate::Workspace;

    fn open(db: &std::path::Path) -> Workspace {
        let (mut ws, _) = Workspace::open(db, ImageDir::beside(db), |_| {}).unwrap();
        ws.set_sync_remote("memory");
        ws
    }

    /// Push everything, then pull everything.
    fn sync(ws: &mut Workspace, engine: &mut Engine, server: &mut MemoryServer) {
        ws.persist();
        loop {
            let saves = engine.next_saves(ws, 50);
            if saves.is_empty() {
                break;
            }
            let results = server.modify(saves);
            engine.on_saved(ws, results);
        }
        loop {
            let batch = server.changes(ws.cursor().as_deref(), 50);
            let more = batch.more;
            engine.on_fetched(ws, batch);
            if !more {
                break;
            }
        }
    }

    #[test]
    fn the_row_being_edited_takes_what_came_in_when_editing_ends() {
        let mut server = MemoryServer::new();
        let (mut a, mut b) = (Workspace::new(Tree::new()), Workspace::new(Tree::new()));
        a.tree.set_actor(1);
        b.tree.set_actor(2);
        let (mut ea, mut eb) = (Engine::new(), Engine::new());
        let x = a.tree.create_at(ROOT, 0, "x");
        let other = a.tree.create_at(ROOT, 1, "other");
        a.commit_with_imports(None, None, None);
        sync(&mut a, &mut ea, &mut server);
        sync(&mut b, &mut eb, &mut server);

        // b types in x while a changes x's note and another row.
        assert!(b.begin_edit(x).is_empty());
        b.tree.set_text(x, "x, typed on b");
        a.tree.set_note(x, "a's note");
        a.tree.set_text(other, "other, from a");
        a.commit_with_imports(None, None, None);
        sync(&mut a, &mut ea, &mut server);
        sync(&mut b, &mut eb, &mut server);
        assert_eq!(b.tree.node(other).text, "other, from a", "other rows change at once");
        assert_eq!(b.tree.node(x).note, "", "the row being edited waits");
        assert_eq!(b.tree.node(x).text, "x, typed on b");

        // Editing another row ends this edit: both changes are there.
        assert_eq!(b.begin_edit(other), [x]);
        assert_eq!((b.tree.node(x).text.as_str(), b.tree.node(x).note.as_str()), ("x, typed on b", "a's note"));
        assert!(b.end_edit().is_empty());
        b.commit_typing(x, None, None);
        sync(&mut b, &mut eb, &mut server);
        sync(&mut a, &mut ea, &mut server);
        assert_eq!((a.tree.node(x).text.as_str(), a.tree.node(x).note.as_str()), ("x, typed on b", "a's note"));
    }

    #[test]
    fn a_record_held_for_editing_is_applied_after_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let (db_a, db_b) = (dir.path().join("a.sqlite"), dir.path().join("b.sqlite"));
        let mut server = MemoryServer::new();
        let (mut a, mut b) = (open(&db_a), open(&db_b));
        let x = a.tree.create_at(ROOT, 0, "x");
        a.commit_with_imports(None, None, None);
        sync(&mut a, &mut Engine::new(), &mut server);
        sync(&mut b, &mut Engine::new(), &mut server);

        b.begin_edit(x);
        a.tree.set_note(x, "a's note");
        a.commit_with_imports(None, None, None);
        sync(&mut a, &mut Engine::new(), &mut server);
        sync(&mut b, &mut Engine::new(), &mut server);
        assert_eq!(b.tree.node(x).note, "");
        // Quit without ending the edit, as a crash would.
        b.flush().unwrap();
        drop(b);
        let b = open(&db_b);
        assert_eq!(b.tree.node(x).note, "a's note");
    }

    #[test]
    fn discarded_records_are_gone_without_a_word_to_the_backend() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("a.sqlite");
        let mut server = MemoryServer::new();
        let mut a = open(&db);
        let tutorial = a.tree.create_at(ROOT, 0, "tutorial");
        a.tree.create_at(tutorial, 0, "lesson");
        let kept = a.tree.create_at(ROOT, 1, "kept");
        a.commit_with_imports(None, None, None);
        a.discard_unsent(tutorial);
        assert!(!a.tree.has_record(tutorial) && a.tree.trashed().is_empty());
        sync(&mut a, &mut Engine::new(), &mut server);
        let sent = server.changes(None, 50).records;
        assert_eq!(sent.len(), 1, "only {kept} goes out, not the discarded records or their tombstones");
        a.flush().unwrap();
        drop(a);
        let a = open(&db);
        assert!(!a.tree.has_record(tutorial) && !a.sync.has_unsent());
    }

    #[test]
    fn preferences_sync_and_the_later_write_wins() {
        let dir = tempfile::tempdir().unwrap();
        let (db_a, db_b) = (dir.path().join("a.sqlite"), dir.path().join("b.sqlite"));
        let mut server = MemoryServer::new();
        let (mut a, mut b) = (open(&db_a), open(&db_b));
        a.set_pref("saved_searches", "is:todo");
        // Not sent yet when a quits: it goes after the restart.
        a.flush().unwrap();
        drop(a);
        let mut a = open(&db_a);
        assert!(a.sync.has_unsent());
        sync(&mut a, &mut Engine::new(), &mut server);
        assert!(!a.sync.has_unsent());
        let seen = b.sync.prefs_revision();
        sync(&mut b, &mut Engine::new(), &mut server);
        assert_eq!(b.pref("saved_searches"), Some("is:todo"));
        assert_ne!(b.sync.prefs_revision(), seen, "the app hears that they changed");

        // Both change them; b's change is the later one and wins on both.
        a.set_pref("saved_searches", "from a");
        // A later millisecond, so the clocks can't tie and leave it to the actor ids.
        std::thread::sleep(std::time::Duration::from_millis(3));
        b.set_pref("saved_searches", "from b");
        sync(&mut a, &mut Engine::new(), &mut server);
        sync(&mut b, &mut Engine::new(), &mut server);
        sync(&mut a, &mut Engine::new(), &mut server);
        assert_eq!(a.pref("saved_searches"), Some("from b"));
        assert_eq!(b.pref("saved_searches"), Some("from b"));
        assert!(!a.sync.has_unsent() && !b.sync.has_unsent());

        // What came in is saved, with its clock: a restart keeps it, and nothing goes back.
        a.flush().unwrap();
        drop(a);
        let a = open(&db_a);
        assert_eq!(a.pref("saved_searches"), Some("from b"));
        assert!(!a.sync.has_unsent());
    }

    #[test]
    fn conflict_copies_are_kept_until_dismissed() {
        let dir = tempfile::tempdir().unwrap();
        let (db_a, db_b) = (dir.path().join("a.sqlite"), dir.path().join("b.sqlite"));
        let mut server = MemoryServer::new();
        let (mut a, mut b) = (open(&db_a), open(&db_b));
        let x = a.tree.create_at(ROOT, 0, "x");
        a.commit_with_imports(None, None, None);
        sync(&mut a, &mut Engine::new(), &mut server);
        sync(&mut b, &mut Engine::new(), &mut server);
        a.tree.set_text(x, "a's text");
        a.commit_typing(x, None, None);
        std::thread::sleep(std::time::Duration::from_millis(3));
        b.tree.set_text(x, "b's text");
        b.commit_typing(x, None, None);
        sync(&mut a, &mut Engine::new(), &mut server);
        sync(&mut b, &mut Engine::new(), &mut server);
        let lost: Vec<_> = b.sync.lost().iter().map(|c| (c.id, c.field.as_str(), c.value.as_str())).collect();
        assert_eq!(lost, [(x, "text", "a's text")]);

        b.flush().unwrap();
        drop(b);
        let mut b = open(&db_b);
        assert_eq!(b.sync.lost().len(), 1, "kept across a restart");
        assert_eq!(b.dismiss_lost(0).map(|c| c.value), Some("a's text".to_string()));
        assert!(b.dismiss_lost(0).is_none());
        b.flush().unwrap();
        drop(b);
        assert!(open(&db_b).sync.lost().is_empty());
    }

    #[test]
    fn two_outlines_sync_and_pick_up_where_they_left_off_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (db_a, db_b) = (dir.path().join("a.sqlite"), dir.path().join("b.sqlite"));
        let mut server = MemoryServer::new();
        let (mut a, mut b) = (open(&db_a), open(&db_b));
        let (mut ea, mut eb) = (Engine::new(), Engine::new());

        let x = a.tree.create_at(ROOT, 0, "from a");
        a.commit_with_imports(None, None, None);
        sync(&mut a, &mut ea, &mut server);
        assert!(!a.sync.has_unsent());
        sync(&mut b, &mut eb, &mut server);
        assert_eq!(b.tree.node(x).text, "from a");
        assert!(!b.history.can_undo(), "what sync brings in is no undo step");

        // An edit made just before quitting is still sent after the restart.
        a.tree.set_text(x, "edited before quitting");
        a.commit_with_imports(None, None, None);
        a.flush().unwrap();
        drop(a);
        let mut a = open(&db_a);
        assert!(a.sync.has_unsent());
        sync(&mut a, &mut Engine::new(), &mut server);
        assert!(!a.sync.has_unsent());

        // b, restarted too, pulls on from where it was: only the new version comes.
        b.flush().unwrap();
        drop(b);
        let mut b = open(&db_b);
        let since = b.cursor();
        assert!(since.is_some());
        assert_eq!(server.changes(since.as_deref(), 50).records.len(), 1);
        sync(&mut b, &mut Engine::new(), &mut server);
        assert_eq!(b.tree.node(x).text, "edited before quitting");
        assert!(b.base(x).is_some(), "the backend's version is remembered");

        // What came in is saved: it is there after another restart.
        b.flush().unwrap();
        drop(b);
        let b = open(&db_b);
        assert_eq!(b.tree.node(x).text, "edited before quitting");
        assert!(!b.sync.has_unsent(), "what came in isn't sent back");
    }
}
