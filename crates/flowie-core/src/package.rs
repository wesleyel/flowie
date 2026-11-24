//! Share packages (`.flowie`): bullets with their children in one file, for
//! sending to someone else. A zip archive holding
//!
//! - `mimetype`: first and stored uncompressed (as in EPUB), so a package is
//!   recognizable without its extension;
//! - `manifest.json`: who exported it, when, and what (`Manifest`);
//! - `outline.json`: the nodes as a flat list in outline order (`Outline`);
//!   flat because deep nesting would hit the JSON parser's depth limit;
//! - `preview.md`: the outline as Markdown, for people without Flowie.
//!
//! Node ids are kept: they are global, so the same bullet has the same id in
//! every outline it reaches. See `docs/roadmap.md` for the design.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

impl Package {
    /// Package `roots` (and their children) of `tree`. Roots inside another
    /// root, missing ones and duplicates are dropped.
    pub fn export(tree: &Tree, roots: &[NodeId], opts: ExportOptions, meta: ExportMeta) -> Package {
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
                image: None,
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

        let outline = Outline { roots: roots.clone(), nodes };
        let json = outline_json(&outline);
        let manifest = Manifest {
            format: FORMAT.into(),
            version: VERSION,
            package_id: uuid::Uuid::new_v4().hyphenated().to_string(),
            title: meta.title,
            description: meta.description,
            exported_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            exported_by: meta.author,
            app: meta.app,
            roots,
            stats: Stats { nodes: outline.nodes.len(), images: 0, bytes: json.len() as u64 },
            outline_sha256: hex(&Sha256::digest(&json)),
        };
        Package { manifest, outline }
    }

    /// The package file's bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let manifest = serde_json::to_vec_pretty(&self.manifest).expect("manifest serializes");
        let preview = crate::io::to_markdown(&self.items());
        zip::write(&[
            ("mimetype", MIMETYPE.as_bytes(), false),
            ("manifest.json", &manifest, true),
            ("outline.json", &outline_json(&self.outline), true),
            ("preview.md", preview.as_bytes(), true),
        ])
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
        Ok(Package { manifest, outline })
    }

    /// The outline as a nested tree (for previews): no ids, and links between
    /// bullets reduced to their labels.
    pub fn items(&self) -> Vec<Item> {
        let mut kids: HashMap<NodeId, Vec<&PackageNode>> = HashMap::new();
        let mut by_id: HashMap<NodeId, &PackageNode> = HashMap::new();
        for n in &self.outline.nodes {
            by_id.insert(n.id, n);
            if let Some(p) = n.parent {
                kids.entry(p).or_default().push(n);
            }
        }
        let unlink = |s: &str| markup::rewrite_links(s, |url| if url.starts_with("flowie:") { LinkEdit::Unlink } else { LinkEdit::Keep });
        fn build(
            n: &PackageNode,
            by_id: &HashMap<NodeId, &PackageNode>,
            kids: &HashMap<NodeId, Vec<&PackageNode>>,
            unlink: &dyn Fn(&str) -> String,
            depth: usize,
        ) -> Item {
            // A kept mirror shows its source.
            let shown = n.mirror_of.and_then(|m| by_id.get(&m).copied()).filter(|_| depth < 64).unwrap_or(n);
            let children = if depth >= 200 {
                Vec::new()
            } else {
                kids.get(&shown.id).into_iter().flatten().map(|c| build(c, by_id, kids, unlink, depth + 1)).collect()
            };
            Item {
                text: unlink(&shown.text),
                note: unlink(&shown.note),
                completed: shown.completed,
                kind: n.kind,
                collapsed: n.collapsed,
                children,
            }
        }
        self.outline.roots.iter().filter_map(|r| by_id.get(r)).map(|n| build(n, &by_id, &kids, &unlink, 0)).collect()
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

fn outline_json(outline: &Outline) -> Vec<u8> {
    serde_json::to_vec_pretty(outline).expect("outline serializes")
}

/// Check a package's outline is a forest the importer can rely on: unique
/// ids, parents inside the package, every node reachable from a root.
/// Mirrors of nodes not in the package become plain bullets.
fn validate(outline: &mut Outline) -> Result<(), PackageError> {
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
    for n in &mut outline.nodes {
        if n.mirror_of.is_some_and(|m| m == n.id || !ids.contains(&m)) {
            n.mirror_of = None;
            n.was_mirror = true;
        }
        n.image = None;
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
        let p = Package::export(&t, &[a], ExportOptions::default(), meta());
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
        let p = Package::export(&t, &[a], ExportOptions { include_completed: false }, meta());
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
        let p = Package::export(&t, &[a], ExportOptions::default(), meta());
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
        let p = Package::export(&t, &[a], ExportOptions::default(), meta());
        let copy = &p.outline.nodes[1];
        assert_eq!((copy.id, copy.text.as_str(), copy.mirror_of, copy.was_mirror), (m, "source", None, true));
        let kid_copy = &p.outline.nodes[2];
        assert_eq!((kid_copy.text.as_str(), kid_copy.parent), ("kid", Some(m)));
        assert_ne!(kid_copy.id, kid);
        assert_eq!(kid_copy.id, Package::export(&t, &[a], ExportOptions::default(), meta()).outline.nodes[2].id);
        assert_eq!(uuid::Uuid::from_u128(kid_copy.id.0).get_version_num(), 4);

        // Both exported: the mirror stays a mirror.
        let p = Package::export(&t, &[src, a], ExportOptions::default(), meta());
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
        let p = Package::export(&t, &[b, a, a], ExportOptions::default(), meta());
        assert_eq!(p.outline.roots, [a]);
        assert_eq!(p.outline.nodes.len(), 2);
    }

    #[test]
    fn reading_refuses_what_it_cannot_trust() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "a");
        t.create_at(a, 0, "b");
        let p = Package::export(&t, &[a], ExportOptions::default(), meta());

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
}
