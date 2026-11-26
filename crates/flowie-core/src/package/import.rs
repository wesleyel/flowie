//! Insert package nodes and match updates to previously imported roots.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use super::{remap_links, ImportOptions, Package, PackageNode, UpdatePlan};
use crate::store::{self, ImportRecord, ImportState};
use crate::{NodeId, Tree, ROOT};

pub struct ImportComparison {
    pub revision: u64,
    pub before: ImportState,
    pub already: bool,
    pub update: Option<UpdatePlan>,
}

fn copied_imports<'a>(package: &Package, tree: &Tree, records: &'a HashMap<NodeId, ImportRecord>) -> Vec<&'a ImportRecord> {
    let roots: HashSet<_> = package.outline.roots.iter().copied().collect();
    let mut batches = BTreeMap::<_, Vec<_>>::new();
    for record in records.values().filter(|record| record.package_id == package.manifest.package_id) {
        if let Some(&(source, _)) = record.source_ids.iter().find(|&&(source, local)| local == record.root && source != local) {
            if roots.contains(&source) {
                batches.entry(record.imported_at).or_default().push((source, record));
            }
        }
    }
    let complete = |batch: &[(NodeId, &'a ImportRecord)]| {
        batch.len() == roots.len()
            && batch.iter().all(|&(_, record)| tree.contains(record.root))
            && batch.iter().map(|&(source, _)| source).collect::<HashSet<_>>() == roots
    };
    let batch = batches.values().rev().find(|batch| complete(batch)).cloned().unwrap_or_else(|| {
        let batch: Vec<_> = batches.into_values().flatten().collect();
        if complete(&batch) {
            batch
        } else {
            Vec::new()
        }
    });
    let by_source: HashMap<_, _> = batch.into_iter().collect();
    package.outline.roots.iter().filter_map(|source| by_source.get(source).copied()).collect()
}

pub fn package_import_state(package: &Package, id_map: &HashMap<NodeId, NodeId>, imported_at: i64) -> ImportState {
    let root_of = package.root_of();
    let mut source_ids = HashMap::<_, Vec<_>>::new();
    let bases = package
        .mapped_base_hashes(id_map)
        .map(|(source, hash)| {
            let root = id_map[&root_of[&source]];
            let local = id_map[&source];
            source_ids.entry(root).or_default().push((source, local));
            (local, root, hash)
        })
        .collect();
    let roots: Vec<_> = package.outline.roots.iter().map(|source| id_map[source]).collect();
    let manifest = &package.manifest;
    let records = roots
        .iter()
        .map(|&root| ImportRecord {
            root,
            package_id: manifest.package_id.clone(),
            title: manifest.title.clone(),
            author_id: manifest.exported_by.id.clone(),
            author_name: manifest.exported_by.name.clone(),
            exported_at: manifest.exported_at_millis(),
            imported_at,
            seen_at: None,
            source_ids: source_ids.remove(&root).unwrap_or_default(),
        })
        .collect();
    ImportState { roots, records, bases }
}

pub fn compare_package(
    package: &Package,
    tree: &Tree,
    imports: &HashMap<NodeId, ImportRecord>,
    db_path: Option<&Path>,
) -> rusqlite::Result<ImportComparison> {
    let connection = db_path.map(store::open).transpose()?;
    let load_bases = |roots: &[NodeId]| {
        connection.as_ref().map(|connection| store::load_bases(connection, roots)).transpose().map(Option::unwrap_or_default)
    };
    let source_roots = &package.outline.roots;
    let source_bases = load_bases(source_roots)?;
    let can_update_original = !source_bases.is_empty() && source_roots.iter().all(|&root| tree.contains(root));
    let copies = if can_update_original { Vec::new() } else { copied_imports(package, tree, imports) };
    let already = !package.present_in(tree).is_empty() || imports.values().any(|record| record.package_id == package.manifest.package_id);
    let (roots, bases) = if copies.is_empty() {
        (source_roots.clone(), source_bases)
    } else {
        let roots: Vec<_> = copies.iter().map(|record| record.root).collect();
        let bases = load_bases(&roots)?;
        (roots, bases)
    };
    let update = if (can_update_original || !copies.is_empty()) && !bases.is_empty() {
        let id_map = copies.iter().flat_map(|record| record.source_ids.iter().copied()).collect();
        let hashes: HashMap<_, _> = bases.iter().map(|(node, _, hash)| (*node, hash.clone())).collect();
        let known = hashes.keys().copied().collect();
        Some(package.plan_update_mapped(tree, &hashes, &known, &id_map))
    } else {
        None
    };
    let records = roots.iter().filter_map(|id| imports.get(id).cloned()).collect();
    let before = ImportState { records, roots, bases };
    Ok(ImportComparison { revision: tree.revision(), before, already, update })
}

