//! The journal: a "Journal" bullet under Home with one page per day, newest
//! first, each titled with its date (YYYY-MM-DD).
//!
//! Its ids are deterministic (UUIDv5 in [`FLOWIE_NS`]), so every device creates
//! the same journal bullet, the same page for a day and the same blank first
//! line on it: two devices that both open "today" offline make one page when
//! their changes meet, not two. Everything here finds the journal by id, never
//! by text, so renaming the journal or a page keeps it.

use crate::io::Item;
use crate::tree::{BlockKind, Node, NodeId, Tree, ROOT};

/// The namespace of Flowie's deterministic ids. Fixed forever: changing it
/// changes every journal id.
pub const FLOWIE_NS: uuid::Uuid = uuid::Uuid::from_u128(0x8f28a995_e909_4d20_8073_1446578cf163);

/// The text the journal bullet is created with.
pub const JOURNAL_TEXT: &str = "Journal";

/// Whether `s` is exactly a date as journal pages are named (YYYY-MM-DD).
/// Ids are derived from this text, so callers check it before using a date
/// that came from outside.
pub fn is_date(s: &str) -> bool {
    s.len() == 10 && crate::markup::date_len(s) == Some(10)
}

fn v5(name: &str) -> NodeId {
    NodeId(uuid::Uuid::new_v5(&FLOWIE_NS, name.as_bytes()).as_u128())
}

/// The journal bullet's id.
pub fn journal_root_id() -> NodeId {
    v5("journal")
}

/// The id of the journal page for `date` (YYYY-MM-DD).
pub fn day_id(date: &str) -> NodeId {
    v5(&format!("journal/{date}"))
}

/// The id of the blank line a journal page for `date` is opened with.
pub fn day_blank_id(date: &str) -> NodeId {
    v5(&format!("journal/{date}/0"))
}

/// The journal bullet, if there is one.
pub fn find_journal(tree: &Tree) -> Option<NodeId> {
    let id = journal_root_id();
    tree.contains(id).then_some(id)
}

/// The journal page for `date` (YYYY-MM-DD), if there is one.
pub fn find_day(tree: &Tree, date: &str) -> Option<NodeId> {
    let id = day_id(date);
    tree.contains(id).then_some(id)
}

/// The journal bullet, created at the end of Home if needed. One in the Trash comes back
/// from there, with the pages still in it, instead of being made again.
pub fn journal_root(tree: &mut Tree) -> NodeId {
    let id = journal_root_id();
    if !revive(tree, id, ROOT, |t| t.children(ROOT).len()) {
        let index = tree.children(ROOT).len();
        create_with_id(tree, id, ROOT, index, JOURNAL_TEXT);
    }
    id
}

/// The journal page for `date` (YYYY-MM-DD). A missing page is created among
/// the journal's pages, newest first, with a blank first line if `with_blank`;
/// a page in the Trash comes back from there as it was instead.
/// Returns `day_id(date)`. Recorded on `tree` like any edit, not committed.
pub fn ensure_day(tree: &mut Tree, date: &str, with_blank: bool) -> NodeId {
    if let Some(id) = find_day(tree, date) {
        return id;
    }
    let journal = journal_root(tree);
    let day = day_id(date);
    let index = |tree: &Tree| {
        let kids = tree.children(journal);
        kids.iter().position(|id| tree.node(*id).text.as_str() < date).unwrap_or(kids.len())
    };
    if revive(tree, day, journal, index) {
        return day;
    }
    create_with_id(tree, day, journal, index(tree), date);
    // (The blank line still exists if it was moved out of a page later deleted.)
    if with_blank && !tree.contains(day_blank_id(date)) {
        create_with_id(tree, day_blank_id(date), day, 0, "");
    }
    day
}

/// Bring back the record `id` if it exists but is out of the outline: if the bullet it sits
/// under is out of reach too (in the Trash, or under a bullet that is), move it to
/// `index(tree)` under `home` first; then take it out of the Trash. False if there is no
/// record to bring back.
fn revive(tree: &mut Tree, id: NodeId, home: NodeId, index: impl Fn(&Tree) -> usize) -> bool {
    if tree.contains(id) {
        return true;
    }
    if !tree.has_record(id) {
        return false;
    }
    if !tree.contains(tree.parent(id)) {
        let at = index(tree);
        tree.move_to(id, home, at);
    }
    tree.untrash(id);
    true
}

/// Append `items` to the end of the journal page for `date` (YYYY-MM-DD),
/// creating the page if needed, and expand the page. A lone blank line on the
/// page (as a fresh page has) is replaced, unless it is `keep` (a bullet being
/// edited). Returns the new top-level ids. Recorded on `tree`, not committed.
pub fn journal_append(tree: &mut Tree, date: &str, items: &[Item], keep: Option<NodeId>) -> Vec<NodeId> {
    let day = ensure_day(tree, date, false);
    if items.is_empty() {
        return Vec::new();
    }
    let kids = tree.children(day).to_vec();
    let placeholder = (kids.len() == 1 && Some(kids[0]) != keep && is_blank_placeholder(tree, kids[0])).then(|| kids[0]);
    let top = tree.insert_items(day, kids.len(), items);
    if let Some(ph) = placeholder {
        tree.delete(ph);
    }
    if tree.node(day).collapsed {
        tree.set_collapsed(day, false);
    }
    top
}

