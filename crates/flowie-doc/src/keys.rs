//! What Return and Backspace do to the outline, shared by the desktop and the native apps
//! (wiki: Release-Plan §3.5). These only change the tree and say where the caret goes; the
//! caller commits the change as one undo step (`Workspace::commit_with_imports`), or uses the
//! `Workspace` methods below, which do both.

use std::ops::Range;

use flowie_core::{Caret, NodeId, Tree, ROOT};

use crate::Workspace;

/// Return in row `id`'s text with the bytes `sel` selected (a caret when empty), on the page
/// at `zoom`. Returns where the caret goes.
///
/// - In the page's title (`id == zoom`): a new first child.
/// - On a mirror: a plain bullet after it.
/// - Otherwise the selection goes, and the row splits at the caret (`Tree::split`): the text
///   after it moves to a new bullet below, where the caret goes. At the start of a row an
///   empty bullet goes above, at the end of an expanded parent a first child, and an empty
///   to-do, numbered item or paragraph turns back into a bullet.
pub fn enter(tree: &mut Tree, id: NodeId, sel: Range<usize>, zoom: NodeId) -> Caret {
    if id == zoom {
        let first = tree.children(id).first().map(|&c| tree.node(c).kind);
        let child = tree.create_at(id, 0, "");
        if let Some(kind) = first {
            tree.set_kind(child, kind.list_kind());
        }
        return Caret { node: child, offset: 0 };
    }
    if tree.node(id).mirror != ROOT {
        let n = tree.create_after(id, "");
        return Caret { node: n, offset: 0 };
    }
    let text = tree.node(tree.source(id)).text.clone();
    let start = floor_boundary(&text, sel.start);
    let end = floor_boundary(&text, sel.end).max(start);
    if start != end {
        tree.set_text(id, &format!("{}{}", &text[..start], &text[end..]));
    }
    let (node, offset) = tree.split(id, start);
    Caret { node, offset }
}

/// Backspace with the caret at the very start of row `id`'s text (nothing selected), on the
/// page at `zoom`; `prev` is the row shown above it. Returns where the caret goes, or `None`
/// when nothing changes.
///
/// - An empty plain row (no note, image, mirror or children) goes away; the caret goes to the
///   end of the row above.
/// - Otherwise the row's text joins the end of the row above (`Tree::merge_into`: its note
///   follows that row's note, its children follow that row's children). A row with children
///   doesn't merge into its parent or into a row that has children of its own, and image and
///   mirror rows don't merge.
pub fn backspace_at_start(tree: &mut Tree, id: NodeId, prev: Option<NodeId>, zoom: NodeId) -> Option<Caret> {
    if id == zoom {
        return None;
    }
    let prev = prev?;
    let real = tree.resolve(id);
    let node = tree.node(real);
    if node.mirror == ROOT && node.text.is_empty() && node.note.is_empty() && node.image.is_empty() && !tree.has_children(real) {
        tree.delete(real);
        let offset = tree.node(tree.source(prev)).text.len();
        return Some(Caret { node: prev, offset });
    }
    if tree.has_children(id) && (prev == tree.parent(id) || tree.has_children(prev)) {
        return None;
    }
    let offset = tree.merge_into(id, prev)?;
    Some(Caret { node: prev, offset })
}

/// The largest char boundary of `text` at or before `at`.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

impl Workspace {
    /// [`enter`], as one undo step.
    pub fn enter(&mut self, id: NodeId, sel: Range<usize>, zoom: NodeId) -> Caret {
        let before = Caret { node: id, offset: sel.start };
        let after = enter(&mut self.tree, id, sel, zoom);
        self.commit_with_imports(Some(before), Some(after), None);
        after
    }

    /// [`backspace_at_start`], as one undo step when something changed.
    pub fn backspace_at_start(&mut self, id: NodeId, prev: Option<NodeId>, zoom: NodeId) -> Option<Caret> {
        let after = backspace_at_start(&mut self.tree, id, prev, zoom)?;
        self.commit_with_imports(Some(Caret { node: id, offset: 0 }), Some(after), None);
        Some(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowie_core::{flatten, BlockKind};

    /// Text of the page at `root`, one line per visible row, indented.
    fn page(tree: &Tree, root: NodeId) -> String {
        flatten(tree, root, &Default::default())
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth as usize), tree.node(tree.source(row.id)).text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The row shown above `id` on the page at `root`.
    fn above(tree: &Tree, root: NodeId, id: NodeId) -> Option<NodeId> {
        let rows = flatten(tree, root, &Default::default());
        let ix = rows.iter().position(|r| r.id == id)?;
        ix.checked_sub(1).map(|i| rows[i].id)
    }

    fn ws() -> Workspace {
        Workspace::new(Tree::new())
    }

    #[test]
    fn return_splits_at_the_caret_and_undoes_in_one_step() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "hello world");
        ws.tree.set_note(a, "note");
        ws.commit_with_imports(None, None, None);
        let caret = ws.enter(a, 5..5, ROOT);
        assert_eq!(page(&ws.tree, ROOT), "hello\n world");
        assert_eq!(caret.offset, 0);
        assert_eq!(ws.tree.node(caret.node).text, " world");
        assert_eq!(ws.tree.node(a).note, "note", "the note stays with the first half");
        assert_eq!(ws.undo(), Some(Some(Caret { node: a, offset: 5 })));
        assert_eq!(page(&ws.tree, ROOT), "hello world");
        assert!(ws.redo().is_some());
        assert_eq!(page(&ws.tree, ROOT), "hello\n world");
    }

