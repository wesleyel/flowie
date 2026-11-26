use std::sync::{PoisonError, RwLock};
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_time::{SystemTime, UNIX_EPOCH};

use super::{NodeId, ROOT};
use crate::hlc::Clocks;

/// How a node is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, serde::Serialize, serde::Deserialize)]
pub enum BlockKind {
    #[default]
    Bullet,
    /// Plain paragraph without a bullet.
    Paragraph,
    H1,
    H2,
    H3,
    /// Checkbox; uses `completed` as its checked state.
    Todo,
    Quote,
    /// Code block: monospace, no markup parsing.
    Code,
    /// Numbered list item (number = position among siblings).
    Numbered,
    /// An image (`Node::image`); the text is its caption.
    Image,
}

impl BlockKind {
    /// Kinds that carry over to the row Enter creates next to them: lists
    /// and paragraphs. Enter on an empty one turns it back into a bullet.
    pub fn is_list(self) -> bool {
        matches!(self, BlockKind::Todo | BlockKind::Numbered | BlockKind::Paragraph)
    }

    /// The kind a new row next to a row of this kind starts with.
    pub fn list_kind(self) -> BlockKind {
        if self.is_list() {
            self
        } else {
            BlockKind::Bullet
        }
    }

    pub const ALL: [BlockKind; 10] = [
        BlockKind::Bullet,
        BlockKind::Paragraph,
        BlockKind::H1,
        BlockKind::H2,
        BlockKind::H3,
        BlockKind::Todo,
        BlockKind::Quote,
        BlockKind::Code,
        BlockKind::Numbered,
        BlockKind::Image,
    ];

    pub fn to_u8(self) -> u8 {
        BlockKind::ALL.iter().position(|k| *k == self).unwrap_or(0) as u8
    }

    pub fn from_u8(v: u8) -> BlockKind {
        BlockKind::ALL.get(v as usize).copied().unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        match self {
            BlockKind::Bullet => "bullet",
            BlockKind::Paragraph => "paragraph",
            BlockKind::H1 => "h1",
            BlockKind::H2 => "h2",
            BlockKind::H3 => "h3",
            BlockKind::Todo => "todo",
            BlockKind::Quote => "quote",
            BlockKind::Code => "code",
            BlockKind::Numbered => "numbered",
            BlockKind::Image => "image",
        }
    }

    pub fn from_name(s: &str) -> Option<BlockKind> {
        BlockKind::ALL.iter().copied().find(|k| k.name() == s)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub parent: NodeId,
    /// Position among its siblings: a fractional-index key (`crate::rank`),
    /// compared as bytes; equal keys fall back to the id.
    pub rank: String,
    pub text: String,
    pub note: String,
    pub completed: bool,
    pub collapsed: bool,
    pub created: i64,
    pub modified: i64,
    #[serde(default)]
    pub kind: BlockKind,
    #[serde(default)]
    /// If non-zero, this node is a mirror: it shows (and edits) that node's
    /// text, note, completion and children.
    pub mirror: NodeId,
    /// The image file (`<sha256>.<ext>` in the image folder), for `BlockKind::Image`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    /// Starred: a fractional-index key (`crate::rank`) giving its place in the
    /// starred list, compared as bytes; empty when not starred.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub star: String,
    /// When this node was moved to the Trash (milliseconds), `None` while it is
    /// in the outline. Set only on the root of a deleted subtree: the nodes below
    /// keep theirs, and are hidden because their parent is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
    /// Sync bookkeeping, kept up to date by `Tree`. Not part of share packages or MCP.
    #[serde(skip)]
    pub sync: NodeSync,
}

/// What sync needs to know about a node besides its fields (per-node-sync.md §3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSync {
    /// When the node was created and each synced field last written.
    pub clocks: Clocks,
    /// The parent the node had before its last move: where the view puts it back when that
    /// move closes a cycle (§3.4).
    pub moved_from: NodeId,
    /// Fields written by a newer Flowie that this one doesn't know, as a JSON object (or
    /// empty), kept so writing the node back doesn't drop them (§4.1).
    pub extra: String,
}

impl Default for NodeSync {
    fn default() -> Self {
        NodeSync { clocks: Clocks::default(), moved_from: ROOT, extra: String::new() }
    }
}

impl Node {
    pub fn new(id: NodeId, parent: NodeId, rank: String) -> Self {
        let now = now_millis();
        Node {
            id,
            parent,
            rank,
            text: String::new(),
            note: String::new(),
            completed: false,
            collapsed: false,
            created: now,
            modified: now,
            kind: BlockKind::Bullet,
            mirror: ROOT,
            image: String::new(),
            star: String::new(),
            deleted_at: None,
            sync: NodeSync { moved_from: parent, ..NodeSync::default() },
        }
    }
}

/// A wall clock: milliseconds since the Unix epoch.
pub type Clock = fn() -> i64;

static CLOCK: RwLock<Option<Clock>> = RwLock::new(None);

/// Replace the clock behind [`now_millis`] for the whole process (`None` goes back to the
/// system clock). For deterministic tests, and later the sync engine's hybrid logical clock.
pub fn set_clock(clock: Option<Clock>) {
    *CLOCK.write().unwrap_or_else(PoisonError::into_inner) = clock;
}

/// The time stamped on `created`/`modified` and `deleted_at`: the clock set with
/// [`set_clock`], or the system clock.
pub fn now_millis() -> i64 {
    match *CLOCK.read().unwrap_or_else(PoisonError::into_inner) {
        Some(clock) => clock(),
        None => system_millis(),
    }
}

/// The system clock. `std::time::SystemTime` panics on `wasm32-unknown-unknown`, so the
/// browser's clock is read there instead.
pub fn system_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
