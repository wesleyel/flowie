//! Flowie for native apps, through UniFFI (wiki: iOS-Cross-Platform §3.3).
//!
//! The iOS app (M3) and the CloudKit bridge (P4) call this: an open outline, the rows of a
//! page, the basic edits, and sync with a self-hosted server. Swift keeps no data of its own
//! and merges nothing: every read and write goes through `flowie-doc`, the same layer the
//! desktop app uses. Ids cross the boundary as 32 hex digits, like on the wire.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use flowie_core::images::ImageDir;
use flowie_core::{flatten_mapped, BlockKind, NodeId, Tree, ViewOptions, ROOT};
use flowie_doc::{SyncDriver, SyncPhase, Workspace};
use flowie_sync::http::HttpBackend;
use flowie_sync::{ChangeBatch, Engine, Envelope, RecordStore, SaveResult};

uniffi::setup_scaffolding!();

/// What can go wrong across the boundary.
#[derive(Debug, uniffi::Error)]
pub enum FlowieError {
    /// The database couldn't be opened or saved.
    Storage { message: String },
    /// No bullet has this id.
    NoSuchBullet { id: String },
}

impl std::error::Error for FlowieError {}

impl std::fmt::Display for FlowieError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlowieError::Storage { message } => write!(f, "{message}"),
            FlowieError::NoSuchBullet { id } => write!(f, "no bullet {id}"),
        }
    }
}

/// One row of a page, as the outline shows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RowView {
    /// The row's id: the bullet's, or a stable virtual id for a row inside a mirror.
    pub id: String,
    /// 0 for the page's own children.
    pub depth: u16,
    /// Raw markup, as stored.
    pub text: String,
    pub note: String,
    /// `bullet`, `paragraph`, `h1`, `h2`, `h3`, `todo`, `numbered`, `quote`, `code` or `image`.
    pub kind: String,
    pub completed: bool,
    pub collapsed: bool,
    pub has_children: bool,
}

/// Where the caret goes after a key: a row, and a byte offset in its text.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CaretView {
    pub id: String,
    pub offset: u32,
}

impl From<flowie_core::Caret> for CaretView {
    fn from(caret: flowie_core::Caret) -> Self {
        CaretView { id: caret.node.to_string(), offset: caret.offset as u32 }
    }
}

/// Where sync stands.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SyncStatus {
    Off,
    Syncing,
    Synced,
    Offline { error: String },
}

/// Told from a background thread when sync has something for [`Outline::pump_sync`]. The app
/// schedules that on the thread it uses the outline from.
#[uniffi::export(with_foreign)]
pub trait SyncObserver: Send + Sync {
    fn wake(&self);
}

/// A record as a native sync engine moves it (CloudKit: one CKRecord). `etag` is opaque to
/// Rust: for CloudKit, the record's archived system fields, which make the next save
/// conditional on that version.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WireRecord {
    pub id: String,
    pub etag: Option<String>,
    /// The envelope as JSON (per-node-sync.md §4.1); stored in one encrypted field.
    pub envelope: String,
}

/// How the native engine answered one save.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SaveOutcome {
    Saved {
        id: String,
        etag: String,
    },
    /// The backend has a newer version (CloudKit: serverRecordChanged): here it is.
    Conflict {
        server: WireRecord,
    },
    /// Not saved for another reason; it is sent again later.
    Failed {
        id: String,
        error: String,
    },
}

fn to_envelope(record: &WireRecord) -> Option<Envelope> {
    let mut envelope: Envelope = serde_json::from_str(&record.envelope).ok()?;
    envelope.etag = record.etag.clone();
    Some(envelope)
}

fn to_wire(mut envelope: Envelope) -> WireRecord {
    let etag = envelope.etag.take();
    WireRecord { id: envelope.id.clone(), etag, envelope: serde_json::to_string(&envelope).expect("an envelope is plain data") }
}

struct State {
    ws: Workspace,
    sync: Option<SyncDriver>,
    /// The engine a native sync engine drives (CloudKit), between `begin_native_sync` and
    /// `end_native_sync`.
    native: Option<Engine>,
}

/// An open outline.
#[derive(uniffi::Object)]
pub struct Outline {
    state: Mutex<State>,
}

fn parse(id: &str) -> Result<NodeId, FlowieError> {
    id.parse().map_err(|_| FlowieError::NoSuchBullet { id: id.to_string() })
}

fn kind_name(kind: BlockKind) -> String {
    format!("{kind:?}").to_lowercase()
}

