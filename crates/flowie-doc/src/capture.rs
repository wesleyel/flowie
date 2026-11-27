//! Quick capture into the journal, for the native apps (wiki: iOS-Cross-Platform §2.1, §5):
//! "Add to Today's Journal" appends text to a day, and the capture entries (the home-screen
//! quick action, the Control Center control, the Action Button) open a day with the caret on
//! an empty row. Days are found and made as the desktop does (`flowie_core::journal`, the same
//! ids on every device), so capturing into a day on two devices makes one page.

use flowie_core::io::Item;
use flowie_core::journal;
use flowie_core::{Caret, NodeId, Tree};

use crate::Workspace;

/// The bullets `text` makes: one per line with something on it, trimmed and taken literally
/// (no Markdown), since a bullet's text is one line.
pub fn items(text: &str) -> Vec<Item> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(Item::new).collect()
}

/// Append `text` ([`items`]) to the end of the journal page for `date` (YYYY-MM-DD), making
/// the page if needed (`journal::journal_append`). An empty last row (a new page's blank line,
/// or one a capture left: [`capture_row`]) is replaced, unless it is `keep`, a bullet being
/// typed in. Returns the caret at the end of the last bullet added, or `None` (nothing
/// changed) when `text` has nothing in it.
pub fn append(tree: &mut Tree, date: &str, text: &str, keep: Option<NodeId>) -> Option<Caret> {
    let items = items(text);
    if items.is_empty() {
        return None;
    }
    let trailing = journal::find_day(tree, date).and_then(|day| empty_last(tree, day)).filter(|&id| Some(id) != keep);
    let last = *journal::journal_append(tree, date, &items, keep).last()?;
    if let Some(id) = trailing.filter(|&id| tree.contains(id)) {
        tree.delete(id);
    }
    Some(Caret { node: last, offset: tree.node(last).text.len() })
}

/// The last top-level row of `day` when it is empty and unfinished, to write in.
fn empty_last(tree: &Tree, day: NodeId) -> Option<NodeId> {
    let last = *tree.children(day).last()?;
    (journal::is_blank_placeholder(tree, last) && !tree.node(last).completed).then_some(last)
}

/// Where to start writing in the journal page for `date` (YYYY-MM-DD): its last top-level
/// bullet when that is an empty, unfinished one (as the blank first line a new page is made
/// with, whose id is the same on every device), otherwise a new empty bullet at the end. Makes
/// the page if needed, as the desktop opens a day. Opening it again reuses the same row, so
/// capturing without typing leaves at most one empty row.
pub fn capture_row(tree: &mut Tree, date: &str) -> Caret {
    let day = journal::ensure_day(tree, date, true);
    if tree.node(day).collapsed {
        tree.set_collapsed(day, false);
    }
    let node = empty_last(tree, day).unwrap_or_else(|| {
        let index = tree.children(day).len();
        tree.create_at(day, index, "")
    });
    Caret { node, offset: 0 }
}

impl Workspace {
    /// [`append`], keeping the bullet being typed in, as one undo step.
    pub fn journal_append(&mut self, date: &str, text: &str) -> Option<Caret> {
        let keep = self.editing();
        let after = append(&mut self.tree, date, text, keep)?;
        self.commit_with_imports(None, Some(after), None);
        Some(after)
    }

