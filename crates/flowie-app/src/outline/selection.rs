//! Move between rows and select whole blocks.

use flowie_core::{NodeId, ROOT};
use gpui::{Context, Window};

use super::*;

impl Outline {
    /// The row above/below the given node in display order (None at the edges).
    pub(super) fn neighbor(&self, id: NodeId, delta: isize) -> Option<NodeId> {
        let ix = self.row_index(id)? as isize + delta;
        (ix >= 0).then(|| self.rows.get(ix as usize).map(|r| r.id)).flatten()
    }

    pub(super) fn char_col(&self) -> usize {
        let Some(e) = &self.edit else { return 0 };
        self.edit_map().column(self.edit_text(), e.caret())
    }

    fn offset_for_col(&self, row: NodeId, col: usize) -> usize {
        let text = &self.ws.tree.node(self.ws.tree.source(row)).text;
        crate::text::rich::EditMap::new(text, self.ws.tree.node(row).kind == flowie_core::BlockKind::Code).offset_for_column(text, col)
    }

    pub fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.edit_text().len();
        if let Some(e) = self.edit.as_mut() {
            if e.sel == (0..len) && matches!(e.target, Target::Text(_)) {
                // Second ⌘A selects every visible bullet.
                if let (Some(first), Some(last)) = (self.rows.first(), self.rows.last()) {
                    self.block = Some(BlockSel { anchor: first.id, head: last.id });
                    self.edit = None;
                }
            } else {
                e.sel = 0..len;
                e.reversed = false;
            }
            cx.notify();
        } else if let (Some(first), Some(last)) = (self.rows.first(), self.rows.last()) {
            self.block = Some(BlockSel { anchor: first.id, head: last.id });
            cx.notify();
        }
    }

    pub fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_move(-1, cx) {
            return;
        }
        if let Some(b) = self.block {
            if let Some(prev) = self.neighbor(b.head, -1) {
                self.block = Some(BlockSel { anchor: prev, head: prev });
                self.reveal(prev);
                cx.notify();
            }
            return;
        }
        if self.vertical(-1, false, cx) {
            return;
        }
        let col = self.char_col();
        match self.edit_target() {
            Some(Target::Text(id)) => {
                let prev = self.neighbor(id, -1).or_else(|| {
                    // From the first row, go up into the zoomed title.
                    (self.zoom != ROOT && self.rows.first().map(|r| r.id) == Some(id)).then_some(self.zoom)
                });
                if let Some(prev) = prev {
                    let off = self.offset_for_col(prev, col);
                    self.start_edit(Target::Text(prev), off, cx);
                }
            }
            Some(Target::Note(id)) => self.start_edit(Target::Text(id), 0, cx),
            None => self.enter_from_idle(cx),
            _ => {}
        }
    }

    pub fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_move(1, cx) {
            return;
        }
        if let Some(b) = self.block {
            if let Some(next) = self.neighbor(b.head, 1) {
                self.block = Some(BlockSel { anchor: next, head: next });
                self.reveal(next);
                cx.notify();
            }
            return;
        }
        if self.vertical(1, false, cx) {
            return;
        }
        let col = self.char_col();
        match self.edit_target() {
            Some(Target::Text(id)) => {
                let next = if id == self.zoom { self.rows.first().map(|r| r.id) } else { self.neighbor(id, 1) };
                if let Some(next) = next {
                    let off = self.offset_for_col(next, col);
                    self.start_edit(Target::Text(next), off, cx);
                }
            }
            Some(Target::Note(id)) => {
                if let Some(next) = self.neighbor(id, 1) {
                    self.start_edit(Target::Text(next), 0, cx);
                }
            }
            Some(Target::Search) => {
                if let Some(first) = self.rows.first().map(|r| r.id) {
                    self.start_edit(Target::Text(first), 0, cx);
                }
            }
            Some(Target::Palette | Target::Field(_)) => {}
            None => self.enter_from_idle(cx),
        }
    }

    pub fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_block(-1, cx);
    }

    pub fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_block(1, cx);
    }

    fn extend_block(&mut self, dir: isize, cx: &mut Context<Self>) {
        if let Some(b) = self.block {
            if let Some(n) = self.neighbor(b.head, dir) {
                self.block = Some(BlockSel { anchor: b.anchor, head: n });
                self.reveal(n);
                cx.notify();
            }
            return;
        }
        if self.vertical(dir, true, cx) {
            return;
        }
        let Some(e) = self.edit.as_ref() else { return };
        let Target::Text(id) = e.target else { return };
        let len = self.edit_text().len();
        let at_edge = if dir < 0 { e.sel.start == 0 && e.sel.end == e.caret() && e.caret() == 0 } else { e.caret() == len };
        if !at_edge && (dir >= 0 || e.caret() != 0) {
            let to = if dir < 0 { 0 } else { len };
            self.move_caret(to, true, cx);
            return;
        }
        if id == self.zoom {
            return;
        }
        let head = self.neighbor(id, dir).unwrap_or(id);
        self.block = Some(BlockSel { anchor: id, head });
        self.edit = None;
        cx.notify();
    }

    /// Whether `id` is inside the block selection (O(1), for rendering).
    pub fn is_block_selected(&self, id: NodeId) -> bool {
        let Some(b) = self.block else { return false };
        let (Some(a), Some(h), Some(i)) = (self.row_index(b.anchor), self.row_index(b.head), self.row_index(id)) else { return false };
        a.min(h) <= i && i <= a.max(h)
    }

    /// Selected row ids in display order.
    pub fn block_ids(&self) -> Vec<NodeId> {
        let Some(b) = self.block else { return Vec::new() };
        let (Some(a), Some(h)) = (self.row_index(b.anchor), self.row_index(b.head)) else { return Vec::new() };
        let (lo, hi) = (a.min(h), a.max(h));
        self.rows[lo..=hi].iter().map(|r| r.id).collect()
    }

    /// Selected rows without those whose ancestor is also selected.
    pub fn block_roots(&self) -> Option<Vec<NodeId>> {
        self.block?;
        let ids = self.block_ids();
        let set: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
        Some(ids.into_iter().filter(|id| !self.ws.tree.ancestors(*id).iter().any(|a| set.contains(a))).collect())
    }

    /// Shift-click: select rows from the current row to `id`.
    pub fn shift_click(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let anchor = self.block.map(|b| b.anchor).or_else(|| self.editing_node()).unwrap_or(id);
        if anchor == self.zoom {
            return;
        }
        self.block = Some(BlockSel { anchor, head: id });
        self.edit = None;
        cx.notify();
    }

    pub(crate) fn targets(&self) -> Vec<NodeId> {
        if self.block.is_some() {
            let ids = self.block_ids();
            let set: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
            return ids.into_iter().filter(|id| !self.ws.tree.ancestors(*id).iter().any(|a| set.contains(a))).collect();
        }
        self.editing_node().filter(|id| *id != self.zoom).into_iter().collect()
    }
}
