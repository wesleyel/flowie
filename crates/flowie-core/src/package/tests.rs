use std::collections::HashSet;

use super::archive::outline_json;
use super::*;
use crate::Tree;

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
    let roots =
        Package::read(&p.to_bytes()).unwrap().insert_into(&mut other, ROOT, 0, ImportOptions { fresh_ids: false, keep_completion: true });
    let n = other.node(other.children(roots[0])[0]);
    assert_eq!((n.kind, n.image.as_str()), (BlockKind::Image, name.as_str()));
    std::fs::remove_dir_all(dir.root()).unwrap();
}

#[test]
fn image_mirrors_round_trip_using_their_sources_attachment() {
    let dir = ImageDir::new(std::env::temp_dir().join(format!("flowie-pkg-image-mirror-{}", NodeId::random())));
    let info = dir.put(&crate::images::tests::png()).unwrap();
    let mut tree = Tree::new();
    let root = tree.create_at(ROOT, 0, "root");
    let source = tree.create_at(root, 0, "caption");
    let mut node = tree.node(source).clone();
    node.kind = BlockKind::Image;
    node.image = info.name.clone();
    tree.put(node);
    let mirror = tree.create_mirror(source, root, 0).unwrap();

    let package = Package::export(&tree, &[root], ExportOptions::default(), meta(), Some(&dir));
    let read = Package::read(&package.to_bytes()).unwrap();
    assert_eq!(read.images.len(), 1);
    let entry = &read.outline.nodes[1];
    assert_eq!((entry.id, entry.kind, entry.mirror_of, entry.image.as_ref()), (mirror, BlockKind::Image, Some(source), None));
    let items = read.items();
    let preview = &items[0].children[0];
    assert_eq!((preview.kind, preview.image.as_str()), (BlockKind::Image, info.name.as_str()));

    let mut imported = Tree::new();
    read.insert_into(&mut imported, ROOT, 0, ImportOptions { fresh_ids: false, keep_completion: true });
    assert_eq!(imported.source(mirror), source);
    assert_eq!(imported.node(mirror).kind, BlockKind::Image);
    assert_eq!(imported.node(source).image, info.name);

    let baked = Package::export(&tree, &[mirror], ExportOptions::default(), meta(), Some(&dir));
    let baked = Package::read(&baked.to_bytes()).unwrap();
    let entry = &baked.outline.nodes[0];
    assert_eq!((entry.kind, entry.mirror_of, entry.image.as_ref()), (BlockKind::Image, None, Some(&info.name)));

    let mut without_attachment = package;
    without_attachment.images.clear();
    let read = Package::read(&without_attachment.to_bytes()).unwrap();
    assert!(read.outline.nodes[1..].iter().all(|node| node.kind == BlockKind::Bullet));
    std::fs::remove_dir_all(dir.root()).unwrap();
}

#[test]
fn package_update_keeps_forward_mirrors_and_skips_missing_sources() {
    let opts = ExportOptions::default();
    let mut sender = Tree::new();
    let root = sender.create_at(ROOT, 0, "root");
    let changing = sender.create_at(root, 0, "becomes a mirror");
    let deleted_here = sender.create_at(root, 1, "will be kept deleted");
    let previous = Package::export(&sender, &[root], opts, meta(), None);
    let mut local = Tree::new();
    previous.insert_into(&mut local, ROOT, 0, ImportOptions { fresh_ids: false, keep_completion: true });
    local.delete(deleted_here);

    let later_source = sender.create_at(root, 2, "later source");
    let new_mirror = sender.create_mirror(later_source, root, 0).unwrap();
    let broken_mirror = sender.create_mirror(deleted_here, root, 0).unwrap();
    let mut changing_node = sender.node(changing).clone();
    changing_node.mirror = later_source;
    sender.put(changing_node);
    let next = Package::export(&sender, &[root], opts, meta(), None);
    let bases: HashMap<_, _> = previous.base_hashes().into_iter().collect();
    let plan = next.plan_update(&local, &bases, &bases.keys().copied().collect());
    assert_eq!(plan.added, [broken_mirror, new_mirror, later_source]);
    assert_eq!(plan.changed, [changing]);
    assert_eq!(plan.deleted_here, 1);

    let touched = next.apply_update(&mut local, &plan, &HashSet::new());
    assert_eq!(touched, [broken_mirror, new_mirror, changing, later_source]);
    assert_eq!(local.mirrors_of(later_source), [new_mirror, changing]);
    assert_eq!(local.node(broken_mirror).mirror, ROOT);
    assert!(!local.contains(deleted_here));
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
    let other_root = sender.create_at(ROOT, 1, "destination");
    let child = sender.create_at(root, 0, "child");
    sender.set_text(child, &format!("[root](flowie:{root})"));
    let roots = [root, other_root];
    let v1 = Package::export(&sender, &roots, ExportOptions::default(), meta(), None);
    let mut local = Tree::new();
    let opts = ImportOptions { fresh_ids: true, keep_completion: true };
    let (copied_roots, id_map) = v1.insert_into_mapped(&mut local, ROOT, 0, opts);
    let copied_root = copied_roots[0];
    let copied_child = id_map[&child];
    let bases: HashMap<NodeId, String> = v1.mapped_base_hashes(&id_map).map(|(source, hash)| (id_map[&source], hash)).collect();
    let known = bases.keys().copied().collect();

    let (duplicate_roots, duplicate_map) = v1.insert_into_mapped(&mut local, ROOT, 2, opts);
    assert!(duplicate_roots.iter().all(|id| !copied_roots.contains(id)));
    assert_eq!(local.node(duplicate_map[&child]).text, format!("[root](flowie:{})", duplicate_roots[0]));
    assert_eq!(local.node(copied_child).text, format!("[root](flowie:{copied_root})"));

    local.set_text(copied_child, "my text");
    sender.set_text(root, "source updated");
    sender.move_to(child, other_root, 0);
    sender.set_text(child, &format!("see [root](flowie:{root})"));
    let added = sender.create_at(other_root, 1, &format!("see [child](flowie:{child})"));
    let updated = Package::export(&sender, &roots, ExportOptions::default(), meta(), None);
    let plan = updated.plan_update_mapped(&local, &bases, &known, &id_map);
    let copied_added = plan.id_map()[&added];
    assert_eq!(plan.changed, [copied_root]);
    assert_eq!(plan.added, [copied_added]);
    assert_eq!(plan.conflicts, [copied_child]);
    assert_eq!(updated.update_conflict(&plan, copied_child).unwrap().id, child);
    assert_eq!(plan.id_map()[&child], copied_child);
    updated.apply_update(&mut local, &plan, &[copied_child].into_iter().collect());
    assert_eq!(local.node(copied_root).text, "source updated");
    assert_eq!(local.parent(copied_child), copied_roots[1]);
    assert_eq!(local.node(copied_child).text, format!("see [root](flowie:{copied_root})"));
    assert_eq!(local.node(copied_added).text, format!("see [child](flowie:{copied_child})"));
    assert_eq!(local.node(duplicate_roots[0]).text, "source");

    let id_map = plan.id_map();
    let bases: HashMap<_, _> = updated.mapped_base_hashes(id_map).map(|(source, hash)| (id_map[&source], hash)).collect();
    let known = bases.keys().copied().collect();
    assert!(updated.plan_update_mapped(&local, &bases, &known, id_map).is_empty());
}
