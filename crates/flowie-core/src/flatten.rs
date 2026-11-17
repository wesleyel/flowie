//! Turns the tree into the flat list of rows the UI shows.

use std::collections::{HashMap, HashSet};

use crate::search::Query;
use crate::tree::{NodeId, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: NodeId,
    /// 0 = direct child of the zoom root.
    pub depth: u16,
}

#[derive(Debug, Clone, Default)]
pub struct ViewOptions {
    pub hide_completed: bool,
    pub query: Option<Query>,
}

/// Visible rows under `root`. Collapsed nodes hide their children. With a
/// query, only matches and their ancestors show (collapse is ignored so
/// matches deep inside collapsed nodes are found).
pub fn flatten(tree: &Tree, root: NodeId, opts: &ViewOptions) -> Vec<Row> {
    let mut rows = Vec::new();
    match &opts.query {
        Some(q) if !q.is_empty() => {
            let keep = search_keep_set(tree, root, q, opts.hide_completed);
            walk(tree, root, 0, &mut rows, &|id| keep.contains(&id), true);
        }
        _ => {
            let hide = opts.hide_completed;
            walk(tree, root, 0, &mut rows, &|id| !(hide && tree.node(id).completed), false);
        }
    }
    rows
}

fn walk(
    tree: &Tree,
    parent: NodeId,
    depth: u16,
    rows: &mut Vec<Row>,
    keep: &dyn Fn(NodeId) -> bool,
    ignore_collapse: bool,
) {
    // Explicit stack to stay safe on very deep outlines.
    let mut stack: Vec<(NodeId, u16)> = tree.children(parent).iter().rev().map(|&c| (c, depth)).collect();
    while let Some((id, d)) = stack.pop() {
        if !keep(id) {
            continue;
        }
        rows.push(Row { id, depth: d });
        let node = tree.node(id);
        if ignore_collapse || !node.collapsed {
            stack.extend(tree.children(id).iter().rev().map(|&c| (c, d + 1)));
        }
    }
}

/// Matching nodes plus all their ancestors up to `root`.
fn search_keep_set(tree: &Tree, root: NodeId, q: &Query, hide_completed: bool) -> HashSet<NodeId> {
    let mut keep = HashSet::new();
    for id in tree.descendants(root) {
        let node = tree.node(id);
        if hide_completed && node.completed {
            continue;
        }
        if q.matches(node) {
            let mut cur = id;
            while cur != root && keep.insert(cur) {
                cur = tree.parent(cur);
            }
        }
    }
    keep
}

/// A row in the displayed list: present rows plus rows animating out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayRow {
    pub row: Row,
    pub leaving: bool,
}

/// Merge the new rows with rows that disappeared since `old`, keeping each
/// leaving row right after the row that preceded it before, so it can
/// shrink in place instead of vanishing.
pub fn merge_leaving(old: &[Row], new: &[Row]) -> Vec<DisplayRow> {
    let new_ids: HashSet<NodeId> = new.iter().map(|r| r.id).collect();
    // anchor (an id present in `new`, or None for the top) -> leaving rows after it
    let mut after: HashMap<Option<NodeId>, Vec<Row>> = HashMap::new();
    let mut anchor = None;
    for r in old {
        if new_ids.contains(&r.id) {
            anchor = Some(r.id);
        } else {
            after.entry(anchor).or_default().push(*r);
        }
    }
    let mut out = Vec::with_capacity(new.len() + after.values().map(Vec::len).sum::<usize>());
    if let Some(top) = after.remove(&None) {
        out.extend(top.into_iter().map(|row| DisplayRow { row, leaving: true }));
    }
    for r in new {
        out.push(DisplayRow { row: *r, leaving: false });
        if let Some(gone) = after.remove(&Some(r.id)) {
            out.extend(gone.into_iter().map(|row| DisplayRow { row, leaving: true }));
        }
    }
    out
}
