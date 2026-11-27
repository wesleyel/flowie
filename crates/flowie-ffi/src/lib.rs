//! Flowie for native apps, through UniFFI (wiki: iOS-Cross-Platform §3.3).
//!
//! The iOS app (M3) and the CloudKit bridge (P4) call this: an open outline, the rows of a
//! page, the basic edits, and sync with a self-hosted server. Swift keeps no data of its own
//! and merges nothing: every read and write goes through `flowie-doc`, the same layer the
//! desktop app uses. Ids cross the boundary as 32 hex digits, like on the wire.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use flowie_core::images::ImageDir;
use flowie_core::{flatten_mapped, BlockKind, Caret, NodeId, Tree, ViewOptions, ROOT};
use flowie_doc::{utf16, Outcome, SyncDriver, SyncPhase, Workspace};
use flowie_sync::http::HttpBackend;
use flowie_sync::{ChangeBatch, Engine, Envelope, RecordStore, SaveResult};

uniffi::setup_scaffolding!();

/// What can go wrong across the boundary.
#[derive(Debug, uniffi::Error)]
pub enum FlowieError {
    /// The database couldn't be opened or saved.
    Storage { message: String },
    /// A newer Flowie wrote the database (its format is `found`; this build reads up to
    /// `current`). Nothing was changed.
    TooNew { found: i64, current: i64 },
    /// Upgrading the database failed and was rolled back: it is as it was, and `backup` holds
    /// the copy taken before.
    UpgradeFailed { backup: String, message: String },
    /// The copy taken before upgrading couldn't be saved, so the upgrade didn't start.
    BackupFailed { message: String },
    /// No bullet has this id.
    NoSuchBullet { id: String },
}

impl From<flowie_core::store::OpenError> for FlowieError {
    fn from(error: flowie_core::store::OpenError) -> Self {
        use flowie_core::store::OpenError;
        match error {
            OpenError::TooNew { found, current } => FlowieError::TooNew { found, current },
            OpenError::Migration { ref backup, .. } => {
                FlowieError::UpgradeFailed { backup: backup.display().to_string(), message: error.to_string() }
            }
            OpenError::Backup { .. } => FlowieError::BackupFailed { message: error.to_string() },
            other => FlowieError::Storage { message: other.to_string() },
        }
    }
}

impl std::error::Error for FlowieError {}

impl std::fmt::Display for FlowieError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlowieError::Storage { message } | FlowieError::BackupFailed { message } | FlowieError::UpgradeFailed { message, .. } => {
                write!(f, "{message}")
            }
            FlowieError::TooNew { found, current } => {
                write!(f, "the database has format {found}, from a newer Flowie (this one reads up to {current})")
            }
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

/// What an edit changed, so the app updates only that (wiki: iOS-Cross-Platform §4.2).
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct EditOutcome {
    /// Rows whose content changed: read them again with [`Outline::rows`] (ones not on the
    /// page shown can be ignored).
    pub changed_ids: Vec<String>,
    /// Rows came, went or moved: read the page again ([`Outline::page`]), its rows too.
    pub structure_changed: bool,
    /// Where the caret goes, when the edit moves it.
    pub focus: Option<FocusView>,
}

/// A selection in a row's text, in UTF-16 code units (what UIKit counts); a caret when
/// `start == end`.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FocusView {
    pub id: String,
    pub start: u32,
    pub end: u32,
}

