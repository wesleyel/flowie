//! Flowie for native apps, through UniFFI (wiki: iOS-Cross-Platform §3.3).
//!
//! The iOS app (M3) and the CloudKit bridge (P4) call this: an open outline, the rows of a
//! page, the basic edits, and sync with a self-hosted server. Swift keeps no data of its own
//! and merges nothing: every read and write goes through `flowie-doc`, the same layer the
//! desktop app uses. Ids cross the boundary as 32 hex digits, like on the wire.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use flowie_core::images::ImageDir;
use flowie_core::search::Query;
use flowie_core::{flatten_mapped, journal, markup, BlockKind, Caret, NodeId, Tree, ViewOptions, ROOT};
use flowie_doc::{find, utf16, Outcome, SyncDriver, SyncPhase, Workspace};
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
    /// Not a date as journal pages are named (YYYY-MM-DD).
    NotADate { date: String },
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
            FlowieError::NotADate { date } => write!(f, "{date:?} is not a date (YYYY-MM-DD)"),
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

/// A bullet as lists name it: search results, stars, a page's title and the path to it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct BulletView {
    pub id: String,
    /// Its text as plain text (a mirror's: its source's), without markup.
    pub text: String,
    pub completed: bool,
    pub starred: bool,
    pub has_children: bool,
    /// The bullets above it, from the top (Home is not in it).
    pub path: Vec<Crumb>,
}

/// One bullet on the path to another.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Crumb {
    pub id: String,
    /// Plain text, without markup.
    pub text: String,
}

/// A deleted bullet, with what is under it, as Recently Deleted lists it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TrashView {
    pub id: String,
    /// Plain text, cut to 80 characters.
    pub text: String,
    /// How many bullets restoring it brings back, itself included.
    pub count: u32,
    /// When it was deleted, in milliseconds since 1970.
    pub deleted_at: i64,
}