#[uniffi::export]
impl Outline {
    /// Open (or create) the outline database at `db`, its images beside it.
    #[uniffi::constructor]
    pub fn open(db: String) -> Result<Arc<Self>, FlowieError> {
        let db = PathBuf::from(db);
        let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {}).map_err(|e| FlowieError::Storage { message: e.to_string() })?;
        Ok(Arc::new(Outline { state: Mutex::new(State { ws, sync: None, native: None }) }))
    }

    /// An outline kept only in memory (previews and tests).
    #[uniffi::constructor]
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Outline { state: Mutex::new(State { ws: Workspace::new(Tree::new()), sync: None, native: None }) })
    }

    /// The rows of the page at `root` (none: Home), expanded bullets' children included.
    pub fn page(&self, root: Option<String>) -> Result<Vec<RowView>, FlowieError> {
        let state = self.lock();
        let tree = &state.ws.tree;
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        if root != ROOT && !tree.contains(root) {
            return Err(FlowieError::NoSuchBullet { id: root.to_string() });
        }
        let (rows, _) = flatten_mapped(tree, root, &ViewOptions::default());
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let node = tree.get(row.id)?;
                let source = tree.node(tree.source(row.id));
                Some(RowView {
                    id: row.id.to_string(),
                    depth: row.depth,
                    text: source.text.clone(),
                    note: source.note.clone(),
                    kind: kind_name(node.kind),
                    completed: source.completed,
                    collapsed: node.collapsed,
                    has_children: tree.has_children(row.id),
                })
            })
            .collect())
    }

    /// Add a bullet with `text` under `parent` (none: Home) at `index`; returns its id. An
    /// undo step.
    pub fn create(&self, parent: Option<String>, index: u32, text: String) -> Result<String, FlowieError> {
        let mut state = self.lock();
        let parent = parent.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        if parent != ROOT && !state.ws.tree.contains(parent) {
            return Err(FlowieError::NoSuchBullet { id: parent.to_string() });
        }
        let id = state.ws.tree.create_at(parent, index as usize, &text.replace('\n', " "));
        state.ws.commit_with_imports(None, None, None);
        Ok(id.to_string())
    }

    /// Replace a bullet's text. Typing in one bullet merges into one undo step.
    pub fn set_text(&self, id: String, text: String) -> Result<(), FlowieError> {
        self.edit(&id, |ws, id| {
            ws.tree.set_text(id, &text.replace('\n', " "));
            ws.commit_typing(id, None, None);
        })
    }

    pub fn set_note(&self, id: String, note: String) -> Result<(), FlowieError> {
        self.edit(&id, |ws, id| {
            ws.tree.set_note(id, &note);
            ws.commit_typing(id, None, None);
        })
    }

    pub fn set_completed(&self, id: String, completed: bool) -> Result<(), FlowieError> {
        self.edit(&id, |ws, id| {
            ws.tree.set_completed(id, completed);
            ws.commit_with_imports(None, None, None);
        })
    }

    /// Make a bullet the last child of the one above it; false if there is none. An undo step.
    pub fn indent(&self, id: String) -> Result<bool, FlowieError> {
        let mut done = false;
        self.edit_row(&id, |ws, id| {
            done = ws.tree.indent(id);
            if done {
                ws.commit_with_imports(None, None, None);
            }
        })?;
        Ok(done)
    }

    /// Move a bullet out to follow its parent, but not out of the page at `root` (none: Home);
    /// false if it can't go further. An undo step.
    pub fn outdent(&self, id: String, root: Option<String>) -> Result<bool, FlowieError> {
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        let mut done = false;
        self.edit_row(&id, |ws, id| {
            done = ws.tree.outdent(id, root);
            if done {
                ws.commit_with_imports(None, None, None);
            }
        })?;
        Ok(done)
    }

    /// Return in a row's text with the bytes `start..end` selected (a caret when equal), on the
    /// page at `root` (none: Home), as on the desktop (`flowie_doc::keys::enter`): the text after
    /// the caret moves to a new bullet below. Returns where the caret goes. An undo step.
    pub fn enter(&self, id: String, start: u32, end: u32, root: Option<String>) -> Result<CaretView, FlowieError> {
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        let mut caret = None;
        self.edit_row(&id, |ws, id| caret = Some(ws.enter(id, start as usize..end.max(start) as usize, root)))?;
        Ok(caret.expect("edit_row ran").into())
    }

    /// Backspace with the caret at the start of a row's text, on the page at `root` (none:
    /// Home), as on the desktop (`flowie_doc::keys::backspace_at_start`): the row joins the one
    /// above, or goes if it is empty. Returns where the caret goes, or none when nothing
    /// changed. An undo step.
    pub fn backspace_at_start(&self, id: String, root: Option<String>) -> Result<Option<CaretView>, FlowieError> {
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        let mut caret = None;
        self.edit_row(&id, |ws, id| {
            let (rows, _) = flatten_mapped(&ws.tree, root, &ViewOptions::default());
            let prev = rows.iter().position(|r| r.id == id).and_then(|i| i.checked_sub(1)).map(|i| rows[i].id);
            caret = ws.backspace_at_start(id, prev, root);
        })?;
        Ok(caret.map(Into::into))
    }

    /// Move a bullet and what is under it to the Trash. An undo step.
    pub fn delete(&self, id: String) -> Result<(), FlowieError> {
        self.edit(&id, |ws, id| {
            ws.tree.trash(id);
            ws.commit_with_imports(None, None, None);
        })
    }

    /// Undo the last step; false if there was none.
    pub fn undo(&self) -> bool {
        self.lock().ws.undo().is_some()
    }

    pub fn redo(&self) -> bool {
        self.lock().ws.redo().is_some()
    }

    /// The bullet whose text is being typed in (a mirror row's source is found from it): what
    /// another device changes in it waits until editing ends. None ends editing.
    pub fn set_editing(&self, id: Option<String>) -> Result<(), FlowieError> {
        let mut state = self.lock();
        match id {
            Some(id) => {
                let id = parse(&id)?;
                let source = state.ws.tree.source(id);
                state.ws.begin_edit(source);
            }
            None => {
                state.ws.end_edit();
            }
        }
        Ok(())
    }

    /// Wait until everything so far is saved.
    pub fn flush(&self) -> Result<(), FlowieError> {
        self.lock().ws.flush().map_err(|message| FlowieError::Storage { message })
    }

    /// Sync with a self-hosted server (`flowie-server`) from now on, replacing any sync
    /// running. `observer` hears when to call [`pump_sync`](Self::pump_sync).
    pub fn start_sync(&self, server: String, token: String, observer: Arc<dyn SyncObserver>) {
        let mut state = self.lock();
        state.sync = None;
        let server = server.trim().trim_end_matches('/').to_string();
        state.ws.set_sync_remote(&format!("{server}#outline"));
        let token = token.trim().to_string();
        let driver = SyncDriver::start(&state.ws, move || Box::new(HttpBackend::new(&server, "outline", &token)), move || observer.wake());
        state.sync = Some(driver);
        let State { ws, sync, .. } = &mut *state;
        if let Some(driver) = sync {
            driver.pump(ws);
        }
    }

    pub fn stop_sync(&self) {
        self.lock().sync = None;
    }

    /// Send what changed here and take what came in. Returns whether the outline changed, so
    /// the app re-reads its page.
    pub fn pump_sync(&self) -> bool {
        let mut state = self.lock();
        let State { ws, sync, .. } = &mut *state;
        sync.as_mut().is_some_and(|driver| driver.pump(ws))
    }

    /// Let a native sync engine (CloudKit's CKSyncEngine) drive sync from now on, keeping its
    /// place under `remote` (for example `icloud`). It schedules the calls below itself.
    pub fn begin_native_sync(&self, remote: String) {
        let mut state = self.lock();
        state.sync = None;
        state.ws.set_sync_remote(&remote);
        state.native = Some(Engine::new());
    }

    pub fn end_native_sync(&self) {
        self.lock().native = None;
    }

    /// Ids with something to send, for the native engine's list of pending saves.
    pub fn unsent_ids(&self, limit: u32) -> Vec<String> {
        let state = self.lock();
        let mut ids: Vec<String> = state.ws.unsent(limit as usize).into_iter().map(|id| id.to_string()).collect();
        if state.ws.prefs_unsent() && ids.len() < limit as usize {
            ids.push(flowie_sync::PREFS_ID.to_string());
        }
        ids
    }

    /// The next records to save, at most `limit`, each from the version last seen.
    pub fn next_saves(&self, limit: u32) -> Vec<WireRecord> {
        let mut state = self.lock();
        let State { ws, native, .. } = &mut *state;
        let Some(engine) = native else { return Vec::new() };
        engine.next_saves(ws, limit as usize).into_iter().map(to_wire).collect()
    }

    /// The native engine answered saves.
    pub fn on_saved(&self, outcomes: Vec<SaveOutcome>) {
        let mut state = self.lock();
        let State { ws, native, .. } = &mut *state;
        let Some(engine) = native else { return };
        let results = outcomes
            .into_iter()
            .filter_map(|outcome| match outcome {
                SaveOutcome::Saved { id, etag } => Some(SaveResult::Saved { id, etag }),
                SaveOutcome::Conflict { server } => to_envelope(&server).map(|server| SaveResult::Conflict { server }),
                SaveOutcome::Failed { id, error } => Some(SaveResult::Failed { id, error }),
            })
            .collect();
        engine.on_saved(ws, results);
    }

    /// Records the native engine fetched, and where it is now (CloudKit keeps its own change
    /// token: pass anything that marks progress, or an empty string). Returns whether the
    /// outline changed.
    pub fn on_fetched(&self, records: Vec<WireRecord>, cursor: String) -> bool {
        let mut state = self.lock();
        let State { ws, native, .. } = &mut *state;
        let Some(engine) = native else { return false };
        let revision = ws.tree.revision();
        let records = records.iter().filter_map(to_envelope).collect();
        engine.on_fetched(ws, ChangeBatch { records, cursor, more: false });
        ws.tree.revision() != revision
    }

    pub fn sync_status(&self) -> SyncStatus {
        let state = self.lock();
        match &state.sync {
            None => SyncStatus::Off,
            Some(driver) => match driver.phase(&state.ws) {
                SyncPhase::Syncing => SyncStatus::Syncing,
                SyncPhase::Synced => SyncStatus::Synced,
                SyncPhase::Offline(error) => SyncStatus::Offline { error },
            },
        }
    }
}

