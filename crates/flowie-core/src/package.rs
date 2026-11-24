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
//! every outline it reaches. See `docs/roadmap.md` for the design.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::images::ImageDir;
use crate::io::Item;
use crate::markup::{self, LinkEdit};
use crate::tree::{BlockKind, NodeId, Tree, ROOT};

pub const MIMETYPE: &str = "application/x-flowie-package";
/// The file extension, without the dot.
pub const EXTENSION: &str = "flowie";
const FORMAT: &str = "flowie.package";
/// The major format version: a reader refuses a newer one.
pub const VERSION: u32 = 1;
/// Most bytes a package may unpack to, all entries together.
const MAX_UNPACKED: u64 = 256 << 20;
/// Most entries a package may have.
const MAX_ENTRIES: usize = 4096;
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

/// Deepest level exported.
const MAX_DEPTH: usize = 1000;
/// Most mirrors expanded into copies inside one another.
const MAX_EXPANSIONS: usize = 8;

fn remap_links(text: &str, id_map: &HashMap<NodeId, NodeId>) -> String {
    if id_map.is_empty() || !text.contains("flowie:") {
        return text.to_string();
    }
    markup::rewrite_links(text, |url| match url.strip_prefix("flowie:").and_then(|id| id.parse::<NodeId>().ok()) {
        Some(id) if id_map.contains_key(&id) => LinkEdit::Url(format!("flowie:{}", id_map[&id])),
        _ => LinkEdit::Keep,
    })
}

impl Package {
    /// Package `roots` (and their children) of `tree`. Roots inside another
    /// root, missing ones and duplicates are dropped.
    /// Image files come from `images`; without it (or the file) an image
    /// bullet goes as its caption.
    pub fn export(tree: &Tree, roots: &[NodeId], opts: ExportOptions, meta: ExportMeta, images: Option<&ImageDir>) -> Package {
        let mut picked: Vec<NodeId> = Vec::new();
        for &r in roots {
            let r = tree.resolve(r);
            if r != ROOT && tree.contains(r) && !picked.contains(&r) {
                picked.push(r);
            }
        }
        let roots: Vec<NodeId> = picked.iter().copied().filter(|&r| !picked.iter().any(|&p| p != r && tree.is_ancestor(p, r))).collect();
        let skip = |id: NodeId| !opts.include_completed && tree.node(tree.source(id)).completed;

        // The nodes exported as themselves; links and mirrors may point at these.
        let mut real: HashSet<NodeId> = HashSet::new();
        let mut stack: Vec<NodeId> = roots.clone();
        while let Some(id) = stack.pop() {
            real.insert(id);
            if tree.node(id).mirror == ROOT {
                stack.extend(tree.children(id).iter().copied().filter(|&c| !skip(c)));
            }
        }

        let fix_links = |s: &str| {
            if !s.contains("flowie:") {
                return s.to_string();
            }
            markup::rewrite_links(s, |url| match url.strip_prefix("flowie:") {
                Some(target) => match target.parse::<NodeId>() {
                    Ok(id) if real.contains(&id) => LinkEdit::Keep,
                    _ => LinkEdit::Unlink,
                },
                None => LinkEdit::Keep,
            })
        };

        // (id in the package, node it shows, parent in the package, depth, mirrors
        // expanded into copies above it); popped in outline order.
        let mut nodes = Vec::new();
        let mut stack: Vec<(NodeId, NodeId, Option<NodeId>, usize, usize)> = roots.iter().rev().map(|&r| (r, r, None, 0, 0)).collect();
        while let Some((out_id, id, parent, depth, expanded)) = stack.pop() {
            let node = tree.node(id);
            let source = (node.mirror != ROOT && tree.contains(node.mirror)).then_some(node.mirror);
            let keep_mirror = source.filter(|s| real.contains(s));
            let shown = match (source, keep_mirror) {
                (Some(s), None) => tree.node(s),
                _ => node,
            };
            let copy = source.is_some() && keep_mirror.is_none();
            nodes.push(PackageNode {
                id: out_id,
                parent,
                text: fix_links(&shown.text),
                note: fix_links(&shown.note),
                kind: node.kind,
                completed: shown.completed,
                collapsed: node.collapsed,
                created: node.created,
                modified: shown.modified,
                image: (node.kind == BlockKind::Image && keep_mirror.is_none() && !shown.image.is_empty()).then(|| shown.image.clone()),
                mirror_of: keep_mirror,
                was_mirror: copy,
            });
            // Mirrors of each other would expand forever.
            let expanded = expanded + usize::from(copy);
            if keep_mirror.is_some() || depth >= MAX_DEPTH || expanded > MAX_EXPANSIONS {
                continue;
            }
            // Below a copy, every node is a copy with an id of its own.
            let copying = out_id != shown.id;
            for &c in tree.children(shown.id).iter().rev() {
                if !skip(c) {
                    stack.push((if copying { copy_id(out_id, c) } else { c }, c, Some(out_id), depth + 1, expanded));
                }
            }
        }
        // A bullet left without children (completed ones left out) isn't folded.
        let parents: HashSet<NodeId> = nodes.iter().filter_map(|n| n.parent).collect();
        for n in &mut nodes {
            n.collapsed &= parents.contains(&n.id) || n.mirror_of.is_some();
        }

        let mut files = std::collections::BTreeMap::new();
        for n in &mut nodes {
            let Some(name) = n.image.clone() else { continue };
            match files.contains_key(&name) {
                true => {}
                false => match images.and_then(|d| d.read(&name).ok()) {
                    Some(bytes) => {
                        files.insert(name, bytes);
                    }
                    None => {
                        n.image = None;
                        n.kind = BlockKind::Bullet;
                    }
                },
            }
        }
        let outline = Outline { roots: roots.clone(), nodes };
        let json = outline_json(&outline);
        let image_bytes: u64 = files.values().map(|b| b.len() as u64).sum();
        let package_id = if roots.len() == 1 {
            format!("root:{}", roots[0])
        } else {
            let mut hash = Sha256::new();
            hash.update(b"flowie.package.roots");
            for root in &roots {
                hash.update(root.to_bytes());
            }
            hex(&hash.finalize())
        };
        let manifest = Manifest {
            format: FORMAT.into(),
            version: VERSION,
            package_id,
            title: meta.title,
            description: meta.description,
            exported_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            exported_by: meta.author,
            app: meta.app,
            roots,
            stats: Stats { nodes: outline.nodes.len(), images: files.len(), bytes: json.len() as u64 + image_bytes },
            outline_sha256: hex(&Sha256::digest(&json)),
        };
        Package { manifest, outline, images: files }
    }