/// A plain bullet with nothing in it: no text, note, image or children, and
/// not a mirror.
pub fn is_blank_placeholder(tree: &Tree, id: NodeId) -> bool {
    let node = tree.node(id);
    node.kind == BlockKind::Bullet
        && node.mirror == ROOT
        && node.text.is_empty()
        && node.note.is_empty()
        && node.image.is_empty()
        && !tree.has_children(id)
}

fn create_with_id(tree: &mut Tree, id: NodeId, parent: NodeId, index: usize, text: &str) {
    let rank = tree.rank_at(parent, index);
    let mut node = Node::new(id, parent, rank);
    node.text = text.to_string();
    tree.put(node);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::parse_text;

    fn texts(tree: &Tree, parent: NodeId) -> Vec<String> {
        tree.children(parent).iter().map(|id| tree.node(*id).text.clone()).collect()
    }

    fn ids(tree: &Tree) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = tree.nodes().map(|n| n.id).collect();
        ids.sort();
        ids
    }

    #[test]
    fn ids_are_stable() {
        assert_eq!(day_id("2026-11-25"), day_id("2026-11-25"));
        assert_ne!(day_id("2026-11-25"), day_id("2026-11-26"));
        assert_ne!(day_id("2026-11-25"), day_blank_id("2026-11-25"));
        assert_ne!(journal_root_id(), day_id("2026-11-25"));
        // Pinned: these ids are shared by every device and must never change.
        assert_eq!(journal_root_id(), NodeId(uuid::Uuid::new_v5(&FLOWIE_NS, b"journal").as_u128()));
        assert_eq!(day_id("2026-11-25").to_string(), uuid::Uuid::new_v5(&FLOWIE_NS, b"journal/2026-11-25").simple().to_string());
        assert_eq!(day_blank_id("2026-11-25").to_string(), uuid::Uuid::new_v5(&FLOWIE_NS, b"journal/2026-11-25/0").simple().to_string());
        assert_eq!(FLOWIE_NS.to_string(), "8f28a995-e909-4d20-8073-1446578cf163");
        assert!(!day_id("2026-11-25").is_virtual());
        assert!(is_date("2026-11-25"));
        assert!(!is_date("2026-11-25 notes") && !is_date("2026-13-01") && !is_date("26-11-25"));
    }

    #[test]
    fn ensure_day_is_idempotent() {
        let mut t = Tree::new();
        let d = ensure_day(&mut t, "2026-11-25", true);
        let before = ids(&t);
        assert_eq!(ensure_day(&mut t, "2026-11-25", true), d);
        assert_eq!(ensure_day(&mut t, "2026-11-25", false), d);
        assert_eq!(ids(&t), before);
        assert_eq!(t.children(ROOT), &[journal_root_id()]);
        assert_eq!(t.children(journal_root_id()), &[d]);
        assert_eq!(t.children(d), &[day_blank_id("2026-11-25")]);
        assert_eq!(find_day(&t, "2026-11-25"), Some(d));
        assert_eq!(find_day(&t, "2026-11-24"), None);
    }

    #[test]
    fn found_by_id_not_text() {
        let mut t = Tree::new();
        t.create_at(ROOT, 0, "Journal");
        let d = ensure_day(&mut t, "2026-11-25", false);
        assert_eq!(t.children(ROOT).len(), 2, "a bullet that only reads \"Journal\" isn't the journal");
        t.set_text(journal_root_id(), "Diary");
        t.set_text(d, "2026-11-25 Thanksgiving prep");
        assert_eq!(ensure_day(&mut t, "2026-11-25", true), d);
        assert_eq!(journal_root(&mut t), journal_root_id());
        assert_eq!(t.children(journal_root_id()).len(), 1);
        assert!(t.children(d).is_empty(), "an existing page gets no blank line");
    }

    #[test]
    fn days_sort_newest_first() {
        let mut t = Tree::new();
        for date in ["2026-11-24", "2026-11-26", "2026-11-25", "2025-01-01"] {
            ensure_day(&mut t, date, false);
        }
        assert_eq!(texts(&t, journal_root_id()), ["2026-11-26", "2026-11-25", "2026-11-24", "2025-01-01"]);
    }

    #[test]
    fn independent_trees_agree() {
        let mut a = Tree::new();
        let mut b = Tree::new();
        a.create_at(ROOT, 0, "something else");
        ensure_day(&mut a, "2026-11-25", true);
        ensure_day(&mut a, "2026-11-24", true);
        ensure_day(&mut b, "2026-11-24", true);
        ensure_day(&mut b, "2026-11-25", true);
        let journal = |t: &Tree| {
            let mut ids = vec![journal_root_id()];
            ids.extend(t.descendants(journal_root_id()));
            ids.sort();
            ids
        };
        assert_eq!(journal(&a), journal(&b));
        assert_eq!(journal(&a).len(), 5);
    }

    #[test]
    fn append_to_a_day() {
        let mut t = Tree::new();
        let top = journal_append(&mut t, "2026-11-25", &parse_text("- one\n  - child\n- two"), None);
        let d = day_id("2026-11-25");
        assert_eq!(top.len(), 2);
        assert_eq!(texts(&t, d), ["one", "two"]);
        assert_eq!(texts(&t, top[0]), ["child"]);
        let more = journal_append(&mut t, "2026-11-25", &parse_text("- three"), None);
        assert_eq!(texts(&t, d), ["one", "two", "three"]);
        assert_eq!(t.children(d)[2], more[0]);
        assert_eq!(t.children(journal_root_id()), &[d], "still one page");
    }

    #[test]
    fn append_replaces_a_fresh_blank_line() {
        let mut t = Tree::new();
        let d = ensure_day(&mut t, "2026-11-25", true);
        t.set_collapsed(d, true);
        journal_append(&mut t, "2026-11-25", &parse_text("- note"), None);
        assert_eq!(texts(&t, d), ["note"]);
        assert!(!t.node(d).collapsed);

        // Not while it's being edited.
        let d = ensure_day(&mut t, "2026-11-24", true);
        let blank = day_blank_id("2026-11-24");
        journal_append(&mut t, "2026-11-24", &parse_text("- note"), Some(blank));
        assert_eq!(texts(&t, d), ["", "note"]);

        // Nor once it has text.
        let d = ensure_day(&mut t, "2026-11-23", true);
        t.set_text(day_blank_id("2026-11-23"), "kept");
        journal_append(&mut t, "2026-11-23", &parse_text("- note"), None);
        assert_eq!(texts(&t, d), ["kept", "note"]);
    }

    #[test]
    fn a_page_in_the_trash_comes_back_as_it_was() {
        let mut t = Tree::new();
        ensure_day(&mut t, "2026-11-26", false);
        let d = ensure_day(&mut t, "2026-11-25", true);
        let blank = day_blank_id("2026-11-25");
        t.set_text(blank, "written that day");
        t.trash(d);
        assert_eq!(find_day(&t, "2026-11-25"), None);
        let before = ids_with_records(&t);

        assert_eq!(ensure_day(&mut t, "2026-11-25", true), d);
        assert!(t.contains(d) && t.node(d).deleted_at.is_none());
        assert_eq!(texts(&t, d), ["written that day"], "its content is back, with no new blank line");
        assert_eq!(texts(&t, journal_root_id()), ["2026-11-26", "2026-11-25"], "in its old place");
        assert_eq!(ids_with_records(&t), before, "nothing is created twice");
        assert!(t.trashed().is_empty());
    }

    #[test]
    fn a_journal_in_the_trash_comes_back_with_its_pages() {
        let mut t = Tree::new();
        let d = ensure_day(&mut t, "2026-11-25", true);
        t.trash(journal_root_id());
        assert_eq!(find_journal(&t), None);
        let before = ids_with_records(&t);
        assert_eq!(ensure_day(&mut t, "2026-11-25", true), d);
        assert!(t.contains(journal_root_id()) && t.contains(d) && t.contains(day_blank_id("2026-11-25")));
        assert_eq!(ids_with_records(&t), before);
        assert_eq!(t.children(ROOT), &[journal_root_id()]);
    }

    #[test]
    fn a_page_under_a_deleted_bullet_moves_back_to_the_journal() {
        let mut t = Tree::new();
        let other = t.create_at(ROOT, 0, "other");
        let d = ensure_day(&mut t, "2026-11-25", false);
        t.move_to(d, other, 0);
        t.trash(other);
        assert_eq!(ensure_day(&mut t, "2026-11-25", false), d);
        assert_eq!(t.parent(d), journal_root_id());
        assert!(t.contains(d));
        assert!(!t.contains(other), "the deleted bullet stays in the Trash");
        assert_eq!(t.trashed(), [other]);

        // Both the page and the bullet above it in the Trash: the page comes back alone.
        t.move_to(d, other, 0);
        t.trash(d);
        assert_eq!(ensure_day(&mut t, "2026-11-25", false), d);
        assert_eq!(t.parent(d), journal_root_id());
        assert!(t.contains(d) && t.node(d).deleted_at.is_none());
    }

    fn ids_with_records(tree: &Tree) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = tree.records().map(|n| n.id).collect();
        ids.sort();
        ids
    }
}
