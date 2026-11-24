//! Keep the flattened rows, list positions, and row animations in sync.

use flowie_core::{merge_leaving, DisplayRow, NodeId, Row};
use gpui::px;

use super::{Outline, Target};
use crate::ui::anim::{self, Channel};

impl Outline {
    /// Recompute visible rows. With `animate`, rows that appear grow in and
    /// rows that disappear shrink out.
    pub fn refresh(&mut self, animate: bool) {
        let opts = self.view_options();
        self.highlight_terms = opts.query.as_ref().map(|q| q.highlight_terms()).unwrap_or_default();
        let (new_rows, virtuals) = flowie_core::flatten_mapped(&self.tree, self.zoom, &opts);
        self.tree.add_virtuals(virtuals);
        let old_display = std::mem::take(&mut self.display);
        let new_display: Vec<DisplayRow> = if animate {
            let old: Vec<Row> = old_display
                .iter()
                .filter(|d| !d.leaving || !self.anims.is_leaving_done(d.row.id))
                .filter(|d| self.tree.contains(d.row.id) || self.ghosts.contains_key(&self.tree.resolve(d.row.id)) || d.leaving)
                .map(|d| d.row)
                .collect();
            let old_ids: std::collections::HashSet<NodeId> = old_display.iter().filter(|d| !d.leaving).map(|d| d.row.id).collect();
            let merged = merge_leaving(&old, &new_rows);
            for d in &merged {
                if d.leaving {
                    if self.anims.t(d.row.id, Channel::Leave).is_none() {
                        self.anims.start(d.row.id, Channel::Leave, anim::ROW_OUT);
                    }
                } else {
                    self.anims.clear_leave(d.row.id);
                    if !old_ids.contains(&d.row.id) {
                        self.anims.start(d.row.id, Channel::Enter, anim::ROW_IN);
                    }
                }
            }
            // Leaving rows of deleted nodes render from their snapshot.
            merged
                .into_iter()
                .filter(|d| !d.leaving || self.tree.contains(d.row.id) || self.ghosts.contains_key(&self.tree.resolve(d.row.id)))
                .collect()
        } else {
            new_rows.iter().map(|&row| DisplayRow { row, leaving: false }).collect()
        };
        self.row_pos = new_rows.iter().enumerate().map(|(ix, r)| (r.id, ix)).collect();
        self.rows = new_rows;
        self.set_display(&old_display, new_display);
    }

    /// Install a new display list: index it, then splice the list view.
    fn set_display(&mut self, old: &[DisplayRow], new: Vec<DisplayRow>) {
        let mut pos = std::collections::HashMap::with_capacity(new.len());
        for (ix, d) in new.iter().enumerate() {
            // A live row wins over a leaving copy of the same id.
            pos.entry(d.row.id)
                .and_modify(|v| {
                    if !d.leaving {
                        *v = ix
                    }
                })
                .or_insert(ix);
        }
        self.display_pos = pos;
        self.splice_list(old, &new);
        self.display = new;
    }

    /// Tell the list which items changed, preserving scroll position.
    fn splice_list(&self, old: &[DisplayRow], new: &[DisplayRow]) {
        let scroll_top = self.list.logical_scroll_top();
        let anchor = old.get(scroll_top.item_ix).map(|d| d.row.id);
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
        let old_end = old.len() - suffix;
        let new_count = new.len() - suffix - prefix;
        // The list has one extra item at the end: the click-to-add footer.
        if self.list.item_count() != old.len() + 1 {
            self.list.reset(new.len() + 1);
            return;
        }
        if old_end == prefix && new_count == 0 {
            return; // nothing changed
        }
        self.list.splice(prefix..old_end, new_count);
        // Keep the row that was at the top where it was, by identity.
        if let Some(item_ix) = anchor.and_then(|id| self.display_pos.get(&id).copied()) {
            self.list.scroll_to(gpui::ListOffset { item_ix, offset_in_item: scroll_top.offset_in_item });
        } else if !old.is_empty() && scroll_top.item_ix == old.len() {
            self.list.scroll_to(gpui::ListOffset { item_ix: new.len(), offset_in_item: scroll_top.offset_in_item });
        } else if let Some(row) = old.get(scroll_top.item_ix) {
            let positions = &self.display_pos;
            let ancestor = old[..scroll_top.item_ix]
                .iter()
                .rev()
                .filter(|d| d.row.depth < row.row.depth)
                .find_map(|d| positions.get(&d.row.id).copied());
            let neighbor = || {
                old[scroll_top.item_ix..]
                    .iter()
                    .chain(old[..scroll_top.item_ix].iter().rev())
                    .find_map(|d| positions.get(&d.row.id).copied())
            };
            self.list.scroll_to(gpui::ListOffset { item_ix: ancestor.or_else(neighbor).unwrap_or(0), offset_in_item: px(0.) });
        }
    }

    /// Called each frame while animating: drop rows that finished leaving.
    pub fn tick(&mut self) {
        if self.display.iter().any(|d| d.leaving && self.anims.is_leaving_done(d.row.id)) {
            let old = std::mem::take(&mut self.display);
            let ids: Vec<NodeId> = old.iter().filter(|d| d.leaving && self.anims.is_leaving_done(d.row.id)).map(|d| d.row.id).collect();
            let new: Vec<DisplayRow> = old.iter().copied().filter(|d| !(d.leaving && self.anims.is_leaving_done(d.row.id))).collect();
            for id in ids {
                self.anims.clear_leave(id);
                let real = self.tree.resolve(id);
                if !self.tree.contains(real) {
                    self.ghosts.remove(&real);
                }
            }
            self.set_display(&old, new);
        }
        self.anims.gc();
        if !self.ghosts.is_empty() && !self.display.iter().any(|d| d.leaving) {
            self.ghosts.clear();
        }
        if self.zoom_tween.is_some_and(|t| t.done()) {
            self.zoom_tween = None;
        }
    }

    pub fn is_animating(&self) -> bool {
        self.anims.any_running() || self.zoom_tween.is_some() || self.display.iter().any(|d| d.leaving) || self.wheel.running()
    }

    pub fn row_index(&self, id: NodeId) -> Option<usize> {
        self.row_pos.get(&id).copied()
    }

    pub fn display_index(&self, id: NodeId) -> Option<usize> {
        self.display_pos.get(&id).copied().filter(|&ix| !self.display[ix].leaving)
    }

    /// Whether the row of `id` is (at least partly) in the viewport.
    pub fn row_on_screen(&self, id: NodeId) -> bool {
        let Some(ix) = self.display_index(id) else { return false };
        let view = self.list.viewport_bounds();
        self.list.bounds_for_item(ix).is_some_and(|b| b.top() < view.bottom() && b.bottom() > view.top())
    }

    pub(crate) fn reveal(&self, id: NodeId) {
        if let Some(ix) = self.display_index(id) {
            self.list.scroll_to_reveal_item(ix);
        }
    }

    pub fn reveal_edit(&self) {
        if let Some(Target::Text(id) | Target::Note(id)) = self.edit.as_ref().map(|e| e.target) {
            if let Some(ix) = self.display_index(id) {
                self.list.scroll_to_reveal_item(ix);
            }
        }
    }
}
