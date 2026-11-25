use crate::flatten::flatten_mapped;
use crate::io;
use crate::markup::{self, Kind};
use crate::search::{find_highlights, Query};
use crate::tree::virtual_id;
use crate::*;

fn texts(tree: &Tree, parent: NodeId) -> Vec<String> {
    tree.children(parent).iter().map(|id| tree.node(*id).text.clone()).collect()
}

fn outline(tree: &Tree, root: NodeId) -> String {
    flatten(tree, root, &ViewOptions::default())
        .iter()
        .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), tree.node(r.id).text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn sample() -> (Tree, [NodeId; 4]) {
    let mut t = Tree::new();
    let a = t.create_at(ROOT, 0, "a");
    let b = t.create_after(a, "b");
    let c = t.create_after(b, "c");
    let d = t.create_at(b, 0, "d");
    t.take_journal();
    (t, [a, b, c, d])
}

#[test]
fn create_and_order() {
    let (t, _) = sample();
    assert_eq!(outline(&t, ROOT), "a\nb\n  d\nc");
}

#[test]
fn indent_outdent() {
    let (mut t, [_a, b, c, _d]) = sample();
    assert!(t.indent(c));
    assert_eq!(outline(&t, ROOT), "a\nb\n  d\n  c");
    assert!(t.outdent(c, ROOT));
    assert_eq!(outline(&t, ROOT), "a\nb\n  d\nc");
    assert!(!t.indent(t.children(ROOT)[0]), "first child cannot indent");
    let _ = b;
}

#[test]
fn split_and_merge() {
    let (mut t, [a, ..]) = sample();
    t.set_text(a, "hello world");
    let (new, caret) = t.split(a, 5);
    assert_eq!((t.node(a).text.as_str(), t.node(new).text.as_str(), caret), ("hello", " world", 0));
    let caret = t.merge_into(new, a).unwrap();
    assert_eq!((t.node(a).text.as_str(), caret), ("hello world", 5));
    assert!(!t.contains(new));
    // Caret at start of non-empty text inserts an empty sibling above.
    let (focus, _) = t.split(a, 0);
    assert_eq!(focus, a);
    assert_eq!(texts(&t, ROOT)[0], "");
}

#[test]
fn merge_preserves_notes_and_protects_images_and_mirrors() {
    let (mut t, [a, b, c, _]) = sample();
    t.set_note(a, "first note");
    t.set_note(b, "second note");
    assert_eq!(t.merge_into(b, a), Some(1));
    assert_eq!(t.node(a).text, "ab");
    assert_eq!(t.node(a).note, "first note\nsecond note");
    assert!(!t.contains(b));

    // Image rows must not be merged (which would drop the image).
    let mut img = t.node(c).clone();
    img.kind = BlockKind::Image;
    img.image = "0000000000000000000000000000000000000000000000000000000000000000.png".into();
    t.put(img);
    assert_eq!(t.merge_into(c, a), None);
    assert!(t.contains(c));
    assert!(!t.node(c).image.is_empty());

    // Mirror rows must not be merged away either.
    let mirror = t.create_mirror(a, ROOT, t.children(ROOT).len()).unwrap();
    assert_eq!(t.merge_into(mirror, a), None);
    assert!(t.contains(mirror));
}

#[test]
fn split_at_end_of_parent_makes_first_child() {
    let (mut t, [_, b, _, d]) = sample();
    let (child, _) = t.split(b, 1);
    assert_eq!(t.parent(child), b);
    assert_eq!(t.children(b), &[child, d]);
}

#[test]
fn split_keeps_list_kind() {
    let (mut t, [a, b, _, d]) = sample();
    t.set_kind(a, BlockKind::Todo);
    t.set_completed(a, true);
    let (new, _) = t.split(a, 1);
    assert_eq!(t.node(new).kind, BlockKind::Todo);
    assert!(!t.node(new).completed);
    // Enter on the empty to-do turns it back into a bullet.
    let (same, _) = t.split(new, 0);
    assert_eq!((same, t.node(new).kind), (new, BlockKind::Bullet));
    // A new first child follows the existing first child.
    t.set_kind(d, BlockKind::Numbered);
    let (child, _) = t.split(b, 1);
    assert_eq!(t.node(child).kind, BlockKind::Numbered);
    // Headings do not carry over.
    t.set_kind(b, BlockKind::H1);
    t.set_collapsed(b, true);
    let (next, _) = t.split(b, 1);
    assert_eq!(t.node(next).kind, BlockKind::Bullet);
}

