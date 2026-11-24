//! Edit text in the outline, search, and auxiliary fields.

use std::ops::Range;

use flowie_core::{NodeId, ROOT};
use gpui::{Context, Window};
use unicode_segmentation::UnicodeSegmentation;

use super::*;

impl Outline {
    pub fn edit_text(&self) -> &str {
        match self.edit.as_ref().map(|e| e.target) {
            Some(Target::Text(id)) => self.tree.get(self.tree.source(id)).map(|n| n.text.as_str()).unwrap_or(""),
            Some(Target::Note(id)) => self.tree.get(self.tree.source(id)).map(|n| n.note.as_str()).unwrap_or(""),
            Some(Target::Search) => &self.query,
            Some(Target::Palette) => self.palette.as_ref().map(|p| p.query.as_str()).unwrap_or(""),
            Some(Target::Field(f)) => self.field_text(f),
            None => "",
        }
    }

    /// How the edited text maps to what the editor shows (internal links show their label only).
    pub fn edit_map(&self) -> crate::text::rich::EditMap {
        let code = match self.edit.as_ref().map(|e| e.target) {
            Some(Target::Text(id)) => self.tree.get(id).is_some_and(|n| n.kind == flowie_core::BlockKind::Code),
            Some(Target::Note(_) | Target::Field(Field::WhatsNew(_))) => false,
            _ => true,
        };
        crate::text::rich::EditMap::new(self.edit_text(), code)
    }

    /// Replace `range` of the edited text with `new`, leaving the caret after it.
    pub fn replace_edit_text(&mut self, range: Range<usize>, new: &str, mark: bool, cx: &mut Context<Self>) {
        let Some(target) = self.edit.as_ref().map(|e| e.target) else { return };
        let before = self.caret();
        let old = self.edit_text().to_string();
        let range = range.start.min(old.len())..range.end.min(old.len());
        // Bullet text is single-line: newlines typed or pasted become spaces.
        let new = match target {
            Target::Note(_) => new.to_string(),
            _ => new.replace(['\n', '\r'], " "),
        };
        let text = format!("{}{}{}", &old[..range.start], new, &old[range.end..]);
        let caret = range.start + new.len();
        if let Some(e) = self.edit.as_mut() {
            e.sel = caret..caret;
            e.reversed = false;
            e.marked = (mark && !new.is_empty()).then_some(range.start..caret);
        }
        match target {
            Target::Text(id) => {
                let src = self.tree.source(id);
                self.tree.set_text(src, &text);
                self.commit_typing(src, before, cx);
                if !mark {
                    self.check_inline(&new);
                }
            }
            Target::Palette => {
                if let Some(p) = self.palette.as_mut() {
                    p.query = text;
                    p.selected = 0;
                }
                self.refresh_palette();
                cx.notify();
            }
            Target::Note(id) => {
                let src = self.tree.source(id);
                self.tree.set_note(src, &text);
                self.commit_typing(src, before, cx);
            }
            Target::Search => {
                self.set_query(text);
                cx.notify();
            }
            Target::Field(f) => {
                self.set_field_text(f, text);
                cx.notify();
            }
        }
    }

    pub fn start_edit(&mut self, target: Target, offset: usize, cx: &mut Context<Self>) {
        self.begin_edit(target, offset, true, cx);
    }

