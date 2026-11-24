//! Commit edits to the undo history and persist changes to the outline.

use flowie_core::{Caret, HistoryKind, NodeId, ROOT};
use gpui::{Context, PromptButton, PromptLevel, Window};

use super::{Edit, Outline, Redo, Target, Undo};
use crate::app::i18n::t;

impl Outline {
    pub(crate) fn caret(&self) -> Option<Caret> {
        match self.edit.as_ref()?.target {
            Target::Text(id) | Target::Note(id) => Some(Caret { node: id, offset: self.edit.as_ref()?.caret() }),
            Target::Search | Target::Palette | Target::Field(_) => None,
        }
    }

    pub(crate) fn persist(&mut self) {
        let (up, del) = self.tree.take_dirty();
        if let Some(w) = &self.writer {
            w.write(up, del);
        }
    }

    /// Finish a structural command: journal for undo, persist, re-flatten with animation.
    pub fn commit_structure(&mut self, before: Option<Caret>, cx: &mut Context<Self>) {
        self.commit_with_imports(before, None, cx);
    }

    pub(crate) fn commit_with_imports(
        &mut self,
        before: Option<Caret>,
        imports: Option<(flowie_core::store::ImportState, flowie_core::store::ImportState)>,
        cx: &mut Context<Self>,
    ) {
        self.commit_with_imports_and_trash(before, imports, Vec::new(), cx);
    }

    pub(crate) fn commit_with_imports_and_trash(
        &mut self,
        before: Option<Caret>,
        mut imports: Option<(flowie_core::store::ImportState, flowie_core::store::ImportState)>,
        trash: Vec<(NodeId, String, Vec<flowie_core::Node>)>,
        cx: &mut Context<Self>,
    ) {
        for n in self.tree.journal_removed() {
            self.ghosts.insert(n.id, n);
        }
        let after = self.caret();
        let follow = after != before || before.is_some_and(|c| self.row_on_screen(c.node));
        if let Some((previous, next)) = imports.as_mut() {
            previous.trash_removed = trash.clone();
            next.trash_added = trash.clone();
        }
        self.history.commit_with_imports(&mut self.tree, HistoryKind::Structure, before, after, imports.clone());
        let (upserts, deletes) = self.tree.take_dirty();
        if let Some((_, state)) = imports {
            self.set_import_state(state.clone());
            if let Some(writer) = &self.writer {
                writer.import_batch(upserts, deletes, state);
            }
        } else {
            if let Some(writer) = &self.writer {
                writer.write(upserts, deletes);
            }
            if let Some(writer) = &self.writer {
                for (root, label, nodes) in trash {
                    writer.trash(root, &label, nodes);
                }
            }
        }
        self.refresh(true);
        if follow {
            self.reveal_edit();
        }
        cx.notify();
    }

    pub(super) fn commit_typing(&mut self, id: NodeId, before: Option<Caret>, cx: &mut Context<Self>) {
        self.see_around(id);
        let after = self.caret();
        self.history.commit(&mut self.tree, HistoryKind::Typing(id), before, after);
        self.persist();
        if !self.query.is_empty() {
            self.refresh(false);
        }
        cx.notify();
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.persist();
        self.remember_scroll();
        if let Some(s) = self.page_scrolls.get(&self.zoom) {
            self.set_meta("scroll", &s.encode(self.zoom));
        }
        if let Some(place) = &self.window_place {
            self.set_meta("window", &place.encode());
        }
        self.writer.as_ref().map_or(Ok(()), |writer| writer.flush())
    }

    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.close_prompt_open {
            return false;
        }
        let Err(error) = self.flush() else { return true };
        self.close_prompt_open = true;
        let detail = t!("outline.quit_unsaved_detail", error = error);
        let answer = window.prompt(
            PromptLevel::Warning,
            t!("outline.quit_unsaved"),
            Some(&detail),
            &[PromptButton::cancel(t!("outline.keep_open")), PromptButton::new(t!("outline.quit_without_saving"))],
            cx,
        );
        cx.spawn_in(window, async move |outline, cx| {
            let quit = matches!(answer.await, Ok(1));
            let _ = outline.update_in(cx, |outline, window, _| {
                outline.close_prompt_open = false;
                if quit {
                    window.remove_window();
                }
            });
        })
        .detach();
        false
    }

    pub fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        // Close Trash or What's New before undoing in the outline.
        if self.close_auxiliary_page(true, cx) {
            return;
        }
        let imports = self.history.undo_imports().cloned();
        if let Some(caret) = self.history.undo(&mut self.tree) {
            self.after_history(caret, imports, cx);
        }
    }

    pub fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let imports = self.history.redo_imports().cloned();
        if let Some(caret) = self.history.redo(&mut self.tree) {
            self.after_history(caret, imports, cx);
        }
    }

    fn after_history(&mut self, caret: Option<Caret>, imports: Option<flowie_core::store::ImportState>, cx: &mut Context<Self>) {
        if let Some(state) = imports {
            self.set_import_state(state.clone());
            let (upserts, deletes) = self.tree.take_dirty();
            if let Some(writer) = &self.writer {
                writer.import_batch(upserts, deletes, state);
            }
        } else {
            self.persist();
        }
        if !self.tree.contains(self.zoom) {
            self.zoom = ROOT;
        }
        self.block = None;
        self.edit = caret.filter(|c| self.tree.contains(c.node)).map(|c| {
            let len = self.tree.node(c.node).text.len();
            Edit::new(Target::Text(c.node), c.offset.min(len))
        });
        // Make sure the caret's row is visible.
        if let Some(Target::Text(id)) = self.edit_target() {
            for a in self.tree.ancestors(id) {
                if self.tree.node(a).collapsed && self.tree.is_ancestor(self.zoom, a) {
                    self.tree.set_collapsed(a, false);
                    self.tree.take_journal();
                    self.persist();
                }
            }
        }
        self.refresh(true);
        self.reveal_edit();
        cx.notify();
    }
}
