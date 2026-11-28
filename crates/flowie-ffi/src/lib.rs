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
use flowie_core::{flatten_mapped, journal, markup, BlockKind, Caret, NodeId, Row, Tree, ViewOptions, ROOT};
use flowie_doc::{find, utf16, FileTaken, NativeSaved, NativeSync, Outcome, SyncDriver, SyncPhase, Wire, WireFile, Workspace};
use flowie_sync::http::HttpBackend;

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
    /// The picture of an image row (`kind` is `image`; `text` is its caption).
    pub image: Option<ImageView>,
}

/// An image row's picture: where its file is and how big it is, known before the file is read
/// (the `blobs` table), so the row's height is right before the picture loads.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ImageView {
    /// Where the file is in the image folder beside the database (content-addressed, so it
    /// never changes). It may not be there yet: synced from another device without its file.
    /// None for an outline without an image folder (in memory).
    pub path: Option<String>,
    /// Its size in pixels; 0 when not known.
    pub width: u32,
    pub height: u32,
}

/// One stretch of a bullet's text and how it shows, as the desktop parses it
/// (`flowie_core::markup::parse`). Offsets are UTF-16 code units; the runs cover the text in
/// order, without gaps.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MarkupRun {
    pub start: u32,
    pub end: u32,
    pub kind: RunKind,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub highlight: bool,
    /// Text color, an index into `red`, `orange`, `yellow`, `green`, `blue`, `purple`, `gray`.
    pub color: Option<u8>,
}

/// What a run is.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum RunKind {
    Text,
    /// Syntax (`**`, `[`, `](url)`, `{red}`, a `\` escape…): hidden when rendered.
    Marker,
    /// Inside backticks (the backticks are markers).
    Code,
    /// A whole `$…$` or `$$…$$`, delimiters included.
    Math {
        display: bool,
    },
    /// A link's label, or a bare web address. `bullet`: the bullet a `flowie:ID` link points to.
    Link {
        url: String,
        bullet: Option<String>,
    },
    Tag,
    Mention,
    /// An ISO date (`2026-11-17`).
    Date,
}

/// The markup runs of a bullet's text, for showing it formatted when it isn't being edited
/// (wiki: iOS-Cross-Platform §2.3); the row being edited shows the text as stored.
#[uniffi::export]
pub fn markup(text: String) -> Vec<MarkupRun> {
    utf16::markup_runs(&text)
        .map(|(range, run)| {
            let kind = match run.kind {
                markup::Kind::Text => RunKind::Text,
                markup::Kind::Marker => RunKind::Marker,
                markup::Kind::Code => RunKind::Code,
                markup::Kind::Math { display, .. } => RunKind::Math { display },
                markup::Kind::Link { url } => {
                    let bullet = url.strip_prefix("flowie:").and_then(|id| id.parse::<NodeId>().ok()).map(|id| id.to_string());
                    RunKind::Link { url, bullet }
                }
                markup::Kind::Tag => RunKind::Tag,
                markup::Kind::Mention => RunKind::Mention,
                markup::Kind::Date => RunKind::Date,
            };
            MarkupRun {
                start: range.start as u32,
                end: range.end as u32,
                kind,
                bold: run.bold,
                italic: run.italic,
                strike: run.strike,
                highlight: run.highlight,
                color: run.color,
            }
        })
        .collect()
}

/// A format command of the text being typed in: the desktop's Bold (⌘B), Italic (⌘I),
/// Strikethrough, Code (⌘E), Highlight (⇧⌘Y) and Link (⌘K).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TextFormat {
    Bold,
    Italic,
    Strikethrough,
    Code,
    Highlight,
    Link,
}

/// A bullet's text after a format command, and what is selected in it (UTF-16 offsets).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FormattedText {
    pub text: String,
    pub start: u32,
    pub end: u32,
}

/// Format the selection `start..end` (UTF-16) of `text` as the desktop does
/// (`markup::toggle_wrap`, `markup::insert_link`): wrapped in the markup and still selected, or
/// unwrapped when it already is; at a caret, the pair with the caret between. Link makes the
/// selection (or `link`) a link's label and selects its address.
#[uniffi::export]
pub fn toggle_format(text: String, start: u32, end: u32, format: TextFormat) -> FormattedText {
    let (a, b) = (utf16::from_utf16(&text, start as usize), utf16::from_utf16(&text, end as usize));
    let sel = a.min(b)..a.max(b);
    let edit = match format {
        TextFormat::Bold => markup::toggle_wrap(&text, sel, "**", "**"),
        TextFormat::Italic => markup::toggle_wrap(&text, sel, "*", "*"),
        TextFormat::Strikethrough => markup::toggle_wrap(&text, sel, "~~", "~~"),
        TextFormat::Code => markup::toggle_wrap(&text, sel, "`", "`"),
        TextFormat::Highlight => markup::toggle_wrap(&text, sel, "==", "=="),
        TextFormat::Link => markup::insert_link(&text, sel),
    };
    let text = edit.apply(&text);
    let start = utf16::to_utf16(&text, edit.sel.start) as u32;
    let end = utf16::to_utf16(&text, edit.sel.end) as u32;
    FormattedText { text, start, end }
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

/// Where a dragged bullet lands on a page (`flowie_doc::drop::place`): `index` among
/// `parent`'s children (none: Home's), shown `depth` levels into the page.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DropPlace {
    pub parent: Option<String>,
    pub index: u32,
    pub depth: u16,
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

/// A bullet elsewhere pointing at a page, as the desktop's footer lists them under the rows:
/// one whose text links to it (`[label](flowie:ID)`), or a mirror of it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct BacklinkView {
    /// The linking bullet, or the mirror.
    pub id: String,
    /// The page tapping it opens, as on the desktop: the linking bullet's own page; for a
    /// mirror, the page it sits on (None: Home).
    pub target: Option<String>,
    /// Its text as stored (a mirror's: its source's), to show rendered as rows are.
    pub text: String,
    pub completed: bool,
    /// Whether it is a mirror (listed under "Mirrored in") rather than a link.
    pub mirror: bool,
    /// The bullets above it, from the top (Home is not in it).
    pub path: Vec<Crumb>,
}

