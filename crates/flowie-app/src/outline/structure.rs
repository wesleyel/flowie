//! Change the tree with insertion, indentation, moves, mirrors, and deletion.

use flowie_core::{NodeId, ROOT};
use gpui::{Bounds, Context, Pixels, Window};

use super::*;
use crate::app::i18n::t;
use crate::overlays::palette::{node_label, Mode};
use crate::ui::anim::{self, Channel};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropTarget {
    /// Show the insertion line above this node (or below it if `below`).
    pub node: NodeId,
    pub below: bool,
    pub depth: u16,
    pub parent: NodeId,
    pub index: usize,
}

#[derive(Clone)]
pub struct DragNode {
    pub id: NodeId,
    pub label: String,
}

impl Outline {
    pub fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_confirm(window, cx) {
            return;
        }
        match self.edit_target() {
            Some(Target::Text(id)) => {
                if id == self.zoom {
                    // Enter in the title adds a first child.
                    let before = self.caret();
                    let first = self.ws.tree.children(id).first().map(|&c| self.ws.tree.node(c).kind);
                    let child = self.ws.tree.create_at(id, 0, "");
                    if let Some(kind) = first {
                        self.ws.tree.set_kind(child, kind.list_kind());
                    }
                    self.edit = Some(Edit::new(Target::Text(child), 0));
                    self.commit_structure(before, cx);
                    return;
                }
                let before = self.caret();
                if self.ws.tree.node(id).mirror != ROOT {
                    // Enter on a mirror adds a plain sibling after it.
                    let n = self.ws.tree.create_after(id, "");
                    self.edit = Some(Edit::new(Target::Text(n), 0));
                    self.commit_structure(before, cx);
                    return;
                }
                let caret = self.edit.as_ref().unwrap().caret();
                if let Some(e) = self.edit.as_ref() {
                    if !e.sel.is_empty() {
                        let r = e.sel.clone();
                        let text = self.edit_text().to_string();
                        self.ws.tree.set_text(id, &format!("{}{}", &text[..r.start], &text[r.end..]));
                    }
                }
                let caret = self.edit.as_ref().map(|e| e.sel.start).unwrap_or(caret);
                let (focus, off) = self.ws.tree.split(id, caret);
                self.edit = Some(Edit::new(Target::Text(focus), off));
                self.commit_structure(before, cx);
            }
            Some(Target::Note(_)) => {
                let r = self.edit.as_ref().unwrap().sel.clone();
                self.replace_edit_text(r, "\n", false, cx);
            }
            Some(Target::Search) => {
                if let Some(first) = self.rows.first().map(|r| r.id) {
                    let len = self.ws.tree.node(first).text.len();
                    self.start_edit(Target::Text(first), len, cx);
                }
            }
            Some(Target::Palette | Target::Field(_)) => {}
            None => self.enter_from_idle(cx),
        }
    }

    pub(super) fn enter_from_idle(&mut self, cx: &mut Context<Self>) {
        if let Some(b) = self.block {
            let len = self.ws.tree.node(b.head).text.len();
            self.start_edit(Target::Text(b.head), len, cx);
        } else if let Some(first) = self.rows.first().map(|r| r.id) {
            self.start_edit(Target::Text(first), 0, cx);
        } else {
            self.append_child_and_edit(cx);
        }
    }

    pub fn append_child_and_edit(&mut self, cx: &mut Context<Self>) {
        // Reuse a trailing empty bullet instead of piling up blanks.
        if let Some(&last) = self.ws.tree.children(self.zoom).last() {
            if self.ws.tree.node(last).text.is_empty() && !self.ws.tree.has_children(last) {
                self.start_edit(Target::Text(last), 0, cx);
                return;
            }
        }
        let before = self.caret();
        let n = self.ws.tree.children(self.zoom).len();
        let id = self.ws.tree.create_at(self.zoom, n, "");
        self.edit = Some(Edit::new(Target::Text(id), 0));
        self.commit_structure(before, cx);
    }

    pub fn indent(&mut self, _: &Indent, window: &mut Window, cx: &mut Context<Self>) {
        if self.inline.is_some() && self.inline_confirm(None, window, cx) {
            return;
        }
        if self.palette.is_some() {
            return;
        }
        let before = self.caret();
        if let Some(ids) = self.block_roots() {
            let mut ok = false;
            for id in ids {
                ok |= self.ws.tree.indent(id);
            }
            if ok {
                self.commit_structure(before, cx);
            }
            return;
        }
        if let Some(Target::Text(id) | Target::Note(id)) = self.edit_target() {
            if id != self.zoom && self.ws.tree.indent(id) {
                self.commit_structure(before, cx);
            }
        }
    }

    pub fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        if let Some(ids) = self.block_roots() {
            let mut ok = false;
            for id in ids.into_iter().rev() {
                ok |= self.ws.tree.outdent(id, self.zoom);
            }
            if ok {
                self.commit_structure(before, cx);
            }
            return;
        }
        if let Some(Target::Text(id) | Target::Note(id)) = self.edit_target() {
            if id != self.zoom && self.ws.tree.outdent(id, self.zoom) {
                self.commit_structure(before, cx);
            }
        }
    }

    pub fn move_to_action(&mut self, _: &MoveTo, _: &mut Window, cx: &mut Context<Self>) {
        let ids = self.targets();
        if !ids.is_empty() {
            self.open_palette(Mode::MoveTo(ids), cx);
        }
    }

    pub fn mirror_bullet(&mut self, _: &MirrorBullet, _: &mut Window, cx: &mut Context<Self>) {
        let after = self.editing_node().or(self.block.map(|b| b.head)).unwrap_or(self.zoom);
        self.open_palette(Mode::Mirror { after }, cx);
    }

    pub fn move_nodes_to(&mut self, ids: &[NodeId], target: NodeId, cx: &mut Context<Self>) {
        let before = None;
        let fallback = ids.first().and_then(|f| {
            let ix = self.row_index(*f)?;
            ix.checked_sub(1).and_then(|i| self.rows.get(i)).map(|r| r.id)
        });
        let mut moved = 0;
        for &id in ids {
            let n = self.ws.tree.children(target).len();
            if self.ws.tree.move_to(id, target, n) {
                moved += 1;
            }
        }
        if moved > 0 {
            self.ws.tree.set_collapsed(target, false);
            self.block = None;
            self.edit = fallback.filter(|f| self.ws.tree.contains(*f)).map(|f| Edit::new(Target::Text(f), 0));
            self.toast(t!("toast.moved", count = moved, target = node_label(self, target)));
            self.commit_structure(before, cx);
        }
    }

    pub fn insert_mirror(&mut self, after: NodeId, source: NodeId, cx: &mut Context<Self>) {
        let (parent, index) = if after == self.zoom {
            (self.zoom, self.ws.tree.children(self.zoom).len())
        } else {
            (self.ws.tree.parent(after), self.ws.tree.index_in_parent(after) + 1)
        };
        let before = None;
        match self.ws.tree.create_mirror(source, parent, index) {
            Some(m) => {
                // Replace an empty bullet the mirror was typed into.
                if after != self.zoom
                    && self.ws.tree.node(after).text.is_empty()
                    && !self.ws.tree.has_children(after)
                    && self.ws.tree.node(after).mirror == ROOT
                {
                    self.ws.tree.delete(after);
                }
                self.edit = Some(Edit::new(Target::Text(m), self.ws.tree.node(source).text.len()));
                self.commit_structure(before, cx);
            }
            None => {
                self.toast(t!("toast.mirror_into_self"));
                cx.notify();
            }
        }
    }

    pub fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        let ids = self.block_roots().or_else(|| self.editing_node().filter(|id| *id != self.zoom).map(|id| vec![id])).unwrap_or_default();
        let mut ok = false;
        for id in ids {
            ok |= self.ws.tree.move_up(id, self.zoom);
        }
        if ok {
            self.commit_structure(before, cx);
        }
    }

    pub fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        let ids = self.block_roots().or_else(|| self.editing_node().filter(|id| *id != self.zoom).map(|id| vec![id])).unwrap_or_default();
        let mut ok = false;
        for id in ids.into_iter().rev() {
            ok |= self.ws.tree.move_down(id, self.zoom);
        }
        if ok {
            self.commit_structure(before, cx);
        }
    }

    pub fn delete_node(&mut self, _: &DeleteNode, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        let ids = self.block_roots().or_else(|| self.editing_node().filter(|id| *id != self.zoom).map(|id| vec![id])).unwrap_or_default();
        let Some(&first) = ids.first() else { return };
        let fallback = self.neighbor(first, -1).or_else(|| {
            let last = *ids.last().unwrap();
            let mut n = self.neighbor(last, 1);
            while let Some(x) = n {
                if ids.iter().any(|d| *d == x || self.ws.tree.is_ancestor(*d, x)) {
                    n = self.neighbor(x, 1);
                } else {
                    break;
                }
            }
            n
        });
        // Deleting a bullet takes its whole subtree, rows a search filter hides
        // included: say so, since they weren't on screen.
        let hidden = self.filter_shown().map_or(0, |shown| {
            let tree = &self.ws.tree;
            ids.iter().flat_map(|&id| tree.descendants(tree.resolve(id))).filter(|d| !shown.contains(d)).count()
        });
        for id in &ids {
            self.ws.tree.trash(*id);
        }
        if hidden > 0 {
            self.toast(crate::app::i18n::t!("toast.deleted_hidden", count = hidden));
        }
        self.block = None;
        self.edit = fallback.filter(|f| self.ws.tree.contains(*f)).map(|f| Edit::new(Target::Text(f), self.ws.tree.node(f).text.len()));
        self.commit_structure(before, cx);
    }

    /// Work out where a dragged node would land for a pointer at `pos` over row `id`.
    pub fn update_drop(&mut self, over: NodeId, bounds: Bounds<Pixels>, pos: gpui::Point<Pixels>, dragged: NodeId, cx: &mut Context<Self>) {
        let Some(ix) = self.row_index(over) else { return };
        let below_half = pos.y > bounds.center().y;
        let gap = if below_half { ix + 1 } else { ix };
        let above = gap.checked_sub(1).and_then(|i| self.rows.get(i)).copied();
        let below = self.rows.get(gap).copied();
        let max_d = above.map_or(0, |a| a.depth + 1);
        let min_d = below.map_or(0, |b| b.depth).min(max_d);
        let x = f32::from(pos.x - bounds.left()) - PAD_X;
        let want = (x / INDENT).floor().max(0.0) as u16;
        let depth = want.clamp(min_d, max_d);
        let (parent, index) = match above {
            None => (self.zoom, 0),
            Some(a) if depth == a.depth + 1 => (a.id, 0),
            Some(a) => {
                let mut x = a.id;
                for _ in 0..(a.depth - depth) {
                    x = self.ws.tree.parent(x);
                }
                (self.ws.tree.parent(x), self.ws.tree.index_in_parent(x) + 1)
            }
        };
        // Can't drop a node into itself.
        let invalid = parent == dragged || self.ws.tree.is_ancestor(dragged, parent);
        let target = (!invalid).then(|| match below {
            Some(b) if !below_half || above.is_none() => DropTarget { node: b.id, below: false, depth, parent, index },
            _ => DropTarget { node: above.map_or(over, |a| a.id), below: true, depth, parent, index },
        });
        if self.drop != target {
            self.drop = target;
            cx.notify();
        }
    }

    pub fn perform_drop(&mut self, dragged: NodeId, cx: &mut Context<Self>) {
        let Some(t) = self.drop.take() else { return };
        let before = self.caret();
        if self.ws.tree.move_to(dragged, t.parent, t.index) {
            self.ws.tree.set_collapsed(t.parent, false);
            self.anims.start(dragged, Channel::Enter, anim::ROW_IN);
            self.commit_structure(before, cx);
        }
        cx.notify();
    }
}
