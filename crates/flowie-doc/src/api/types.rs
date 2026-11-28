//! What the outline facade ([`super::Outline`]) takes and gives: plain data, which `flowie-ffi`
//! hands to Swift as UniFFI records and `flowie-wasm` to JavaScript as JSON. Ids are 32 hex
//! digits, as on the wire; text offsets are UTF-16 code units (what UIKit and JavaScript count).

use serde::{Deserialize, Serialize, Serializer};

/// What can go wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
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

impl From<flowie_core::store::OpenError> for Error {
    fn from(error: flowie_core::store::OpenError) -> Self {
        use flowie_core::store::OpenError;
        match error {
            OpenError::TooNew { found, current } => Error::TooNew { found, current },
            OpenError::Migration { ref backup, .. } => {
                Error::UpgradeFailed { backup: backup.display().to_string(), message: error.to_string() }
            }
            OpenError::Backup { .. } => Error::BackupFailed { message: error.to_string() },
            other => Error::Storage { message: other.to_string() },
        }
    }
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Storage { message } | Error::BackupFailed { message } | Error::UpgradeFailed { message, .. } => write!(f, "{message}"),
            Error::TooNew { found, current } => {
                write!(f, "the database has format {found}, from a newer Flowie (this one reads up to {current})")
            }
            Error::NoSuchBullet { id } => write!(f, "no bullet {id}"),
            Error::NotADate { date } => write!(f, "{date:?} is not a date (YYYY-MM-DD)"),
        }
    }
}

/// One row of a page, as the outline shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize)]
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

/// What a run is. As JSON, an object tagged by `type` (`{"type":"link","url":…,"bullet":…}`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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

/// A format command of the text being typed in: the desktop's Bold (⌘B), Italic (⌘I),
/// Strikethrough, Code (⌘E), Highlight (⇧⌘Y) and Link (⌘K). As JSON, its name in lower case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextFormat {
    Bold,
    Italic,
    Strikethrough,
    Code,
    Highlight,
    Link,
}

/// A bullet's text after a format command, and what is selected in it (UTF-16 offsets).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FormattedText {
    pub text: String,
    pub start: u32,
    pub end: u32,
}

/// What an edit changed, so the app updates only that (wiki: iOS-Cross-Platform §4.2).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EditOutcome {
    /// Rows whose content changed: read them again with [`Outline::rows`](super::Outline::rows)
    /// (ones not on the page shown can be ignored).
    pub changed_ids: Vec<String>,
    /// Rows came, went or moved: read the page again, its rows too.
    pub structure_changed: bool,
    /// Where the caret goes, when the edit moves it.
    pub focus: Option<FocusView>,
}

impl EditOutcome {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.changed_ids.is_empty() && !self.structure_changed
    }
}

/// A selection in a row's text, in UTF-16 code units; a caret when `start == end`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FocusView {
    pub id: String,
    pub start: u32,
    pub end: u32,
}

/// Where a dragged bullet lands on a page (`crate::drop::place`): `index` among `parent`'s
/// children (none: Home's), shown `depth` levels into the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropPlace {
    pub parent: Option<String>,
    pub index: u32,
    pub depth: u16,
}

/// A bullet as lists name it: search results, stars, a page's title and the path to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Crumb {
    pub id: String,
    /// Plain text, without markup.
    pub text: String,
}

/// A bullet elsewhere pointing at a page, as the desktop's footer lists them under the rows:
/// one whose text links to it (`[label](flowie:ID)`), or a mirror of it.
#[derive(Debug, Clone, PartialEq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize)]
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
    /// As JSON, a decimal string: JavaScript's numbers can't hold every u64.
    #[serde(serialize_with = "decimal")]
    pub digest: u64,
}

fn decimal<S: Serializer>(n: &u64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&n.to_string())
}

/// A deleted bullet, with what is under it, as Recently Deleted lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrashView {
    pub id: String,
    /// Plain text, cut to 80 characters.
    pub text: String,
    /// How many bullets restoring it brings back, itself included.
    pub count: u32,
    /// When it was deleted, in milliseconds since 1970.
    pub deleted_at: i64,
}