/// A page for the system's search (Spotlight): a bullet with children, a starred bullet, or a
/// day of the journal. `digest` changes when anything shown or searched changes, so the app
/// indexes again only the pages whose digest it hasn't indexed.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct IndexedPage {
    pub id: String,
    /// Its text as plain text, without markup.
    pub title: String,
    /// The bullets above it, from the top, as plain text (Home is not in it).
    pub path: Vec<String>,
    /// The start of its children's text, one a line: found by search, not shown.
    pub content: String,
    pub starred: bool,
    /// The date (YYYY-MM-DD) of a journal page.
    pub day: Option<String>,
    pub completed: bool,
    pub digest: u64,
}

/// How much of a page's children's text goes into the index (characters).
const INDEXED_CONTENT: usize = 600;

/// A stable hash (FNV-1a), the same in every build and on every run, unlike std's.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn add(&mut self, bytes: &[u8]) {
        for b in bytes.iter().chain([&0xff]) {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// Every page to index (`IndexedPage`), in outline order. Mirrors are left out (their source
/// is indexed), and so are bullets without text, except the journal's days.
fn index_pages(tree: &Tree) -> Vec<IndexedPage> {
    let journal = journal::find_journal(tree);
    let mut out = Vec::new();
    // Depth-first, with the path (plain texts) to the bullet being looked at.
    let mut path: Vec<String> = Vec::new();
    let mut stack: Vec<(NodeId, usize)> = tree.children(ROOT).iter().rev().map(|&c| (c, 0)).collect();
    while let Some((id, depth)) = stack.pop() {
        path.truncate(depth);
        let node = tree.node(id);
        if node.mirror != ROOT {
            continue;
        }
        let title = markup::plain_text(&node.text);
        let children = tree.children(id);
        let starred = tree.is_starred(id);
        let day = (journal == Some(tree.parent(id)) && journal::is_date(&node.text)).then(|| node.text.clone());
        if (!children.is_empty() || starred || day.is_some()) && (!title.trim().is_empty() || day.is_some()) {
            let mut content = String::new();
            for &child in children {
                if content.chars().count() >= INDEXED_CONTENT {
                    break;
                }
                let text = plain(tree, child);
                if !text.trim().is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&text);
                }
            }
            content = content.chars().take(INDEXED_CONTENT).collect();
            let mut hash = Fnv::new();
            for part in path.iter().chain([&title, &content]) {
                hash.add(part.as_bytes());
            }
            hash.add(&[u8::from(starred), u8::from(node.completed)]);
            out.push(IndexedPage {
                id: id.to_string(),
                title: title.clone(),
                path: path.clone(),
                content,
                starred,
                day,
                completed: node.completed,
                digest: hash.0,
            });
        }
        path.push(title);
        stack.extend(children.iter().rev().map(|&c| (c, depth + 1)));
    }
    out
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
    ws: Workspace,
    sync: Option<SyncDriver>,
    /// The engine a native sync engine drives (CloudKit), between `begin_native_sync` and
    /// `end_native_sync`.
    native: Option<NativeSync>,
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

    fn page_rows(&mut self, root: NodeId) -> Vec<Row> {
        let (rows, virtuals) = flatten_mapped(&self.ws.tree, root, &self.view());
        self.ws.tree.add_virtuals(virtuals);
        rows
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
        has_children: tree.has_children(source.id),
        path: tree.ancestors(id).into_iter().map(|a| Crumb { id: a.to_string(), text: plain(tree, a) }).collect(),
    }
}

/// The date of a journal page: a bullet under the journal titled with a date.
fn journal_date(tree: &Tree, id: NodeId) -> Option<String> {
    let journal = journal::find_journal(tree)?;
    if !tree.contains(id) || tree.parent(id) != journal {
        return None;
    }
    let text = &tree.node(id).text;
    journal::is_date(text).then(|| text.clone())
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

fn row_view(ws: &Workspace, id: NodeId, depth: u16) -> Option<RowView> {
    let tree = &ws.tree;
    let node = tree.get(id)?;
    let source = tree.node(tree.source(id));
    // As the desktop's `row_image`: the row's own kind, its source's file.
    let image = (node.kind == BlockKind::Image && !source.image.is_empty()).then(|| {
        let blob = ws.blobs.get(&source.image);
        ImageView {
            path: ws.image_dir.as_ref().and_then(|dir| dir.path(&source.image)).map(|p| p.to_string_lossy().into_owned()),
            width: blob.map_or(0, |b| b.width),
            height: blob.map_or(0, |b| b.height),
        }
    });
    Some(RowView {
        id: id.to_string(),
        depth,
        text: source.text.clone(),
        note: source.note.clone(),
        kind: kind_name(node.kind),
        completed: source.completed,
        collapsed: node.collapsed,
        has_children: tree.has_children(source.id),
        image,
    })
}

/// Rows in [`Outline::page_packed`]'s format.
fn pack_rows(rows: &[RowView]) -> Vec<u8> {
    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    let text: usize = rows.iter().map(|r| r.id.len() + r.text.len() + r.note.len() + r.kind.len()).sum();
    let mut out = Vec::with_capacity(4 + rows.len() * 19 + text);
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        let image = row.image.as_ref();
        let flags = u8::from(row.completed)
            | u8::from(row.collapsed) << 1
            | u8::from(row.has_children) << 2
            | u8::from(image.is_some()) << 3
            | u8::from(image.is_some_and(|i| i.path.is_some())) << 4;
        out.extend_from_slice(&row.depth.to_le_bytes());
        out.push(flags);
        for s in [&row.id, &row.text, &row.note, &row.kind] {
            string(&mut out, s);
        }
        if let Some(image) = image {
            out.extend_from_slice(&image.width.to_le_bytes());
            out.extend_from_slice(&image.height.to_le_bytes());
            if let Some(path) = &image.path {
                string(&mut out, path);
            }
        }
    }
    out
}

