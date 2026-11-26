//! Undo/redo over journaled tree changes. Consecutive typing in the same
//! node coalesces into one step.

use std::time::Duration;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::Instant;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_time::Instant;

use crate::tree::{Change, NodeId, Tree};

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
                last.changes.extend(changes);
                last.caret_after = caret_after;
                last.at = Instant::now();
                return;
            }
        }
        self.undo.push(Step { imports, changes, kind, caret_before, caret_after, at: Instant::now() });
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
        let touches = |c: &Change| ids.contains(&c.id) || [&c.before, &c.after].into_iter().flatten().any(|n| ids.contains(&n.parent));
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