impl EditOutcome {
    /// Whether nothing changed.
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.changed_ids.is_empty() && !self.structure_changed
    }

    /// `hiding`: completed rows are hidden and the edit may have completed or reopened some,
    /// so rows changing count as rows coming and going.
    fn from(tree: &Tree, outcome: Outcome, hiding: bool) -> Self {
        let focus = outcome.focus.filter(|caret| tree.contains(caret.node)).map(|caret| {
            let offset = utf16::to_utf16(&tree.node(tree.source(caret.node)).text, caret.offset) as u32;
            FocusView { id: caret.node.to_string(), start: offset, end: offset }
        });
        EditOutcome {
            changed_ids: outcome.changed_ids.iter().map(ToString::to_string).collect(),
            structure_changed: outcome.structure_changed || (hiding && !outcome.changed_ids.is_empty()),
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
    /// Completed rows are left off pages (this device's setting, as on the desktop).
    hide_completed: bool,
}

impl State {
    fn new(ws: Workspace, hide_completed: bool) -> Self {
        State { ws, sync: None, native: None, hide_completed }
    }

    fn view(&self) -> ViewOptions {
        ViewOptions { hide_completed: self.hide_completed, query: None }
    }
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

/// Run an edit that returns where the caret goes, and say what it changed. `text_only`: the
/// edit changes only text, never whether a row is completed.
fn tracked(state: &mut State, text_only: bool, f: impl FnOnce(&mut Workspace) -> Option<Caret>) -> EditOutcome {
    let hiding = state.hide_completed && !text_only;
    let (focus, outcome) = state.ws.tracked(f);
    EditOutcome::from(&state.ws.tree, Outcome { focus, ..outcome }, hiding)
}

/// Bullet `id`'s text as plain text (a mirror's source's).
fn plain(tree: &Tree, id: NodeId) -> String {
    markup::plain_text(&tree.node(tree.source(id)).text)
}

fn bullet_view(tree: &Tree, id: NodeId) -> BulletView {
    let source = tree.node(tree.source(id));
    BulletView {
        id: id.to_string(),
        text: markup::plain_text(&source.text),
        completed: source.completed,
        starred: tree.is_starred(id),
        has_children: tree.has_children(id),
        path: tree.ancestors(id).into_iter().map(|a| Crumb { id: a.to_string(), text: plain(tree, a) }).collect(),
    }
}

/// A date from outside, checked: journal ids are derived from it.
fn day(date: &str) -> Result<&str, FlowieError> {
    if journal::is_date(date) {
        Ok(date)
    } else {
        Err(FlowieError::NotADate { date: date.to_string() })
    }
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
/// it isn't under `root`, a collapsed bullet hides it, or (with `hide_completed`) it or a bullet
/// above it is completed. Rows inside mirrors (virtual ids) are not found this way.
fn depth_on_page(tree: &Tree, root: NodeId, id: NodeId, hide_completed: bool) -> Option<u16> {
    let done = |id: NodeId| hide_completed && tree.node(tree.source(id)).completed;
    if id.is_virtual() || id == root || !tree.contains(id) || done(id) {
        return None;
    }
    let mut depth = 0;
    let mut cur = tree.parent(id);
    while cur != root {
        if cur == ROOT || tree.node(cur).collapsed || done(cur) {
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
        let (ws, meta) = Workspace::open(&db, ImageDir::beside(&db), |_| {})?;
        let hide_completed = meta.iter().any(|(k, v)| k == "hide_completed" && v == "1");
        Ok(Arc::new(Outline { state: Mutex::new(State::new(ws, hide_completed)) }))
    }

    /// An outline kept only in memory (previews and tests).
    #[uniffi::constructor]
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Outline { state: Mutex::new(State::new(Workspace::new(Tree::new()), false)) })
    }

    /// The rows of the page at `root` (none: Home), expanded bullets' children included, and
    /// completed ones left out when [`hide_completed`](Self::hide_completed).
    pub fn page(&self, root: Option<String>) -> Result<Vec<RowView>, FlowieError> {
        let state = self.lock();
        let tree = &state.ws.tree;
        let root = page_root(tree, root)?;
        let (rows, _) = flatten_mapped(tree, root, &state.view());
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
                let depth = depth_on_page(tree, root, id, state.hide_completed).or_else(|| {
                    let rows = page.get_or_insert_with(|| flatten_mapped(tree, root, &state.view()).0);
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
        Ok(EditOutcome::from(&state.ws.tree, outcome, false))
    }

    /// Replace a bullet's text. Typing in one bullet merges into one undo step.
    pub fn set_text(&self, id: String, text: String) -> Result<EditOutcome, FlowieError> {
        self.edit_text(&id, |ws, id| {
            ws.tree.set_text(id, &text.replace('\n', " "));
            ws.commit_typing(id, None, None);
            None
        })
    }

    pub fn set_note(&self, id: String, note: String) -> Result<EditOutcome, FlowieError> {
        self.edit_text(&id, |ws, id| {
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
        let view = self.lock().view();
        self.edit_row(&id, |ws, id| {
            let (rows, _) = flatten_mapped(&ws.tree, root, &view);
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

    /// Fold or unfold a bullet with children, as the desktop's fold toggle: its children leave
    /// or join the page. Nothing changes for a bullet without children. An undo step; folding
    /// doesn't sync (each device keeps its own).
    pub fn set_collapsed(&self, id: String, collapsed: bool) -> Result<EditOutcome, FlowieError> {
        self.edit_row(&id, |ws, id| {
            if ws.tree.has_children(ws.tree.source(id)) && ws.tree.node(id).collapsed != collapsed {
                ws.tree.set_collapsed(id, collapsed);
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Move a bullet, with what is under it, to the end of `parent` (none: Home), unfolding
    /// `parent` so it shows there: the desktop's Move To. Nothing changes when `parent` is the
    /// bullet itself or under it. An undo step.
    pub fn move_to(&self, id: String, parent: Option<String>) -> Result<EditOutcome, FlowieError> {
        let parent = parent.as_deref().map(|p| self.existing(p)).transpose()?.unwrap_or(ROOT);
        self.edit_row(&id, |ws, id| {
            let n = ws.tree.children(parent).len();
            if ws.tree.move_to(id, parent, n) {
                ws.tree.set_collapsed(parent, false);
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Whether pages leave out completed rows: this device's setting, the desktop's Hide
    /// Completed.
    pub fn hide_completed(&self) -> bool {
        self.lock().hide_completed
    }

    /// Show or hide completed rows on pages from now on (saved; read every page again).
    pub fn set_hide_completed(&self, hide: bool) {
        let mut state = self.lock();
        state.hide_completed = hide;
        state.ws.set_meta("hide_completed", if hide { "1" } else { "0" });
    }

    /// A bullet as lists name it, with the path to it.
    pub fn bullet(&self, id: String) -> Result<BulletView, FlowieError> {
        let node = self.existing(&id)?;
        Ok(bullet_view(&self.lock().ws.tree, node))
    }

    /// The starred bullets, in the starred list's order.
    pub fn starred(&self) -> Vec<BulletView> {
        let state = self.lock();
        let tree = &state.ws.tree;
        tree.starred().map(|id| bullet_view(tree, id)).collect()
    }

    /// Star a bullet (at the end of the starred list) or unstar it. Saved, and synced, but not
    /// an undo step, as on the desktop.
    pub fn set_starred(&self, id: String, starred: bool) -> Result<(), FlowieError> {
        let node = self.existing(&id)?;
        self.lock().ws.set_starred(node, starred);
        Ok(())
    }

    /// The bullets `query` matches, with the desktop's filter syntax (`flowie_core::search`:
    /// words, `"phrases"`, `#tags`, `-exclude`, `is:done`, `has:note`, `date:`, `changed:7d`,
    /// …), the most recently changed first; at most `limit`. Nothing for an empty query.
    pub fn search(&self, query: String, limit: u32) -> Vec<BulletView> {
        let query = Query::parse(&query);
        if query.is_empty() {
            return Vec::new();
        }
        let state = self.lock();
        let tree = &state.ws.tree;
        find::search(tree, &query).into_iter().take(limit as usize).map(|id| bullet_view(tree, id)).collect()
    }

    /// Bullets to pick by name, as the desktop's palette finds them (`find::quick_find`): every
    /// word of `query` in the text, starred ones first; `exclude` and what is under them left
    /// out (for Move To, the bullets being moved). At most `limit`.
    pub fn quick_find(&self, query: String, exclude: Vec<String>, limit: u32) -> Vec<BulletView> {
        let exclude: Vec<NodeId> = exclude.iter().filter_map(|id| id.parse().ok()).collect();
        let state = self.lock();
        let tree = &state.ws.tree;
        find::quick_find(tree, &query, limit as usize, &exclude).into_iter().map(|id| bullet_view(tree, id)).collect()
    }

    /// A Markdown link to a bullet, as the desktop's Copy Link makes it: `[text](flowie:ID)`,
    /// `untitled` standing for an empty text.
    pub fn link(&self, id: String, untitled: String) -> Result<String, FlowieError> {
        let node = self.existing(&id)?;
        Ok(find::link(&self.lock().ws.tree, node, &untitled))
    }

    /// What is in the Trash, newest first, each entry a deleted bullet with what is under it.
    /// Entries past the retention period are deleted for good first, as at launch.
    pub fn trashed(&self) -> Vec<TrashView> {
        let mut state = self.lock();
        state.ws.purge_expired();
        let tree = &state.ws.tree;
        tree.trashed()
            .into_iter()
            .map(|root| {
                let node = tree.node(root);
                let shown = (node.mirror != ROOT).then(|| tree.record(node.mirror)).flatten().unwrap_or(node);
                TrashView {
                    id: root.to_string(),
                    text: markup::plain_text(&shown.text).chars().take(80).collect(),
                    count: tree.trashed_subtree(root).len() as u32,
                    deleted_at: node.deleted_at.unwrap_or_default(),
                }
            })
            .collect()
    }

    /// Bring a Trash entry back where it was (to the end of Home if that is gone too); the
    /// focus goes to the end of its text. An undo step.
    pub fn restore(&self, id: String) -> Result<EditOutcome, FlowieError> {
        let node = parse(&id)?;
        let mut state = self.lock();
        let outcome = tracked(&mut state, false, |ws| {
            if !ws.tree.untrash(node) {
                return None;
            }
            ws.commit_with_imports(None, None, None);
            Some(Caret { node, offset: ws.tree.node(node).text.len() })
        });
        if outcome.focus.is_none() {
            return Err(FlowieError::NoSuchBullet { id });
        }
        Ok(outcome)
    }

    /// The journal page for `date` (YYYY-MM-DD), if there is one. Its id is the same on every
    /// device (`flowie_core::journal`).
    pub fn journal_day(&self, date: String) -> Result<Option<String>, FlowieError> {
        let date = day(&date)?;
        Ok(journal::find_day(&self.lock().ws.tree, date).map(|id| id.to_string()))
    }

    /// Open the journal page for `date` (YYYY-MM-DD) to write in, as the desktop does: a
    /// missing page is made among the journal's pages with a blank first line (ids the same on
    /// every device, so two devices make one page), and the focus goes to the end of its first
    /// row. An undo step when it made something.
    pub fn open_day(&self, date: String) -> Result<EditOutcome, FlowieError> {
        let date = day(&date)?.to_string();
        let mut state = self.lock();
        let view = state.view();
        Ok(tracked(&mut state, false, |ws| {
            let page = journal::ensure_day(&mut ws.tree, &date, true);
            if !ws.tree.journal_is_empty() {
                ws.commit_with_imports(None, None, None);
            }
            let (rows, _) = flatten_mapped(&ws.tree, page, &view);
            let first = rows.first()?.id;
            Some(Caret { node: first, offset: ws.tree.node(ws.tree.source(first)).text.len() })
        }))
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
        tracked(&mut self.lock(), false, f)
    }

    /// Change a bullet's content: a mirror's row changes its source.
    fn edit(&self, id: &str, f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>) -> Result<EditOutcome, FlowieError> {
        self.edit_row(id, |ws, id| {
            let id = ws.tree.source(id);
            f(ws, id)
        })
    }

    /// Change a bullet's text or note only (a mirror's row changes its source).
    fn edit_text(&self, id: &str, f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>) -> Result<EditOutcome, FlowieError> {
        self.edit_with(id, true, |ws, id| {
            let id = ws.tree.source(id);
            f(ws, id)
        })
    }

    /// Change a bullet's place: the row itself moves, a mirror included.
    fn edit_row(&self, id: &str, f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>) -> Result<EditOutcome, FlowieError> {
        self.edit_with(id, false, f)
    }

    fn edit_with(
        &self,
        id: &str,
        text_only: bool,
        f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>,
    ) -> Result<EditOutcome, FlowieError> {
        let id = parse(id)?;
        let mut state = self.lock();
        if !state.ws.tree.contains(id) {
            return Err(FlowieError::NoSuchBullet { id: id.to_string() });
        }
        Ok(tracked(&mut state, text_only, |ws| f(ws, id)))
    }

    /// A bullet the outline has (not one in the Trash), parsed.
    fn existing(&self, id: &str) -> Result<NodeId, FlowieError> {
        let node = parse(id)?;
        if self.lock().ws.tree.contains(node) {
            Ok(node)
        } else {
            Err(FlowieError::NoSuchBullet { id: id.to_string() })
        }
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

    #[test]
    fn journal_days_are_found_and_opened_like_on_the_desktop() {
        let outline = Outline::in_memory();
        assert_eq!(outline.journal_day("2026-11-27".into()).unwrap(), None, "nothing is made by looking");
        assert!(outline.page(None).unwrap().is_empty());
        let out = outline.open_day("2026-11-27".into()).unwrap();
        assert!(out.structure_changed);
        let day = outline.journal_day("2026-11-27".into()).unwrap().unwrap();
        assert_eq!(day, journal::day_id("2026-11-27").to_string(), "the same id on every device");
        let rows = outline.page(Some(day.clone())).unwrap();
        assert_eq!(rows.len(), 1, "a blank first line");
        assert_eq!(out.focus, Some(FocusView { id: rows[0].id.clone(), start: 0, end: 0 }));
        assert_eq!(rows[0].id, journal::day_blank_id("2026-11-27").to_string());
        // Opening it again makes nothing, and the caret goes to the end of its first row.
        outline.set_text(rows[0].id.clone(), "日记".into()).unwrap();
        let again = outline.open_day("2026-11-27".into()).unwrap();
        assert!(!again.structure_changed && again.changed_ids.is_empty());
        assert_eq!(again.focus.map(|f| f.start), Some(2));
        assert_eq!(outline.page(None).unwrap().iter().filter(|r| r.text == "2026-11-27").count(), 1, "one page for the day");
        // Days sort newest first under the journal.
        outline.open_day("2026-11-28".into()).unwrap();
        let journal = journal::journal_root_id().to_string();
        let days: Vec<String> = outline.page(Some(journal)).unwrap().into_iter().filter(|r| r.depth == 0).map(|r| r.text).collect();
        assert_eq!(days, ["2026-11-28", "2026-11-27"]);
        assert!(matches!(outline.open_day("2026-11-27; drop".into()), Err(FlowieError::NotADate { .. })));
        assert!(matches!(outline.journal_day("tomorrow".into()), Err(FlowieError::NotADate { .. })));
    }

    #[test]
    fn folding_hides_children_and_undoes() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "a");
        let b = add(&outline, Some(&a), 0, "b");
        assert!(outline.set_collapsed(b.clone(), true).unwrap().is_empty(), "nothing to fold");
        let out = outline.set_collapsed(a.clone(), true).unwrap();
        assert!(out.structure_changed);
        let page = outline.page(None).unwrap();
        assert_eq!(page.len(), 1);
        assert!(page[0].collapsed && page[0].has_children);
        assert!(outline.set_collapsed(a.clone(), true).unwrap().is_empty(), "already folded");
        // A folded bullet's page still shows its children.
        assert_eq!(outline.page(Some(a.clone())).unwrap().len(), 1);
        outline.set_collapsed(a.clone(), false).unwrap();
        assert_eq!(outline.page(None).unwrap().len(), 2);
        assert!(outline.undo().structure_changed);
        assert_eq!(outline.page(None).unwrap().len(), 1, "undo folds it again");
    }

    #[test]
    fn stars_list_and_name_their_bullets() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "**Projects**");
        let b = add(&outline, Some(&a), 0, "Flowie");
        assert!(outline.starred().is_empty());
        outline.set_starred(b.clone(), true).unwrap();
        outline.set_starred(a.clone(), true).unwrap();
        let stars = outline.starred();
        assert_eq!(stars.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Flowie", "Projects"], "in starring order, plain");
        assert_eq!(stars[0].path, [Crumb { id: a.clone(), text: "Projects".into() }]);
        assert!(stars[0].starred && stars[1].has_children);
        outline.set_starred(b.clone(), false).unwrap();
        assert_eq!(outline.starred().len(), 1);
        assert!(!outline.bullet(b.clone()).unwrap().starred);
        // Stars aren't undo steps.
        assert!(outline.undo().structure_changed, "undo undoes the last edit, not the star");
        assert_eq!(outline.page(None).unwrap().len(), 1);
        assert!(outline.bullet(a).unwrap().starred);
        assert!(matches!(outline.set_starred("nope".into(), true), Err(FlowieError::NoSuchBullet { .. })));
    }

    #[test]
    fn search_takes_the_desktop_filters() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "Errands");
        let milk = add(&outline, Some(&a), 0, "buy milk #shop");
        let bread = add(&outline, Some(&a), 1, "buy bread #shop");
        outline.set_completed(bread.clone(), true).unwrap();
        outline.set_note(milk.clone(), "oat".into()).unwrap();
        let ids = |hits: Vec<BulletView>| hits.into_iter().map(|h| h.id).collect::<Vec<_>>();
        assert_eq!(ids(outline.search("#shop is:open".into(), 10)), std::slice::from_ref(&milk));
        assert_eq!(ids(outline.search("oat".into(), 10)), std::slice::from_ref(&milk), "notes are searched");
        assert_eq!(ids(outline.search("#shop -milk".into(), 10)), std::slice::from_ref(&bread));
        assert_eq!(outline.search("#shop".into(), 1).len(), 1, "at most the limit");
        assert!(outline.search("  ".into(), 10).is_empty());
        let hit = &outline.search("has:note".into(), 10)[0];
        assert_eq!((hit.text.as_str(), hit.path[0].text.as_str()), ("buy milk #shop", "Errands"));
    }

    #[test]
    fn move_to_puts_a_bullet_last_under_another() {
        let outline = Outline::in_memory();
        let inbox = add(&outline, None, 0, "Inbox");
        let task = add(&outline, Some(&inbox), 0, "a task");
        let project = add(&outline, None, 1, "Project");
        add(&outline, Some(&project), 0, "first step");
        outline.set_collapsed(project.clone(), true).unwrap();
        // The picker leaves out the bullet and what is under it.
        let found: Vec<String> = outline.quick_find("".into(), vec![inbox.clone()], 10).into_iter().map(|b| b.text).collect();
        assert_eq!(found.len(), 2);
        assert!(!found.contains(&"Inbox".to_string()) && !found.contains(&"a task".to_string()));
        assert_eq!(outline.quick_find("proj".into(), vec![], 10)[0].id, project);
        let out = outline.move_to(task.clone(), Some(project.clone())).unwrap();
        assert!(out.structure_changed);
        assert_eq!(
            texts(&outline),
            [("Inbox".into(), 0), ("Project".into(), 0), ("first step".into(), 1), ("a task".into(), 1)],
            "last, unfolded"
        );
        assert!(outline.move_to(project.clone(), Some(task.clone())).unwrap().is_empty(), "not under itself");
        outline.move_to(task.clone(), None).unwrap();
        assert_eq!(texts(&outline).last(), Some(&("a task".to_string(), 0)), "to the end of Home");
        outline.undo();
        outline.undo();
        assert_eq!(texts(&outline)[1], ("a task".into(), 1), "each move an undo step");
        assert!(matches!(outline.move_to(task, Some("nope".into())), Err(FlowieError::NoSuchBullet { .. })));
    }

    #[test]
    fn completed_rows_hide_and_the_setting_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite").to_string_lossy().into_owned();
        let outline = Outline::open(db.clone()).unwrap();
        let a = add(&outline, None, 0, "a");
        let b = add(&outline, Some(&a), 0, "b");
        let c = add(&outline, Some(&b), 0, "c");
        add(&outline, None, 1, "d");
        assert!(!outline.hide_completed());
        outline.set_hide_completed(true);
        let out = outline.set_completed(b.clone(), true).unwrap();
        assert!(out.structure_changed, "the row goes from the page");
        assert_eq!(texts(&outline), [("a".into(), 0), ("d".into(), 0)], "with what is under it");
        assert!(outline.rows(vec![b.clone(), c.clone()], None).unwrap().is_empty());
        // Typing never changes what shows.
        assert!(!outline.set_text(a.clone(), "a!".into()).unwrap().structure_changed);
        // Undoing the completion brings it back as rows coming.
        outline.undo();
        assert!(outline.undo().structure_changed);
        assert_eq!(outline.page(None).unwrap().len(), 4);
        // Return at the start of the row after a hidden one joins the row shown above.
        outline.set_completed(b.clone(), true).unwrap();
        let d = outline.page(None).unwrap()[1].id.clone();
        let joined = outline.backspace_at_start(d, None).unwrap().focus.unwrap();
        assert_eq!(joined.id, a);
        outline.flush().unwrap();
        drop(outline);
        let outline = Outline::open(db).unwrap();
        assert!(outline.hide_completed(), "kept for this device");
        outline.set_hide_completed(false);
        assert_eq!(outline.page(None).unwrap().len(), 3);
    }

    #[test]
    fn deleted_bullets_list_and_restore() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "keep");
        let b = add(&outline, None, 1, "**gone** for now");
        add(&outline, Some(&b), 0, "child");
        outline.delete(b.clone()).unwrap();
        let trash = outline.trashed();
        assert_eq!(trash.len(), 1);
        assert_eq!((trash[0].id.as_str(), trash[0].text.as_str(), trash[0].count), (b.as_str(), "gone for now", 2));
        assert!(trash[0].deleted_at > 0);
        assert!(matches!(outline.bullet(b.clone()), Err(FlowieError::NoSuchBullet { .. })));
        let out = outline.restore(b.clone()).unwrap();
        assert!(out.structure_changed);
        assert_eq!(out.focus, Some(FocusView { id: b.clone(), start: 16, end: 16 }));
        assert_eq!(texts(&outline), [("keep".into(), 0), ("**gone** for now".into(), 0), ("child".into(), 1)]);
        assert!(outline.trashed().is_empty());
        assert!(matches!(outline.restore(a), Err(FlowieError::NoSuchBullet { .. })), "not in the Trash");
        assert!(outline.undo().structure_changed);
        assert_eq!(outline.trashed().len(), 1, "undo deletes it again");
    }

    #[test]
    fn links_and_paths_name_bullets() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "Work [2026]");
        let b = add(&outline, Some(&a), 0, "");
        assert_eq!(outline.link(a.clone(), "Untitled".into()).unwrap(), format!("[Work 2026](flowie:{a})"));
        assert_eq!(outline.link(b.clone(), "无标题".into()).unwrap(), format!("[无标题](flowie:{b})"));
        let info = outline.bullet(b).unwrap();
        assert_eq!(info.path, [Crumb { id: a.clone(), text: "Work [2026]".into() }]);
        assert!(outline.bullet(a).unwrap().path.is_empty(), "Home isn't on the path");
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