#[test]
fn merge_moves_children() {
    let (mut t, [a, b, _, d]) = sample();
    t.merge_into(b, a).unwrap();
    assert_eq!(t.node(a).text, "ab");
    assert_eq!(t.children(a), &[d]);
}

#[test]
fn move_up_down_and_across_parents() {
    let (mut t, [a, b, c, d]) = sample();
    assert!(t.move_down(a, ROOT));
    assert_eq!(texts(&t, ROOT), ["b", "a", "c"]);
    assert!(t.move_up(a, ROOT));
    assert_eq!(texts(&t, ROOT), ["a", "b", "c"]);
    // d is b's only child: moving it up puts it at the end of a's children.
    assert!(t.move_up(d, ROOT));
    assert_eq!(t.parent(d), a);
    assert!(t.move_down(d, ROOT));
    assert_eq!(t.parent(d), b);
    assert!(!t.move_to(b, d, 0), "cannot move into own subtree");
    let _ = c;
}

#[test]
fn many_inserts_between_trigger_renumber() {
    let mut t = Tree::new();
    let first = t.create_at(ROOT, 0, "first");
    t.create_after(first, "last");
    for i in 0..200 {
        t.create_at(ROOT, 1, &format!("{i}"));
    }
    let ts = texts(&t, ROOT);
    assert_eq!(ts.first().unwrap(), "first");
    assert_eq!(ts.last().unwrap(), "last");
    assert_eq!(ts[1], "199");
    assert_eq!(ts[200], "0");
}

#[test]
fn rollback_to_mark_keeps_earlier_changes() {
    let (mut t, [a, b, c, _d]) = sample();
    t.set_text(a, "A");
    let mark = t.journal_mark();
    t.set_text(c, "c1");
    t.set_text(c, "c2");
    t.move_to(c, b, 0);
    let e = t.create_at(ROOT, 0, "e");
    t.delete(b);
    t.rollback_to(mark);
    assert_eq!(outline(&t, ROOT), "A\nb\n  d\nc");
    assert!(!t.contains(e));
    assert_eq!(t.journal_mark(), mark);
}

#[test]
fn undo_redo_roundtrip() {
    let (mut t, [a, b, c, d]) = sample();
    let mut h = History::new();
    let before = outline(&t, ROOT);
    t.indent(c);
    t.set_text(c, "C!");
    h.commit(&mut t, HistoryKind::Structure, None, None);
    t.delete(b);
    h.commit(&mut t, HistoryKind::Structure, None, None);
    assert_eq!(outline(&t, ROOT), "a");
    h.undo(&mut t);
    assert_eq!(outline(&t, ROOT), "a\nb\n  d\n  C!");
    h.undo(&mut t);
    assert_eq!(outline(&t, ROOT), before);
    h.redo(&mut t);
    h.redo(&mut t);
    assert_eq!(outline(&t, ROOT), "a");
    let _ = (a, d);
}

#[test]
fn typing_coalesces() {
    let (mut t, [a, ..]) = sample();
    let mut h = History::new();
    for s in ["a1", "a12", "a123"] {
        t.set_text(a, s);
        h.commit(&mut t, HistoryKind::Typing(a), None, None);
    }
    h.undo(&mut t);
    assert_eq!(t.node(a).text, "a");
    assert!(!h.can_undo());
}

#[test]
fn collapsed_hides_children() {
    let (mut t, [_, b, ..]) = sample();
    t.set_collapsed(b, true);
    assert_eq!(outline(&t, ROOT), "a\nb\nc");
    assert_eq!(outline(&t, b), "d", "zooming shows children regardless");
}

#[test]
fn search_keeps_ancestors_and_ignores_collapse() {
    let (mut t, [_, b, _, d]) = sample();
    t.set_text(d, "deep #Work item");
    t.set_collapsed(b, true);
    let opts = ViewOptions { query: Some(Query::parse("#work")), ..Default::default() };
    let rows = flatten(&t, ROOT, &opts);
    let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![b, d]);
    let q = Query::parse("-deep");
    assert!(!q.matches(t.node(d)));
    assert!(Query::parse("is:open").matches(t.node(d)));
    assert!(!Query::parse("#wor").matches(t.node(d)), "tags match whole tokens");
}

