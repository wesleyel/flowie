//! What an edit changed, for a view that keeps its rows and updates only what changed (the
//! native apps; wiki: iOS-Cross-Platform §3.2, §4.2). Read from the changes each commit, undo
//! and redo records, so it needs nothing from the edit itself but where the caret goes.

use std::collections::HashSet;

use flowie_core::tree::{Change, Field};
use flowie_core::{Caret, NodeId, Tree};

use crate::Workspace;

/// What an edit changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// Rows to read again, whose bullet's text, note, completion, kind or image changed:
    /// the bullet's own row and the mirrors showing it. Bullets gone since are left out
    /// (`structure_changed` says rows went).
    pub changed_ids: Vec<NodeId>,
    /// Rows may have come, gone or moved (a bullet created, deleted, moved, collapsed,
    /// expanded or made a mirror), so depths and children may differ too: read the page
    /// again. Also set when a changed bullet shows inside a mirror, where its row has a
    /// virtual id.
    pub structure_changed: bool,
    /// Where the caret goes (a byte offset in the bullet's text), when the edit moves it.
    pub focus: Option<Caret>,
}

impl Outcome {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.changed_ids.is_empty() && !self.structure_changed
    }
}

/// The changes seen while an edit runs (`Workspace::tracked`).
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    content: HashSet<NodeId>,
    structure: bool,
}

impl Tracker {
    pub(crate) fn note(&mut self, changes: &[Change]) {
        for change in changes {
            match change.field() {
                Some(Field::Text | Field::Note | Field::Completed | Field::Kind | Field::Image) => {
                    self.content.insert(change.id);
                }
                Some(Field::Modified) => {}
                // Created, removed, or Position, Deleted, Collapsed, Mirror.
                _ => self.structure = true,
            }
        }
    }

    fn finish(self, tree: &Tree) -> Outcome {
        let mut structure = self.structure;
        let mut content: Vec<NodeId> = self.content.into_iter().filter(|id| tree.contains(*id)).collect();
        content.sort();
        let mut seen = HashSet::new();
        let mut changed_ids = Vec::new();
        for id in content {
            for row in std::iter::once(id).chain(tree.mirrors_of(id)) {
                if seen.insert(row) {
                    changed_ids.push(row);
                }
            }
            if !structure && tree.ancestors(id).into_iter().any(|a| !tree.mirrors_of(a).is_empty()) {
                structure = true;
            }
        }
        Outcome { changed_ids, structure_changed: structure, focus: None }
    }
}

impl Workspace {
    /// Run `edit`, which changes the tree and commits (or undoes, or redoes), and say what it
    /// changed; the caller sets where the caret goes.
    pub fn tracked<R>(&mut self, edit: impl FnOnce(&mut Workspace) -> R) -> (R, Outcome) {
        let outer = self.tracker.replace(Tracker::default());
        debug_assert!(outer.is_none(), "tracked edits don't nest");
        let result = edit(self);
        let mut tracker = self.tracker.take().unwrap_or_default();
        // Changes left uncommitted count too.
        tracker.note(self.tree.journal());
        (result, tracker.finish(&self.tree))
    }

    /// Note what the next commit records, while an edit is tracked.
    pub(crate) fn track_journal(&mut self) {
        if let Some(tracker) = &mut self.tracker {
            tracker.note(self.tree.journal());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowie_core::ROOT;

    fn ws() -> (Workspace, NodeId, NodeId) {
        let mut ws = Workspace::new(Tree::new());
        let a = ws.tree.create_at(ROOT, 0, "a");
        let b = ws.tree.create_at(ROOT, 1, "b");
        ws.commit_with_imports(None, None, None);
        (ws, a, b)
    }

    #[test]
    fn typing_changes_one_row_and_keeps_the_structure() {
        let (mut ws, a, _) = ws();
        let ((), out) = ws.tracked(|ws| {
            ws.tree.set_text(a, "a, typed");
            ws.commit_typing(a, None, None);
        });
        assert_eq!(out, Outcome { changed_ids: vec![a], structure_changed: false, focus: None });
        // Undoing it changes the same row back.
        let (_, out) = ws.tracked(|ws| ws.undo());
        assert_eq!((out.changed_ids, out.structure_changed), (vec![a], false));
        let (_, out) = ws.tracked(|ws| ws.redo());
        assert_eq!((out.changed_ids, out.structure_changed), (vec![a], false));
        assert_eq!(ws.tree.node(a).text, "a, typed");
    }

    #[test]
    fn structural_edits_say_rows_changed() {
        let (mut ws, a, b) = ws();
        let check = |ws: &mut Workspace, edit: &dyn Fn(&mut Workspace)| {
            let ((), out) = ws.tracked(|ws| edit(ws));
            assert!(out.structure_changed, "{out:?}");
            let (_, undone) = ws.tracked(|ws| ws.undo());
            assert!(undone.structure_changed);
        };
        check(&mut ws, &|ws| {
            ws.tree.indent(b);
            ws.commit_with_imports(None, None, None);
        });
        check(&mut ws, &|ws| {
            ws.tree.create_at(ROOT, 2, "c");
            ws.commit_with_imports(None, None, None);
        });
        check(&mut ws, &|ws| {
            ws.tree.trash(a);
            ws.commit_with_imports(None, None, None);
        });
        ws.tree.create_at(a, 0, "child");
        ws.commit_with_imports(None, None, None);
        check(&mut ws, &|ws| {
            ws.tree.set_collapsed(a, true);
            ws.commit_with_imports(None, None, None);
        });
        // Return splits a row: the row's text changes and a row comes.
        let (caret, out) = ws.tracked(|ws| ws.enter(b, 1..1, ROOT));
        assert!(out.structure_changed);
        assert_ne!(caret.node, b);
    }

    #[test]
    fn nothing_changed_is_empty() {
        let (mut ws, a, _) = ws();
        let (_, out) = ws.tracked(|ws| ws.backspace_at_start(a, None, ROOT));
        assert!(out.is_empty());
        let (_, out) = ws.tracked(|ws| ws.redo());
        assert!(out.is_empty());
        let ((), out) = ws.tracked(|ws| {
            ws.tree.set_text(a, "a");
            ws.commit_typing(a, None, None);
        });
        assert!(out.is_empty(), "the same text");
    }

    #[test]
    fn a_mirrored_bullet_changes_its_mirrors_rows_too() {
        let (mut ws, a, b) = ws();
        let child = ws.tree.create_at(a, 0, "child");
        let mirror = ws.tree.create_mirror(a, b, 0).unwrap();
        ws.commit_with_imports(None, None, None);
        let ((), out) = ws.tracked(|ws| {
            ws.tree.set_text(a, "a, edited");
            ws.commit_typing(a, None, None);
        });
        assert_eq!((out.changed_ids, out.structure_changed), (vec![a, mirror], false));
        // A bullet under a mirrored one also shows inside the mirror, under a virtual id.
        let ((), out) = ws.tracked(|ws| {
            ws.tree.set_text(child, "child, edited");
            ws.commit_typing(child, None, None);
        });
        assert_eq!((out.changed_ids, out.structure_changed), (vec![child], true));
    }
}