impl Package {
    /// Ids of this package's bullets that `tree` has already, in the outline or in the
    /// Trash: it was imported before.
    pub fn present_in(&self, tree: &Tree) -> Vec<NodeId> {
        self.outline.nodes.iter().map(|n| n.id).filter(|&id| tree.has_record(id)).collect()
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
        let root_ids = if opts.fresh_ids { self.outline.roots.iter().map(|&id| (id, tree.alloc_id())).collect() } else { HashMap::new() };
        let id_map = self.complete_id_map(&root_ids);
        let map = |id: NodeId| id_map[&id];
        let by_id: HashMap<NodeId, &PackageNode> = self.outline.nodes.iter().map(|n| (n.id, n)).collect();
        let kids = self.children();
        let now = crate::tree::now_millis();
        let mut roots = Vec::new();
        let index = index.min(tree.children(parent).len());
        let ranks = tree.ranks_at(parent, index, self.outline.roots.len());
        for (&r, rank) in self.outline.roots.iter().zip(ranks) {
            // (package node, parent in the tree, rank), parents before children.
            // Ranks are made here, for this tree: packages carry only order.
            let mut stack = vec![(r, parent, rank)];
            while let Some((id, under, rank)) = stack.pop() {
                let Some(n) = by_id.get(&id) else { continue };
                let new_id = map(id);
                let children = kids.get(&id).map(Vec::as_slice).unwrap_or(&[]);
                tree.put(crate::tree::Node {
                    id: new_id,
                    parent: under,
                    rank,
                    text: remap_links(&n.text, &id_map).into_owned(),
                    note: remap_links(&n.note, &id_map).into_owned(),
                    completed: n.completed && opts.keep_completion,
                    collapsed: n.collapsed && (!children.is_empty() || n.mirror_of.is_some()),
                    created: if n.created > 0 { n.created } else { now },
                    modified: if n.modified > 0 { n.modified } else { now },
                    kind: n.kind,
                    mirror: n.mirror_of.map_or(ROOT, map),
                    image: n.image.clone().unwrap_or_default(),
                    star: String::new(),
                    deleted_at: None,
                    sync: Default::default(),
                });
                if under == parent {
                    roots.push(new_id);
                }
                // `new_id` is new, so its children are the only ones.
                for (&c, rank) in children.iter().zip(crate::rank::n_between(None, None, children.len())).rev() {
                    stack.push((c, new_id, rank));
                }
            }
        }
        (roots, id_map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::images::ImageDir;
    use crate::package::{ExportMeta, ExportOptions};
    use crate::BlockKind;

    #[test]
    fn update_uses_a_tracked_copy_when_the_original_has_no_baseline() {
        let work = std::env::temp_dir().join(format!("flowie-package-baseline-{}", NodeId::random()));
        let db = work.join("flowie.sqlite");
        let mut connection = store::open(&db).unwrap();
        let mut sender = Tree::new();
        let root = sender.create_at(ROOT, 0, "root");
        let child = sender.create_at(root, 0, "before");
        let previous = Package::export(&sender, &[root], ExportOptions::default(), ExportMeta::default(), None);
        let mut receiver = Tree::from_nodes(sender.nodes().cloned());
        let (copies, id_map) =
            previous.insert_into_mapped(&mut receiver, ROOT, 1, ImportOptions { fresh_ids: true, keep_completion: true });
        let state = package_import_state(&previous, &id_map, 1);
        store::write_import_batch(&mut connection, &[], &[], &state).unwrap();
        let mut imports: HashMap<_, _> = state.records.into_iter().map(|record| (record.root, record)).collect();
        sender.set_text(child, "after");
        let updated = Package::export(&sender, &[root], ExportOptions::default(), ExportMeta::default(), None);

        let comparison = compare_package(&updated, &receiver, &imports, Some(&db)).unwrap();
        assert_eq!(comparison.before.roots, copies);
        let plan = comparison.update.unwrap();
        assert_eq!(plan.changed, [id_map[&child]]);
        updated.apply_update(&mut receiver, &plan, &HashSet::new());
        assert_eq!(receiver.node(id_map[&child]).text, "after");
        assert_eq!(receiver.node(child).text, "before");

        let original = package_import_state(&previous, &HashMap::from([(root, root), (child, child)]), 2);
        store::write_import_batch(&mut connection, &[], &[], &original).unwrap();
        imports.extend(original.records.into_iter().map(|record| (record.root, record)));
        let comparison = compare_package(&updated, &receiver, &imports, Some(&db)).unwrap();
        assert_eq!(comparison.before.roots, [root]);
        assert_eq!(comparison.update.unwrap().changed, [child]);
        drop(connection);
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn copied_imports_use_one_complete_batch_and_keep_all_mappings() {
        let mut sender = Tree::new();
        let first = sender.create_at(ROOT, 0, "first");
        let second = sender.create_at(ROOT, 1, "second");
        let child = sender.create_at(first, 0, "child");
        let package = Package::export(&sender, &[first, second], ExportOptions::default(), ExportMeta::default(), None);
        let mut receiver = Tree::new();
        let opts = ImportOptions { fresh_ids: true, keep_completion: true };
        let mut records = HashMap::new();
        let insert = |receiver: &mut Tree, records: &mut HashMap<_, _>, imported_at| {
            let (roots, id_map) = package.insert_into_mapped(receiver, ROOT, 0, opts);
            let state = package_import_state(&package, &id_map, imported_at);
            assert_eq!(state.roots, roots);
            assert!(state.bases.iter().any(|(node, root, _)| *node == id_map[&child] && *root == roots[0]));
            records.extend(state.records.into_iter().map(|record| (record.root, record)));
            (roots, id_map)
        };
        let (older, _) = insert(&mut receiver, &mut records, 10);
        let (newer, newer_map) = insert(&mut receiver, &mut records, 20);
        let (incomplete, _) = insert(&mut receiver, &mut records, 30);
        receiver.delete(incomplete[1]);

        let batch = copied_imports(&package, &receiver, &records);
        assert_eq!(batch.iter().map(|record| record.root).collect::<Vec<_>>(), newer);
        assert!(batch[0].source_ids.contains(&(child, newer_map[&child])));
        receiver.delete(newer[0]);
        let batch = copied_imports(&package, &receiver, &records);
        assert_eq!(batch.iter().map(|record| record.root).collect::<Vec<_>>(), older);
        receiver.delete(older[0]);
        assert!(copied_imports(&package, &receiver, &records).is_empty());
    }

    #[test]
    fn package_image_write_failure_can_be_retried() {
        let work = std::env::temp_dir().join(format!("flowie-package-write-{}", NodeId::random()));
        let sender = ImageDir::new(work.join("sender"));
        let receiver = ImageDir::new(work.join("receiver"));
        let mut tree = Tree::new();
        let root = tree.create_at(ROOT, 0, "album");
        for bytes in [b"GIF89a\x05\x00\x07\x00", b"GIF89a\x06\x00\x07\x00"] {
            let image = sender.put(bytes).unwrap();
            let id = tree.create_at(root, tree.children(root).len(), "caption");
            let mut node = tree.node(id).clone();
            node.kind = BlockKind::Image;
            node.image = image.name;
            tree.put(node);
        }
        let package = Package::export(&tree, &[root], ExportOptions::default(), ExportMeta::default(), Some(&sender));
        let names: Vec<_> = package.images.keys().collect();
        let blocked = receiver.path(names[1]).unwrap();
        std::fs::create_dir_all(&blocked).unwrap();
        assert!(package.write_images(&receiver).is_err());
        assert!(receiver.has(names[0]));
        assert!(!receiver.has(names[1]));

        std::fs::remove_dir(&blocked).unwrap();
        let stored = package.write_images(&receiver).unwrap();
        assert_eq!(stored.iter().map(|blob| &blob.name).collect::<Vec<_>>(), names);
        assert!(names.iter().all(|name| receiver.has(name)));
        std::fs::remove_dir_all(work).unwrap();
    }
}
