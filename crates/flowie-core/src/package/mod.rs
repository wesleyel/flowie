//! Share packages (`.flowie`): bullets with their children in one file, for
//! sending to someone else. A zip archive holding
//!
//! - `mimetype`: first and stored uncompressed (as in EPUB), so a package is
//!   recognizable without its extension;
//! - `manifest.json`: who exported it, when, and what (`Manifest`);
//! - `outline.json`: the nodes as a flat list in outline order (`Outline`);
//!   flat because deep nesting would hit the JSON parser's depth limit;
//! - `preview.md`: the outline as Markdown, for people without Flowie;
//! - `images/<sha256>.<ext>`: image files, stored as they are (already compressed).
//!
//! Node ids are kept: they are global, so the same bullet has the same id in
//! every outline it reaches.

mod archive;
mod export;
mod import;
mod merge;
mod zip;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::images::{BlobInfo, ImageDir, ImageError};
use crate::io::Item;
use crate::markup::{self, LinkEdit};
use crate::tree::{BlockKind, NodeId, ROOT};

pub use import::{compare_package, package_import_state, ImportComparison};
pub use merge::{content_hash, UpdatePlan};

pub const MIMETYPE: &str = "application/x-flowie-package";
/// The file extension, without the dot.
pub const EXTENSION: &str = "flowie";
const FORMAT: &str = "flowie.package";
/// The major format version: a reader refuses a newer one.
pub const VERSION: u32 = 1;

/// Largest outline accepted from a package; prevents unbounded tree expansion.
const MAX_NODES: usize = 100_000;

/// Who exported a package.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub device: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub nodes: usize,
    #[serde(default)]
    pub images: usize,
    /// Unpacked size of the outline and images.
    #[serde(default)]
    pub bytes: u64,
}

/// `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    /// Shared by all roots of one export.
    pub package_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// RFC 3339, UTC.
    pub exported_at: String,
    pub exported_by: Person,
    /// The exporting app and its version ("Flowie 1.0.0").
    #[serde(default)]
    pub app: String,
    pub roots: Vec<NodeId>,
    #[serde(default)]
    pub stats: Stats,
    pub outline_sha256: String,
}

impl Manifest {
    /// `exported_at` in milliseconds since the epoch (0 if unreadable).
    pub fn exported_at_millis(&self) -> i64 {
        chrono::DateTime::parse_from_rfc3339(&self.exported_at).map(|t| t.timestamp_millis()).unwrap_or(0)
    }
}

/// One node in `outline.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageNode {
    pub id: NodeId,
    /// `None` for a root.
    pub parent: Option<NodeId>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub note: String,
    #[serde(default, with = "kind_name")]
    pub kind: BlockKind,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub modified: i64,
    /// An image's hash in `images/` (once images exist).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// A mirror of this node, which is in the package too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_of: Option<NodeId>,
    /// A mirror whose source wasn't exported, written as a plain copy.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub was_mirror: bool,
}

/// Block kinds by name ("todo"), so the file stays readable; a kind this
/// version doesn't know reads as a bullet.
mod kind_name {
    use super::BlockKind;

    pub fn serialize<S: serde::Serializer>(k: &BlockKind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(k.name())
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BlockKind, D::Error> {
        let s = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(d)?;
        Ok(BlockKind::from_name(&s).unwrap_or_default())
    }
}

/// `outline.json`: siblings in array order, parents before their children.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub roots: Vec<NodeId>,
    pub nodes: Vec<PackageNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub manifest: Manifest,
    pub outline: Outline,
    /// Image files by name (`images::is_name`), each checked against its name's hash.
    pub images: std::collections::BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    /// Completed bullets below the roots (roots are always exported).
    pub include_completed: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions { include_completed: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportOptions {
    /// New ids for every bullet ("Import a copy").
    pub fresh_ids: bool,
    /// Keep completed bullets completed; off, everything arrives open.
    pub keep_completion: bool,
}

/// What the manifest says besides the outline.
#[derive(Debug, Clone, Default)]
pub struct ExportMeta {
    pub title: String,
    pub description: String,
    pub author: Person,
    pub app: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    /// Not a zip, or not a Flowie package.
    NotAPackage,
    /// Made by a newer Flowie with a format this one can't read.
    Newer(u32),
    TooLarge,
    /// A package, but broken: the detail is for logs.
    Damaged(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageError::NotAPackage => write!(f, "not a Flowie package"),
            PackageError::Newer(v) => write!(f, "package format {v} is newer than this Flowie reads ({VERSION})"),
            PackageError::TooLarge => write!(f, "package is too large"),
            PackageError::Damaged(why) => write!(f, "damaged package: {why}"),
        }
    }
}

impl std::error::Error for PackageError {}

fn damaged(why: impl Into<String>) -> PackageError {
    PackageError::Damaged(why.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The id a plain copy of `original` gets under the copied node `copy`
/// (mirrors exported without their source): the same in every export, so a
/// later version of the package matches up. A version 4 UUID like any other.
fn copy_id(copy: NodeId, original: NodeId) -> NodeId {
    let mut h = Sha256::new();
    h.update(b"flowie.copy");
    h.update(copy.to_bytes());
    h.update(original.to_bytes());
    let d = h.finalize();
    let bits = u128::from_be_bytes(d[..16].try_into().unwrap());
    NodeId((bits & !(0xF << 76) & !(0b11 << 62)) | (4 << 76) | (0b10 << 62))
}

fn remap_links<'a>(text: &'a str, id_map: &HashMap<NodeId, NodeId>) -> Cow<'a, str> {
    if id_map.is_empty() || !text.contains("flowie:") {
        return Cow::Borrowed(text);
    }
    markup::rewrite_links(text, |url| {
        url.strip_prefix("flowie:")
            .and_then(|url| url.parse::<NodeId>().ok())
            .and_then(|id| id_map.get(&id).filter(|&&local| local != id))
            .map_or(LinkEdit::Keep, |local| LinkEdit::Url(format!("flowie:{local}")))
    })
    .into()
}

impl Package {
    pub fn write_images(&self, dir: &ImageDir) -> Result<Vec<BlobInfo>, ImageError> {
        self.images.values().map(|bytes| dir.put(bytes)).collect()
    }