#[test]
fn hide_completed() {
    let (mut t, [a, ..]) = sample();
    t.set_completed(a, true);
    let opts = ViewOptions { hide_completed: true, ..Default::default() };
    assert_eq!(flatten(&t, ROOT, &opts).len(), 3);
}

#[test]
fn merge_leaving_keeps_position() {
    let r = |n| Row { id: NodeId(n), depth: 0 };
    let old = [r(1), r(2), r(3), r(4)];
    let new = [r(1), r(4), r(5)];
    let merged: Vec<(u128, bool)> = merge_leaving(&old, &new).iter().map(|d| (d.row.id.0, d.leaving)).collect();
    assert_eq!(merged, vec![(1, false), (2, true), (3, true), (4, false), (5, false)]);
}

#[test]
fn highlights() {
    assert_eq!(find_highlights("Hello hello", &["hello".into()]), vec![0..5, 6..11]);
}

#[test]
fn indented_text_roundtrip() {
    let (t, [a, b, c, _]) = sample();
    let text = t.to_indented_text(&[a, b, c]);
    assert_eq!(text, "- a\n- b\n  - d\n- c\n");
    let mut t2 = Tree::new();
    let top = t2.insert_indented_text(ROOT, 0, &text);
    assert_eq!(top.len(), 3);
    assert_eq!(outline(&t2, ROOT), "a\nb\n  d\nc");
    let top = t2.insert_indented_text(ROOT, 0, "x\n  y\n    z\n  w\n");
    assert_eq!(top.len(), 1);
    assert_eq!(outline(&t2, top[0]), "y\n  z\nw");
}

fn kinds(s: &str) -> Vec<(String, Kind)> {
    markup::parse(s).into_iter().map(|r| (s[r.range.clone()].to_string(), r.kind)).collect()
}

#[test]
fn markup_math() {
    let runs = kinds("Euler $e^{i\\pi}+1=0$ ok");
    assert_eq!(runs[0].0, "Euler ");
    assert_eq!(runs[1].0, "$e^{i\\pi}+1=0$");
    assert!(matches!(runs[1].1, Kind::Math { display: false, .. }));
    assert!(kinds("costs $5 and $6 today").iter().all(|(_, k)| *k == Kind::Text));
    assert!(kinds("$ x $").iter().all(|(_, k)| *k == Kind::Text));
    let runs = kinds("$$\\int_0^1 x$$");
    assert!(matches!(runs[0].1, Kind::Math { display: true, .. }));
    assert_eq!(markup::plain_text("\\$5"), "$5");
}

#[test]
fn markup_styles_and_links() {
    let runs = markup::parse("a **bold** and *it* #tag @bob");
    let bold = runs.iter().find(|r| r.bold).unwrap();
    assert_eq!(bold.range, 4..8);
    assert!(runs.iter().any(|r| r.italic && r.kind == Kind::Text));
    assert!(runs.iter().any(|r| r.kind == Kind::Tag));
    assert!(runs.iter().any(|r| r.kind == Kind::Mention));
    assert_eq!(markup::plain_text("see [docs](https://x.y) now"), "see docs now");
    let k = kinds("go https://example.com/a, ok");
    assert!(k.iter().any(|(s, k)| s == "https://example.com/a" && matches!(k, Kind::Link { .. })));
    assert!(kinds("snake_case_name").iter().all(|(_, k)| *k == Kind::Text), "intra-word _ is not italic");
    assert!(kinds("中文 #标签").iter().any(|(s, k)| s == "#标签" && *k == Kind::Tag));
    assert!(kinds("a#b").iter().all(|(_, k)| *k == Kind::Text));
}

