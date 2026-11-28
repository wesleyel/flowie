//! Where a dragged bullet lands, shared by the desktop and the native apps: the gap between
//! two rows of a page, and a depth picked by how far right the pointer is. The caller draws
//! the insertion line at that depth and, on drop, moves the bullet (`Workspace::drop_bullet`).

use flowie_core::{NodeId, Tree};

use crate::Workspace;

/// Where a dropped bullet goes: `index` among `parent`'s children, shown `depth` levels into
/// the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub parent: NodeId,
    pub index: usize,
    pub depth: u16,
}

/// Where `dragged` lands in the gap below the row `above` (its id and depth; none: the top of
/// the page at `zoom`), over a row at depth `below` (none: the end of the page), with the
/// pointer `want` levels in. The depth goes no deeper than a child of the row above and no
/// shallower than the row below, as on the desktop. None when that would put the bullet
/// under itself. `dragged` is none for something new (text dropped from another app).
pub fn place(
    tree: &Tree,
    zoom: NodeId,
    above: Option<(NodeId, u16)>,
    below: Option<u16>,
    want: u16,
    dragged: Option<NodeId>,
) -> Option<Place> {
    let max_d = above.map_or(0, |(_, d)| d + 1);
    let min_d = below.unwrap_or(0).min(max_d);
    let depth = want.clamp(min_d, max_d);
    let (parent, index) = match above {
        None => (zoom, 0),
        Some((a, d)) if depth == d + 1 => (a, 0),
        Some((a, d)) => {
            let mut x = a;
            for _ in 0..(d - depth) {
                x = tree.parent(x);
            }
            (tree.parent(x), tree.index_in_parent(x) + 1)
        }
    };
    let invalid = dragged.is_some_and(|d| tree.resolve(parent) == tree.resolve(d) || tree.is_ancestor(d, parent));
    (!invalid).then_some(Place { parent, index, depth })
}

impl Workspace {
    /// Move `dragged`, with what is under it, to `place`, unfolding its new parent so it shows
    /// there (the bullet itself keeps its folding). One undo step; false when nothing moved.
    pub fn drop_bullet(&mut self, dragged: NodeId, place: Place) -> bool {
        if !self.tree.move_to(dragged, place.parent, place.index) {
            return false;
        }
        self.tree.set_collapsed(place.parent, false);
        self.commit_with_imports(None, None, None);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowie_core::{flatten, ROOT};

    fn page(tree: &Tree) -> String {
        flatten(tree, ROOT, &Default::default())
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth as usize), tree.node(row.id).text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_depth_picks_sibling_child_or_a_shallower_level() {
        let mut ws = Workspace::new(Tree::new());
        let a = ws.tree.create_at(ROOT, 0, "a");
        let b = ws.tree.create_at(a, 0, "b");
        let c = ws.tree.create_at(ROOT, 1, "c");
        let d = ws.tree.create_at(ROOT, 2, "d");
        ws.commit_with_imports(None, None, None);
        let t = &ws.tree;
        // Below b (depth 1), above c (depth 0): child of b, sibling of b, or after a.
        assert_eq!(place(t, ROOT, Some((b, 1)), Some(0), 5, Some(d)), Some(Place { parent: b, index: 0, depth: 2 }));
        assert_eq!(place(t, ROOT, Some((b, 1)), Some(0), 1, Some(d)), Some(Place { parent: a, index: 1, depth: 1 }));
        assert_eq!(place(t, ROOT, Some((b, 1)), Some(0), 0, Some(d)), Some(Place { parent: ROOT, index: 1, depth: 0 }));
        // At the top, whatever the pointer.
        assert_eq!(place(t, ROOT, None, Some(0), 3, Some(d)), Some(Place { parent: ROOT, index: 0, depth: 0 }));
        // Not under itself.
        assert_eq!(place(t, ROOT, Some((b, 1)), Some(0), 5, Some(a)), None);
        assert_eq!(place(t, ROOT, Some((a, 0)), Some(1), 1, Some(a)), None);
        let to = place(&ws.tree, ROOT, Some((b, 1)), Some(0), 1, Some(d)).unwrap();
        assert!(ws.drop_bullet(d, to));
        assert_eq!(page(&ws.tree), "a\n  b\n  d\nc");
        ws.undo();
        assert_eq!(page(&ws.tree), "a\n  b\nc\nd");
        assert_eq!(ws.tree.index_in_parent(c), 1);
    }

    #[test]
    fn a_folded_bullet_stays_folded_and_its_new_parent_unfolds() {
        let mut ws = Workspace::new(Tree::new());
        let a = ws.tree.create_at(ROOT, 0, "a");
        let x = ws.tree.create_at(a, 0, "x");
        ws.tree.set_collapsed(a, true);
        let b = ws.tree.create_at(ROOT, 1, "b");
        ws.tree.create_at(b, 0, "y");
        ws.tree.set_collapsed(b, true);
        ws.commit_with_imports(None, None, None);
        let to = place(&ws.tree, ROOT, Some((a, 0)), None, 1, Some(b)).unwrap();
        assert!(ws.drop_bullet(b, to));
        assert!(!ws.tree.node(a).collapsed);
        assert!(ws.tree.node(b).collapsed);
        assert_eq!(page(&ws.tree), "a\n  b\n  x");
        assert_eq!(ws.tree.parent(x), a);
    }
}