    /// The package file's bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let manifest = serde_json::to_vec_pretty(&self.manifest).expect("manifest serializes");
        let preview = crate::io::to_markdown(&self.items());
        let outline = outline_json(&self.outline);
        let names: Vec<String> = self.images.keys().map(|n| format!("images/{n}")).collect();
        let mut entries: Vec<(&str, &[u8], bool)> = vec![
            ("mimetype", MIMETYPE.as_bytes(), false),
            ("manifest.json", &manifest, true),
            ("outline.json", &outline, true),
            ("preview.md", preview.as_bytes(), true),
        ];
        entries.extend(names.iter().zip(self.images.values()).map(|(n, b)| (n.as_str(), b.as_slice(), false)));
        zip::write(&entries)
    }

    /// Read a package file. Only the entries above are read; anything else
    /// in the archive is ignored.
    pub fn read(bytes: &[u8]) -> Result<Package, PackageError> {
        let archive = zip::Archive::parse(bytes)?;
        let mut budget = MAX_UNPACKED;
        match archive.entries.first() {
            Some(e) if e.name == "mimetype" => {
                if archive.read(e, &mut budget)? != MIMETYPE.as_bytes() {
                    return Err(PackageError::NotAPackage);
                }
            }
            _ => return Err(PackageError::NotAPackage),
        }
        let entry = |name: &str| archive.entries.iter().find(|e| e.name == name).ok_or_else(|| damaged(format!("no {name}")));
        let manifest_bytes = archive.read(entry("manifest.json")?, &mut budget)?;
        // The format and version first, so a newer package says so instead of failing to parse.
        #[derive(Deserialize)]
        struct Head {
            format: String,
            version: u32,
        }
        let head: Head = serde_json::from_slice(&manifest_bytes).map_err(|_| PackageError::NotAPackage)?;
        if head.format != FORMAT {
            return Err(PackageError::NotAPackage);
        }
        if head.version > VERSION {
            return Err(PackageError::Newer(head.version));
        }
        let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| damaged(format!("manifest.json: {e}")))?;
        let json = archive.read(entry("outline.json")?, &mut budget)?;
        if hex(&Sha256::digest(&json)) != manifest.outline_sha256.to_ascii_lowercase() {
            return Err(damaged("outline.json does not match its checksum"));
        }
        let mut outline: Outline = serde_json::from_slice(&json).map_err(|e| damaged(format!("outline.json: {e}")))?;
        validate(&mut outline)?;
        // Images the outline refers to, each checked against the hash in its name.
        let mut files = std::collections::BTreeMap::new();
        for n in &mut outline.nodes {
            let Some(name) = n.image.clone() else { continue };
            if !files.contains_key(&name) {
                let path = format!("images/{name}");
                let ok = match archive.entries.iter().find(|e| e.name == path) {
                    Some(e) => {
                        let bytes = archive.read(e, &mut budget)?;
                        let good = crate::images::info(&bytes).is_ok_and(|i| i.name == name);
                        if good {
                            files.insert(name.clone(), bytes);
                        }
                        good
                    }
                    None => false,
                };
                if !ok {
                    n.image = None;
                }
            }
        }
        for n in &mut outline.nodes {
            if n.kind == BlockKind::Image && n.image.is_none() {
                n.kind = BlockKind::Bullet;
            }
        }
        Ok(Package { manifest, outline, images: files })
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

    /// Ids of this package's bullets that `tree` has already: it was imported before.
    pub fn present_in(&self, tree: &Tree) -> Vec<NodeId> {
        self.outline.nodes.iter().map(|n| n.id).filter(|&id| tree.contains(id)).collect()
    }

    /// Add the package's bullets to `tree` at `index` under `parent`, one
    /// recorded change per node (one undo step for the caller). Returns the
    /// new roots. With `fresh_ids` every bullet gets a new id and links and
    /// mirrors between them follow; otherwise the package's ids are kept,
    /// which needs none of them to be in the tree yet (`present_in`).
    pub fn insert_into(&self, tree: &mut Tree, parent: NodeId, index: usize, opts: ImportOptions) -> Vec<NodeId> {
        self.insert_into_mapped(tree, parent, index, opts).0
    }

    pub fn insert_into_mapped(
        &self,
        tree: &mut Tree,
        parent: NodeId,
        index: usize,
        opts: ImportOptions,
    ) -> (Vec<NodeId>, HashMap<NodeId, NodeId>) {
        self.insert_into_with_id_map(tree, parent, index, opts, &HashMap::new())
    }

    pub fn insert_into_with_id_map(
        &self,
        tree: &mut Tree,
        parent: NodeId,
        index: usize,
        opts: ImportOptions,
        existing_map: &HashMap<NodeId, NodeId>,
    ) -> (Vec<NodeId>, HashMap<NodeId, NodeId>) {
        let root_of = self.root_of();
        let mut fresh = HashMap::new();
        if opts.fresh_ids {
            for &source_root in &self.outline.roots {
                let copied_root = existing_map.get(&source_root).copied().unwrap_or_else(|| tree.alloc_id());
                fresh.insert(source_root, copied_root);
                for node in self.outline.nodes.iter().filter(|node| root_of[&node.id] == source_root) {
                    let copied = existing_map.get(&node.id).copied().unwrap_or_else(|| {
                        if node.id == source_root {
                            copied_root
                        } else {
                            copy_id(copied_root, node.id)
                        }
                    });
                    fresh.insert(node.id, copied);
                }
            }
        }
        let map = |id: NodeId| fresh.get(&id).copied().unwrap_or(id);
        let relink = |s: &str| {
            if fresh.is_empty() || !s.contains("flowie:") {
                return s.to_string();
            }
            markup::rewrite_links(s, |url| match url.strip_prefix("flowie:").and_then(|t| t.parse::<NodeId>().ok()) {
                Some(id) if fresh.contains_key(&id) => LinkEdit::Url(format!("flowie:{}", map(id))),
                _ => LinkEdit::Keep,
            })
        };
        let by_id: HashMap<NodeId, &PackageNode> = self.outline.nodes.iter().map(|n| (n.id, n)).collect();
        let kids = self.children();
        let now = crate::tree::now_millis();
        let mut roots = Vec::new();
        let index = index.min(tree.children(parent).len());
        for (i, &r) in self.outline.roots.iter().enumerate() {
            let rank = tree.rank_at(parent, index + i);
            // (package node, parent in the tree, rank), parents before children.
            let mut stack = vec![(r, parent, rank)];
            while let Some((id, under, rank)) = stack.pop() {
                let Some(n) = by_id.get(&id) else { continue };
                let new_id = map(id);
                let children = kids.get(&id).map(Vec::as_slice).unwrap_or(&[]);
                tree.put(crate::tree::Node {
                    id: new_id,
                    parent: under,
                    rank,
                    text: relink(&n.text),
                    note: relink(&n.note),
                    completed: n.completed && opts.keep_completion,
                    collapsed: n.collapsed && (!children.is_empty() || n.mirror_of.is_some()),
                    created: if n.created > 0 { n.created } else { now },
                    modified: if n.modified > 0 { n.modified } else { now },
                    kind: n.kind,
                    mirror: n.mirror_of.map_or(ROOT, map),
                    image: n.image.clone().unwrap_or_default(),
                });
                if under == parent {
                    roots.push(new_id);
                }
                for (j, &c) in children.iter().enumerate().rev() {
                    stack.push((c, new_id, j as f64 + 1.0));
                }
            }
        }
        let id_map = self.outline.nodes.iter().map(|node| (node.id, map(node.id))).collect();
        (roots, id_map)
    }

    /// Each bullet's content hash as this package has it: the base the next
    /// version is merged against (`plan_update`).
    pub fn base_hashes(&self) -> Vec<(NodeId, String)> {
        self.outline.nodes.iter().map(|node| (node.id, package_hash(node))).collect()
    }

    fn complete_id_map(&self, id_map: &HashMap<NodeId, NodeId>) -> HashMap<NodeId, NodeId> {
        if id_map.is_empty() {
            return self.outline.nodes.iter().map(|node| (node.id, node.id)).collect();
        }
        let root_of = self.root_of();
        let mut mapped = id_map.clone();
        for node in &self.outline.nodes {
            mapped.entry(node.id).or_insert_with(|| {
                let source_root = root_of[&node.id];
                let copy_root = id_map.get(&source_root).copied().unwrap_or(source_root);
                copy_id(copy_root, node.id)
            });
        }
        mapped
    }

    pub fn package_hash_mapped(&self, node: &PackageNode, id_map: &HashMap<NodeId, NodeId>) -> String {
        let complete = self.complete_id_map(id_map);
        let parent = node.parent.map(|id| complete.get(&id).copied().unwrap_or(id));
        let mirror = node.mirror_of.map(|id| complete.get(&id).copied().unwrap_or(id));
        let text = remap_links(&node.text, &complete);
        let note = remap_links(&node.note, &complete);
        content_hash(&text, &note, node.kind, node.completed, node.image.as_deref(), parent, mirror)
    }

    pub fn base_hashes_mapped(&self, id_map: &HashMap<NodeId, NodeId>) -> Vec<(NodeId, String)> {
        let complete = self.complete_id_map(id_map);
        self.outline
            .nodes
            .iter()
            .map(|node| {
                let hash = self.package_hash_mapped(node, &complete);
                (complete[&node.id], hash)
            })
            .collect()
    }

    /// Each bullet's root in this package.
    pub fn root_of(&self) -> HashMap<NodeId, NodeId> {
        let parent: HashMap<NodeId, Option<NodeId>> = self.outline.nodes.iter().map(|n| (n.id, n.parent)).collect();
        let mut out = HashMap::new();
        for n in &self.outline.nodes {
            let mut cur = n.id;
            // Validated: every chain ends at a root.
            while let Some(Some(p)) = parent.get(&cur) {
                cur = *p;
            }
            out.insert(n.id, cur);
        }
        out
    }

    /// The content hash of `id` as `tree` has it now (`None` if it's gone).
    fn local_hash(&self, tree: &Tree, id: NodeId, id_map: &HashMap<NodeId, NodeId>) -> Option<String> {
        let n = tree.get(id)?;
        let is_root = self.outline.roots.iter().any(|root| id_map.get(root).copied().unwrap_or(*root) == id);
        let parent = (!is_root).then_some(n.parent);
        let mirror = (n.mirror != ROOT).then_some(n.mirror);
        let image = (!n.image.is_empty()).then_some(n.image.as_str());
        Some(content_hash(&n.text, &n.note, n.kind, n.completed, image, parent, mirror))
    }

    /// Compare this (newer) version of a package with what `tree` has, given
    /// each bullet's content when it was last imported (`bases`) and the ids
    /// imported from earlier versions of it (`known`). Changed only in the
    /// package: take it; only here: keep it; on both sides: a conflict.
    pub fn plan_update(&self, tree: &Tree, bases: &HashMap<NodeId, String>, known: &HashSet<NodeId>) -> UpdatePlan {
        self.plan_update_mapped(tree, bases, known, &HashMap::new())
    }

    pub fn plan_update_mapped(
        &self,
        tree: &Tree,
        bases: &HashMap<NodeId, String>,
        known: &HashSet<NodeId>,
        id_map: &HashMap<NodeId, NodeId>,
    ) -> UpdatePlan {
        let complete = self.complete_id_map(id_map);
        let map = |id: NodeId| complete[&id];
        let mut plan = UpdatePlan::default();
        let ids: HashSet<NodeId> = self.outline.nodes.iter().map(|n| map(n.id)).collect();
        for n in &self.outline.nodes {
            let id = map(n.id);
            let theirs = self.package_hash_mapped(n, &complete);
            match (self.local_hash(tree, id, &complete), bases.get(&id)) {
                // The same on both sides.
                (Some(mine), _) if mine == theirs => {}
                // Unchanged in the package (changed here, or nowhere): keep.
                (Some(_), Some(base)) if *base == theirs => {}
                (Some(mine), Some(base)) if mine == *base => plan.changed.push(id),
                // Changed on both sides, or no base to tell.
                (Some(_), _) => plan.conflicts.push(id),
                (None, Some(_)) => plan.deleted_here += 1,
                (None, None) => plan.added.push(id),
            }
        }
        // Gone from the package and unchanged here: remove (only the top of a removed subtree).
        let mut gone: Vec<NodeId> = known
            .iter()
            .copied()
            .filter(|id| !ids.contains(id) && tree.contains(*id))
            .filter(|id| bases.get(id).is_some_and(|b| self.local_hash(tree, *id, &complete).as_ref() == Some(b)))
            .collect();
        let set: HashSet<NodeId> = gone.iter().copied().collect();
        gone.retain(|id| !tree.ancestors(*id).iter().any(|a| set.contains(a)));
        // Nor anything holding bullets the package still has, or new ones of the user's.
        gone.retain(|id| tree.descendants(*id).iter().all(|d| set.contains(d)));
        plan.removed = gone;
        plan
    }

    /// Apply the package side of `plan` to `tree`: changed bullets, new
    /// ones, and the conflicts listed in `take_theirs`. Removals are left to
    /// the caller (they go to the Trash). Returns the bullets touched.
    pub fn apply_update(&self, tree: &mut Tree, plan: &UpdatePlan, take_theirs: &HashSet<NodeId>) -> Vec<NodeId> {
        self.apply_update_mapped(tree, plan, take_theirs, &HashMap::new())
    }

    pub fn apply_update_mapped(
        &self,
        tree: &mut Tree,
        plan: &UpdatePlan,
        take_theirs: &HashSet<NodeId>,
        id_map: &HashMap<NodeId, NodeId>,
    ) -> Vec<NodeId> {
        let complete = self.complete_id_map(id_map);
        let map = |id: NodeId| complete[&id];
        let by_id: HashMap<NodeId, &PackageNode> = self.outline.nodes.iter().map(|n| (map(n.id), n)).collect();
        let mut kids: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for n in &self.outline.nodes {
            kids.entry(n.parent.map(map).unwrap_or(ROOT)).or_default().push(map(n.id));
        }
        let take: HashSet<NodeId> = plan.changed.iter().chain(plan.conflicts.iter().filter(|c| take_theirs.contains(c))).copied().collect();
        let added: HashSet<NodeId> = plan.added.iter().copied().collect();
        // Where a bullet goes under `parent`: after its nearest earlier sibling in the package that is here.
        let slot = |tree: &Tree, parent: NodeId, id: NodeId| -> usize {
            let sibs = kids.get(&parent).map(Vec::as_slice).unwrap_or(&[]);
            let at = sibs.iter().position(|s| *s == id).unwrap_or(0);
            sibs[..at].iter().rev().find(|s| tree.contains(**s) && tree.parent(**s) == parent).map_or(0, |s| tree.index_in_parent(*s) + 1)
        };
        let mut touched = Vec::new();
        // Parents before children, in package order.
        let mut stack: Vec<NodeId> = self.outline.roots.iter().rev().map(|id| map(*id)).collect();
        while let Some(id) = stack.pop() {
            if let Some(c) = kids.get(&id) {
                stack.extend(c.iter().rev());
            }
            let n = by_id[&id];
            if added.contains(&id) {
                let Some(parent) = n.parent.map(map).filter(|p| tree.contains(*p)) else { continue };
                let index = slot(tree, parent, id);
                let rank = tree.rank_at(parent, index);
                tree.put(crate::tree::Node {
                    id,
                    parent,
                    rank,
                    text: remap_links(&n.text, &complete),
                    note: remap_links(&n.note, &complete),
                    completed: n.completed,
                    collapsed: n.collapsed,
                    created: n.created,
                    modified: n.modified,
                    kind: n.kind,
                    mirror: n.mirror_of.map(map).filter(|m| tree.contains(*m)).unwrap_or(ROOT),
                    image: n.image.clone().unwrap_or_default(),
                });
                touched.push(id);
            } else if take.contains(&id) && tree.contains(id) {
                let mut node = tree.node(id).clone();
                node.text = remap_links(&n.text, &complete);
                node.note = remap_links(&n.note, &complete);
                node.kind = n.kind;
                node.completed = n.completed;
                node.image = n.image.clone().unwrap_or_default();
                node.mirror = n.mirror_of.map(map).filter(|m| tree.contains(*m)).unwrap_or(ROOT);
                tree.put(node);
                if let Some(parent) = n.parent.map(map).filter(|p| tree.contains(*p) && tree.parent(id) != *p) {
                    let index = slot(tree, parent, id);
                    tree.move_to(id, parent, index);
                }
                touched.push(id);
            }
        }
        touched
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

/// What updating to a newer version of a package does.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdatePlan {
    /// Changed in the package only: taken.
    pub changed: Vec<NodeId>,
    /// New in the package: added (under their parent, if it's here).
    pub added: Vec<NodeId>,
    /// Removed from the package and unchanged here: to the Trash.
    pub removed: Vec<NodeId>,
    /// Changed on both sides: the user picks.
    pub conflicts: Vec<NodeId>,
    /// Bullets the user deleted, still in the package: left deleted.
    pub deleted_here: usize,
}