#[test]
fn store_roundtrip() {
    let dir = std::env::temp_dir().join(format!("flowie-test-{}", std::process::id()));
    let path = dir.join("t.sqlite");
    let _ = std::fs::remove_file(&path);
    let (mut t, [a, b, ..]) = sample();
    t.set_note(a, "note");
    {
        let w = store::Writer::spawn(store::open(&path).unwrap());
        let (up, del) = t.take_dirty();
        w.write(up, del);
        t.delete(b);
        let (up, del) = t.take_dirty();
        w.write(up, del);
        w.set_meta("zoom", "0");
        w.flush().unwrap();
    }
    let conn = store::open(&path).unwrap();
    let (nodes, meta) = store::load(&conn).unwrap();
    let t2 = Tree::from_nodes(nodes);
    assert_eq!(outline(&t2, ROOT), "a\nc");
    assert_eq!(t2.node(a).note, "note");
    assert_eq!(meta, vec![("zoom".to_string(), "0".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn markdown_notes_kinds_roundtrip() {
    let md = "- # Title\n  a note\n  - [ ] task\n  - [x] done task\n  - > quoted\n  - ~~finished~~\n1. first\n";
    let items = io::parse_text(md);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].kind, BlockKind::H1);
    assert_eq!(items[0].note, "a note");
    assert_eq!(items[0].children[0].kind, BlockKind::Todo);
    assert!(items[0].children[1].completed);
    assert_eq!(items[0].children[2].kind, BlockKind::Quote);
    assert!(items[0].children[3].completed && items[0].children[3].kind == BlockKind::Bullet);
    assert_eq!(items[1].kind, BlockKind::Numbered);
    assert_eq!(io::parse_text(&io::to_markdown(&items)), items);
}

#[test]
fn plain_indented_lines_without_markers() {
    let items = io::parse_text("a\n    b\n    c\nd");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].children.len(), 2);
}

#[test]
fn import_opml_exported_by_rc3() {
    let items = io::parse_any(include_str!("../tests/fixtures/rc3-home.opml"));
    let mut tree = Tree::new();
    let roots = tree.insert_items(ROOT, 0, &items);
    assert_eq!(roots.len(), 2);
    use BlockKind::{Bullet, Code, Numbered, Paragraph, Quote, Todo, H1, H2, H3};
    let rc3_kinds = [Bullet, Paragraph, H1, H2, H3, Todo, Quote, Code, Numbered];
    for &root in &roots {
        assert_eq!(tree.node(root).text, "项目 & plan");
        assert_eq!(tree.node(root).mirror, ROOT);
        assert!(!tree.node(root).collapsed);
        assert!(tree.mirrors_of(root).is_empty());
        assert!(tree.backlinks(root).is_empty());
        let children = tree.children(root);
        assert_eq!(children.iter().map(|&id| tree.node(id).kind).collect::<Vec<_>>(), rc3_kinds);
        let paragraph = tree.node(children[1]);
        assert_eq!(paragraph.text, "**段落** [项目](flowie:1) [网站](https://example.org) #迁移");
        assert_eq!(paragraph.note, "参见 [项目](flowie:1)");
        let task = tree.node(children[5]);
        assert_eq!(task.note, "第一行\n第二行");
        assert!(task.completed);
        assert_eq!(tree.node(children[7]).text, "if x < y && y > 0 {}");
    }
    let source_task = tree.children(roots[0])[5];
    let mirror_copy = tree.children(roots[1])[5];
    tree.set_completed(source_task, false);
    assert!(tree.node(mirror_copy).completed);
}

#[test]
fn opml_keeps_known_type_names_but_omits_image_files() {
    let image = Item { kind: BlockKind::Image, image: format!("{}.png", "a".repeat(64)), ..Item::new("图注") };
    let xml = io::to_opml("Images", std::slice::from_ref(&image));
    assert!(!xml.contains(&image.image));
    assert_eq!(io::parse_any(&xml), [Item { kind: BlockKind::Image, ..Item::new("图注") }]);
    assert_eq!(io::parse_any(&xml.replace("flowie:kind=\"image\"", "flowie:kind=\"future-type\"")), [Item::new("图注")]);
}

