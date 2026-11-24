//! Build a package from the selected subtrees.

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use super::archive::{outline_json, strip_missing_image_kinds};
use super::{copy_id, hex, BlockKind, ExportMeta, ExportOptions, Manifest, NodeId, Outline, Package, PackageNode, Stats, FORMAT, VERSION};
use crate::images::ImageDir;
use crate::markup::{self, LinkEdit};
use crate::{Tree, ROOT};

/// Deepest level exported.
const MAX_DEPTH: usize = 1000;
/// Most mirrors expanded into copies inside one another.
const MAX_EXPANSIONS: usize = 8;

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
                    }
                },
            }
        }
        strip_missing_image_kinds(&mut nodes);
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
}