    /// The outline as a bounded preview: no ids, and links between bullets
    /// reduced to their labels. Mirrors count against the same budget.
    pub fn items(&self) -> Vec<Item> {
        self.preview_items(MAX_NODES, 200)
    }

    pub fn preview_items(&self, max_items: usize, max_depth: usize) -> Vec<Item> {
        let mut kids: HashMap<NodeId, Vec<&PackageNode>> = HashMap::new();
        let mut by_id: HashMap<NodeId, &PackageNode> = HashMap::new();
        for n in &self.outline.nodes {
            by_id.insert(n.id, n);
            if let Some(p) = n.parent {
                kids.entry(p).or_default().push(n);
            }
        }
        struct Preview<'a> {
            by_id: HashMap<NodeId, &'a PackageNode>,
            kids: HashMap<NodeId, Vec<&'a PackageNode>>,
            remaining: usize,
            max_depth: usize,
        }
        impl Preview<'_> {
            fn build(&mut self, n: &PackageNode, depth: usize) -> Option<Item> {
                self.remaining = self.remaining.checked_sub(1)?;
                // A kept mirror shows its source.
                let shown = n.mirror_of.and_then(|m| self.by_id.get(&m).copied()).filter(|_| depth < 64).unwrap_or(n);
                let mut children = Vec::new();
                if depth < self.max_depth {
                    let count = self.kids.get(&shown.id).map_or(0, Vec::len);
                    for index in 0..count {
                        let child = self.kids[&shown.id][index];
                        let Some(item) = self.build(child, depth + 1) else { break };
                        children.push(item);
                    }
                }
                let unlink =
                    |s: &str| markup::rewrite_links(s, |url| if url.starts_with("flowie:") { LinkEdit::Unlink } else { LinkEdit::Keep });
                Some(Item {
                    text: unlink(&shown.text),
                    note: unlink(&shown.note),
                    completed: shown.completed,
                    kind: n.kind,
                    collapsed: n.collapsed,
                    children,
                    image: shown.image.clone().unwrap_or_default(),
                    ..Default::default()
                })
            }
        }
        let mut preview = Preview { by_id, kids, remaining: max_items, max_depth: max_depth.min(200) };
        let mut items = Vec::new();
        for root in &self.outline.roots {
            let Some(node) = preview.by_id.get(root).copied() else { continue };
            let Some(item) = preview.build(node, 0) else { break };
            items.push(item);
        }
        items
    }

    fn complete_id_map(&self, id_map: &HashMap<NodeId, NodeId>) -> HashMap<NodeId, NodeId> {
        if id_map.is_empty() {
            return self.outline.nodes.iter().map(|node| (node.id, node.id)).collect();
        }
        let root_of = self.root_of();
        self.outline
            .nodes
            .iter()
            .map(|node| {
                let id = id_map.get(&node.id).copied().unwrap_or_else(|| {
                    let source_root = root_of[&node.id];
                    match id_map.get(&source_root) {
                        Some(&copy_root) if copy_root != source_root => copy_id(copy_root, node.id),
                        _ => node.id,
                    }
                });
                (node.id, id)
            })
            .collect()
    }

    /// Each bullet's root in this package.
    pub fn root_of(&self) -> HashMap<NodeId, NodeId> {
        let children = self.children();
        let mut out = HashMap::with_capacity(self.outline.nodes.len());
        let mut stack = Vec::new();
        for &root in &self.outline.roots {
            stack.push(root);
            while let Some(id) = stack.pop() {
                out.insert(id, root);
                stack.extend(children.get(&id).into_iter().flatten().copied());
            }
        }
        out
    }

    /// Each node's children in order, keyed by parent (`ROOT` for the roots).
    pub fn children(&self) -> HashMap<NodeId, Vec<NodeId>> {
        let mut out: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for n in &self.outline.nodes {
            out.entry(n.parent.unwrap_or(ROOT)).or_default().push(n.id);
        }
        out
    }
}
