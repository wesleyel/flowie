//! Encode and validate the on-disk share package format.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    damaged, hex, zip, BlockKind, Manifest, NodeId, Outline, Package, PackageError, PackageNode, FORMAT, MAX_NODES, MIMETYPE, VERSION,
};
use crate::ROOT;

/// Most bytes a package may unpack to, all entries together.
const MAX_UNPACKED: u64 = 256 << 20;

impl Package {
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
        strip_missing_image_kinds(&mut outline.nodes);
        Ok(Package { manifest, outline, images: files })
    }
}

pub(super) fn strip_missing_image_kinds(nodes: &mut [PackageNode]) {
    let image_sources: HashSet<_> = nodes.iter().filter(|node| node.image.is_some()).map(|node| node.id).collect();
    for node in nodes {
        if node.kind == BlockKind::Image && node.image.is_none() && !node.mirror_of.is_some_and(|id| image_sources.contains(&id)) {
            node.kind = BlockKind::Bullet;
        }
    }
}

pub(super) fn outline_json(outline: &Outline) -> Vec<u8> {
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