impl UpdatePlan {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty() && self.conflicts.is_empty()
    }
}

/// A bullet's content for merging: what it says and where it hangs (`parent`
/// is `None` for a package's roots, which the receiver places), not its
/// order among siblings or its folding.
pub fn content_hash(
    text: &str,
    note: &str,
    kind: BlockKind,
    completed: bool,
    image: Option<&str>,
    parent: Option<NodeId>,
    mirror: Option<NodeId>,
) -> String {
    let json = serde_json::json!([text, note, kind.name(), completed, image, parent, mirror]);
    hex(&Sha256::digest(json.to_string().as_bytes()))
}

fn package_hash(n: &PackageNode) -> String {
    content_hash(&n.text, &n.note, n.kind, n.completed, n.image.as_deref(), n.parent, n.mirror_of)
}

fn outline_json(outline: &Outline) -> Vec<u8> {
    serde_json::to_vec_pretty(outline).expect("outline serializes")
}

/// Check a package's outline is a forest the importer can rely on: unique
/// ids, parents inside the package, every node reachable from a root.
/// Mirrors of nodes not in the package become plain bullets.
fn validate(outline: &mut Outline) -> Result<(), PackageError> {
    if outline.nodes.len() > MAX_NODES {
        return Err(PackageError::TooLarge);
    }
    let mut ids = HashSet::new();
    for n in &outline.nodes {
        if n.id == ROOT || n.id.is_virtual() {
            return Err(damaged(format!("reserved id {}", n.id)));
        }
        if !ids.insert(n.id) {
            return Err(damaged(format!("id {} appears twice", n.id)));
        }
    }
    let mut roots = HashSet::new();
    for &r in &outline.roots {
        if !roots.insert(r) {
            return Err(damaged(format!("root {r} listed twice")));
        }
    }
    for n in &outline.nodes {
        match n.parent {
            None if !roots.contains(&n.id) => return Err(damaged(format!("{} has no parent and is not a root", n.id))),
            Some(_) if roots.contains(&n.id) => return Err(damaged(format!("root {} has a parent", n.id))),
            Some(p) if !ids.contains(&p) => return Err(damaged(format!("{} has a parent outside the package", n.id))),
            _ => {}
        }
    }
    if outline.roots.is_empty() || !outline.roots.iter().all(|r| ids.contains(r)) {
        return Err(damaged("roots missing"));
    }
    // With one parent each, a node outside every root's subtree is on a cycle.
    let mut kids: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for n in &outline.nodes {
        if let Some(p) = n.parent {
            kids.entry(p).or_default().push(n.id);
        }
    }
    let mut seen = 0;
    let mut stack = outline.roots.clone();
    while let Some(id) = stack.pop() {
        seen += 1;
        stack.extend(kids.get(&id).into_iter().flatten());
    }
    if seen != outline.nodes.len() {
        return Err(damaged("parent cycle"));
    }
    // Mirrors as Flowie makes them: of a node in the package that is no
    // mirror itself and not above the mirror, with no children of their own.
    let parent: HashMap<NodeId, NodeId> = outline.nodes.iter().filter_map(|n| Some((n.id, n.parent?))).collect();
    let is_mirror: HashSet<NodeId> = outline.nodes.iter().filter(|n| n.mirror_of.is_some()).map(|n| n.id).collect();
    let above = |source: NodeId, mut id: NodeId| {
        while let Some(&p) = parent.get(&id) {
            if p == source {
                return true;
            }
            id = p;
        }
        false
    };
    for n in &mut outline.nodes {
        let ok = n
            .mirror_of
            .is_none_or(|m| m != n.id && ids.contains(&m) && !is_mirror.contains(&m) && !above(m, n.id) && !kids.contains_key(&n.id));
        if !ok {
            n.mirror_of = None;
            n.was_mirror = true;
        }
        n.image = n.image.take().filter(|i| crate::images::is_name(i));
    }
    Ok(())
}

