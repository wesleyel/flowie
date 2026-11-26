//! Keep the flattened rows, list positions, and row animations in sync.

use flowie_core::{merge_leaving, DisplayRow, NodeId, Row, ROOT};
use gpui::px;

use super::{Outline, Target, HEAD};

/// Above this many rows appearing or disappearing at once, they do without the
/// grow and shrink animation.
const MAX_ANIMATED_ROWS: usize = 150;

/// Above this many new rows at once, the list is reset rather than spliced (see
/// `Outline::row_height_hint`).
const BULK_ROWS: usize = 200;

/// Maximum rows sampled when estimating GPUI's uniform fallback height.
const HINT_SAMPLE: usize = 64;

use crate::ui::anim::{self, Channel};

/// Evenly spaced indices across `0..len`, capped at `max_samples`.
fn sample_indices(len: usize, max_samples: usize) -> impl Iterator<Item = usize> {
    let step = len.div_ceil(max_samples.max(1)).max(1);
    (0..len).step_by(step)
}

impl Outline {
    /// Recompute visible rows. With `animate`, rows that appear grow in and
    /// rows that disappear shrink out.
    pub fn refresh(&mut self, animate: bool) {
        self.refresh_with(animate, false);
    }

    /// Recompute visible rows and reset GPUI's list measurements (page zoom, search).
    pub(crate) fn refresh_reset(&mut self) {
        self.refresh_with(false, true);
    }

    fn refresh_with(&mut self, animate: bool, mut reset_list: bool) {
        // Never draw a page that no longer exists (deleted while zoomed into it).
        if self.zoom != ROOT && !self.ws.tree.contains(self.zoom) {
            self.zoom = ROOT;
            reset_list = true;
        }
        let opts = self.view_options();
        self.highlight_terms = opts.query.as_ref().map(|q| q.highlight_terms()).unwrap_or_default();
        let (new_rows, virtuals) = flowie_core::flatten_mapped(&self.ws.tree, self.zoom, &opts);
        self.ws.tree.add_virtuals(virtuals);
        // Rows growing in or shrinking out are each drawn every frame until done, so
        // folding a big subtree (or expand all) would animate thousands at once and
        // stutter: past a screenful or so, rows come and go at once instead.
        let shown = self.display.iter().filter(|d| !d.leaving).count();
        let animate = animate && new_rows.len().abs_diff(shown) <= MAX_ANIMATED_ROWS;
        let old_display = std::mem::take(&mut self.display);
        let new_display: Vec<DisplayRow> = if animate {
            let old: Vec<Row> = old_display
                .iter()
                .filter(|d| !d.leaving || !self.anims.is_leaving_done(d.row.id))
                .filter(|d| self.ws.tree.contains(d.row.id) || self.ghosts.contains_key(&self.ws.tree.resolve(d.row.id)) || d.leaving)
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
                .filter(|d| !d.leaving || self.ws.tree.contains(d.row.id) || self.ghosts.contains_key(&self.ws.tree.resolve(d.row.id)))
                .collect()
        } else {
            new_rows.iter().map(|&row| DisplayRow { row, leaving: false }).collect()
        };
        self.row_pos = new_rows.iter().enumerate().map(|(ix, r)| (r.id, ix)).collect();
        self.rows = new_rows;
        self.set_display(&old_display, new_display, reset_list);
    }

    /// Install a new display list: index it, then splice or reset the list view.
    fn set_display(&mut self, old: &[DisplayRow], new: Vec<DisplayRow>, reset_list: bool) {
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
        if reset_list {
            self.list.reset_with_uniform_height(new.len() + HEAD + 1, self.row_height_hint(&new));
        } else {
            self.splice_list(old, &new);
        }
        self.display = new;
        self.heights.invalidate();
    }

