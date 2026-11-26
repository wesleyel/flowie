//! Undo/redo over journaled tree changes. Consecutive typing in the same
//! node coalesces into one step.

use std::time::Duration;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::Instant;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_time::Instant;

use crate::tree::{Change, Node, NodeId, Tree};

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
    changes: Vec<Kept>,
    kind: Kind,
    caret_before: Option<Caret>,
    caret_after: Option<Caret>,
    at: Instant,
}

/// What a step keeps of one change: the node's state before the step (None =
/// absent), boxed so that a node the step created costs a few bytes, not a whole
/// `Node`.
#[derive(Debug)]
struct Kept {
    id: NodeId,
    before: Option<Box<Node>>,
}

/// What a step keeps of its changes (oldest first): undoing it only needs the state
/// each node had before the step, which is the `before` of its first change
/// (`Tree::revert` takes the rest from the tree). Later changes to the same node and
/// every `after` would only hold more copies: a whole imported outline, for one.
fn for_undo(changes: impl IntoIterator<Item = Change>) -> Vec<Kept> {
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<Kept> =
        changes.into_iter().filter(|c| seen.insert(c.id)).map(|c| Kept { id: c.id, before: c.before.map(Box::new) }).collect();
    // `collect` reuses the journal's buffer, sized for whole `Change`s: give it back.
    kept.shrink_to_fit();
    kept
}

/// Revert a step's changes; returns what the inverse step keeps.
fn revert(tree: &mut Tree, kept: Vec<Kept>) -> Vec<Kept> {
    let changes: Vec<Change> = kept.into_iter().map(|k| Change { id: k.id, before: k.before.map(|b| *b), after: None }).collect();
    // The ids are already unique, so `for_undo` only boxes the inverse's states.
    for_undo(tree.revert(&changes))
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
                let seen: std::collections::HashSet<NodeId> = last.changes.iter().map(|k| k.id).collect();
                last.changes.extend(for_undo(changes.into_iter().filter(|c| !seen.contains(&c.id))));
                last.caret_after = caret_after;
                last.at = Instant::now();
                return;
            }
        }
        self.undo.push(Step { imports, changes: for_undo(changes), kind, caret_before, caret_after, at: Instant::now() });
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
        // A node moved into a purged subtree is purged with it, so its own id covers that
        // move; `before` covers moves out (steps keep no `after`, see `Tree::revert`).
        let touches = |c: &Kept| ids.contains(&c.id) || c.before.as_ref().is_some_and(|n| ids.contains(&n.parent));
        for stack in [&mut self.undo, &mut self.redo] {
            for step in stack.iter_mut() {
                step.changes.retain(|c| !touches(c));
            }
            stack.retain(|step| !step.changes.is_empty() || step.imports.is_some());
        }
    }

    pub fn undo(&mut self, tree: &mut Tree) -> Option<Option<Caret>> {
        let mut step = self.undo.pop()?;
        let inverse = revert(tree, std::mem::take(&mut step.changes));
        let caret = step.caret_before;
        self.redo.push(Step { changes: inverse, kind: Kind::Structure, ..step });
        Some(caret)
    }

    pub fn redo(&mut self, tree: &mut Tree) -> Option<Option<Caret>> {
        let mut step = self.redo.pop()?;
        let inverse = revert(tree, std::mem::take(&mut step.changes));
        let caret = step.caret_after;
        self.undo.push(Step { changes: inverse, kind: Kind::Structure, ..step });
        Some(caret)
    }
}