    /// Start editing; `reveal` scrolls the target into view (keyboard), a
    /// mouse press on a visible row passes false.
    pub fn begin_edit(&mut self, target: Target, offset: usize, reveal: bool, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            if e.target != target {
                self.history.seal();
            }
        }
        self.block = None;
        self.inline = None;
        let len = match target {
            Target::Text(id) => self.tree.get(self.tree.source(id)).map_or(0, |n| n.text.len()),
            Target::Note(id) => self.tree.get(self.tree.source(id)).map_or(0, |n| n.note.len()),
            Target::Search => self.query.len(),
            Target::Palette => self.palette.as_ref().map_or(0, |p| p.query.len()),
            Target::Field(f) => self.field_text(f).len(),
        };
        self.edit = Some(Edit::new(target, offset.min(len)));
        if reveal {
            self.reveal_edit();
        }
        cx.notify();
    }

    pub fn editing_node(&self) -> Option<NodeId> {
        match self.edit.as_ref()?.target {
            Target::Text(id) | Target::Note(id) => Some(id),
            Target::Search | Target::Palette | Target::Field(_) => None,
        }
    }

    pub(super) fn edit_target(&self) -> Option<Target> {
        self.edit.as_ref().map(|e| e.target)
    }

    pub(super) fn move_caret(&mut self, offset: usize, select: bool, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_mut() else { return };
        if select {
            if e.reversed {
                e.sel.start = offset;
            } else {
                e.sel.end = offset;
            }
            if e.sel.end < e.sel.start {
                e.reversed = !e.reversed;
                e.sel = e.sel.end..e.sel.start;
            }
        } else {
            e.sel = offset..offset;
            e.reversed = false;
        }
        cx.notify();
    }

    // Caret steps treat an internal link as one character (see `EditMap`).

    pub(super) fn prev_boundary(&self, offset: usize) -> usize {
        let at = self.edit_text().grapheme_indices(true).rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0);
        self.edit_map().snap(at, false)
    }

    pub(super) fn next_boundary(&self, offset: usize) -> usize {
        let text = self.edit_text();
        let at = text.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(text.len());
        self.edit_map().snap(at, true)
    }

    fn prev_word(&self, offset: usize) -> usize {
        let text = self.edit_text();
        let at = text.split_word_bound_indices().rev().find(|(i, w)| *i < offset && !w.trim().is_empty()).map(|(i, _)| i).unwrap_or(0);
        self.edit_map().snap(at, false)
    }

    fn next_word(&self, offset: usize) -> usize {
        let text = self.edit_text();
        let at = text
            .split_word_bound_indices()
            .find(|(i, w)| i + w.len() > offset && !w.trim().is_empty())
            .map(|(i, w)| i + w.len())
            .unwrap_or(text.len());
        self.edit_map().snap(at, true)
    }

    pub fn toggle_note(&mut self, _: &ToggleNote, _: &mut Window, cx: &mut Context<Self>) {
        match self.edit_target() {
            Some(Target::Text(id)) if id != self.zoom || self.zoom != ROOT => {
                let len = self.tree.node(id).note.len();
                self.start_edit(Target::Note(id), len, cx);
            }
            Some(Target::Note(id)) => {
                let len = self.tree.node(id).text.len();
                self.start_edit(Target::Text(id), len, cx);
            }
            _ => {}
        }
    }

    pub fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.block.is_some() {
            self.delete_node(&DeleteNode, window, cx);
            return;
        }
        let Some(e) = self.edit.as_ref() else { return };
        if !e.sel.is_empty() {
            let r = e.sel.clone();
            self.replace_edit_text(r, "", false, cx);
            return;
        }
        let caret = e.caret();
        if caret > 0 {
            let start = self.prev_boundary(caret);
            self.replace_edit_text(start..caret, "", false, cx);
            return;
        }
        // At the very start: merge into the row above.
        if let Some(Target::Text(id)) = self.edit_target() {
            if id == self.zoom {
                return;
            }
            let before = self.caret();
            let Some(prev) = self.neighbor(id, -1) else { return };
            let real = self.tree.resolve(id);
            let node = self.tree.node(real);
            // Only a plain row with nothing in it goes away; notes, images and mirrors are preserved.
            if node.mirror == ROOT && node.text.is_empty() && node.note.is_empty() && node.image.is_empty() && !self.tree.has_children(real)
            {
                self.tree.delete(real);
                let len = self.tree.node(self.tree.source(prev)).text.len();
                self.edit = Some(Edit::new(Target::Text(prev), len));
                self.commit_structure(before, cx);
                return;
            }
            if self.tree.has_children(id) && (prev == self.tree.parent(id) || self.tree.has_children(prev)) {
                return;
            }
            if let Some(off) = self.tree.merge_into(id, prev) {
                self.edit = Some(Edit::new(Target::Text(prev), off));
                self.commit_structure(before, cx);
            }
        } else if let Some(Target::Note(id)) = self.edit_target() {
            if self.tree.node(id).note.is_empty() {
                let len = self.tree.node(id).text.len();
                self.start_edit(Target::Text(id), len, cx);
            }
        }
    }

    pub fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.block.is_some() {
            self.delete_node(&DeleteNode, window, cx);
            return;
        }
        let Some(e) = self.edit.as_ref() else { return };
        if !e.sel.is_empty() {
            let r = e.sel.clone();
            self.replace_edit_text(r, "", false, cx);
            return;
        }
        let caret = e.caret();
        if caret < self.edit_text().len() {
            let end = self.next_boundary(caret);
            self.replace_edit_text(caret..end, "", false, cx);
            return;
        }
        if let Some(Target::Text(id)) = self.edit_target() {
            let Some(next) = self.neighbor(id, 1) else { return };
            if self.tree.has_children(next) {
                return;
            }
            let before = self.caret();
            if let Some(off) = self.tree.merge_into(next, id) {
                self.edit = Some(Edit::new(Target::Text(id), off));
                self.commit_structure(before, cx);
            }
        }
    }

    pub fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        let caret = e.caret();
        let start = if e.sel.is_empty() { self.prev_word(caret) } else { e.sel.start };
        let end = if e.sel.is_empty() { caret } else { e.sel.end };
        self.replace_edit_text(start..end, "", false, cx);
    }

    /// ⌃K: delete to the end of the text into the kill buffer; at the end,
    /// join the next bullet as Delete does.
    pub fn kill_line(&mut self, _: &KillLine, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        let (start, len) = (e.sel.start, self.edit_text().len());
        if start >= len {
            return self.delete(&Delete, window, cx);
        }
        self.kill_buffer = self.edit_text()[start..].to_string();
        self.replace_edit_text(start..len, "", false, cx);
    }

    /// ⌃Y: insert the kill buffer at the caret.
    pub fn yank(&mut self, _: &Yank, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        if self.kill_buffer.is_empty() {
            return;
        }
        let r = e.sel.clone();
        let text = self.kill_buffer.clone();
        self.replace_edit_text(r, &text, false, cx);
    }

    pub fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        if !e.sel.is_empty() {
            let s = e.sel.start;
            self.move_caret(s, false, cx);
            return;
        }
        let caret = e.caret();
        if caret == 0 {
            if let Some(Target::Text(id)) = self.edit_target() {
                if let Some(prev) = self.neighbor(id, -1) {
                    let len = self.tree.node(prev).text.len();
                    self.start_edit(Target::Text(prev), len, cx);
                }
            }
            return;
        }
        let to = self.prev_boundary(caret);
        self.move_caret(to, false, cx);
    }

    pub fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        if !e.sel.is_empty() {
            let s = e.sel.end;
            self.move_caret(s, false, cx);
            return;
        }
        let caret = e.caret();
        if caret >= self.edit_text().len() {
            if let Some(Target::Text(id)) = self.edit_target() {
                if let Some(next) = self.neighbor(id, 1) {
                    self.start_edit(Target::Text(next), 0, cx);
                }
            }
            return;
        }
        let to = self.next_boundary(caret);
        self.move_caret(to, false, cx);
    }

    pub fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.prev_word(e.caret());
            self.move_caret(to, false, cx);
        }
    }

    pub fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.next_word(e.caret());
            self.move_caret(to, false, cx);
        }
    }

    pub fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.prev_boundary(e.caret());
            self.move_caret(to, true, cx);
        }
    }

    pub fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.next_boundary(e.caret());
            self.move_caret(to, true, cx);
        }
    }

    pub fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.prev_word(e.caret());
            self.move_caret(to, true, cx);
        }
    }

    pub fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.next_word(e.caret());
            self.move_caret(to, true, cx);
        }
    }

    pub fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(0, false, cx);
    }

    pub fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.edit_text().len();
        self.move_caret(len, false, cx);
    }

    pub fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(0, true, cx);
    }

    pub fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.edit_text().len();
        self.move_caret(len, true, cx);
    }

    /// Visual line movement inside the editor; leaves the row at its edges.
    pub(super) fn vertical(&mut self, dir: isize, select: bool, cx: &mut Context<Self>) -> bool {
        let Some(e) = self.edit.as_ref() else { return false };
        let (Some(layout), Some(bounds)) = (e.layout.clone(), e.bounds) else { return false };
        let map = self.edit_map();
        let caret = map.to_display(e.caret().min(self.edit_text().len()));
        let Some(pos) = layout.position_for_index(caret) else { return false };
        let lh = layout.line_height();
        let target_y = if dir < 0 { pos.y - lh * 0.5 } else { pos.y + lh * 1.5 };
        if target_y < bounds.top() || target_y > bounds.bottom() {
            return false;
        }
        let ix = match layout.index_for_position(gpui::point(pos.x, target_y)) {
            Ok(i) | Err(i) => i,
        };
        self.move_caret(map.to_raw(ix).min(self.edit_text().len()), select, cx);
        true
    }

    /// Wrap the selection in `open`/`close`, or insert the pair around the caret.
    pub(crate) fn wrap(&mut self, open: &str, close: &str, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        if matches!(e.target, Target::Search | Target::Palette | Target::Field(_)) {
            return;
        }
        let r = e.sel.clone();
        let inner = self.edit_text()[r.clone()].to_string();
        // Toggle off when the selection is already wrapped.
        let text = self.edit_text();
        if !inner.is_empty() && r.start >= open.len() && text[..r.start].ends_with(open) && text[r.end..].starts_with(close) {
            let outer = r.start - open.len()..r.end + close.len();
            self.replace_edit_text(outer.clone(), &inner, false, cx);
            if let Some(e) = self.edit.as_mut() {
                e.sel = outer.start..outer.start + inner.len();
            }
            return;
        }
        self.replace_edit_text(r.clone(), &format!("{open}{inner}{close}"), false, cx);
        if let Some(e) = self.edit.as_mut() {
            let s = r.start + open.len();
            e.sel = s..s + inner.len();
        }
        cx.notify();
    }

    pub fn bold(&mut self, _: &Bold, _: &mut Window, cx: &mut Context<Self>) {
        self.wrap("**", "**", cx);
    }

    pub fn italic(&mut self, _: &Italic, _: &mut Window, cx: &mut Context<Self>) {
        self.wrap("*", "*", cx);
    }

    pub fn inline_code(&mut self, _: &InlineCode, _: &mut Window, cx: &mut Context<Self>) {
        self.wrap("`", "`", cx);
    }

    pub fn wrap_math(&mut self, _: &WrapMath, _: &mut Window, cx: &mut Context<Self>) {
        self.wrap("$", "$", cx);
    }

    pub fn insert_link(&mut self, _: &InsertLink, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing_node().is_none() {
            return;
        }
        let Some(e) = self.edit.as_ref() else { return };
        let r = e.sel.clone();
        let label = self.edit_text()[r.clone()].to_string();
        let label = if label.is_empty() { "link".to_string() } else { label };
        let insert = format!("[{label}](https://)");
        self.replace_edit_text(r.clone(), &insert, false, cx);
        if let Some(e) = self.edit.as_mut() {
            let s = r.start + label.len() + 3;
            e.sel = s..s + "https://".len();
        }
        cx.notify();
    }

    pub fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        if self.close_app_menu() {
            cx.notify();
            return;
        }
        if self.close_menu() {
            cx.notify();
            return;
        }
        if self.popover.close() {
            cx.notify();
            return;
        }
        if self.palette.is_some() {
            self.close_palette(cx);
            return;
        }
        if self.inline.take().is_some() {
            cx.notify();
            return;
        }
        if self.calendar.take().is_some() {
            cx.notify();
            return;
        }
        if self.close_auxiliary_page(true, cx) {
            return;
        }
        match self.edit_target() {
            Some(Target::Search) => {
                self.set_query(String::new());
                self.edit = self.rows.first().map(|r| Edit::new(Target::Text(r.id), 0));
            }
            Some(Target::Note(id)) => {
                let len = self.tree.node(id).text.len();
                self.start_edit(Target::Text(id), len, cx);
            }
            Some(Target::Text(id)) => {
                if id != self.zoom {
                    self.history.seal();
                    self.block = Some(BlockSel { anchor: id, head: id });
                    self.edit = None;
                } else if !self.query.is_empty() {
                    self.set_query(String::new());
                }
            }
            Some(Target::Palette | Target::Field(_)) => {}
            None => {
                if self.block.take().is_none() && !self.query.is_empty() {
                    self.set_query(String::new());
                }
            }
        }
        cx.notify();
    }
}