impl EditOutcome {
    /// Whether nothing changed.
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.changed_ids.is_empty() && !self.structure_changed
    }

    fn from(tree: &Tree, outcome: Outcome) -> Self {
        let focus = outcome.focus.filter(|caret| tree.contains(caret.node)).map(|caret| {
            let offset = utf16::to_utf16(&tree.node(tree.source(caret.node)).text, caret.offset) as u32;
            FocusView { id: caret.node.to_string(), start: offset, end: offset }
        });
        EditOutcome {
            changed_ids: outcome.changed_ids.iter().map(ToString::to_string).collect(),
            structure_changed: outcome.structure_changed,
            focus,
        }
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

/// Run an edit that returns where the caret goes, and say what it changed.
fn tracked(ws: &mut Workspace, f: impl FnOnce(&mut Workspace) -> Option<Caret>) -> EditOutcome {
    let (focus, outcome) = ws.tracked(f);
    EditOutcome::from(&ws.tree, Outcome { focus, ..outcome })
}

/// The page's own bullet (none: Home).
fn page_root(tree: &Tree, root: Option<String>) -> Result<NodeId, FlowieError> {
    let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
    if root != ROOT && !tree.contains(root) {
        return Err(FlowieError::NoSuchBullet { id: root.to_string() });
    }
    Ok(root)
}

fn row_view(tree: &Tree, id: NodeId, depth: u16) -> Option<RowView> {
    let node = tree.get(id)?;
    let source = tree.node(tree.source(id));
    Some(RowView {
        id: id.to_string(),
        depth,
        text: source.text.clone(),
        note: source.note.clone(),
        kind: kind_name(node.kind),
        completed: source.completed,
        collapsed: node.collapsed,
        has_children: tree.has_children(id),
    })
}

/// The depth of bullet `id`'s row on the page at `root`, found by walking up from it: none if
/// it isn't under `root` or a collapsed bullet hides it. Rows inside mirrors (virtual ids) are
/// not found this way.
fn depth_on_page(tree: &Tree, root: NodeId, id: NodeId) -> Option<u16> {
    if id.is_virtual() || id == root || !tree.contains(id) {
        return None;
    }
    let mut depth = 0;
    let mut cur = tree.parent(id);
    while cur != root {
        if cur == ROOT || tree.node(cur).collapsed {
            return None;
        }
        depth += 1;
        cur = tree.parent(cur);
    }
    Some(depth)
}

#[uniffi::export]
impl Outline {
    /// Open (or create) the outline database at `db`, its images beside it.
    #[uniffi::constructor]
    pub fn open(db: String) -> Result<Arc<Self>, FlowieError> {
        let db = PathBuf::from(db);
        // On an error the database file is left as it was (`store::open`).
        let (ws, _) = Workspace::open(&db, ImageDir::beside(&db), |_| {})?;
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
        let root = page_root(tree, root)?;
        let (rows, _) = flatten_mapped(tree, root, &ViewOptions::default());
        Ok(rows.into_iter().filter_map(|row| row_view(tree, row.id, row.depth)).collect())
    }

    /// The rows with these ids on the page at `root` (none: Home), as [`page`](Self::page)
    /// has them, in the order asked; ids not on the page are left out. For the rows an edit
    /// changed (`EditOutcome::changed_ids`), without reading the whole page.
    pub fn rows(&self, ids: Vec<String>, root: Option<String>) -> Result<Vec<RowView>, FlowieError> {
        let state = self.lock();
        let tree = &state.ws.tree;
        let root = page_root(tree, root)?;
        // Rows inside a mirror (virtual ids) are found on the page.
        let mut page = None;
        Ok(ids
            .iter()
            .filter_map(|id| {
                let id = id.parse::<NodeId>().ok()?;
                let depth = depth_on_page(tree, root, id).or_else(|| {
                    let rows = page.get_or_insert_with(|| flatten_mapped(tree, root, &ViewOptions::default()).0);
                    rows.iter().find(|row| row.id == id).map(|row| row.depth)
                })?;
                row_view(tree, id, depth)
            })
            .collect())
    }

    /// Add a bullet with `text` under `parent` (none: Home) at `index`; the focus is on it, at
    /// the end of its text. An undo step.
    pub fn create(&self, parent: Option<String>, index: u32, text: String) -> Result<EditOutcome, FlowieError> {
        let mut state = self.lock();
        let parent = parent.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        if parent != ROOT && !state.ws.tree.contains(parent) {
            return Err(FlowieError::NoSuchBullet { id: parent.to_string() });
        }
        let text = text.replace('\n', " ");
        let (id, mut outcome) = state.ws.tracked(|ws| {
            let id = ws.tree.create_at(parent, index as usize, &text);
            ws.commit_with_imports(None, None, None);
            id
        });
        outcome.focus = Some(Caret { node: id, offset: text.len() });
        Ok(EditOutcome::from(&state.ws.tree, outcome))
    }

    /// Replace a bullet's text. Typing in one bullet merges into one undo step.
    pub fn set_text(&self, id: String, text: String) -> Result<EditOutcome, FlowieError> {
        self.edit(&id, |ws, id| {
            ws.tree.set_text(id, &text.replace('\n', " "));
            ws.commit_typing(id, None, None);
            None
        })
    }

    pub fn set_note(&self, id: String, note: String) -> Result<EditOutcome, FlowieError> {
        self.edit(&id, |ws, id| {
            ws.tree.set_note(id, &note);
            ws.commit_typing(id, None, None);
            None
        })
    }

    pub fn set_completed(&self, id: String, completed: bool) -> Result<EditOutcome, FlowieError> {
        self.edit(&id, |ws, id| {
            ws.tree.set_completed(id, completed);
            ws.commit_with_imports(None, None, None);
            None
        })
    }

    /// Make a bullet the last child of the one above it (nothing changes if there is none). An
    /// undo step.
    pub fn indent(&self, id: String) -> Result<EditOutcome, FlowieError> {
        self.edit_row(&id, |ws, id| {
            if ws.tree.indent(id) {
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Move a bullet out to follow its parent, but not out of the page at `root` (none: Home;
    /// nothing changes if it can't go further). An undo step.
    pub fn outdent(&self, id: String, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        self.edit_row(&id, |ws, id| {
            if ws.tree.outdent(id, root) {
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Move a bullet above the one before it; the first of its parent becomes the last child
    /// of the bullet before its parent, but not out of the page at `root` (none: Home). As the
    /// desktop's Move Up (`Tree::move_up`); nothing changes if it can't go further. An undo
    /// step.
    pub fn move_up(&self, id: String, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        self.edit_row(&id, |ws, id| {
            if ws.tree.move_up(id, root) {
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Move a bullet below the one after it; the last of its parent becomes the first child
    /// of the bullet after its parent, but not out of the page at `root` (none: Home). As the
    /// desktop's Move Down (`Tree::move_down`). An undo step.
    pub fn move_down(&self, id: String, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        self.edit_row(&id, |ws, id| {
            if ws.tree.move_down(id, root) {
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Return in a row's text with `start..end` selected (UTF-16 offsets; a caret when equal),
    /// on the page at `root` (none: Home), as on the desktop (`flowie_doc::keys::enter`): the
    /// text after the caret moves to a new bullet below, where the focus goes. An undo step.
    pub fn enter(&self, id: String, start: u32, end: u32, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        self.edit_row(&id, |ws, id| {
            let text = &ws.tree.node(ws.tree.source(id)).text;
            let sel = utf16::from_utf16(text, start as usize)..utf16::from_utf16(text, end.max(start) as usize);
            Some(ws.enter(id, sel, root))
        })
    }

    /// Backspace with the caret at the start of a row's text, on the page at `root` (none:
    /// Home), as on the desktop (`flowie_doc::keys::backspace_at_start`): the row joins the one
    /// above, or goes if it is empty, and the focus goes where they meet. Nothing changes on
    /// the first row. An undo step.
    pub fn backspace_at_start(&self, id: String, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        self.edit_row(&id, |ws, id| {
            let (rows, _) = flatten_mapped(&ws.tree, root, &ViewOptions::default());
            let prev = rows.iter().position(|r| r.id == id).and_then(|i| i.checked_sub(1)).map(|i| rows[i].id);
            ws.backspace_at_start(id, prev, root)
        })
    }

    /// Move a bullet and what is under it to the Trash. An undo step.
    pub fn delete(&self, id: String) -> Result<EditOutcome, FlowieError> {
        self.edit(&id, |ws, id| {
            ws.tree.trash(id);
            ws.commit_with_imports(None, None, None);
            None
        })
    }

    /// Undo the last step (nothing changes if there is none); the focus goes where the caret
    /// was before it, when that is known.
    pub fn undo(&self) -> EditOutcome {
        self.track(|ws| ws.undo().flatten())
    }

    /// Redo the last undone step; the focus goes where the caret was after it.
    pub fn redo(&self) -> EditOutcome {
        self.track(|ws| ws.redo().flatten())
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

    /// Run an edit that returns where the caret goes, and say what it changed.
    fn track(&self, f: impl FnOnce(&mut Workspace) -> Option<Caret>) -> EditOutcome {
        tracked(&mut self.lock().ws, f)
    }

    /// Change a bullet's content: a mirror's row changes its source.
    fn edit(&self, id: &str, f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>) -> Result<EditOutcome, FlowieError> {
        self.edit_row(id, |ws, id| {
            let id = ws.tree.source(id);
            f(ws, id)
        })
    }

    /// Change a bullet's place: the row itself moves, a mirror included.
    fn edit_row(&self, id: &str, f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>) -> Result<EditOutcome, FlowieError> {
        let id = parse(id)?;
        let mut state = self.lock();
        if !state.ws.tree.contains(id) {
            return Err(FlowieError::NoSuchBullet { id: id.to_string() });
        }
        Ok(tracked(&mut state.ws, |ws| f(ws, id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Add a bullet; its id.
    fn add(outline: &Outline, parent: Option<&str>, index: u32, text: &str) -> String {
        outline.create(parent.map(String::from), index, text.into()).unwrap().focus.unwrap().id
    }

    fn texts(outline: &Outline) -> Vec<(String, u16)> {
        outline.page(None).unwrap().into_iter().map(|r| (r.text, r.depth)).collect()
    }

    #[test]
    fn edits_show_on_the_page_and_undo() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "first");
        let b = add(&outline, Some(&a), 0, "child");
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
        assert!(outline.undo().structure_changed);
        assert_eq!(outline.page(None).unwrap().len(), 2);
        assert!(matches!(outline.set_text("nope".into(), "x".into()), Err(FlowieError::NoSuchBullet { .. })));

        let c = add(&outline, None, 1, "second");
        assert!(outline.indent(c.clone()).unwrap().structure_changed);
        assert_eq!(
            outline.page(None).unwrap().iter().map(|r| (r.text.as_str(), r.depth)).collect::<Vec<_>>(),
            [("first", 0), ("child", 1), ("second", 1)]
        );
        assert!(outline.outdent(c.clone(), Some(a.clone())).unwrap().is_empty(), "not out of the page shown");
        assert!(outline.outdent(c.clone(), None).unwrap().structure_changed);
        assert_eq!(outline.page(None).unwrap()[2].depth, 0);
        assert!(outline.outdent(c.clone(), None).unwrap().is_empty());
        assert!(outline.undo().structure_changed);
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
        let x = add(&a, None, 0, "from a");
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
        let a = add(&outline, None, 0, "hello world");
        let b = add(&outline, None, 1, "b");
        add(&outline, Some(&b), 0, "child");

        let out = outline.enter(a.clone(), 5, 5, None).unwrap();
        assert!(out.structure_changed);
        let focus = out.focus.unwrap();
        assert_eq!((focus.start, focus.end), (0, 0));
        let rows = outline.page(None).unwrap();
        assert_eq!((rows[1].id.as_str(), rows[1].text.as_str()), (focus.id.as_str(), " world"));
        // Backspace at the start of the new row joins it back, the caret where they meet.
        let back = outline.backspace_at_start(focus.id.clone(), None).unwrap().focus.unwrap();
        assert_eq!((back.id.as_str(), back.start, back.end), (a.as_str(), 5, 5));
        assert_eq!(texts(&outline)[0].0, "hello world");
        // A row with children joins a leaf above, children and all.
        outline.backspace_at_start(b.clone(), None).unwrap().focus.unwrap();
        assert_eq!(texts(&outline), [("hello worldb".to_string(), 0), ("child".to_string(), 1)]);
        assert_eq!(outline.backspace_at_start(a.clone(), None).unwrap(), EditOutcome::default(), "the first row");
        // Each key is one undo step.
        assert!(outline.undo().structure_changed);
        assert!(outline.undo().structure_changed);
        assert_eq!(texts(&outline)[..2], [("hello".to_string(), 0), (" world".to_string(), 0)]);
        // Undo puts the caret back where it was before Return.
        let undone = outline.undo();
        assert_eq!(undone.focus, Some(FocusView { id: a.clone(), start: 5, end: 5 }));
        assert_eq!(texts(&outline)[0].0, "hello world");
    }

    #[test]
    fn rows_move_up_and_down_within_the_page() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "a");
        let b = add(&outline, None, 1, "b");
        add(&outline, Some(&a), 0, "a1");
        let out = outline.move_up(b.clone(), None).unwrap();
        assert!(out.structure_changed);
        assert_eq!(texts(&outline), [("b".into(), 0), ("a".into(), 0), ("a1".into(), 1)]);
        assert!(outline.move_up(b.clone(), None).unwrap().is_empty(), "already first");
        assert!(outline.move_down(b.clone(), None).unwrap().structure_changed);
        assert_eq!(texts(&outline), [("a".into(), 0), ("a1".into(), 1), ("b".into(), 0)]);
        // The first child goes into the bullet before its parent, except out of the page.
        let b1 = add(&outline, Some(&b), 0, "b1");
        assert!(outline.move_up(b1.clone(), Some(b.clone())).unwrap().is_empty(), "not out of the page shown");
        outline.move_up(b1.clone(), None).unwrap();
        assert_eq!(texts(&outline), [("a".into(), 0), ("a1".into(), 1), ("b1".into(), 1), ("b".into(), 0)]);
        assert!(outline.move_down(b1.clone(), None).unwrap().structure_changed);
        assert_eq!(texts(&outline)[2..], [("b".into(), 0), ("b1".into(), 1)]);
        // Each move is an undo step.
        outline.undo();
        assert_eq!(texts(&outline)[2], ("b1".into(), 1));
        assert!(matches!(outline.move_up("nope".into(), None), Err(FlowieError::NoSuchBullet { .. })));
    }

    #[test]
    fn offsets_cross_in_utf16() {
        let outline = Outline::in_memory();
        // 日记: 3 bytes and 1 unit each; 😀: 4 bytes, 2 units.
        let a = add(&outline, None, 0, "日记😀x");
        let out = outline.create(None, 1, "日😀".into()).unwrap();
        assert_eq!(out.focus.map(|f| (f.start, f.end)), Some((3, 3)), "the end of the text, in UTF-16");
        // Return after 记 (unit 2), and with 记😀 (units 1..4) selected.
        let focus = outline.enter(a.clone(), 2, 2, None).unwrap().focus.unwrap();
        assert_eq!(texts(&outline)[..2], [("日记".to_string(), 0), ("😀x".to_string(), 0)]);
        let back = outline.backspace_at_start(focus.id, None).unwrap().focus.unwrap();
        assert_eq!((back.start, back.end), (2, 2), "after 日记, in UTF-16");
        outline.enter(a.clone(), 1, 4, None).unwrap();
        assert_eq!(texts(&outline)[..2], [("日".to_string(), 0), ("x".to_string(), 0)]);
        // A caret between the halves of 😀 moves past it.
        let b = add(&outline, None, 0, "😀😀");
        outline.enter(b, 1, 1, None).unwrap();
        assert_eq!(texts(&outline)[..2], [("😀".to_string(), 0), ("😀".to_string(), 0)]);
    }

    #[test]
    fn typing_changes_only_its_row() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "a");
        let b = add(&outline, Some(&a), 0, "b");
        let out = outline.set_text(b.clone(), "b, typed 中文 ✓".into()).unwrap();
        assert_eq!(out, EditOutcome { changed_ids: vec![b.clone()], structure_changed: false, focus: None });
        // The changed row reads the same as on the page.
        let page = outline.page(None).unwrap();
        assert_eq!(outline.rows(out.changed_ids.clone(), None).unwrap(), [page[1].clone()]);
        assert_eq!(outline.rows(vec![b.clone(), a.clone()], Some(a.clone())).unwrap()[..], [RowView { depth: 0, ..page[1].clone() }]);
        assert!(outline.rows(vec!["nope".into()], None).unwrap().is_empty());
        let out = outline.set_completed(b.clone(), true).unwrap();
        assert_eq!((out.changed_ids.as_slice(), out.structure_changed), (std::slice::from_ref(&b), false));
        let out = outline.set_note(a.clone(), "note".into()).unwrap();
        assert_eq!((out.changed_ids.as_slice(), out.structure_changed), (std::slice::from_ref(&a), false));
        // Undoing typing changes the row back.
        let out = outline.undo();
        assert_eq!((out.changed_ids, out.structure_changed), (vec![a.clone()], false));
        // A row hidden under a collapsed bullet isn't on the page.
        let mut state = outline.lock();
        let id = a.parse().unwrap();
        state.ws.tree.set_collapsed(id, true);
        drop(state);
        assert!(outline.rows(vec![b.clone()], None).unwrap().is_empty());
        // Deleting is structural.
        let out = outline.delete(b).unwrap();
        assert!(out.structure_changed);
        assert!(outline.redo().is_empty(), "nothing to redo after an edit");
    }

    /// How long reading a page takes on a big outline (20 000 rows), for deciding whether rows
    /// should be read lazily (wiki: iOS-Cross-Platform §4.2). Run with `cargo test --release -p
    /// flowie-ffi -- --ignored --nocapture page_timing`.
    #[test]
    #[ignore]
    fn page_timing() {
        let outline = Outline::in_memory();
        {
            let mut state = outline.lock();
            let tree = &mut state.ws.tree;
            for i in 0..2000 {
                let top = tree.create_at(ROOT, i, &format!("Top-level bullet number {i} with some text"));
                for j in 0..9 {
                    tree.create_at(top, j, &format!("A child bullet {j} under {i}, with **markup** and 中文"));
                }
            }
            state.ws.commit_with_imports(None, None, None);
        }
        let started = std::time::Instant::now();
        let n = 20;
        let mut rows = 0;
        for _ in 0..n {
            rows = outline.page(None).unwrap().len();
        }
        let page = started.elapsed() / n;
        let id = outline.page(None).unwrap()[rows / 2].id.clone();
        let started = std::time::Instant::now();
        for i in 0..n {
            let out = outline.set_text(id.clone(), format!("typed {i}")).unwrap();
            outline.rows(out.changed_ids, None).unwrap();
        }
        let typing = started.elapsed() / n;
        eprintln!("page(): {rows} rows in {page:?}; set_text + rows(): {typing:?}");
        assert_eq!(rows, 20_000);
    }

    #[test]
    fn a_database_from_a_newer_flowie_is_refused_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outline.sqlite");
        let db = path.to_string_lossy().into_owned();
        let outline = Outline::open(db.clone()).unwrap();
        add(&outline, None, 0, "kept");
        outline.flush().unwrap();
        drop(outline);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA user_version = 999; UPDATE meta SET value = '999' WHERE key = 'min_reader';",
        )
        .unwrap();
        drop(conn);
        let bytes = std::fs::read(&path).unwrap();
        match Outline::open(db) {
            Err(FlowieError::TooNew { found: 999, .. }) => {}
            Err(other) => panic!("{other}"),
            Ok(_) => panic!("opened"),
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file is untouched");
    }

    #[test]
    fn an_outline_on_disk_keeps_its_edits() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite").to_string_lossy().into_owned();
        let outline = Outline::open(db.clone()).unwrap();
        let id = add(&outline, None, 0, "saved");
        outline.set_text(id.clone(), "saved, edited".into()).unwrap();
        outline.flush().unwrap();
        drop(outline);
        let outline = Outline::open(db).unwrap();
        assert_eq!(outline.page(None).unwrap()[0].text, "saved, edited");
    }
}