#[test]
fn opml_mirrors_and_links_round_trip_and_follow_a_reimported_copy() {
    let mut original = Tree::new();
    let source = original.create_at(ROOT, 0, "项目 & <计划> \"一\"");
    let mirror = original.create_mirror(source, ROOT, 0).unwrap();
    let reference = original.create_at(ROOT, 1, &format!("[项目](flowie:{source})"));
    original.set_note(reference, &format!("[镜像](flowie:{mirror})"));
    for (index, kind) in BlockKind::ALL.into_iter().enumerate() {
        let child = original.create_at(source, index, kind.name());
        original.set_kind(child, kind);
    }
    let xml = io::to_opml("Home", &original.to_opml_items(original.children(ROOT)));
    let document = roxmltree::Document::parse(&xml).unwrap();
    let outlines: Vec<_> = document.descendants().filter(|element| element.has_tag_name("outline")).collect();
    assert!(outlines.iter().all(|element| element.attribute("text").is_some()));
    assert_eq!(outlines[0].attribute("text"), Some(original.node(source).text.as_str()));
    assert_eq!(outlines[0].children().filter(|element| element.is_element()).count(), 0);

    let renamed_prefix = xml.replace("xmlns:flowie=", "xmlns:f=").replace(" flowie:", " f:");
    let items = io::parse_any(&renamed_prefix);
    let mut imported = Tree::new();
    let first = imported.insert_items(ROOT, 0, &items);
    assert_eq!(first, [mirror, reference, source]);
    let copy = imported.insert_items(ROOT, first.len(), &items);
    for roots in [&first, &copy] {
        let (mirror, reference, source) = (roots[0], roots[1], roots[2]);
        assert_eq!(imported.source(mirror), source);
        assert_eq!(imported.mirrors_of(source), [mirror]);
        assert_eq!(imported.backlinks(source), [reference]);
        assert_eq!(imported.backlinks(mirror), [reference]);
        assert_eq!(imported.node(reference).text, format!("[项目](flowie:{source})"));
        assert_eq!(imported.node(reference).note, format!("[镜像](flowie:{mirror})"));
        let kinds: Vec<_> = imported.children(source).iter().map(|&child| imported.node(child).kind).collect();
        assert_eq!(kinds, BlockKind::ALL);
    }
    imported.set_text(copy[2], "修改副本");
    assert_eq!(imported.node(imported.source(copy[0])).text, "修改副本");
    assert_eq!(imported.node(imported.source(mirror)).text, original.node(source).text);
}

#[test]
fn opml_duplicate_ids_do_not_remap_ambiguous_links_or_mirrors() {
    let duplicate = NodeId::random();
    let reference = Item { text: format!("[ambiguous](flowie:{duplicate})"), ..Item::new("ref") };
    let mirror = Item { id: Some(NodeId::random()), mirror_of: Some(duplicate), ..Item::new("snapshot") };
    let items = [
        Item { id: Some(duplicate), ..Item::new("first") },
        Item { children: vec![reference], ..Item::new("parent") },
        Item { id: Some(duplicate), ..Item::new("second") },
        mirror,
    ];
    let mut tree = Tree::new();
    let roots = tree.insert_items(ROOT, 0, &items);
    assert_eq!(tree.node(roots[1]).text, "parent");
    let link = tree.children(roots[1])[0];
    assert_eq!(tree.node(link).text, "ambiguous");
    assert_eq!(tree.node(roots[3]).mirror, ROOT);
    assert_eq!(tree.node(roots[3]).text, "snapshot");
}

#[test]
fn opml_mirror_alone_uses_existing_source_or_snapshot() {
    let mut original = Tree::new();
    let source = original.create_at(ROOT, 0, "单独导出");
    let mirror = original.create_mirror(source, ROOT, 1).unwrap();
    let xml = io::to_opml("Selection", &original.to_opml_items(&[mirror]));
    let items = io::parse_any(&xml);
    let mut empty = Tree::new();
    let fallback = empty.insert_items(ROOT, 0, &items)[0];
    assert_eq!(empty.node(fallback).text, "单独导出");
    assert_eq!(empty.node(fallback).mirror, ROOT);
    let restored = original.insert_items(ROOT, 0, &items)[0];
    assert_eq!(original.source(restored), source);
    let foreign = io::parse_any(&xml.replace("https://github.com/wesleyel/flowie/ns/opml/1", "urn:other"));
    assert_eq!(foreign, [Item::new("单独导出")]);
}

#[test]
fn opml_roundtrip_and_workflowy_html() {
    let (mut t, [a, b, ..]) = sample();
    t.set_note(a, "line1\nline2 & <x>");
    t.set_completed(b, true);
    let items = t.to_items(t.children(ROOT));
    let xml = io::to_opml("Test", &items);
    assert_eq!(io::parse_opml(&xml), items);
    let wf = r#"<opml><body><outline text="&lt;b&gt;Bold&lt;/b&gt; and &lt;a href=&quot;https://x.y&quot;&gt;link&lt;/a&gt;" _complete="true"><outline text="kid"/></outline></body></opml>"#;
    let items = io::parse_any(wf);
    assert_eq!(items[0].text, "**Bold** and [link](https://x.y)");
    assert!(items[0].completed);
    assert_eq!(items[0].children[0].text, "kid");
}