    /// Tell the list which items changed, preserving scroll position.
    fn splice_list(&self, old: &[DisplayRow], new: &[DisplayRow]) {
        let scroll_top = self.list.logical_scroll_top();
        // `top` is the display row at the top; None while the page header is.
        let top = scroll_top.item_ix.checked_sub(HEAD);
        let anchor = top.and_then(|ix| old.get(ix)).map(|d| d.row.id);
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
        let old_end = old.len() - suffix;
        let new_count = new.len() - suffix - prefix;
        // Besides the rows, the list has the page header first and the
        // click-to-add footer last.
        if self.list.item_count() != old.len() + HEAD + 1 {
            self.list.reset_with_uniform_height(new.len() + HEAD + 1, self.row_height_hint(new));
            return;
        }
        if old_end == prefix && new_count == 0 {
            return; // nothing changed
        }
        if new_count > BULK_ROWS {
            // Many new rows (expand all, a large paste): start over with a height
            // estimate for every row, so the scrollbar is about right from the start.
            self.list.reset_with_uniform_height(new.len() + HEAD + 1, self.row_height_hint(new));
        } else {
            self.list.splice(prefix + HEAD..old_end + HEAD, new_count);
        }
        // With the header at the top, the rows below it can change freely.
        let Some(top) = top else { return };
        // Keep the row that was at the top where it was, by identity.
        if let Some(ix) = anchor.and_then(|id| self.display_pos.get(&id).copied()) {
            self.list.scroll_to(gpui::ListOffset { item_ix: ix + HEAD, offset_in_item: scroll_top.offset_in_item });
        } else if !old.is_empty() && top == old.len() {
            self.list.scroll_to(gpui::ListOffset { item_ix: new.len() + HEAD, offset_in_item: scroll_top.offset_in_item });
        } else if let Some(row) = old.get(top) {
            let positions = &self.display_pos;
            let ancestor = old[..top].iter().rev().filter(|d| d.row.depth < row.row.depth).find_map(|d| positions.get(&d.row.id).copied());
            let neighbor = || old[top..].iter().chain(old[..top].iter().rev()).find_map(|d| positions.get(&d.row.id).copied());
            let item_ix = ancestor.or_else(neighbor).map_or(0, |ix| ix + HEAD);
            self.list.scroll_to(gpui::ListOffset { item_ix, offset_in_item: px(0.) });
        }
    }

    /// Uniform fallback height for GPUI's `ListState` before items are measured
    /// (so wheel scrolling and `scroll_to` have a sensible scale before the real
    /// `HeightMap` takes over). Reuses cached heights when available and samples
    /// at most `HINT_SAMPLE` rows so bulk refreshes stay O(1).
    pub fn row_height_hint(&self, rows: &[DisplayRow]) -> gpui::Pixels {
        if rows.is_empty() {
            return px(super::LINE_H + 6.0);
        }
        let tree = &self.ws.tree;
        // The text column's width: the list's (or a typical window's before the
        // first layout), at most the content width setting.
        let list_w = f32::from(self.list.viewport_bounds().size.width);
        let list_w = if list_w > 0. { list_w } else { 900. };
        let column = self.config.content_width.max_px().map_or(list_w, |max| list_w.min(max));
        let (mut total, mut count) = (0.0f32, 0usize);
        for ix in sample_indices(rows.len(), HINT_SAMPLE) {
            let d = rows[ix];
            if d.leaving {
                continue;
            }
            if let Some(h) = self.heights.cached_height(d.row.id, d.row.depth) {
                total += h;
                count += 1;
                continue;
            }
            let Some(node) = tree.get(d.row.id) else { continue };
            let src = tree.node(tree.source(d.row.id));
            let (font_size, line_h, top_pad) = super::row_metrics(node.kind);
            // About half an em per character of the system font.
            let text_w = column - 2. * super::PAD_X - d.row.depth as f32 * super::INDENT - 24.;
            let per_line = (text_w / (font_size * 0.5)).max(20.);
            let lines = (src.text.chars().count() as f32 / per_line).ceil().max(1.);
            let note = if src.note.is_empty() { 0.0 } else { 20.0 };
            let image = self.row_image(d.row.id).map_or(0.0, |(_, info)| Self::image_box(info).1 + 6.0);
            total += line_h * lines + top_pad + 3.0 + note + image;
            count += 1;
        }
        px(if count == 0 { super::LINE_H + 6.0 } else { total / count as f32 })
    }