    /// [`capture_row`], as one undo step when it made something.
    pub fn journal_capture(&mut self, date: &str) -> Caret {
        let after = capture_row(&mut self.tree, date);
        if !self.tree.journal_is_empty() {
            self.commit_with_imports(None, Some(after), None);
        }
        after
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowie_core::journal::{day_blank_id, day_id, journal_root_id};
    use flowie_core::ROOT;

    const DAY: &str = "2026-11-27";

    fn ws() -> Workspace {
        Workspace::new(Tree::new())
    }

    fn texts(ws: &Workspace, parent: NodeId) -> Vec<String> {
        ws.tree.children(parent).iter().map(|id| ws.tree.node(*id).text.clone()).collect()
    }

    #[test]
    fn appending_makes_the_desktops_day_once_and_undoes_in_one_step() {
        let mut ws = ws();
        let caret = ws.journal_append(DAY, "  - 日😀 **two**  ").unwrap();
        assert_eq!(ws.tree.children(ROOT), &[journal_root_id()]);
        assert_eq!(ws.tree.children(journal_root_id()), &[day_id(DAY)]);
        assert_eq!(texts(&ws, day_id(DAY)), ["- 日😀 **two**"], "taken literally, trimmed");
        assert_eq!(caret, Caret { node: ws.tree.children(day_id(DAY))[0], offset: "- 日😀 **two**".len() });

        ws.journal_append(DAY, "second\n\n  third \r\n").unwrap();
        assert_eq!(texts(&ws, day_id(DAY)), ["- 日😀 **two**", "second", "third"], "one bullet a line");
        assert_eq!(ws.tree.children(journal_root_id()), &[day_id(DAY)], "still one page for the day");

        assert!(ws.undo().is_some());
        assert_eq!(texts(&ws, day_id(DAY)), ["- 日😀 **two**"], "both lines go in one step");
        assert!(ws.undo().is_some());
        assert!(ws.tree.children(ROOT).is_empty(), "the page made goes with the first capture");
        assert!(ws.redo().is_some());
        assert_eq!(texts(&ws, day_id(DAY)), ["- 日😀 **two**"]);
    }

    #[test]
    fn nothing_to_append_changes_nothing() {
        let mut ws = ws();
        assert_eq!(ws.journal_append(DAY, " \n\t\n"), None);
        assert!(ws.tree.children(ROOT).is_empty());
        assert_eq!(ws.undo(), None);
    }

    #[test]
    fn appending_replaces_a_new_pages_blank_line_unless_it_is_being_typed_in() {
        let mut ws = ws();
        let row = ws.journal_capture(DAY);
        assert_eq!(row.node, day_blank_id(DAY));
        ws.journal_append(DAY, "a").unwrap();
        assert_eq!(texts(&ws, day_id(DAY)), ["a"]);

        let other = "2026-11-28";
        let row = ws.journal_capture(other);
        ws.begin_edit(row.node);
        ws.journal_append(other, "b").unwrap();
        assert_eq!(texts(&ws, day_id(other)), ["", "b"], "the row being typed in stays");
    }

    #[test]
    fn capture_reuses_an_empty_last_row_and_makes_the_same_ids_on_every_device() {
        let mut a = ws();
        let mut b = ws();
        let on_a = a.journal_capture(DAY);
        let on_b = b.journal_capture(DAY);
        assert_eq!(on_a, Caret { node: day_blank_id(DAY), offset: 0 }, "a new day's first line, as on the desktop");
        assert_eq!(on_a, on_b, "two devices capturing into a new day make the same rows");

        // Opening it again doesn't pile up empty rows, nor make an undo step.
        assert_eq!(a.journal_capture(DAY), on_a);
        assert_eq!(texts(&a, day_id(DAY)), [""]);
        assert!(a.undo().is_some());
        assert!(a.undo().is_none(), "only making the day was a step");
        assert!(a.redo().is_some());

        // After something is written, a new empty row at the end; then that one is reused.
        a.tree.set_text(on_a.node, "written");
        a.commit_typing(on_a.node, None, None);
        let next = a.journal_capture(DAY);
        assert_ne!(next.node, on_a.node);
        assert_eq!(texts(&a, day_id(DAY)), ["written", ""]);
        assert_eq!(a.journal_capture(DAY), next);
        assert!(a.undo().is_some());
        assert_eq!(texts(&a, day_id(DAY)), ["written"], "the new row is one step");

        // A finished empty row isn't reused; a folded day opens.
        a.tree.set_collapsed(day_id(DAY), true);
        let done = a.journal_capture(DAY).node;
        a.tree.set_completed(done, true);
        a.commit_with_imports(None, None, None);
        assert_ne!(a.journal_capture(DAY).node, done);
        assert!(!a.tree.node(day_id(DAY)).collapsed);
    }

    #[test]
    fn appending_fills_an_empty_row_a_capture_left() {
        let mut ws = ws();
        ws.journal_append(DAY, "first").unwrap();
        let left = ws.journal_capture(DAY).node;
        ws.journal_append(DAY, "second").unwrap();
        assert_eq!(texts(&ws, day_id(DAY)), ["first", "second"]);
        assert!(!ws.tree.contains(left));
        assert!(ws.undo().is_some());
        assert_eq!(texts(&ws, day_id(DAY)), ["first", ""], "undo brings the empty row back");
    }

    #[test]
    fn capture_brings_a_deleted_day_back() {
        let mut ws = ws();
        ws.journal_append(DAY, "kept").unwrap();
        ws.tree.trash(day_id(DAY));
        ws.commit_with_imports(None, None, None);
        let row = ws.journal_capture(DAY);
        assert_eq!(texts(&ws, day_id(DAY)), ["kept", ""]);
        assert_eq!(ws.tree.parent(row.node), day_id(DAY));
        assert_eq!(ws.tree.children(journal_root_id()), &[day_id(DAY)]);
    }
}
