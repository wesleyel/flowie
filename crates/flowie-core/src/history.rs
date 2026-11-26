//! Undo/redo over journaled tree changes. Consecutive typing in the same
//! node coalesces into one step.

use std::time::Duration;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::Instant;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_time::Instant;

use std::collections::HashMap;

use crate::tree::{Change, Edit, Field, NodeId, Tree};

/// Where the caret was, so undo/redo can put it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub node: NodeId,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Typing(NodeId),
    Structure,
}

#[derive(Debug)]
struct Step {
    imports: Option<(crate::imports::ImportState, crate::imports::ImportState)>,
    changes: Vec<Change>,
    kind: Kind,
    caret_before: Option<Caret>,
    caret_after: Option<Caret>,
    at: Instant,
}

/// What a step keeps of its changes (oldest first): one patch per field of each record,
/// from the value before the step to the value after it, so typing a sentence keeps one
/// text patch instead of one per keystroke, and none for a field that ends where it
/// started. A record created or removed in between starts its fields afresh, so they still
/// undo in order around it.
fn compact(changes: Vec<Change>) -> Vec<Change> {
    let mut kept: Vec<Change> = Vec::with_capacity(changes.len());
    let mut open: HashMap<(NodeId, Field), usize> = HashMap::new();
    for change in changes {
        match change.edit {
            Edit::Set(patch) => {
                let key = (change.id, patch.after.field());
                if let Some(&ix) = open.get(&key) {
                    if let Edit::Set(first) = &mut kept[ix].edit {
                        first.after = patch.after;
                    }
                    continue;
                }
                open.insert(key, kept.len());
                kept.push(Change { id: change.id, edit: Edit::Set(patch) });
            }
            edit => {
                for field in Field::ALL {
                    open.remove(&(change.id, field));
                }
                kept.push(Change { id: change.id, edit });
            }
        }
    }
    kept.retain(|c| !matches!(&c.edit, Edit::Set(patch) if patch.before == patch.after));
    kept.shrink_to_fit();
    kept
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

const COALESCE: Duration = Duration::from_millis(1200);
const LIMIT: usize = 2000;

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record everything journaled in `tree` since the last commit as one step.
    pub fn commit(&mut self, tree: &mut Tree, kind: Kind, caret_before: Option<Caret>, caret_after: Option<Caret>) {
        self.commit_with_imports(tree, kind, caret_before, caret_after, None);
    }

    pub fn commit_with_imports(
        &mut self,
        tree: &mut Tree,
        kind: Kind,
        caret_before: Option<Caret>,
        caret_after: Option<Caret>,
        imports: Option<(crate::imports::ImportState, crate::imports::ImportState)>,
    ) {
        let changes = tree.take_journal();
        if changes.is_empty() && imports.is_none() {
            return;
        }
        self.redo.clear();
        if let (Kind::Typing(id), Some(last)) = (kind, self.undo.last_mut()) {
            if last.kind == Kind::Typing(id) && last.at.elapsed() < COALESCE {
                let mut all = std::mem::take(&mut last.changes);
                all.extend(changes);
                last.changes = compact(all);
                last.caret_after = caret_after;
                last.at = Instant::now();
                return;
            }
        }
        self.undo.push(Step { imports, changes: compact(changes), kind, caret_before, caret_after, at: Instant::now() });
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    /// Stop the current typing step from absorbing further keystrokes.
    pub fn seal(&mut self) {
        if let Some(last) = self.undo.last_mut() {
            last.kind = Kind::Structure;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_imports(&self) -> Option<&crate::imports::ImportState> {
        self.undo.last()?.imports.as_ref().map(|pair| &pair.0)
    }
    pub fn redo_imports(&self) -> Option<&crate::imports::ImportState> {
        self.redo.last()?.imports.as_ref().map(|pair| &pair.1)
    }

    /// Drop every recorded change to `ids` (records deleted for good, `Tree::purge`), and
    /// every move into or out of them, so undo and redo never bring back part of what is
    /// gone or put a node under it. Steps left with nothing to do go too.
    pub fn forget(&mut self, ids: &[NodeId]) {
        if ids.is_empty() {
            return;
        }
        let ids: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
        let touches = |c: &Change| ids.contains(&c.id) || c.parents().iter().any(|p| ids.contains(p));
        for stack in [&mut self.undo, &mut self.redo] {
            for step in stack.iter_mut() {
                step.changes.retain(|c| !touches(c));
            }
            stack.retain(|step| !step.changes.is_empty() || step.imports.is_some());
        }
    }

    pub fn undo(&mut self, tree: &mut Tree) -> Option<Option<Caret>> {
        let step = self.undo.pop()?;
        let inverse = tree.revert(&step.changes);
        let caret = step.caret_before;
        self.redo.push(Step { changes: inverse, kind: Kind::Structure, ..step });
        Some(caret)
    }

    pub fn redo(&mut self, tree: &mut Tree) -> Option<Option<Caret>> {
        let step = self.redo.pop()?;
        let inverse = tree.revert(&step.changes);
        let caret = step.caret_after;
        self.undo.push(Step { changes: inverse, kind: Kind::Structure, ..step });
        Some(caret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::ROOT;

    #[test]
    fn a_change_stays_small() {
        // An import keeps one of these per bullet (#18).
        assert!(std::mem::size_of::<Change>() <= 48, "{}", std::mem::size_of::<Change>());
    }

    #[test]
    fn a_step_keeps_one_patch_per_field() {
        let mut t = Tree::new();
        let a = t.create_at(ROOT, 0, "");
        t.take_journal();
        let mut h = History::new();
        for len in 1..=50 {
            t.set_text(a, &"x".repeat(len));
            h.commit(&mut t, Kind::Typing(a), None, None);
        }
        t.set_note(a, "n");
        t.set_note(a, "");
        h.commit(&mut t, Kind::Typing(a), None, None);
        let step = &h.undo[0].changes;
        let texts = step.iter().filter(|c| c.field() == Some(Field::Text)).count();
        assert_eq!(texts, 1, "{step:?}");
        assert!(step.iter().all(|c| c.field() != Some(Field::Note)), "a note back where it was keeps no patch");

        h.undo(&mut t).unwrap();
        assert_eq!(t.node(a).text, "");
        h.redo(&mut t).unwrap();
        assert_eq!(t.node(a).text, "x".repeat(50));
    }

    #[test]
    fn a_record_created_and_removed_in_one_step_undoes_in_order() {
        let mut t = Tree::new();
        let mut h = History::new();
        let a = t.create_at(ROOT, 0, "one");
        t.set_text(a, "two");
        t.remove(a);
        let b = t.create_at(ROOT, 0, "three");
        t.set_text(b, "four");
        h.commit(&mut t, Kind::Structure, None, None);
        h.undo(&mut t).unwrap();
        assert!(!t.contains(a) && !t.contains(b));
        h.redo(&mut t).unwrap();
        assert!(!t.contains(a));
        assert_eq!(t.node(b).text, "four");
    }
}