/// Just enough zip: stored and deflated entries, no zip64, no encryption.
mod zip {
    use std::io::{Read, Write};

    use super::{damaged, PackageError, MAX_ENTRIES};

    const LOCAL: u32 = 0x0403_4b50;
    const CENTRAL: u32 = 0x0201_4b50;
    const END: u32 = 0x0605_4b50;
    /// Bit 11: names are UTF-8.
    const UTF8: u16 = 1 << 11;
    /// 1980-01-01, the earliest DOS date: entries carry no meaningful time.
    const DOS_DATE: u16 = (1 << 5) | 1;

    /// (name, data, deflate it).
    pub fn write(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, data, deflate) in entries {
            let crc = crc32fast::hash(data);
            let packed = if deflate {
                let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(data).expect("writing to memory");
                enc.finish().expect("writing to memory")
            } else {
                data.to_vec()
            };
            let method: u16 = if deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            let fields = |v: &mut Vec<u8>| {
                v.extend(20u16.to_le_bytes()); // version needed: 2.0
                v.extend(UTF8.to_le_bytes());
                v.extend(method.to_le_bytes());
                v.extend(0u16.to_le_bytes()); // time
                v.extend(DOS_DATE.to_le_bytes());
                v.extend(crc.to_le_bytes());
                v.extend((packed.len() as u32).to_le_bytes());
                v.extend((data.len() as u32).to_le_bytes());
                v.extend((name.len() as u16).to_le_bytes());
                v.extend(0u16.to_le_bytes()); // extra field length
            };
            out.extend(LOCAL.to_le_bytes());
            fields(&mut out);
            out.extend(name.as_bytes());
            out.extend(&packed);

            central.extend(CENTRAL.to_le_bytes());
            central.extend((3u16 << 8 | 20).to_le_bytes()); // made by: Unix, 2.0
            fields(&mut central);
            central.extend(0u16.to_le_bytes()); // comment length
            central.extend(0u16.to_le_bytes()); // disk
            central.extend(0u16.to_le_bytes()); // internal attributes
            central.extend((0o100644u32 << 16).to_le_bytes()); // a regular file, rw-r--r--
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let central_offset = out.len() as u32;
        out.extend(&central);
        out.extend(END.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(central_offset.to_le_bytes());
        out.extend(0u16.to_le_bytes()); // comment length
        out
    }

    pub struct Entry {
        pub name: String,
        method: u16,
        crc: u32,
        packed: u64,
        size: u64,
        offset: u64,
    }

    pub struct Archive<'a> {
        bytes: &'a [u8],
        /// In central directory order.
        pub entries: Vec<Entry>,
    }