impl Outline {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Change a bullet's content: a mirror's row changes its source.
    fn edit(&self, id: &str, f: impl FnOnce(&mut Workspace, NodeId)) -> Result<(), FlowieError> {
        self.edit_row(id, |ws, id| {
            let id = ws.tree.source(id);
            f(ws, id)
        })
    }

    /// Change a bullet's place: the row itself moves, a mirror included.
    fn edit_row(&self, id: &str, f: impl FnOnce(&mut Workspace, NodeId)) -> Result<(), FlowieError> {
        let mut state = self.lock();
        let id = parse(id)?;
        if !state.ws.tree.contains(id) {
            return Err(FlowieError::NoSuchBullet { id: id.to_string() });
        }
        f(&mut state.ws, id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_show_on_the_page_and_undo() {
        let outline = Outline::in_memory();
        let a = outline.create(None, 0, "first".into()).unwrap();
        let b = outline.create(Some(a.clone()), 0, "child".into()).unwrap();
        outline.set_completed(b.clone(), true).unwrap();
        let page = outline.page(None).unwrap();
        assert_eq!(
            page.iter().map(|r| (r.text.as_str(), r.depth, r.completed)).collect::<Vec<_>>(),
            [("first", 0, false), ("child", 1, true)]
        );
        assert!(page[0].has_children);
        assert_eq!(outline.page(Some(a.clone())).unwrap().len(), 1);
        outline.delete(b.clone()).unwrap();
        assert_eq!(outline.page(None).unwrap().len(), 1);
        assert!(outline.undo());
        assert_eq!(outline.page(None).unwrap().len(), 2);
        assert!(matches!(outline.set_text("nope".into(), "x".into()), Err(FlowieError::NoSuchBullet { .. })));

        let c = outline.create(None, 1, "second".into()).unwrap();
        assert!(outline.indent(c.clone()).unwrap());
        assert_eq!(
            outline.page(None).unwrap().iter().map(|r| (r.text.as_str(), r.depth)).collect::<Vec<_>>(),
            [("first", 0), ("child", 1), ("second", 1)]
        );
        assert!(!outline.outdent(c.clone(), Some(a.clone())).unwrap(), "not out of the page shown");
        assert!(outline.outdent(c.clone(), None).unwrap());
        assert_eq!(outline.page(None).unwrap()[2].depth, 0);
        assert!(!outline.outdent(c.clone(), None).unwrap());
        assert!(outline.undo());
        assert_eq!(outline.page(None).unwrap()[2].depth, 1);
        assert_eq!(outline.sync_status(), SyncStatus::Off);
    }

    /// What CKSyncEngine does, against the in-memory backend: send what is pending, answer
    /// with the server's outcomes, fetch what changed.
    fn native_round(outline: &Outline, server: &mut flowie_sync::MemoryServer, cursor: &mut Option<String>) {
        loop {
            let saves = outline.next_saves(50);
            if saves.is_empty() {
                break;
            }
            let results = server.modify(saves.iter().filter_map(to_envelope).collect());
            let outcomes = results
                .into_iter()
                .map(|r| match r {
                    SaveResult::Saved { id, etag } => SaveOutcome::Saved { id, etag },
                    SaveResult::Conflict { server } => SaveOutcome::Conflict { server: to_wire(server) },
                    SaveResult::Failed { id, error } => SaveOutcome::Failed { id, error },
                })
                .collect();
            outline.on_saved(outcomes);
        }
        let batch = server.changes(cursor.as_deref(), 500);
        *cursor = Some(batch.cursor.clone());
        outline.on_fetched(batch.records.into_iter().map(to_wire).collect(), batch.cursor);
    }

    #[test]
    fn a_native_engine_syncs_two_outlines_through_its_backend() {
        let mut server = flowie_sync::MemoryServer::new();
        let (a, b) = (Outline::in_memory(), Outline::in_memory());
        let (mut ca, mut cb) = (None, None);
        for outline in [&a, &b] {
            outline.begin_native_sync("icloud".into());
        }
        let x = a.create(None, 0, "from a".into()).unwrap();
        assert_eq!(a.unsent_ids(10), std::slice::from_ref(&x));
        native_round(&a, &mut server, &mut ca);
        assert!(a.unsent_ids(10).is_empty());
        native_round(&b, &mut server, &mut cb);
        assert_eq!(b.page(None).unwrap()[0].text, "from a");

        // Both edit x; b saves second and its save conflicts, then both edits survive.
        a.set_text(x.clone(), "a's text".into()).unwrap();
        b.set_note(x.clone(), "b's note".into()).unwrap();
        native_round(&a, &mut server, &mut ca);
        native_round(&b, &mut server, &mut cb);
        native_round(&b, &mut server, &mut cb);
        native_round(&a, &mut server, &mut ca);
        for outline in [&a, &b] {
            let row = &outline.page(None).unwrap()[0];
            assert_eq!((row.text.as_str(), row.note.as_str()), ("a's text", "b's note"));
            assert!(outline.unsent_ids(10).is_empty());
        }
    }

    #[test]
    fn return_and_backspace_split_and_merge_rows() {
        let outline = Outline::in_memory();
        let a = outline.create(None, 0, "hello world".into()).unwrap();
        let b = outline.create(None, 1, "b".into()).unwrap();
        outline.create(Some(b.clone()), 0, "child".into()).unwrap();
        let texts = |o: &Outline| o.page(None).unwrap().into_iter().map(|r| (r.text, r.depth)).collect::<Vec<_>>();

        let caret = outline.enter(a.clone(), 5, 5, None).unwrap();
        assert_eq!(caret.offset, 0);
        let rows = outline.page(None).unwrap();
        assert_eq!((rows[1].id.as_str(), rows[1].text.as_str()), (caret.id.as_str(), " world"));
        // Backspace at the start of the new row joins it back, the caret where they meet.
        let back = outline.backspace_at_start(caret.id.clone(), None).unwrap().unwrap();
        assert_eq!((back.id.as_str(), back.offset), (a.as_str(), 5));
        assert_eq!(texts(&outline)[0].0, "hello world");
        // A row with children joins a leaf above, children and all.
        outline.backspace_at_start(b.clone(), None).unwrap().unwrap();
        assert_eq!(texts(&outline), [("hello worldb".to_string(), 0), ("child".to_string(), 1)]);
        assert_eq!(outline.backspace_at_start(a.clone(), None).unwrap(), None, "the first row");
        // Each key is one undo step.
        assert!(outline.undo());
        assert!(outline.undo());
        assert_eq!(texts(&outline)[..2], [("hello".to_string(), 0), (" world".to_string(), 0)]);
        assert!(outline.undo());
        assert_eq!(texts(&outline)[0].0, "hello world");
        // A caret inside a character falls back to its start.
        let c = outline.create(None, 0, "日记".into()).unwrap();
        outline.enter(c, 4, 4, None).unwrap();
        assert_eq!(texts(&outline)[..2], [("日".to_string(), 0), ("记".to_string(), 0)]);
    }

    #[test]
    fn an_outline_on_disk_keeps_its_edits() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite").to_string_lossy().into_owned();
        let outline = Outline::open(db.clone()).unwrap();
        let id = outline.create(None, 0, "saved".into()).unwrap();
        outline.set_text(id.clone(), "saved, edited".into()).unwrap();
        outline.flush().unwrap();
        drop(outline);
        let outline = Outline::open(db).unwrap();
        assert_eq!(outline.page(None).unwrap()[0].text, "saved, edited");
    }
}