    #[test]
    fn return_replaces_the_selection() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "one two three");
        ws.commit_with_imports(None, None, None);
        let caret = ws.enter(a, 3..8, ROOT);
        assert_eq!(page(&ws.tree, ROOT), "one\nthree");
        assert_eq!(ws.tree.node(caret.node).text, "three");
        assert!(ws.undo().is_some());
        assert_eq!(page(&ws.tree, ROOT), "one two three", "one step brings the selection back too");
    }

    #[test]
    fn return_at_the_ends_of_a_row() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "a");
        let b = ws.tree.create_at(ROOT, 1, "b");
        let c = ws.tree.create_at(b, 0, "c");
        ws.commit_with_imports(None, None, None);
        // At the start: an empty bullet above, the caret stays.
        assert_eq!(ws.enter(a, 0..0, ROOT), Caret { node: a, offset: 0 });
        assert_eq!(page(&ws.tree, ROOT), "\na\nb\n  c");
        // At the end of a leaf: an empty bullet below.
        let below = ws.enter(a, 1..1, ROOT);
        assert_eq!(page(&ws.tree, ROOT), "\na\n\nb\n  c");
        assert_eq!(ws.tree.parent(below.node), ROOT);
        // At the end of an expanded parent: a first child.
        let child = ws.enter(b, 1..1, ROOT);
        assert_eq!(ws.tree.children(b), &[child.node, c]);
        // A collapsed one gets a sibling instead.
        ws.tree.set_collapsed(b, true);
        let next = ws.enter(b, 1..1, ROOT);
        assert_eq!(ws.tree.parent(next.node), ROOT);
    }

    #[test]
    fn return_on_an_empty_row() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "");
        let t = ws.tree.create_at(ROOT, 1, "");
        ws.tree.set_kind(t, BlockKind::Todo);
        ws.commit_with_imports(None, None, None);
        // An empty bullet gets another below it, like any row ending at the caret.
        let next = ws.enter(a, 0..0, ROOT);
        assert_ne!(next.node, a);
        assert_eq!(ws.tree.children(ROOT), &[a, next.node, t]);
        // An empty to-do turns into a bullet first.
        assert_eq!(ws.enter(t, 0..0, ROOT), Caret { node: t, offset: 0 });
        assert_eq!(ws.tree.node(t).kind, BlockKind::Bullet);
        assert_eq!(ws.tree.children(ROOT).len(), 3);
    }

    #[test]
    fn return_in_the_title_and_on_a_mirror() {
        let mut ws = ws();
        let page_root = ws.tree.create_at(ROOT, 0, "page");
        let first = ws.tree.create_at(page_root, 0, "first");
        ws.tree.set_kind(first, BlockKind::Todo);
        ws.commit_with_imports(None, None, None);
        let child = ws.enter(page_root, 2..2, page_root);
        assert_eq!(ws.tree.children(page_root), &[child.node, first]);
        assert_eq!(ws.tree.node(child.node).kind, BlockKind::Todo);
        assert_eq!(ws.tree.node(page_root).text, "page", "the title isn't split");

        let mirror = ws.tree.create_mirror(first, ROOT, 1).unwrap();
        ws.commit_with_imports(None, None, None);
        let after = ws.enter(mirror, 2..2, ROOT);
        assert_eq!(ws.tree.children(ROOT), &[page_root, mirror, after.node]);
        assert_eq!(ws.tree.node(first).text, "first");
    }

    #[test]
    fn return_keeps_to_byte_boundaries() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "日记");
        ws.commit_with_imports(None, None, None);
        // Byte 4 is inside 记: the split falls back to the boundary before it.
        ws.enter(a, 4..4, ROOT);
        assert_eq!(page(&ws.tree, ROOT), "日\n记");
    }

    #[test]
    fn backspace_at_the_start_merges_into_the_row_above() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "hello ");
        let b = ws.tree.create_at(ROOT, 1, "world");
        ws.tree.set_note(a, "a's note");
        ws.tree.set_note(b, "b's note");
        ws.tree.set_completed(b, true);
        ws.commit_with_imports(None, None, None);
        let caret = ws.backspace_at_start(b, above(&ws.tree, ROOT, b), ROOT).unwrap();
        assert_eq!(caret, Caret { node: a, offset: 6 });
        assert_eq!(page(&ws.tree, ROOT), "hello world");
        assert_eq!(ws.tree.node(a).note, "a's note\nb's note");
        assert!(!ws.tree.node(a).completed, "the row above keeps its own state");
        assert_eq!(ws.undo(), Some(Some(Caret { node: b, offset: 0 })));
        assert_eq!(page(&ws.tree, ROOT), "hello \nworld");
        assert!(ws.tree.node(b).completed);
    }

    #[test]
    fn backspace_takes_the_children_along() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "a");
        let b = ws.tree.create_at(ROOT, 1, "b");
        let c = ws.tree.create_at(b, 0, "c");
        ws.commit_with_imports(None, None, None);
        let caret = ws.backspace_at_start(b, Some(a), ROOT).unwrap();
        assert_eq!(caret, Caret { node: a, offset: 1 });
        assert_eq!(page(&ws.tree, ROOT), "ab\n  c");
        assert_eq!(ws.tree.children(a), &[c]);
        assert!(ws.undo().is_some());
        assert_eq!(page(&ws.tree, ROOT), "a\nb\n  c");
    }

    #[test]
    fn backspace_on_an_empty_row_deletes_it() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "a");
        let empty = ws.tree.create_at(ROOT, 1, "");
        ws.commit_with_imports(None, None, None);
        assert_eq!(ws.backspace_at_start(empty, Some(a), ROOT), Some(Caret { node: a, offset: 1 }));
        assert!(!ws.tree.contains(empty));
        assert!(ws.undo().is_some());
        assert!(ws.tree.contains(empty));
        // An empty row with a note merges instead, so the note is kept.
        ws.tree.set_note(empty, "keep");
        ws.commit_with_imports(None, None, None);
        assert_eq!(ws.backspace_at_start(empty, Some(a), ROOT), Some(Caret { node: a, offset: 1 }));
        assert_eq!(ws.tree.node(a).note, "keep");
    }

    #[test]
    fn backspace_does_nothing_where_the_desktop_does_nothing() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "a");
        let a1 = ws.tree.create_at(a, 0, "a1");
        let b = ws.tree.create_at(ROOT, 1, "b");
        let b1 = ws.tree.create_at(b, 0, "b1");
        ws.tree.create_at(b1, 0, "c");
        ws.commit_with_imports(None, None, None);
        let whole = page(&ws.tree, ROOT);
        // The first row, and the page's title.
        assert_eq!(ws.backspace_at_start(a, None, ROOT), None);
        assert_eq!(ws.backspace_at_start(b, Some(b1), b), None);
        // A row with children into a row with children of its own, or into its parent.
        assert_eq!(ws.backspace_at_start(b, Some(a), ROOT), None);
        assert_eq!(ws.backspace_at_start(b1, Some(b), ROOT), None);
        // An image row.
        let mut img = ws.tree.node(a1).clone();
        img.kind = BlockKind::Image;
        img.image = "0".repeat(64) + ".png";
        ws.tree.put(img);
        ws.commit_with_imports(None, None, None);
        assert_eq!(ws.backspace_at_start(a1, Some(a), ROOT), None);
        assert_eq!(page(&ws.tree, ROOT), whole);
        assert!(ws.undo().is_some(), "only the image change is a step");
    }

    #[test]
    fn return_on_a_completed_row_with_a_note_and_children() {
        let mut ws = ws();
        let a = ws.tree.create_at(ROOT, 0, "done task");
        ws.tree.set_note(a, "note");
        ws.tree.set_completed(a, true);
        let kid = ws.tree.create_at(a, 0, "kid");
        ws.commit_with_imports(None, None, None);
        let caret = ws.enter(a, 4..4, ROOT);
        assert_eq!(page(&ws.tree, ROOT), "done\n  kid\n task");
        let (head, tail) = (ws.tree.node(a), ws.tree.node(caret.node));
        assert!(head.completed && head.note == "note", "the first half keeps its state and note");
        assert!(!tail.completed && tail.note.is_empty(), "the new bullet starts plain");
        assert_eq!(ws.tree.children(a), &[kid], "the children stay with the first half");
        assert!(ws.undo().is_some());
        assert_eq!(page(&ws.tree, ROOT), "done task\n  kid", "one step undoes it");
        assert!(ws.tree.node(a).completed);
    }

    #[test]
    fn backspace_on_and_below_a_mirror() {
        let mut ws = ws();
        let src = ws.tree.create_at(ROOT, 0, "source");
        let mirror = ws.tree.create_mirror(src, ROOT, 1).unwrap();
        let below = ws.tree.create_at(ROOT, 2, " more");
        ws.commit_with_imports(None, None, None);
        // A mirror row doesn't merge (its link would be lost), and doesn't go when empty.
        assert_eq!(ws.backspace_at_start(mirror, Some(src), ROOT), None);
        assert!(ws.tree.contains(mirror));
        // A row below a mirror joins the mirrored bullet's text.
        assert_eq!(ws.backspace_at_start(below, Some(mirror), ROOT), Some(Caret { node: mirror, offset: 6 }));
        assert_eq!(ws.tree.node(src).text, "source more");
        assert!(!ws.tree.contains(below));
        assert!(ws.undo().is_some());
        assert_eq!(ws.tree.node(src).text, "source");
        assert!(ws.tree.contains(below));
    }
}