#[test]
fn markup_dates_colors_highlight() {
    let s = "due 2026-11-17 {red}hot{/} ==mark==";
    let runs = markup::parse(s);
    assert!(runs.iter().any(|r| r.kind == Kind::Date && &s[r.range.clone()] == "2026-11-17"));
    assert!(runs.iter().any(|r| r.color == Some(0) && &s[r.range.clone()] == "hot"));
    assert!(runs.iter().any(|r| r.highlight && &s[r.range.clone()] == "mark"));
    assert_eq!(markup::plain_text(s), "due 2026-11-17 hot mark");
    assert!(markup::dates("v2026-11-17 2026-13-01").is_empty());
}

#[test]
fn search_operators() {
    let (mut t, [a, b, c, d]) = sample();
    t.set_text(a, "ship 2026-11-20");
    t.set_text(b, "plan 2026-12-05");
    t.set_kind(c, BlockKind::Todo);
    t.set_note(d, "has a note");
    assert!(Query::parse("date:2026-11").matches(t.node(a)));
    assert!(!Query::parse("date:2026-11").matches(t.node(b)));
    assert!(Query::parse("before:2026-12-01").matches(t.node(a)));
    assert!(Query::parse("after:2026-12-01").matches(t.node(b)));
    assert!(Query::parse("is:todo").matches(t.node(c)));
    assert!(Query::parse("has:note").matches(t.node(d)));
    assert!(Query::parse("changed:1h").matches(t.node(a)));
}

#[test]
fn expand_to_level_and_restore_and_backlinks() {
    let (mut t, [a, b, c, d]) = sample();
    let e = t.create_at(d, 0, "e");
    t.expand_to_level(ROOT, 1);
    assert!(t.node(b).collapsed && !t.node(d).collapsed || t.node(b).collapsed);
    t.expand_to_level(ROOT, usize::MAX);
    assert!(!t.node(b).collapsed && !t.node(d).collapsed);
    // trash round trip
    let snapshot = t.subtree_nodes(b);
    t.delete(b);
    assert!(!t.contains(e));
    t.restore(&snapshot);
    assert_eq!(outline(&t, ROOT), "a\nb\n  d\n    e\nc");
    // links
    t.set_text(c, &format!("see [b](flowie:{b})"));
    assert_eq!(t.backlinks(b), vec![c]);
    // mirrors
    let m = t.create_mirror(b, ROOT, 0).unwrap();
    assert_eq!(t.source(m), b);
    assert!(t.create_mirror(b, d, 0).is_none(), "no mirror inside its own source");
    let _ = a;
}

#[test]
fn node_ids() {
    let mut t = Tree::new();
    let a = t.create_at(ROOT, 0, "a");
    assert!(!a.is_virtual() && a != ROOT);
    assert_eq!((a.0 >> 76) & 0xF, 4, "version 4 UUID");
    assert_eq!(a.to_string().len(), 32);
    assert_eq!(a.to_string().parse::<NodeId>(), Ok(a));
    let hyphenated = uuid::Uuid::from_u128(a.0).hyphenated().to_string();
    assert_eq!(hyphenated.parse::<NodeId>(), Ok(a));
    assert!("123".parse::<NodeId>().is_err());
    assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{a}\""));
    assert_eq!(serde_json::from_str::<NodeId>(&format!("\"{a}\"")).unwrap(), a);
    let v = virtual_id(a, ROOT);
    assert!(v.is_virtual() && v != a);
    assert_eq!(virtual_id(a, ROOT), v, "stable");
}

#[test]
fn mirrors_show_and_edit_source() {
    let (mut t, [a, b, _c, d]) = sample();
    let m = t.create_mirror(b, a, 0).unwrap();
    let (rows, map) = flatten_mapped(&t, ROOT, &ViewOptions::default());
    t.add_virtuals(map);
    // a, m (mirror), d (virtual under m), b, d, c
    assert_eq!(rows.len(), 6);
    let virt_d = rows[2].id;
    assert_ne!(virt_d, d);
    assert_eq!(t.resolve(virt_d), d);
    assert_eq!(t.node(virt_d).id, d);
    // Editing through the virtual row edits the real node.
    t.set_text(virt_d, "changed");
    assert_eq!(t.node(d).text, "changed");
    // Children added under the mirror go to the source.
    let n = t.create_at(m, 0, "new");
    assert_eq!(t.parent(n), b);
    // A mirror inside its own source does not loop.
    let m2 = t.create_mirror(a, ROOT, 0).unwrap();
    t.move_to(m2, d, 0);
    let rows = flatten(&t, ROOT, &ViewOptions::default());
    assert!(rows.len() < 100);
}

