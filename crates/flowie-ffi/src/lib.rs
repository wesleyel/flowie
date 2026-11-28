//! Flowie for native apps, through UniFFI (wiki: iOS-Cross-Platform §3.3).
//!
//! The iOS app (M3) and the CloudKit bridge (P4) call this: an open outline, the rows of a
//! page, the edits, and sync with a self-hosted server or a native engine. Swift keeps no data
//! of its own and merges nothing: the outline is `flowie_doc::api`, the facade the browser
//! (`flowie-wasm`) binds too, and this crate only hands its types to Swift and adds the native
//! sync (threads, CloudKit). Ids cross the boundary as 32 hex digits, like on the wire.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use flowie_doc::api::{
    self, BacklinkView, BulletView, Crumb, DropPlace, EditOutcome, FocusView, FormattedText, ImageView, IndexedPage, MarkupRun, RowView,
    RunKind, TextFormat, TrashView,
};
use flowie_doc::{FileTaken, NativeSaved, NativeSync, SyncDriver, SyncPhase, Wire, WireFile};
use flowie_sync::http::HttpBackend;

uniffi::setup_scaffolding!();

/// What can go wrong across the boundary (`flowie_doc::api::Error`).
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

impl From<api::Error> for FlowieError {
    fn from(error: api::Error) -> Self {
        match error {
            api::Error::Storage { message } => FlowieError::Storage { message },
            api::Error::TooNew { found, current } => FlowieError::TooNew { found, current },
            api::Error::UpgradeFailed { backup, message } => FlowieError::UpgradeFailed { backup, message },
            api::Error::BackupFailed { message } => FlowieError::BackupFailed { message },
            api::Error::NoSuchBullet { id } => FlowieError::NoSuchBullet { id },
            api::Error::NotADate { date } => FlowieError::NotADate { date },
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

// The facade's records, handed to Swift as they are (UniFFI remote types: these declarations
// must match `flowie_doc::api`'s field for field; see there for what each means).

#[uniffi::remote(Record)]
pub struct RowView {
    pub id: String,
    pub depth: u16,
    pub text: String,
    pub note: String,
    pub kind: String,
    pub completed: bool,
    pub collapsed: bool,
    pub has_children: bool,
    pub image: Option<ImageView>,
}

#[uniffi::remote(Record)]
pub struct ImageView {
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[uniffi::remote(Record)]
pub struct MarkupRun {
    pub start: u32,
    pub end: u32,
    pub kind: RunKind,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub highlight: bool,
    pub color: Option<u8>,
}

#[uniffi::remote(Enum)]
pub enum RunKind {
    Text,
    Marker,
    Code,
    Math { display: bool },
    Link { url: String, bullet: Option<String> },
    Tag,
    Mention,
    Date,
}

#[uniffi::remote(Enum)]
pub enum TextFormat {
    Bold,
    Italic,
    Strikethrough,
    Code,
    Highlight,
    Link,
}

#[uniffi::remote(Record)]
pub struct FormattedText {
    pub text: String,
    pub start: u32,
    pub end: u32,
}

#[uniffi::remote(Record)]
pub struct EditOutcome {
    pub changed_ids: Vec<String>,
    pub structure_changed: bool,
    pub focus: Option<FocusView>,
}

#[uniffi::remote(Record)]
pub struct FocusView {
    pub id: String,
    pub start: u32,
    pub end: u32,
}

#[uniffi::remote(Record)]
pub struct DropPlace {
    pub parent: Option<String>,
    pub index: u32,
    pub depth: u16,
}

#[uniffi::remote(Record)]
pub struct BulletView {
    pub id: String,
    pub text: String,
    pub completed: bool,
    pub starred: bool,
    pub has_children: bool,
    pub path: Vec<Crumb>,
}

#[uniffi::remote(Record)]
pub struct Crumb {
    pub id: String,
    pub text: String,
}

#[uniffi::remote(Record)]
pub struct BacklinkView {
    pub id: String,
    pub target: Option<String>,
    pub text: String,
    pub completed: bool,
    pub mirror: bool,
    pub path: Vec<Crumb>,
}

#[uniffi::remote(Record)]
pub struct IndexedPage {
    pub id: String,
    pub title: String,
    pub path: Vec<String>,
    pub content: String,
    pub starred: bool,
    pub day: Option<String>,
    pub completed: bool,
    pub digest: u64,
}

#[uniffi::remote(Record)]
pub struct TrashView {
    pub id: String,
    pub text: String,
    pub count: u32,
    pub deleted_at: i64,
}

/// The markup runs of a bullet's text (`flowie_doc::api::markup`).
#[uniffi::export]
pub fn markup(text: String) -> Vec<MarkupRun> {
    api::markup(&text)
}

/// Format the selection `start..end` (UTF-16) of `text` as the desktop does
/// (`flowie_doc::api::toggle_format`).
#[uniffi::export]
pub fn toggle_format(text: String, start: u32, end: u32, format: TextFormat) -> FormattedText {
    api::toggle_format(&text, start, end, format)
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
    /// The envelope as JSON (per-node-sync.md §4.1); stored in one encrypted field. Empty for
    /// an image file.
    pub envelope: String,
    /// An image file (CloudKit: a `Blob` record) instead of a record; `id` is its name there.
    pub file: Option<WireFileRecord>,
}

/// What came of records a native engine fetched ([`Outline::on_fetched`]).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FetchedRecords {
    /// The outline changed, or an image file came that a row shows (its `image` now has a
    /// size): the app re-reads its page.
    pub changed: bool,
    /// What became of each image file among them, for the sync log.
    pub files: Vec<FetchedFile>,
}

/// What became of one fetched image file: `kept`, `had it already`, or why it was dropped.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FetchedFile {
    pub id: String,
    pub kept: bool,
    pub verdict: String,
}

/// An image file as a native sync engine moves it (CloudKit: a `Blob` record's CKAsset and the
/// fields beside it): where the file is (in the image folder, to send; the engine's download,
/// when fetched), its size in pixels (0 when not known) and its MIME type.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WireFileRecord {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub mime: String,
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

impl From<Wire> for WireRecord {
    fn from(wire: Wire) -> Self {
        let file = wire.file.map(|f| WireFileRecord { path: f.path, width: f.width, height: f.height, mime: f.mime });
        WireRecord { id: wire.id, etag: wire.etag, envelope: wire.envelope, file }
    }
}

impl From<WireRecord> for Wire {
    fn from(record: WireRecord) -> Self {
        let file = record.file.map(|f| WireFile { path: f.path, width: f.width, height: f.height, mime: f.mime });
        Wire { id: record.id, etag: record.etag, envelope: record.envelope, file }
    }
}

impl From<SaveOutcome> for NativeSaved {
    fn from(outcome: SaveOutcome) -> Self {
        match outcome {
            SaveOutcome::Saved { id, etag } => NativeSaved::Saved { id, etag },
            SaveOutcome::Conflict { server } => NativeSaved::Conflict { server: server.into() },
            SaveOutcome::Failed { id, error } => NativeSaved::Failed { id, error },
        }
    }
}

struct State {
    outline: api::Outline,
    sync: Option<SyncDriver>,
    /// The engine a native sync engine drives (CloudKit), between `begin_native_sync` and
    /// `end_native_sync`.
    native: Option<NativeSync>,
}

/// An open outline: `flowie_doc::api::Outline` (see there for what each call does), behind a
/// lock for Swift, with native sync on top.
#[derive(uniffi::Object)]
pub struct Outline {
    state: Mutex<State>,
}

impl Outline {
    fn new(outline: api::Outline) -> Arc<Self> {
        Arc::new(Outline { state: Mutex::new(State { outline, sync: None, native: None }) })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` on the facade.
    fn with<T>(&self, f: impl FnOnce(&mut api::Outline) -> T) -> T {
        f(&mut self.lock().outline)
    }
}

#[uniffi::export]
impl Outline {
    /// Open (or create) the outline database at `db`, its images beside it.
    #[uniffi::constructor]
    pub fn open(db: String) -> Result<Arc<Self>, FlowieError> {
        Ok(Outline::new(api::Outline::open(Path::new(&db))?))
    }

    /// An outline kept only in memory (previews and tests).
    #[uniffi::constructor]
    pub fn in_memory() -> Arc<Self> {
        Outline::new(api::Outline::in_memory())
    }

    pub fn page(&self, root: Option<String>) -> Result<Vec<RowView>, FlowieError> {
        Ok(self.with(|o| o.page(root.as_deref()))?)
    }

    pub fn page_packed(&self, root: Option<String>) -> Result<Vec<u8>, FlowieError> {
        Ok(self.with(|o| o.page_packed(root.as_deref()))?)
    }

    pub fn page_levels(&self, root: Option<String>) -> Result<u8, FlowieError> {
        Ok(self.with(|o| o.page_levels(root.as_deref()))?)
    }

    pub fn rows(&self, ids: Vec<String>, root: Option<String>) -> Result<Vec<RowView>, FlowieError> {
        Ok(self.with(|o| o.rows(&ids, root.as_deref()))?)
    }

    pub fn create(&self, parent: Option<String>, index: u32, text: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.create(parent.as_deref(), index, &text))?)
    }

    pub fn set_text(&self, id: String, text: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.set_text(&id, &text))?)
    }

    pub fn set_note(&self, id: String, note: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.set_note(&id, &note))?)
    }

    pub fn set_completed(&self, id: String, completed: bool) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.set_completed(&id, completed))?)
    }

    pub fn indent(&self, id: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.indent(&id))?)
    }

    pub fn outdent(&self, id: String, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.outdent(&id, root.as_deref()))?)
    }

    pub fn move_up(&self, id: String, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.move_up(&id, root.as_deref()))?)
    }

    pub fn move_down(&self, id: String, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.move_down(&id, root.as_deref()))?)
    }

    pub fn enter(&self, id: String, start: u32, end: u32, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.enter(&id, start, end, root.as_deref()))?)
    }

    pub fn backspace_at_start(&self, id: String, root: Option<String>) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.backspace_at_start(&id, root.as_deref()))?)
    }

    pub fn delete(&self, id: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.delete(&id))?)
    }

    pub fn undo(&self) -> EditOutcome {
        self.with(|o| o.undo())
    }

    pub fn redo(&self) -> EditOutcome {
        self.with(|o| o.redo())
    }

    pub fn can_undo(&self) -> bool {
        self.with(|o| o.can_undo())
    }

    pub fn can_redo(&self) -> bool {
        self.with(|o| o.can_redo())
    }

    pub fn set_collapsed(&self, id: String, collapsed: bool) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.set_collapsed(&id, collapsed))?)
    }

    pub fn expand_to_level(&self, root: Option<String>, levels: u32) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.expand_to_level(root.as_deref(), levels))?)
    }

    pub fn move_to(&self, id: String, parent: Option<String>) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.move_to(&id, parent.as_deref()))?)
    }

    pub fn drop_place(
        &self,
        root: Option<String>,
        dragged: Option<String>,
        above: Option<String>,
        above_depth: u16,
        below_depth: Option<u16>,
        depth: u16,
    ) -> Result<Option<DropPlace>, FlowieError> {
        Ok(self.with(|o| o.drop_place(root.as_deref(), dragged.as_deref(), above.as_deref(), above_depth, below_depth, depth))?)
    }

    pub fn drop_bullet(&self, id: String, place: DropPlace) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.drop_bullet(&id, &place))?)
    }

    pub fn hide_completed(&self) -> bool {
        self.with(|o| o.hide_completed())
    }

    pub fn set_hide_completed(&self, hide: bool) {
        self.with(|o| o.set_hide_completed(hide));
    }

    pub fn bullet(&self, id: String) -> Result<BulletView, FlowieError> {
        Ok(self.with(|o| o.bullet(&id))?)
    }

    pub fn backlinks(&self, id: String) -> Result<Vec<BacklinkView>, FlowieError> {
        Ok(self.with(|o| o.backlinks(&id))?)
    }

    pub fn index_pages(&self) -> Vec<IndexedPage> {
        self.with(|o| o.index_pages())
    }

    pub fn starred(&self) -> Vec<BulletView> {
        self.with(|o| o.starred())
    }

    pub fn saved_searches(&self) -> Vec<String> {
        self.with(|o| o.saved_searches())
    }

    pub fn set_starred(&self, id: String, starred: bool) -> Result<(), FlowieError> {
        Ok(self.with(|o| o.set_starred(&id, starred))?)
    }

    pub fn search(&self, query: String, limit: u32) -> Vec<BulletView> {
        self.with(|o| o.search(&query, limit))
    }

    pub fn quick_find(&self, query: String, exclude: Vec<String>, limit: u32) -> Vec<BulletView> {
        self.with(|o| o.quick_find(&query, &exclude, limit))
    }

    pub fn link(&self, id: String, untitled: String) -> Result<String, FlowieError> {
        Ok(self.with(|o| o.link(&id, &untitled))?)
    }

    pub fn trashed(&self) -> Vec<TrashView> {
        self.with(|o| o.trashed())
    }

    pub fn restore(&self, id: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.restore(&id))?)
    }

    pub fn journal_day(&self, date: String) -> Result<Option<String>, FlowieError> {
        Ok(self.with(|o| o.journal_day(&date))?)
    }

    pub fn journal_date(&self, id: String) -> Option<String> {
        self.with(|o| o.journal_date(&id))
    }

    pub fn open_day(&self, date: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.open_day(&date))?)
    }

    pub fn journal_append(&self, date: String, text: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.journal_append(&date, &text))?)
    }

    pub fn journal_share(&self, date: String, key: String, text: String, url: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.journal_share(&date, &key, &text, &url))?)
    }

    pub fn journal_capture(&self, date: String) -> Result<EditOutcome, FlowieError> {
        Ok(self.with(|o| o.journal_capture(&date))?)
    }

    pub fn set_editing(&self, id: Option<String>) -> Result<(), FlowieError> {
        Ok(self.with(|o| o.set_editing(id.as_deref()))?)
    }

    pub fn flush(&self) -> Result<(), FlowieError> {
        Ok(self.with(|o| o.flush())?)
    }

    pub fn is_empty(&self) -> bool {
        self.with(|o| o.is_empty())
    }

    /// Sync with a self-hosted server (`flowie-server`) from now on, replacing any sync
    /// running. `observer` hears when to call [`pump_sync`](Self::pump_sync).
    pub fn start_sync(&self, server: String, token: String, observer: Arc<dyn SyncObserver>) {
        let mut state = self.lock();
        state.sync = None;
        let server = server.trim().trim_end_matches('/').to_string();
        state.outline.ws.use_remote(&format!("{server}#outline"));
        let token = token.trim().to_string();
        let driver =
            SyncDriver::start(&state.outline.ws, move || Box::new(HttpBackend::new(&server, "outline", &token)), move || observer.wake());
        state.sync = Some(driver);
        let State { outline, sync, .. } = &mut *state;
        if let Some(driver) = sync {
            driver.pump(&mut outline.ws);
        }
    }

    pub fn stop_sync(&self) {
        self.lock().sync = None;
    }

    /// Send what changed here and take what came in. Returns whether the outline changed, so
    /// the app re-reads its page.
    pub fn pump_sync(&self) -> bool {
        let mut state = self.lock();
        let State { outline, sync, .. } = &mut *state;
        sync.as_mut().is_some_and(|driver| driver.pump(&mut outline.ws))
    }

    /// Let a native sync engine (CloudKit's CKSyncEngine) drive sync from now on, keeping its
    /// place under `remote` (for example `icloud`). It schedules the calls below itself.
    /// Returns whether this is a fresh start with that backend (`Workspace::use_remote`):
    /// everything here waits to be sent, and the native engine should drop the place it kept
    /// in the backend's changes and fetch from the beginning.
    pub fn begin_native_sync(&self, remote: String) -> bool {
        let mut state = self.lock();
        state.sync = None;
        let fresh = state.outline.ws.use_remote(&remote);
        state.native = Some(NativeSync::new());
        fresh
    }

    pub fn end_native_sync(&self) {
        self.lock().native = None;
    }

    /// Send every record again: the backend lost them (the user deleted the app's iCloud data,
    /// or another account signed in).
    pub fn resend_all(&self) {
        let mut state = self.lock();
        let State { outline, native, .. } = &mut *state;
        match native {
            Some(native) => native.resend_all(&mut outline.ws),
            None => outline.ws.resend_all(),
        }
    }

    /// Ids with something to send, for the native engine's list of pending saves (image files
    /// whose save failed included).
    pub fn unsent_ids(&self, limit: u32) -> Vec<String> {
        let state = self.lock();
        match &state.native {
            Some(native) => native.to_send(&state.outline.ws, limit as usize),
            None => NativeSync::unsent_ids(&state.outline.ws, limit as usize),
        }
    }

    /// The next records to save, at most `limit`, each from the version last seen.
    pub fn next_saves(&self, limit: u32) -> Vec<WireRecord> {
        let mut state = self.lock();
        let State { outline, native, .. } = &mut *state;
        let Some(native) = native else { return Vec::new() };
        native.next_saves(&mut outline.ws, limit as usize).into_iter().map(WireRecord::from).collect()
    }

    /// The native engine answered saves.
    pub fn on_saved(&self, outcomes: Vec<SaveOutcome>) {
        let mut state = self.lock();
        let State { outline, native, .. } = &mut *state;
        let Some(native) = native else { return };
        native.on_saved(&mut outline.ws, outcomes.into_iter().map(NativeSaved::from).collect());
    }

    /// Records the native engine fetched, and where it is now (CloudKit keeps its own change
    /// token: pass anything that marks progress, or an empty string). Says whether the outline
    /// changed, and what became of each image file.
    pub fn on_fetched(&self, records: Vec<WireRecord>, cursor: String) -> FetchedRecords {
        let mut state = self.lock();
        let State { outline, native, .. } = &mut *state;
        let records: Vec<Wire> = records.into_iter().map(Wire::from).collect();
        let Some(native) = native else {
            let files = records.iter().filter(|r| r.file.is_some());
            let files = files.map(|r| FetchedFile { id: r.id.clone(), kept: false, verdict: "dropped: sync isn't running".into() });
            return FetchedRecords { changed: false, files: files.collect() };
        };
        let fetched = native.on_fetched(&mut outline.ws, &records, &cursor);
        let files = fetched.files.into_iter().map(|(id, taken)| FetchedFile {
            id,
            kept: matches!(taken, FileTaken::Kept | FileTaken::Had),
            verdict: taken.to_string(),
        });
        FetchedRecords { changed: fetched.changed, files: files.collect() }
    }

    /// The names of image files rows here show but this device doesn't have, at most
    /// `limit`: the native engine asks the backend for them by name.
    pub fn missing_files(&self, limit: u32) -> Vec<String> {
        NativeSync::missing_files(&self.lock().outline.ws, limit as usize)
    }

    pub fn sync_status(&self) -> SyncStatus {
        let state = self.lock();
        match &state.sync {
            None => SyncStatus::Off,
            Some(driver) => match driver.phase(&state.outline.ws) {
                SyncPhase::Syncing => SyncStatus::Syncing,
                SyncPhase::Synced => SyncStatus::Synced,
                SyncPhase::Offline(error) => SyncStatus::Offline { error },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use flowie_core::{BlockKind, ROOT};

    use super::*;

    /// Add a bullet; its id.
    fn add(outline: &Outline, parent: Option<&str>, index: u32, text: &str) -> String {
        outline.create(parent.map(String::from), index, text.into()).unwrap().focus.unwrap().id
    }

    /// The binding hands the facade's results and errors through (the facade's own tests are
    /// in `flowie_doc::api`).
    #[test]
    fn calls_reach_the_facade_and_errors_cross() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "first");
        add(&outline, Some(&a), 0, "child");
        assert_eq!(outline.page(None).unwrap().len(), 2);
        assert!(matches!(outline.set_text("nope".into(), "x".into()), Err(FlowieError::NoSuchBullet { .. })));
        assert!(matches!(outline.open_day("tomorrow".into()), Err(FlowieError::NotADate { .. })));
        assert!(outline.undo().structure_changed);
        assert_eq!(markup("**b**".into()).len(), 3);
        assert_eq!(toggle_format("a".into(), 0, 1, TextFormat::Bold).text, "**a**");
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
            let results = server.modify(saves.iter().filter_map(|w| Wire::from(w.clone()).to_envelope()).collect());
            let outcomes = results
                .into_iter()
                .map(|r| match r {
                    flowie_sync::SaveResult::Saved { id, etag } => SaveOutcome::Saved { id, etag },
                    flowie_sync::SaveResult::Conflict { server } => SaveOutcome::Conflict { server: Wire::from_envelope(server).into() },
                    flowie_sync::SaveResult::Failed { id, error } => SaveOutcome::Failed { id, error },
                })
                .collect();
            outline.on_saved(outcomes);
        }
        let batch = server.changes(cursor.as_deref(), 500);
        *cursor = Some(batch.cursor.clone());
        outline.on_fetched(batch.records.into_iter().map(|e| Wire::from_envelope(e).into()).collect(), batch.cursor);
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
    fn an_image_file_synced_natively_gives_its_row_a_size() {
        let dir = tempfile::tempdir().unwrap();
        let open = |name: &str| Outline::open(dir.path().join(name).to_string_lossy().into_owned()).unwrap();
        let (mac, iphone) = (open("mac.sqlite"), open("iphone.sqlite"));
        mac.begin_native_sync("icloud".into());
        iphone.begin_native_sync("icloud".into());
        let gif = b"GIF89a\x06\x00\x04\x00".to_vec();
        let id = {
            let mut state = mac.lock();
            let ws = &mut state.outline.ws;
            let info = ws.image_dir.as_ref().unwrap().put(&gif).unwrap();
            let name = info.name.clone();
            ws.record_blob(info);
            let id = ws.tree.create_at(ROOT, 0, "");
            let mut node = ws.tree.node(id).clone();
            node.kind = BlockKind::Image;
            node.image = name;
            ws.tree.put(node);
            ws.commit_with_imports(None, None, None);
            id.to_string()
        };
        let saves = mac.next_saves(10);
        assert_eq!(saves.len(), 2);
        let file = saves[0].file.clone().expect("the file goes first");
        assert_eq!((file.width, file.height, file.mime.as_str()), (6, 4, "image/gif"));
        assert!(saves[1].file.is_none() && saves[1].id == id);
        mac.on_saved(saves.iter().map(|w| SaveOutcome::Saved { id: w.id.clone(), etag: "e".into() }).collect());
        assert!(mac.unsent_ids(10).is_empty());

        // CloudKit hands the file over from where it downloaded it, after the row.
        let download = dir.path().join("asset");
        std::fs::write(&download, &gif).unwrap();
        let mut row = saves[1].clone();
        row.etag = Some("e".into());
        assert!(iphone.on_fetched(vec![row], "icloud".into()).changed);
        let before = iphone.page(None).unwrap()[0].image.clone().unwrap();
        assert_eq!((before.width, before.height), (0, 0), "a placeholder until the file comes");
        let blob = WireRecord { file: Some(WireFileRecord { path: download.to_string_lossy().into_owned(), ..file }), ..saves[0].clone() };
        let fetched = iphone.on_fetched(vec![blob], "icloud".into());
        assert!(fetched.changed, "the row is shown again");
        assert_eq!((fetched.files.len(), fetched.files[0].kept, fetched.files[0].verdict.as_str()), (1, true, "kept"));
        let after = iphone.page(None).unwrap()[0].image.clone().unwrap();
        assert_eq!((after.width, after.height), (6, 4));
        assert!(std::path::Path::new(after.path.as_deref().unwrap()).is_file());
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
}