/// How many levels of bullets the page at `root` can show (`0`, `1`, `2`, or `3` for three or
/// more), regardless of what is currently collapsed, leaving out completed bullets when
/// `hide_completed`. Used by the fold button: "Expand All" is offered only past two levels.
fn page_levels(tree: &Tree, root: NodeId, hide_completed: bool) -> u8 {
    let visible = |id: NodeId| !(hide_completed && tree.node(tree.source(id)).completed);
    let root_src = tree.source(root);
    let mut levels = 0u8;
    for &c0 in tree.children(root_src) {
        if !visible(c0) {
            continue;
        }
        levels = levels.max(1);
        let s0 = tree.source(c0);
        let m0 = (s0 != c0).then_some(s0);
        for &c1 in tree.children(s0) {
            if !visible(c1) {
                continue;
            }
            levels = levels.max(2);
            let s1 = tree.source(c1);
            if s1 != c1 && m0 == Some(s1) {
                continue;
            }
            if tree.children(s1).iter().any(|&c2| visible(c2)) {
                return 3;
            }
        }
    }
    levels
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
    /// Rows inside a mirror have virtual ids, which edits then take (as on the desktop: they
    /// change the mirrored bullets).
    pub fn page(&self, root: Option<String>) -> Result<Vec<RowView>, FlowieError> {
        let mut state = self.lock();
        let root = page_root(&state.ws.tree, root)?;
        Ok(state.page_rows(root).into_iter().filter_map(|row| row_view(&state.ws, row.id, row.depth)).collect())
    }

    /// [`page`](Self::page) as one byte buffer, which Swift reads into rows far faster than
    /// UniFFI reads the list field by field: 20 000 rows took 45–75 ms that way on iOS, which
    /// every structural edit paid (wiki: TODO-ios ios:24). Little-endian: the row count (u32),
    /// then per row its depth (u16), flags (u8: 1 completed, 2 collapsed, 4 has children, 8
    /// image, 16 image file), id, text, note and kind as strings (u32 byte length, UTF-8), and
    /// for an image its width and height (u32) and file path (a string, with flag 16).
    pub fn page_packed(&self, root: Option<String>) -> Result<Vec<u8>, FlowieError> {
        Ok(pack_rows(&self.page(root)?))
    }

    /// How many levels of bullets the page at `root` (none: Home) has when unfolded, capped at `3`
    /// (`0`: empty, `1`: flat rows, `2`: two levels, `3`: three or more), leaving out completed
    /// ones when [`hide_completed`](Self::hide_completed).
    pub fn page_levels(&self, root: Option<String>) -> Result<u8, FlowieError> {
        let state = self.lock();
        let root = page_root(&state.ws.tree, root)?;
        Ok(page_levels(&state.ws.tree, root, state.hide_completed))
    }

    /// The rows with these ids on the page at `root` (none: Home), as [`page`](Self::page)
    /// has them, in the order asked; ids not on the page are left out. For the rows an edit
    /// changed (`EditOutcome::changed_ids`), without reading the whole page.
    pub fn rows(&self, ids: Vec<String>, root: Option<String>) -> Result<Vec<RowView>, FlowieError> {
        let mut state = self.lock();
        let root = page_root(&state.ws.tree, root)?;
        // Rows inside a mirror (virtual ids) are found on the page.
        let mut page = None;
        Ok(ids
            .iter()
            .filter_map(|id| {
                let id = id.parse::<NodeId>().ok()?;
                let depth = depth_on_page(&state.ws.tree, root, id, state.hide_completed).or_else(|| {
                    let rows = page.get_or_insert_with(|| state.page_rows(root));
                    rows.iter().find(|row| row.id == id).map(|row| row.depth)
                })?;
                row_view(&state.ws, id, depth)
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

    /// Save the notes sheet as one undo step, including when opened just after typing in the body.
    pub fn set_note(&self, id: String, note: String) -> Result<EditOutcome, FlowieError> {
        self.edit_text(&id, |ws, id| {
            ws.tree.set_note(id, &note);
            ws.commit_with_imports(None, None, None);
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

    /// Whether there is a step to undo (the system's Undo shows enabled then).
    pub fn can_undo(&self) -> bool {
        self.lock().ws.history.can_undo()
    }

    /// Whether there is an undone step to redo.
    pub fn can_redo(&self) -> bool {
        self.lock().ws.history.can_redo()
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

    /// Show `levels` levels of the page at `root` (none: Home), as the desktop's Show N Levels
    /// and Expand All: bullets with children above that depth unfold, the ones at it fold.
    /// `1` folds every top-level row (Collapse All); `u32::MAX` unfolds everything. One undo
    /// step; folding doesn't sync (each device keeps its own).
    pub fn expand_to_level(&self, root: Option<String>, levels: u32) -> Result<EditOutcome, FlowieError> {
        let mut state = self.lock();
        let root = page_root(&state.ws.tree, root)?;
        let levels = if levels == u32::MAX { usize::MAX } else { levels as usize };
        Ok(tracked(&mut state, false, |ws| {
            let root = ws.tree.source(root);
            ws.tree.expand_to_level(root, levels);
            ws.commit_with_imports(None, None, None);
            None
        }))
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

    /// Where bullet `dragged` lands on the page at `root` (none: Home) when dropped in the gap
    /// below row `above` (at `above_depth`; none: the top of the page), over a row at
    /// `below_depth` (none: the end of the page), with the finger `depth` levels in: as on the
    /// desktop, no deeper than a child of the row above and no shallower than the row below.
    /// None when it would go under itself. `dragged` may come from another page; none for
    /// something new (text dropped from another app).
    pub fn drop_place(
        &self,
        root: Option<String>,
        dragged: Option<String>,
        above: Option<String>,
        above_depth: u16,
        below_depth: Option<u16>,
        depth: u16,
    ) -> Result<Option<DropPlace>, FlowieError> {
        let dragged = dragged.as_deref().map(|d| self.existing(d)).transpose()?;
        let above = above.as_deref().map(|a| self.existing(a)).transpose()?;
        let state = self.lock();
        let root = page_root(&state.ws.tree, root)?;
        let place = flowie_doc::drop::place(&state.ws.tree, root, above.map(|a| (a, above_depth)), below_depth, depth, dragged);
        Ok(place.map(|p| DropPlace { parent: (p.parent != ROOT).then(|| p.parent.to_string()), index: p.index as u32, depth: p.depth }))
    }

    /// Move bullet `id`, with what is under it, to `place` (from [`Outline::drop_place`]),
    /// unfolding its new parent; a folded bullet stays folded. Nothing changes when it would go
    /// under itself. An undo step.
    pub fn drop_bullet(&self, id: String, place: DropPlace) -> Result<EditOutcome, FlowieError> {
        let parent = place.parent.as_deref().map(|p| self.existing(p)).transpose()?.unwrap_or(ROOT);
        let to = flowie_doc::drop::Place { parent, index: place.index as usize, depth: place.depth };
        self.edit_row(&id, |ws, id| {
            ws.drop_bullet(id, to);
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

    /// What points at a bullet's page, as the desktop's footer lists it (`Tree::backlinks`, then
    /// `Tree::mirrors_of`), each in outline order: the bullets linking to it first, then its
    /// mirrors. A mirror's page points at its source's.
    pub fn backlinks(&self, id: String) -> Result<Vec<BacklinkView>, FlowieError> {
        let node = self.existing(&id)?;
        let state = self.lock();
        let tree = &state.ws.tree;
        let src = tree.source(node);
        let view = |id: NodeId, mirror: bool| {
            let source = tree.node(tree.source(id));
            let parent = tree.parent(id);
            BacklinkView {
                id: id.to_string(),
                target: if !mirror {
                    Some(id.to_string())
                } else if parent == ROOT {
                    None
                } else {
                    Some(parent.to_string())
                },
                text: source.text.clone(),
                completed: source.completed,
                mirror,
                path: tree.ancestors(id).into_iter().map(|a| Crumb { id: a.to_string(), text: plain(tree, a) }).collect(),
            }
        };
        let links = tree.backlinks(src).into_iter().map(|id| view(id, false));
        let mirrors = tree.mirrors_of(src).into_iter().map(|id| view(id, true));
        Ok(links.chain(mirrors).collect())
    }

    /// The pages for the system's search (Spotlight): bullets with children, starred bullets
    /// and the journal's days, not every bullet; each with a digest of what is indexed, so the
    /// app sends only what changed.
    pub fn index_pages(&self) -> Vec<IndexedPage> {
        index_pages(&self.lock().ws.tree)
    }

    /// The starred bullets, in the starred list's order.
    pub fn starred(&self) -> Vec<BulletView> {
        let state = self.lock();
        let tree = &state.ws.tree;
        tree.starred().map(|id| bullet_view(tree, id)).collect()
    }

    /// The saved searches (the `saved_searches` pref, which syncs), in their order: the iPad
    /// sidebar lists them, as the desktop's palette does.
    pub fn saved_searches(&self) -> Vec<String> {
        let state = self.lock();
        state.ws.pref("saved_searches").unwrap_or_default().split('\n').filter(|q| !q.trim().is_empty()).map(String::from).collect()
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

    /// The date (YYYY-MM-DD) bullet `id` is the journal page of, if it is one: a page under the
    /// journal titled with a date. The app opens it as that day, not as a plain bullet page.
    pub fn journal_date(&self, id: String) -> Option<String> {
        let node = parse(&id).ok()?;
        journal_date(&self.lock().ws.tree, node)
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

    /// "Add to Today's Journal": `text` at the end of the journal page for `date`
    /// (YYYY-MM-DD), one bullet a line, taken literally (`flowie_doc::capture::append`). The
    /// page is made as the desktop makes it, with the same id on every device; its lone blank
    /// line is replaced unless it is being typed in. One undo step; the focus is the end of the
    /// last bullet added, none (and nothing changed) when `text` is blank.
    pub fn journal_append(&self, date: String, text: String) -> Result<EditOutcome, FlowieError> {
        let date = day(&date)?;
        Ok(tracked(&mut self.lock(), false, |ws| ws.journal_append(date, &text)))
    }

    /// A share from the share extension (its inbox file `key`): `text`, one bullet a line, with
    /// `url` as a link on the first (`flowie_doc::capture::share`), at the end of the journal
    /// page for `date` (YYYY-MM-DD), made as [`journal_append`](Self::journal_append) makes it.
    /// A share already in the outline (in the Trash too) adds nothing: importing an inbox file
    /// again never adds it twice. One undo step; the focus is the end of the last bullet added,
    /// none when nothing changed.
    pub fn journal_share(&self, date: String, key: String, text: String, url: String) -> Result<EditOutcome, FlowieError> {
        let date = day(&date)?;
        Ok(tracked(&mut self.lock(), false, |ws| ws.journal_share(date, &key, &text, &url)))
    }

    /// Open the journal page for `date` (YYYY-MM-DD) to capture into: the focus goes to its
    /// last row when that is empty, otherwise to a new empty row at the end
    /// (`flowie_doc::capture::capture_row`). An undo step when it made something.
    pub fn journal_capture(&self, date: String) -> Result<EditOutcome, FlowieError> {
        let date = day(&date)?;
        Ok(tracked(&mut self.lock(), false, |ws| Some(ws.journal_capture(date))))
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
        state.ws.use_remote(&format!("{server}#outline"));
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
    /// Returns whether this is a fresh start with that backend (`Workspace::use_remote`):
    /// everything here waits to be sent, and the native engine should drop the place it kept
    /// in the backend's changes and fetch from the beginning.
    pub fn begin_native_sync(&self, remote: String) -> bool {
        let mut state = self.lock();
        state.sync = None;
        let fresh = state.ws.use_remote(&remote);
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
        let State { ws, native, .. } = &mut *state;
        match native {
            Some(native) => native.resend_all(ws),
            None => ws.resend_all(),
        }
    }

    /// Ids with something to send, for the native engine's list of pending saves (image files
    /// whose save failed included).
    pub fn unsent_ids(&self, limit: u32) -> Vec<String> {
        let state = self.lock();
        match &state.native {
            Some(native) => native.to_send(&state.ws, limit as usize),
            None => NativeSync::unsent_ids(&state.ws, limit as usize),
        }
    }

    /// The next records to save, at most `limit`, each from the version last seen.
    pub fn next_saves(&self, limit: u32) -> Vec<WireRecord> {
        let mut state = self.lock();
        let State { ws, native, .. } = &mut *state;
        let Some(native) = native else { return Vec::new() };
        native.next_saves(ws, limit as usize).into_iter().map(WireRecord::from).collect()
    }

    /// The native engine answered saves.
    pub fn on_saved(&self, outcomes: Vec<SaveOutcome>) {
        let mut state = self.lock();
        let State { ws, native, .. } = &mut *state;
        let Some(native) = native else { return };
        native.on_saved(ws, outcomes.into_iter().map(NativeSaved::from).collect());
    }

    /// Records the native engine fetched, and where it is now (CloudKit keeps its own change
    /// token: pass anything that marks progress, or an empty string). Says whether the outline
    /// changed, and what became of each image file.
    pub fn on_fetched(&self, records: Vec<WireRecord>, cursor: String) -> FetchedRecords {
        let mut state = self.lock();
        let State { ws, native, .. } = &mut *state;
        let records: Vec<Wire> = records.into_iter().map(Wire::from).collect();
        let Some(native) = native else {
            let files = records.iter().filter(|r| r.file.is_some());
            let files = files.map(|r| FetchedFile { id: r.id.clone(), kept: false, verdict: "dropped: sync isn't running".into() });
            return FetchedRecords { changed: false, files: files.collect() };
        };
        let fetched = native.on_fetched(ws, &records, &cursor);
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
        NativeSync::missing_files(&self.lock().ws, limit as usize)
    }

    /// Whether the outline has nothing in it yet: no bullets at the top level (Trash aside).
    /// A new device shows that it is fetching the outline while it is.
    pub fn is_empty(&self) -> bool {
        self.lock().ws.tree.children(ROOT).is_empty()
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
    fn backlinks_list_the_bullets_linking_to_a_page_then_its_mirrors() {
        let outline = Outline::in_memory();
        let page = add(&outline, None, 0, "Page");
        let project = add(&outline, None, 1, "Project");
        let later = add(&outline, Some(&project), 0, &format!("see [it](flowie:{page}) **now**"));
        let first = add(&outline, None, 0, &format!("[Page](flowie:{page})"));
        assert!(outline.backlinks(project.clone()).unwrap().is_empty());
        let mirror = {
            let mut state = outline.lock();
            let (p, src) = (parse(&project).unwrap(), parse(&page).unwrap());
            state.ws.tree.create_mirror(src, p, 1).unwrap().to_string()
        };
        let links = outline.backlinks(page.clone()).unwrap();
        // In outline order, links before mirrors; the text as stored, to render.
        assert_eq!(
            links.iter().map(|b| (b.id.as_str(), b.mirror)).collect::<Vec<_>>(),
            [(first.as_str(), false), (later.as_str(), false), (mirror.as_str(), true)]
        );
        assert_eq!(links[1].text, format!("see [it](flowie:{page}) **now**"));
        assert_eq!(links[1].path, [Crumb { id: project.clone(), text: "Project".into() }]);
        assert!(links[0].path.is_empty());
        // A link opens its own bullet; a mirror, the page it is on.
        assert_eq!(links[1].target.as_deref(), Some(later.as_str()));
        assert_eq!(links[2].target.as_deref(), Some(project.as_str()));
        assert_eq!(links[2].text, "Page");
        // From the mirror's page, the same list; gone with the link.
        assert_eq!(outline.backlinks(mirror.clone()).unwrap().len(), 3);
        outline.delete(first.clone()).unwrap();
        assert_eq!(outline.backlinks(page.clone()).unwrap().len(), 2);
        assert!(outline.backlinks("nope".into()).is_err());
    }

    #[test]
    fn format_commands_write_the_desktops_markup_in_utf16() {
        let f = |text: &str, start, end, format| {
            let out = toggle_format(text.into(), start, end, format);
            (out.text, out.start, out.end)
        };
        // "加粗" is two UTF-16 units, as is the emoji before it.
        assert_eq!(f("😀加粗 x", 2, 4, TextFormat::Bold), ("😀**加粗** x".into(), 4, 6));
        assert_eq!(f("😀**加粗** x", 4, 6, TextFormat::Bold), ("😀加粗 x".into(), 2, 4));
        assert_eq!(f("a b", 2, 3, TextFormat::Italic).0, "a *b*");
        assert_eq!(f("a b", 2, 3, TextFormat::Strikethrough).0, "a ~~b~~");
        assert_eq!(f("a b", 2, 3, TextFormat::Code).0, "a `b`");
        assert_eq!(f("ab", 1, 1, TextFormat::Highlight), ("a====b".into(), 3, 3));
        assert_eq!(f("a b", 2, 3, TextFormat::Link), ("a [b](https://)".into(), 6, 14));
        assert_eq!(f("", 0, 0, TextFormat::Link), ("[link](https://)".into(), 7, 15));
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
        assert_eq!(outline.journal_date(day.clone()).as_deref(), Some("2026-11-27"), "a day opens as its day");
        assert_eq!(outline.journal_date(rows[0].id.clone()), None);
        assert_eq!(outline.journal_date(journal::journal_root_id().to_string()), None);
        assert!(matches!(outline.open_day("2026-11-27; drop".into()), Err(FlowieError::NotADate { .. })));
        assert!(matches!(outline.journal_day("tomorrow".into()), Err(FlowieError::NotADate { .. })));
    }

    #[test]
    fn journal_capture_and_append_use_the_desktops_day() {
        let outline = Outline::in_memory();
        let date = "2026-11-27";
        let page = Some(journal::day_id(date).to_string());
        assert!(matches!(outline.journal_append("yesterday".into(), "oops".into()), Err(FlowieError::NotADate { .. })));
        assert!(matches!(outline.journal_capture("2026-13-01".into()), Err(FlowieError::NotADate { .. })));

        // Capturing opens the day with the desktop's blank first line, again and again.
        let out = outline.journal_capture(date.into()).unwrap();
        let blank = journal::day_blank_id(date).to_string();
        assert_eq!(out.focus, Some(FocusView { id: blank.clone(), start: 0, end: 0 }));
        assert!(out.structure_changed);
        assert_eq!(outline.journal_capture(date.into()).unwrap().focus.map(|f| f.id), Some(blank.clone()));
        assert_eq!(outline.page(page.clone()).unwrap().len(), 1);

        // Appending replaces that blank line; the focus is in UTF-16.
        let raw = " **日😀** \nsecond";
        let out = outline.journal_append(date.into(), raw.into()).unwrap();
        let rows = outline.page(page.clone()).unwrap();
        assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["**日😀**", "second"]);
        assert_eq!(out.focus, Some(FocusView { id: rows[1].id.clone(), start: 6, end: 6 }));
        assert_eq!(outline.journal_day(date.into()).unwrap(), page);
        let blank_out = outline.journal_append(date.into(), " \n ".into()).unwrap();
        assert!(blank_out.focus.is_none() && blank_out.changed_ids.is_empty());

        outline.undo();
        let rows = outline.page(page.clone()).unwrap();
        assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), [""], "one step");
        assert_eq!(rows[0].id, blank);
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
    fn expanding_to_a_level_folds_below_it_and_undoes() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "a");
        let b = add(&outline, Some(&a), 0, "b");
        add(&outline, Some(&b), 0, "c");
        add(&outline, None, 1, "d");
        let names = |o: &Outline| o.page(None).unwrap().into_iter().map(|r| r.text).collect::<Vec<_>>();
        let out = outline.expand_to_level(None, 1).unwrap();
        assert!(out.structure_changed);
        assert_eq!(names(&outline), ["a", "d"], "collapse all");
        outline.expand_to_level(None, 2).unwrap();
        assert_eq!(names(&outline), ["a", "b", "d"]);
        assert!(outline.page(None).unwrap()[1].collapsed);
        outline.expand_to_level(None, u32::MAX).unwrap();
        assert_eq!(names(&outline), ["a", "b", "c", "d"]);
        assert!(outline.expand_to_level(None, u32::MAX).unwrap().is_empty(), "already expanded");
        assert!(outline.undo().structure_changed);
        assert_eq!(names(&outline), ["a", "b", "d"], "one undo step");
        // On a bullet's page, levels count from it.
        outline.expand_to_level(Some(a.clone()), 1).unwrap();
        assert_eq!(outline.page(Some(a.clone())).unwrap().len(), 1);
        assert!(outline.expand_to_level(Some("nope".into()), 1).is_err());
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
    fn index_pages_are_parents_stars_and_days() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "**Projects**");
        let b = add(&outline, Some(&a), 0, "Flowie");
        let c = add(&outline, Some(&b), 0, "ship it");
        add(&outline, None, 1, "lone");
        let star = add(&outline, None, 2, "starred leaf");
        outline.set_starred(star.clone(), true).unwrap();
        outline.open_day("2026-11-27".into()).unwrap();
        let pages = outline.index_pages();
        let names: Vec<&str> = pages.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(names, ["Projects", "Flowie", "starred leaf", "Journal", "2026-11-27"], "not leaves, in outline order");
        assert_eq!(pages[1].path, ["Projects"]);
        assert_eq!(pages[1].content, "ship it");
        assert!(pages[2].starred);
        assert_eq!(pages[4].day.as_deref(), Some("2026-11-27"));
        assert_eq!(pages[4].path, ["Journal"]);
        // The digest follows what is indexed: a child's text, the path, a star.
        let before = pages[1].digest;
        assert_eq!(outline.index_pages()[1].digest, before, "stable");
        outline.set_text(c.clone(), "shipped".into()).unwrap();
        let after = outline.index_pages();
        assert_ne!(after[1].digest, before);
        assert_eq!(after[0].digest, pages[0].digest, "the parent's own children didn't change");
        outline.set_text(a.clone(), "Work".into()).unwrap();
        assert_ne!(outline.index_pages()[1].digest, after[1].digest, "the path changed");
        // Deleted pages leave the list.
        outline.delete(b).unwrap();
        assert_eq!(outline.index_pages().iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["starred leaf", "Journal", "2026-11-27"]);
    }

    #[test]
    fn saved_searches_list_the_pref() {
        let outline = Outline::in_memory();
        assert!(outline.saved_searches().is_empty());
        outline.lock().ws.set_pref("saved_searches", "is:todo\n\n#work");
        assert_eq!(outline.saved_searches(), ["is:todo", "#work"]);
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
    fn drop_place_and_drop_bullet_move_a_bullet_between_pages() {
        let outline = Outline::in_memory();
        let a = add(&outline, None, 0, "a");
        let b = add(&outline, Some(&a), 0, "b");
        let c = add(&outline, None, 1, "c");
        // c dropped on a's page, below b, one level in: under b.
        let place = outline.drop_place(Some(a.clone()), Some(c.clone()), Some(b.clone()), 0, None, 1).unwrap().unwrap();
        assert_eq!(place, DropPlace { parent: Some(b.clone()), index: 0, depth: 1 });
        assert!(outline.drop_bullet(c.clone(), place).unwrap().structure_changed);
        assert_eq!(
            outline.page(Some(a.clone())).unwrap().iter().map(|r| (r.text.as_str(), r.depth)).collect::<Vec<_>>(),
            [("b", 0), ("c", 1)]
        );
        outline.undo();
        assert_eq!(outline.page(None).unwrap().iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        // At the top of Home: Home's first child. Not under itself.
        let top = outline.drop_place(None, Some(c.clone()), None, 0, Some(0), 3).unwrap().unwrap();
        assert_eq!(top, DropPlace { parent: None, index: 0, depth: 0 });
        assert_eq!(outline.drop_place(Some(a.clone()), Some(a.clone()), Some(b), 0, None, 1).unwrap(), None);
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

    #[test]
    fn markup_runs_count_utf16() {
        let id = NodeId::random();
        // 日: 3 bytes, 1 unit; 😀: 4 bytes, 2 units.
        let text =
            format!("日😀 **粗** *斜* ~~删~~ [链接](flowie:{id}) #标签 2026-11-27 $x^2$ {{red}}红{{/}} ==亮== `c` https://example.com/a");
        let runs = markup(text.clone());
        let units: Vec<u16> = text.encode_utf16().collect();
        let shown = |r: &MarkupRun| String::from_utf16(&units[r.start as usize..r.end as usize]).unwrap();
        // The runs cover the text in order.
        assert_eq!(runs.first().map(|r| r.start), Some(0));
        assert_eq!(runs.last().map(|r| r.end as usize), Some(units.len()));
        assert!(runs.windows(2).all(|w| w[0].end == w[1].start));
        let find = |kind: fn(&RunKind) -> bool| runs.iter().find(|r| kind(&r.kind)).unwrap();
        let bold = runs.iter().find(|r| r.bold && r.kind == RunKind::Text).unwrap();
        assert_eq!((bold.start, shown(bold).as_str()), (6, "粗"), "after 日😀 and a space and **");
        let link = find(|k| matches!(k, RunKind::Link { bullet: Some(_), .. }));
        assert_eq!(shown(link), "链接");
        assert_eq!(link.kind, RunKind::Link { url: format!("flowie:{id}"), bullet: Some(id.to_string()) });
        assert_eq!(shown(find(|k| *k == RunKind::Tag)), "#标签");
        assert_eq!(shown(find(|k| *k == RunKind::Date)), "2026-11-27");
        assert_eq!(shown(find(|k| matches!(k, RunKind::Math { display: false }))), "$x^2$");
        assert_eq!(shown(find(|k| *k == RunKind::Code)), "c");
        let red = runs.iter().find(|r| r.color == Some(0)).unwrap();
        assert_eq!(shown(red), "红");
        assert!(runs.iter().any(|r| r.italic && shown(r) == "斜"));
        assert!(runs.iter().any(|r| r.strike && shown(r) == "删"));
        assert!(runs.iter().any(|r| r.highlight && shown(r) == "亮"));
        let web = runs.last().unwrap();
        assert_eq!(web.kind, RunKind::Link { url: "https://example.com/a".into(), bullet: None });
        assert!(markup(String::new()).is_empty());
        assert_eq!(markup("[x](flowie:nope)".into())[1].kind, RunKind::Link { url: "flowie:nope".into(), bullet: None });
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
            let ws = &mut state.ws;
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
    fn image_rows_say_where_their_file_is_and_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("outline.sqlite");
        let outline = Outline::open(db.to_string_lossy().into_owned()).unwrap();
        // A 300×200 PNG header, stored as the desktop stores images.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(300u32.to_be_bytes());
        png.extend(200u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        let (shown, missing, plain) = {
            let mut state = outline.lock();
            let ws = &mut state.ws;
            let info = ws.image_dir.as_ref().unwrap().put(&png).unwrap();
            let name = info.name.clone();
            ws.record_blob(info);
            let mut ids = Vec::new();
            for (image, caption) in [(name, "a caption"), (format!("{}.png", "ab".repeat(32)), "")] {
                let id = ws.tree.create_at(ROOT, ids.len(), caption);
                let mut node = ws.tree.node(id).clone();
                node.kind = BlockKind::Image;
                node.image = image;
                ws.tree.put(node);
                ids.push(id);
            }
            let plain = ws.tree.create_at(ROOT, 2, "no picture");
            ws.commit_with_imports(None, None, None);
            (ids[0].to_string(), ids[1].to_string(), plain.to_string())
        };
        let page = outline.page(None).unwrap();
        let image = page[0].image.clone().unwrap();
        assert_eq!((page[0].kind.as_str(), page[0].text.as_str()), ("image", "a caption"));
        assert_eq!((image.width, image.height), (300, 200), "from the blobs table");
        let path = std::path::PathBuf::from(image.path.unwrap());
        assert!(path.starts_with(dir.path().join("outline.sqlite.images")) && path.exists());
        // Synced without its file: where it would be, and no size.
        let missing_image = page[1].image.clone().unwrap();
        assert_eq!((missing_image.width, missing_image.height), (0, 0));
        assert!(!std::path::Path::new(&missing_image.path.unwrap()).exists());
        assert_eq!(page[2].image, None);
        assert_eq!(outline.rows(vec![shown.clone()], None).unwrap()[0].image, page[0].image, "rows() says the same");
        // Sizes are kept with the database.
        outline.flush().unwrap();
        drop(outline);
        let outline = Outline::open(db.to_string_lossy().into_owned()).unwrap();
        let again = outline.rows(vec![shown, missing, plain], None).unwrap();
        assert_eq!(again[0].image.as_ref().map(|i| (i.width, i.height)), Some((300, 200)));
        assert!(again[1].image.is_some() && again[2].image.is_none());
    }

    #[test]
    fn a_packed_page_reads_back_as_the_page() {
        struct Reader<'a>(&'a [u8]);
        impl Reader<'_> {
            fn take(&mut self, n: usize) -> &[u8] {
                let (head, rest) = self.0.split_at(n);
                self.0 = rest;
                head
            }
            fn u32(&mut self) -> u32 {
                u32::from_le_bytes(self.take(4).try_into().unwrap())
            }
            fn string(&mut self) -> String {
                let len = self.u32() as usize;
                String::from_utf8(self.take(len).to_vec()).unwrap()
            }
        }
        fn unpack(bytes: &[u8]) -> Vec<RowView> {
            let mut r = Reader(bytes);
            let rows = (0..r.u32())
                .map(|_| {
                    let depth = u16::from_le_bytes(r.take(2).try_into().unwrap());
                    let flags = r.take(1)[0];
                    let [id, text, note, kind] = std::array::from_fn(|_| r.string());
                    let image = (flags & 8 != 0).then(|| ImageView {
                        width: r.u32(),
                        height: r.u32(),
                        path: (flags & 16 != 0).then(|| r.string()),
                    });
                    RowView {
                        id,
                        depth,
                        text,
                        note,
                        kind,
                        completed: flags & 1 != 0,
                        collapsed: flags & 2 != 0,
                        has_children: flags & 4 != 0,
                        image,
                    }
                })
                .collect();
            assert!(r.0.is_empty(), "nothing left over");
            rows
        }
        let outline = Outline::in_memory();
        let parent = add(&outline, None, 0, "parent, **bold** 中文 ✓");
        add(&outline, Some(&parent), 0, "child");
        outline.set_note(parent.clone(), "a note".into()).unwrap();
        let done = add(&outline, None, 1, "");
        outline.set_completed(done, true).unwrap();
        let page = outline.page(None).unwrap();
        assert_eq!(unpack(&outline.page_packed(None).unwrap()), page);
        let image = |path: Option<&str>| ImageView { path: path.map(str::to_string), width: 300, height: 200 };
        let mut rows = page;
        rows[0].collapsed = true;
        rows[1].image = Some(image(Some("/images/ab/cd.png")));
        rows[2].image = Some(image(None));
        assert_eq!(unpack(&pack_rows(&rows)), rows);
        assert_eq!(unpack(&pack_rows(&[])), vec![]);
    }

    #[test]
    fn rows_inside_a_mirror_show_and_edit_their_source() {
        let outline = Outline::in_memory();
        let source = add(&outline, None, 0, "source");
        let child = add(&outline, Some(&source), 0, "child");
        let mirror = {
            let mut state = outline.lock();
            let source = source.parse().unwrap();
            let mirror = state.ws.tree.create_mirror(source, ROOT, 1).unwrap();
            state.ws.commit_with_imports(None, None, None);
            mirror.to_string()
        };
        let virtual_child = flowie_core::tree::virtual_id(mirror.parse().unwrap(), child.parse().unwrap()).to_string();
        assert_eq!(outline.rows(vec![virtual_child], None).unwrap()[0].text, "child");
        let page = outline.page(None).unwrap();
        assert_eq!(page.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["source", "child", "source", "child"]);
        assert!(page[2].has_children && outline.bullet(mirror.clone()).unwrap().has_children);
        assert_eq!(page[2].id, mirror);
        let inside = page[3].id.clone();
        assert_ne!(inside, child, "a virtual id");
        outline.set_text(inside.clone(), "child, edited in the mirror".into()).unwrap();
        outline.set_note(mirror.clone(), "a note through the mirror".into()).unwrap();
        let page = outline.page(None).unwrap();
        assert_eq!(page[1].text, "child, edited in the mirror", "the source changed");
        assert_eq!(page[0].note, "a note through the mirror");
        assert_eq!(outline.rows(vec![inside], None).unwrap()[0].text, "child, edited in the mirror");
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
        let started = std::time::Instant::now();
        let pages = outline.index_pages().len();
        eprintln!("index_pages(): {pages} pages in {:?}", started.elapsed());
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

    #[test]
    fn page_levels_counts_unfolded_visible_depth_up_to_three() {
        let outline = Outline::in_memory();
        assert_eq!(outline.page_levels(None).unwrap(), 0);
        let a = add(&outline, None, 0, "a");
        assert_eq!(outline.page_levels(None).unwrap(), 1);
        let b = add(&outline, Some(&a), 0, "b");
        assert_eq!(outline.page_levels(None).unwrap(), 2);
        // Collapsing `a` leaves only depth-0 rows on screen, yet the page still has 2 levels.
        outline.set_collapsed(a.clone(), true).unwrap();
        assert_eq!(outline.page_levels(None).unwrap(), 2);
        let c = add(&outline, Some(&b), 0, "c");
        assert_eq!(outline.page_levels(None).unwrap(), 3);
        assert_eq!(outline.page_levels(Some(a.clone())).unwrap(), 2);
        assert_eq!(outline.page_levels(Some(b.clone())).unwrap(), 1);
        assert_eq!(outline.page_levels(Some(c.clone())).unwrap(), 0);
        // Hiding completed rows drops levels whose only bullets are completed.
        outline.set_completed(c, true).unwrap();
        outline.set_hide_completed(true);
        assert_eq!(outline.page_levels(None).unwrap(), 2);
    }
}
