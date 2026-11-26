//! Plan and apply three-way updates using package content hashes.

use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};

use super::{hex, remap_links, Package, PackageNode};
use crate::{BlockKind, NodeId, Tree, ROOT};

impl Package {
    /// Each bullet's content hash as this package has it: the base the next
    /// version is merged against (`plan_update`).
    pub fn base_hashes(&self) -> Vec<(NodeId, String)> {
        self.mapped_base_hashes(&HashMap::new()).collect()
    }

    /// The source ID and mapped content hash of each bullet, using the complete
    /// mapping from an insertion or update plan.
    pub fn mapped_base_hashes<'a>(&'a self, id_map: &'a HashMap<NodeId, NodeId>) -> impl Iterator<Item = (NodeId, String)> + 'a {
        self.outline.nodes.iter().map(move |node| (node.id, package_hash(node, id_map)))
    }

    /// The content hash of `id` as `tree` has it now (`None` if it's gone).
    fn local_hash(tree: &Tree, id: NodeId, is_root: bool) -> Option<String> {
        let n = tree.get(id)?;
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
        let roots: HashSet<_> = self.outline.roots.iter().map(|root| complete[root]).collect();
        let ids: HashSet<_> = complete.values().copied().collect();
        let mut plan = UpdatePlan { id_map: complete, ..Default::default() };
        for (index, n) in self.outline.nodes.iter().enumerate() {
            let id = plan.id_map[&n.id];
            let theirs = package_hash(n, &plan.id_map);
            match (Self::local_hash(tree, id, n.parent.is_none()), bases.get(&id)) {
                // The same on both sides.
                (Some(mine), _) if mine == theirs => {}
                // Unchanged in the package (changed here, or nowhere): keep.
                (Some(_), Some(base)) if *base == theirs => {}
                (Some(mine), Some(base)) if mine == *base => plan.changed.push(id),
                // Changed on both sides, or no base to tell.
                (Some(_), _) => {
                    plan.conflicts.push(id);
                    plan.conflict_sources.insert(id, index);
                }
                (None, Some(_)) => plan.deleted_here += 1,
                (None, None) => plan.added.push(id),
            }
        }
        // Gone from the package and unchanged here: remove (only the top of a removed subtree).
        let mut gone: Vec<NodeId> = known
            .iter()
            .copied()
            .filter(|id| !ids.contains(id) && tree.contains(*id))
            .filter(|id| bases.get(id).is_some_and(|b| Self::local_hash(tree, *id, roots.contains(id)).as_ref() == Some(b)))
            .collect();
        let set: HashSet<NodeId> = gone.iter().copied().collect();
        gone.retain(|id| !tree.ancestors(*id).iter().any(|a| set.contains(a)));
        // Nor anything holding bullets the package still has, or new ones of the user's.
        gone.retain(|id| tree.descendants(*id).iter().all(|d| set.contains(d)));
        gone.sort_unstable();
        plan.removed = gone;
        plan
    }

    /// The package's side of a conflict keyed by the receiver's node ID.
    pub fn update_conflict<'a>(&'a self, plan: &UpdatePlan, id: NodeId) -> Option<&'a PackageNode> {
        self.outline.nodes.get(*plan.conflict_sources.get(&id)?)
    }

    /// Apply the package side of `plan` to `tree`: changed bullets, new
    /// ones, and the conflicts listed in `take_theirs`. Removals are left to
    /// the caller (they go to the Trash, `Tree::trash`). Returns the bullets touched.
    pub fn apply_update(&self, tree: &mut Tree, plan: &UpdatePlan, take_theirs: &HashSet<NodeId>) -> Vec<NodeId> {
        let complete = plan.id_map();
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
                    text: remap_links(&n.text, complete).into_owned(),
                    note: remap_links(&n.note, complete).into_owned(),
                    completed: n.completed,
                    collapsed: n.collapsed,
                    created: n.created,
                    modified: n.modified,
                    kind: n.kind,
                    mirror: ROOT,
                    image: n.image.clone().unwrap_or_default(),
                    star: String::new(),
                    deleted_at: None,
                });
                touched.push(id);
            } else if take.contains(&id) && tree.contains(id) {
                let mut node = tree.node(id).clone();
                node.text = remap_links(&n.text, complete).into_owned();
                node.note = remap_links(&n.note, complete).into_owned();
                node.kind = n.kind;
                node.completed = n.completed;
                node.image = n.image.clone().unwrap_or_default();
                tree.put(node);
                if let Some(parent) = n.parent.map(map).filter(|p| tree.contains(*p) && tree.parent(id) != *p) {
                    let index = slot(tree, parent, id);
                    tree.move_to(id, parent, index);
                }
                touched.push(id);
            }
        }
        for &id in &touched {
            let source = by_id[&id].mirror_of.map(map).filter(|&source| tree.contains(source)).unwrap_or(ROOT);
            if tree.node(id).mirror != source {
                let mut node = tree.node(id).clone();
                node.mirror = source;
                tree.put(node);
            }
        }
        touched
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
    id_map: HashMap<NodeId, NodeId>,
    conflict_sources: HashMap<NodeId, usize>,
}

impl UpdatePlan {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty() && self.conflicts.is_empty()
    }

    pub fn id_map(&self) -> &HashMap<NodeId, NodeId> {
        &self.id_map
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

fn package_hash(n: &PackageNode, id_map: &HashMap<NodeId, NodeId>) -> String {
    let map = |id| id_map.get(&id).copied().unwrap_or(id);
    let text = remap_links(&n.text, id_map);
    let note = remap_links(&n.note, id_map);
    content_hash(&text, &note, n.kind, n.completed, n.image.as_deref(), n.parent.map(map), n.mirror_of.map(map))
}