    fn u16_at(b: &[u8], at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
    }

    fn u32_at(b: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
    }

    impl<'a> Archive<'a> {
        pub fn parse(bytes: &'a [u8]) -> Result<Archive<'a>, PackageError> {
            let not = || PackageError::NotAPackage;
            if u32_at(bytes, 0) != Some(LOCAL) {
                return Err(not());
            }
            // The end record is the last 22 bytes plus a comment of up to 64 KiB.
            let last = bytes.len().checked_sub(22).ok_or_else(not)?;
            let end = (last.saturating_sub(0xFFFF)..=last).rev().find(|&i| u32_at(bytes, i) == Some(END)).ok_or_else(not)?;
            let count = u16_at(bytes, end + 10).ok_or_else(not)? as usize;
            let dir_offset = u32_at(bytes, end + 16).ok_or_else(not)? as usize;
            if count > MAX_ENTRIES {
                return Err(PackageError::TooLarge);
            }
            let mut entries = Vec::with_capacity(count);
            let mut at = dir_offset;
            for _ in 0..count {
                let field = |off: usize| u16_at(bytes, at + off).ok_or_else(|| damaged("central directory cut short"));
                let field32 = |off: usize| u32_at(bytes, at + off).ok_or_else(|| damaged("central directory cut short"));
                if field32(0)? != CENTRAL {
                    return Err(damaged("bad central directory"));
                }
                let flags = field(8)?;
                let (name_len, extra_len, comment_len) = (field(28)? as usize, field(30)? as usize, field(32)? as usize);
                let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(|| damaged("central directory cut short"))?;
                if flags & 1 != 0 {
                    return Err(damaged("encrypted entry"));
                }
                entries.push(Entry {
                    name: String::from_utf8_lossy(name).into_owned(),
                    method: field(10)?,
                    crc: field32(16)?,
                    packed: field32(20)? as u64,
                    size: field32(24)? as u64,
                    offset: field32(42)? as u64,
                });
                at += 46 + name_len + extra_len + comment_len;
            }
            Ok(Archive { bytes, entries })
        }

        /// An entry's contents, counted against `budget` (bytes left to unpack).
        pub fn read(&self, e: &Entry, budget: &mut u64) -> Result<Vec<u8>, PackageError> {
            if e.size > *budget {
                return Err(PackageError::TooLarge);
            }
            let short = || damaged(format!("{} cut short", e.name));
            let at = usize::try_from(e.offset).map_err(|_| short())?;
            if u32_at(self.bytes, at) != Some(LOCAL) {
                return Err(damaged(format!("{}: bad local header", e.name)));
            }
            let start =
                at + 30 + u16_at(self.bytes, at + 26).ok_or_else(short)? as usize + u16_at(self.bytes, at + 28).ok_or_else(short)? as usize;
            let data = self.bytes.get(start..start + e.packed as usize).ok_or_else(short)?;
            let out = match e.method {
                0 => data.to_vec(),
                8 => {
                    let mut out = Vec::with_capacity(e.size.min(1 << 20) as usize);
                    flate2::read::DeflateDecoder::new(data)
                        .take(e.size + 1)
                        .read_to_end(&mut out)
                        .map_err(|err| damaged(format!("{}: {err}", e.name)))?;
                    out
                }
                m => return Err(damaged(format!("{}: compression method {m}", e.name))),
            };
            if out.len() as u64 != e.size || crc32fast::hash(&out) != e.crc {
                return Err(damaged(format!("{} does not match its checksum", e.name)));
            }
            *budget -= e.size;
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> ExportMeta {
        ExportMeta {
            title: "Notes".into(),
            description: "for you".into(),
            author: Person { id: "a1".into(), name: "Ann".into(), device: "d1".into() },
            app: "Flowie test".into(),
        }
    }

    fn texts(p: &Package) -> Vec<(String, Option<NodeId>)> {
        p.outline.nodes.iter().map(|n| (n.text.clone(), n.parent)).collect()
    }

    #[test]
    fn round_trip_keeps_ids_order_and_fields() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "a");
        let b = t.create_at(a, 0, "b");
        let c = t.create_after(b, "c");
        let d = t.create_at(b, 0, "d");
        t.set_kind(c, BlockKind::Todo);
        t.set_completed(c, true);
        let mut node = t.node(b).clone();
        node.note = "a note".into();
        t.put(node);
        let p = Package::export(&t, &[a], ExportOptions::default(), meta(), None);
        assert_eq!(texts(&p), [("a".into(), None), ("b".into(), Some(a)), ("d".into(), Some(b)), ("c".into(), Some(a))]);
        assert_eq!(p.outline.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [a, b, d, c]);
        assert_eq!(p.manifest.stats.nodes, 4);
        let bytes = p.to_bytes();
        // `mimetype` is the first entry, stored, so its text sits at a fixed offset.
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..38 + MIMETYPE.len()], MIMETYPE.as_bytes());
        let back = Package::read(&bytes).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.outline.nodes[3].kind, BlockKind::Todo);
        assert!(back.outline.nodes[3].completed);
        assert_eq!(back.outline.nodes[1].note, "a note");
        assert!(String::from_utf8(outline_json(&p.outline)).unwrap().contains("\"kind\": \"todo\""));
    }

    #[test]
    fn completed_children_can_be_left_out() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "a");
        let b = t.create_at(a, 0, "b");
        t.create_at(b, 0, "under b");
        t.create_after(b, "c");
        t.set_completed(b, true);
        t.set_completed(a, true);
        let p = Package::export(&t, &[a], ExportOptions { include_completed: false }, meta(), None);
        assert_eq!(texts(&p), [("a".into(), None), ("c".into(), Some(a))]);
    }

    #[test]
    fn links_outside_become_labels() {
        let mut t = Tree::new();
        let out = t.create_at(ROOT, 0, "outside");
        let a = t.create_after(out, "a");
        let b = t.create_at(a, 0, "b");
        let mut node = t.node(b).clone();
        node.text = format!("see [**A**](flowie:{a}) and [out](flowie:{out}) and [web](https://x.org)");
        t.put(node);
        let p = Package::export(&t, &[a], ExportOptions::default(), meta(), None);
        assert_eq!(p.outline.nodes[1].text, format!("see [**A**](flowie:{a}) and out and [web](https://x.org)"));
        assert_eq!(p.items()[0].children[0].text, "see **A** and out and [web](https://x.org)");
    }

    #[test]
    fn mirrors_keep_the_link_only_when_the_source_comes_along() {
        let mut t = Tree::new();
        let src = t.create_at(ROOT, 0, "source");
        let kid = t.create_at(src, 0, "kid");
        let a = t.create_after(src, "a");
        let m = t.create_mirror(src, a, 0).unwrap();

        // The source is outside: the mirror becomes a copy, with fresh ids below it.
        let p = Package::export(&t, &[a], ExportOptions::default(), meta(), None);
        let copy = &p.outline.nodes[1];
        assert_eq!((copy.id, copy.text.as_str(), copy.mirror_of, copy.was_mirror), (m, "source", None, true));
        let kid_copy = &p.outline.nodes[2];
        assert_eq!((kid_copy.text.as_str(), kid_copy.parent), ("kid", Some(m)));
        assert_ne!(kid_copy.id, kid);
        assert_eq!(kid_copy.id, Package::export(&t, &[a], ExportOptions::default(), meta(), None).outline.nodes[2].id);
        assert_eq!(uuid::Uuid::from_u128(kid_copy.id.0).get_version_num(), 4);

        // Both exported: the mirror stays a mirror.
        let p = Package::export(&t, &[src, a], ExportOptions::default(), meta(), None);
        let mirror = p.outline.nodes.iter().find(|n| n.id == m).unwrap();
        assert_eq!((mirror.mirror_of, mirror.was_mirror), (Some(src), false));
        assert_eq!(p.outline.nodes.len(), 4);
        let items = p.items();
        assert_eq!(items[1].children[0].children[0].text, "kid");
    }

    #[test]
    fn nested_and_repeated_roots_are_dropped() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "a");
        let b = t.create_at(a, 0, "b");
        let p = Package::export(&t, &[b, a, a], ExportOptions::default(), meta(), None);
        assert_eq!(p.outline.roots, [a]);
        assert_eq!(p.outline.nodes.len(), 2);
    }

    #[test]
    fn reading_refuses_what_it_cannot_trust() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "a");
        t.create_at(a, 0, "b");
        let p = Package::export(&t, &[a], ExportOptions::default(), meta(), None);

        assert_eq!(Package::read(b"hello"), Err(PackageError::NotAPackage));
        assert_eq!(Package::read(&zip::write(&[("readme.txt", b"hi", false)])), Err(PackageError::NotAPackage));

        let mut newer = p.clone();
        newer.manifest.version = VERSION + 1;
        assert_eq!(Package::read(&newer.to_bytes()), Err(PackageError::Newer(VERSION + 1)));

        let mut tampered = p.clone();
        tampered.outline.nodes[1].text = "changed".into();
        assert!(matches!(Package::read(&tampered.to_bytes()), Err(PackageError::Damaged(_))));

        // A cycle: b's parent is c, c's parent is b.
        let mut cyclic = p.clone();
        let (b, c) = (cyclic.outline.nodes[1].id, NodeId::random());
        cyclic.outline.nodes[1].parent = Some(c);
        cyclic.outline.nodes.push(PackageNode { id: c, parent: Some(b), ..cyclic.outline.nodes[1].clone() });
        let json = outline_json(&cyclic.outline);
        cyclic.manifest.outline_sha256 = hex(&Sha256::digest(&json));
        assert_eq!(Package::read(&cyclic.to_bytes()), Err(damaged("parent cycle")));

        // A flipped bit in the compressed data.
        let mut bytes = p.to_bytes();
        let n = bytes.len();
        bytes[n / 2] ^= 0x40;
        assert!(Package::read(&bytes).is_err());
    }

    #[test]
    fn rewrite_links_edits_only_links() {
        let s = "[a](flowie:1) **[b](x)** https://y.org [c](z)";
        let out = markup::rewrite_links(s, |u| match u {
            "flowie:1" => LinkEdit::Unlink,
            "x" => LinkEdit::Url("w".into()),
            _ => LinkEdit::Keep,
        });
        assert_eq!(out, "a **[b](w)** https://y.org [c](z)");
    }

    fn sample() -> (Tree, NodeId, NodeId, NodeId) {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "a");
        let b = t.create_at(a, 0, "b");
        let c = t.create_after(b, "c");
        let mut node = t.node(c).clone();
        node.text = format!("see [b](flowie:{b})");
        t.put(node);
        t.set_completed(b, true);
        t.create_mirror(b, c, 0).unwrap();
        t.take_journal();
        (t, a, b, c)
    }

    fn shape(t: &Tree, root: NodeId) -> Vec<String> {
        let mut out = vec![t.node(root).text.clone()];
        out.extend(t.descendants(root).into_iter().map(|d| t.node(t.source(d)).text.clone()));
        out
    }

    #[test]
    fn import_keeps_ids_order_and_mirrors() {
        let (t, a, b, c) = sample();
        let p = Package::read(&Package::export(&t, &[a], ExportOptions::default(), meta(), None).to_bytes()).unwrap();
        let mut other = Tree::new();
        let home = other.create_at(ROOT, 0, "home");
        assert!(p.present_in(&other).is_empty());
        let roots = p.insert_into(&mut other, ROOT, 1, ImportOptions { fresh_ids: false, keep_completion: true });
        assert_eq!(roots, [a]);
        assert_eq!(other.children(ROOT), [home, a]);
        assert_eq!(other.children(a), [b, c]);
        assert_eq!(shape(&other, a), shape(&t, a));
        assert!(other.node(b).completed);
        assert_eq!(other.backlinks(b), [c]);
        assert_eq!(other.mirrors_of(b).len(), 1);
        assert_eq!(p.present_in(&other).len(), 4);
    }

    #[test]
    fn import_a_copy_gets_new_ids_and_follows_them() {
        let (t, a, b, _) = sample();
        let p = Package::export(&t, &[a], ExportOptions::default(), meta(), None);
        let mut same = Tree::from_nodes(t.nodes().cloned());
        let roots = p.insert_into(&mut same, ROOT, 0, ImportOptions { fresh_ids: true, keep_completion: false });
        let copy = roots[0];
        assert_ne!(copy, a);
        assert_eq!(shape(&same, copy).len(), shape(&t, a).len());
        let (b2, c2) = (same.children(copy)[0], same.children(copy)[1]);
        assert!(!same.node(b2).completed);
        assert_eq!(same.node(c2).text, format!("see [b](flowie:{b2})"));
        assert_eq!(same.node(same.children(c2)[0]).mirror, b2);
        // The original is untouched.
        assert_eq!(same.backlinks(b).len(), 1);
    }

    #[test]
    fn impossible_mirrors_arrive_as_plain_bullets() {
        let (t, a, ..) = sample();
        let mut p = Package::export(&t, &[a], ExportOptions::default(), meta(), None);
        // A mirror of its own ancestor.
        let m = p.outline.nodes.iter_mut().find(|n| n.mirror_of.is_some()).unwrap();
        m.mirror_of = Some(a);
        let json = outline_json(&p.outline);
        p.manifest.outline_sha256 = hex(&Sha256::digest(&json));
        let back = Package::read(&p.to_bytes()).unwrap();
        let m = back.outline.nodes.iter().find(|n| n.was_mirror).unwrap();
        assert_eq!(m.mirror_of, None);
    }

    #[test]
    fn images_travel_in_the_package() {
        let dir = ImageDir::new(std::env::temp_dir().join(format!("flowie-pkg-images-{}", NodeId::random())));
        let png = crate::images::tests::png();
        let name = dir.put(&png).unwrap().name;
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "trip");
        let pic = t.create_at(a, 0, "a caption");
        let lost = t.create_at(a, 1, "file gone");
        for (id, image) in [(pic, name.clone()), (lost, format!("{}.png", "0".repeat(64)))] {
            let mut n = t.node(id).clone();
            n.kind = BlockKind::Image;
            n.image = image;
            t.put(n);
        }
        let p = Package::export(&t, &[a], ExportOptions::default(), meta(), Some(&dir));
        assert_eq!(p.manifest.stats.images, 1);
        assert_eq!(p.outline.nodes[2].kind, BlockKind::Bullet);
        let back = Package::read(&p.to_bytes()).unwrap();
        assert_eq!(back.images.get(&name), Some(&png));
        assert_eq!(back.outline.nodes[1].image.as_deref(), Some(name.as_str()));

        // An image whose bytes don't match its name is left out.
        let mut bad = p.clone();
        bad.images.insert(name.clone(), b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x09\0\0\0\x09".to_vec());
        let back = Package::read(&bad.to_bytes()).unwrap();
        assert!(back.images.is_empty());
        assert_eq!((back.outline.nodes[1].kind, back.outline.nodes[1].image.clone()), (BlockKind::Bullet, None));

        let mut other = Tree::new();
        let roots = Package::read(&p.to_bytes()).unwrap().insert_into(
            &mut other,
            ROOT,
            0,
            ImportOptions { fresh_ids: false, keep_completion: true },
        );
        let n = other.node(other.children(roots[0])[0]);
        assert_eq!((n.kind, n.image.as_str()), (BlockKind::Image, name.as_str()));
        std::fs::remove_dir_all(dir.root()).unwrap();
    }

    #[test]
    fn updates_merge_three_ways() {
        // Version 1, imported.
        let mut sender = Tree::new();
        let a = sender.create_at(ROOT, 0, "a");
        let b = sender.create_at(a, 0, "b");
        let c = sender.create_after(b, "c");
        let d = sender.create_after(c, "d");
        let e = sender.create_after(d, "e");
        let v1 = Package::export(&sender, &[a], ExportOptions::default(), meta(), None);
        let mut me = Tree::new();
        let home = me.create_at(ROOT, 0, "home");
        v1.insert_into(&mut me, home, 0, ImportOptions { fresh_ids: false, keep_completion: true });
        let bases: HashMap<NodeId, String> = v1.base_hashes().into_iter().collect();
        let known: HashSet<NodeId> = bases.keys().copied().collect();

        // Both sides edit: b only there, c only here, d on both sides; e removed there; f added there.
        for (id, text) in [(b, "b2"), (d, "d-theirs")] {
            sender.set_text(id, text);
        }
        sender.delete(e);
        let f = sender.create_after(b, "f");
        me.set_text(c, "c-mine");
        me.set_text(d, "d-mine");
        let v2 = Package::export(&sender, &[a], ExportOptions::default(), meta(), None);

        let plan = v2.plan_update(&me, &bases, &known);
        assert_eq!(plan.changed, [b]);
        assert_eq!(plan.added, [f]);
        assert_eq!(plan.conflicts, [d]);
        assert_eq!(plan.removed, [e]);

        let touched = v2.apply_update(&mut me, &plan, &[d].into_iter().collect());
        assert_eq!(touched.len(), 3);
        for id in &plan.removed {
            me.delete(*id);
        }
        let texts: Vec<String> = me.children(a).iter().map(|id| me.node(*id).text.clone()).collect();
        assert_eq!(texts, ["b2", "f", "c-mine", "d-theirs"]);
        assert_eq!(me.parent(a), home);

        // Updating again with the new bases finds nothing to do.
        let bases: HashMap<NodeId, String> = v2.base_hashes().into_iter().collect();
        let known: HashSet<NodeId> = bases.keys().copied().collect();
        let again = v2.plan_update(&me, &bases, &known);
        assert!(again.is_empty(), "{again:?}");
    }

    #[test]
    fn copied_package_update_uses_saved_id_map_and_remaps_links() {
        let mut sender = Tree::new();
        let root = sender.create_at(ROOT, 0, "source");
        let child = sender.create_at(root, 0, "[child](flowie:00000000000000000000000000000000)");
        sender.set_text(child, &format!("[root](flowie:{root})"));
        let v1 = Package::export(&sender, &[root], ExportOptions::default(), meta(), None);
        let mut local = Tree::new();
        let (copied_roots, id_map) = v1.insert_into_mapped(&mut local, ROOT, 0, ImportOptions { fresh_ids: true, keep_completion: true });
        let copied_root = copied_roots[0];
        let bases: HashMap<NodeId, String> = v1.base_hashes_mapped(&id_map).into_iter().collect();
        let known = bases.keys().copied().collect();

        sender.set_text(root, "source updated");
        let plan =
            Package::export(&sender, &[root], ExportOptions::default(), meta(), None).plan_update_mapped(&local, &bases, &known, &id_map);
        assert_eq!(plan.changed, [copied_root]);
        let updated = Package::export(&sender, &[root], ExportOptions::default(), meta(), None);
        updated.apply_update_mapped(&mut local, &plan, &HashSet::new(), &id_map);
        assert_eq!(local.node(copied_root).text, "source updated");
        assert_eq!(local.node(id_map[&child]).text, format!("[root](flowie:{copied_root})"));
    }
}