    /// Called each frame while animating: drop rows that finished leaving.
    pub fn tick(&mut self) {
        if self.display.iter().any(|d| d.leaving && self.anims.is_leaving_done(d.row.id)) {
            let old = std::mem::take(&mut self.display);
            let ids: Vec<NodeId> = old.iter().filter(|d| d.leaving && self.anims.is_leaving_done(d.row.id)).map(|d| d.row.id).collect();
            let new: Vec<DisplayRow> = old.iter().copied().filter(|d| !(d.leaving && self.anims.is_leaving_done(d.row.id))).collect();
            for id in ids {
                self.anims.clear_leave(id);
                let real = self.ws.tree.resolve(id);
                if !self.ws.tree.contains(real) {
                    self.ghosts.remove(&real);
                }
            }
            self.set_display(&old, new, false);
        }
        self.anims.gc();
        if !self.ghosts.is_empty() && !self.display.iter().any(|d| d.leaving) {
            self.ghosts.clear();
        }
        if self.zoom_tween.is_some_and(|t| t.done()) {
            self.zoom_tween = None;
        }
        if self.search_tween.is_some_and(|(t, _)| t.done()) {
            self.search_tween = None;
        }
    }

    pub fn is_animating(&self) -> bool {
        self.anims.any_running()
            || self.zoom_tween.is_some()
            || self.search_tween.is_some()
            || self.display.iter().any(|d| d.leaving)
            || self.wheel.running()
            || self.follow_edit > 0
    }

    pub fn row_index(&self, id: NodeId) -> Option<usize> {
        self.row_pos.get(&id).copied()
    }

    pub fn display_index(&self, id: NodeId) -> Option<usize> {
        self.display_pos.get(&id).copied().filter(|&ix| !self.display[ix].leaving)
    }

    /// Whether the row of `id` is (at least partly) in the viewport.
    /// The list item showing `id`: its row, or the page header for the zoomed node (its title).
    fn list_item(&self, id: NodeId) -> Option<usize> {
        if id == self.zoom && id != ROOT {
            return Some(0);
        }
        self.display_index(id).map(|ix| ix + HEAD)
    }

    pub fn row_on_screen(&self, id: NodeId) -> bool {
        let Some(ix) = self.list_item(id) else { return false };
        let view = self.list.viewport_bounds();
        self.list.bounds_for_item(ix).is_some_and(|b| b.top() < view.bottom() && b.bottom() > view.top())
    }

    pub(crate) fn reveal(&self, id: NodeId) {
        if let Some(ix) = self.list_item(id) {
            self.list.scroll_to_reveal_item(ix);
        }
    }

    pub fn reveal_edit(&self) {
        if let Some(Target::Text(id) | Target::Note(id)) = self.edit.as_ref().map(|e| e.target) {
            self.reveal(id);
        }
    }

    /// Reveal the edited row now and over the next frames. A row that was
    /// just added hasn't been measured, so the list counts it as 0 px tall
    /// and the first reveal stops short of it (with the row still growing in).
    pub fn follow_edit(&mut self) {
        self.reveal_edit();
        self.follow_edit = 30;
    }

    /// One frame of [`Outline::follow_edit`]: stop once the edited row is
    /// fully in view at its full height, or after a few frames.
    pub fn follow_edit_frame(&mut self) {
        let Some(Target::Text(id) | Target::Note(id)) = self.edit.as_ref().map(|e| e.target) else {
            self.follow_edit = 0;
            return;
        };
        let growing = self.anims.t(id, Channel::Enter).is_some_and(|t| t < 1.0);
        if !growing && self.row_fully_on_screen(id) {
            self.follow_edit = 0;
            return;
        }
        self.reveal(id);
        self.follow_edit = self.follow_edit.saturating_sub(1);
    }

    fn row_fully_on_screen(&self, id: NodeId) -> bool {
        let Some(ix) = self.list_item(id) else { return true };
        let view = self.list.viewport_bounds();
        self.list.bounds_for_item(ix).is_some_and(|b| b.top() >= view.top() && b.bottom() <= view.bottom())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_indices_is_bounded_and_spread() {
        assert_eq!(sample_indices(0, HINT_SAMPLE).collect::<Vec<_>>(), Vec::<usize>::new());
        assert_eq!(sample_indices(10, HINT_SAMPLE).collect::<Vec<_>>(), (0..10).collect::<Vec<_>>());
        assert_eq!(sample_indices(HINT_SAMPLE, HINT_SAMPLE).collect::<Vec<_>>(), (0..HINT_SAMPLE).collect::<Vec<_>>());

        let big: Vec<usize> = sample_indices(100_000, HINT_SAMPLE).collect();
        assert!(big.len() <= HINT_SAMPLE);
        assert_eq!(big.first(), Some(&0));
        assert!(big.last().is_some_and(|&last| last > 90_000));
    }
}
