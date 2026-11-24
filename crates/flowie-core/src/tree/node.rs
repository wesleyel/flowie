use super::{NodeId, ROOT};

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
    /// List kinds carry over to the row Enter creates next to them.
    pub fn is_list(self) -> bool {
        matches!(self, BlockKind::Todo | BlockKind::Numbered)
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
    pub rank: f64,
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
}

impl Node {
    pub fn new(id: NodeId, parent: NodeId, rank: f64) -> Self {
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
        }
    }
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