#[test]
fn link_and_mirror_index_follow_every_mutation() {
    let (mut t, [a, b, c, _d]) = sample();
    // text and note both count, once per source node; code spans don't
    t.set_text(a, &format!("[x](flowie:{b}) and [y](flowie:{b}) `[z](flowie:{c})`"));
    t.set_note(c, &format!("[n](flowie:{b})"));
    assert_eq!(t.backlinks(b), vec![a, c]);
    assert_eq!(t.link_count(c), 0);
    // editing the link away updates the index
    t.take_journal();
    t.set_text(a, "plain");
    assert_eq!(t.backlinks(b), vec![c]);
    // undo restores it
    let changes = t.take_journal();
    let inverse = t.revert(&changes);
    assert_eq!(t.backlinks(b), vec![a, c]);
    t.revert(&inverse);
    assert_eq!(t.backlinks(b), vec![c]);
    // deleting the source drops it
    t.delete(c);
    assert_eq!(t.link_count(b), 0);
    // mirrors
    let m = t.create_mirror(b, ROOT, 0).unwrap();
    assert_eq!(t.mirrors_of(b), vec![m]);
    t.delete(m);
    assert!(t.mirrors_of(b).is_empty());
    // a tree loaded from nodes is indexed too
    t.set_text(a, &format!("[x](flowie:{b})"));
    let loaded = Tree::from_nodes(t.nodes().cloned().collect::<Vec<_>>());
    assert_eq!(loaded.backlinks(b), vec![a]);
}

#[test]
fn recolor_text() {
    use markup::{colors_in, next_color, recolor};
    // Whole text.
    let r = recolor("hello", 0..5, Some(0));
    assert_eq!((r.text.as_str(), r.range.clone()), ("{red}hello{/}", 5..10));
    // Changing the color replaces the span instead of nesting it.
    let r = recolor(&r.text, 0..r.text.len(), Some(4));
    assert_eq!(r.text, "{blue}hello{/}");
    assert_eq!(colors_in(&r.text, 0..r.text.len()), [Some(4)]);
    // Part of a span splits it, and the selection follows the text.
    let r = recolor("{red}abcdef{/}", 7..9, Some(4));
    assert_eq!(r.text, "{red}ab{/}{blue}cd{/}{red}ef{/}");
    assert_eq!(&r.text[r.range.clone()], "cd");
    // None removes the color; neighbors merge back.
    let r = recolor("{red}ab{/}{blue}cd{/}{red}ef{/}", 0..31, None);
    assert_eq!(r.text, "abcdef");
    // Other markup stays intact, and offsets map across the new markers.
    let r = recolor("a **b** $x$", 0..11, Some(1));
    assert_eq!(r.text, "{orange}a **b** $x${/}");
    assert_eq!(r.map(0), 8);
    assert_eq!(r.map(11), 19);
    // Cycling goes through every color, then back to none.
    assert_eq!(next_color(None), Some(0));
    assert_eq!(next_color(Some(5)), Some(6));
    assert_eq!(next_color(Some(6)), None);
}

#[test]
fn is_new_matches_the_given_ids() {
    let (t, [a, b, ..]) = sample();
    let mut q = Query::parse("is:new");
    q.set_new([b].into_iter().collect());
    assert!(!q.matches(t.node(a)));
    assert!(q.matches(t.node(b)));
}

#[test]
fn markdown_images_round_trip() {
    let name = format!("{}.png", "ab".repeat(32));
    let items = io::parse_text("- ![A *cat*](pics/cat.png)\n  - under it");
    assert_eq!(items[0].kind, BlockKind::Image);
    assert_eq!((items[0].text.as_str(), items[0].image.as_str()), ("A *cat*", "pics/cat.png"));
    assert_eq!(items[0].children.len(), 1);
    let mut stored = items[0].clone();
    stored.image = name.clone();
    stored.children.clear();
    assert_eq!(io::to_markdown(&[stored.clone()]), format!("- ![A *cat*](images/{name})\n"));
    let mut t = Tree::new();
    let ids = t.insert_items(ROOT, 0, &[stored]);
    assert_eq!(t.node(ids[0]).image, name);
    let mut q = Query::parse("has:image");
    assert!(q.matches(t.node(ids[0])));
    q = Query::parse("-has:image");
    assert!(!q.matches(t.node(ids[0])));
}
