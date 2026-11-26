//! Commit edits through the workspace, then update the view: rows, animation, caret.

use flowie_core::{Caret, NodeId, ROOT};
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

    /// Finish a structural command: journal for undo, persist, re-flatten with animation.
    pub fn commit_structure(&mut self, before: Option<Caret>, cx: &mut Context<Self>) {
        self.commit_with_imports(before, None, cx);
    }

    /// Like `commit_structure`, for a change elsewhere on the page (folding a bullet by its
    /// arrow): the view follows the caret only if the command moved it. Rows appearing or
    /// going above the caret don't pull the page after it.
    pub fn commit_elsewhere(&mut self, before: Option<Caret>, cx: &mut Context<Self>) {
        self.commit_inner(before, None, false, cx);
    }

    pub(crate) fn commit_with_imports(
        &mut self,
        before: Option<Caret>,
        imports: Option<(flowie_core::store::ImportState, flowie_core::store::ImportState)>,
        cx: &mut Context<Self>,
    ) {
        self.commit_inner(before, imports, true, cx);
    }

    /// Commit, refresh, and keep the caret in view: always when the command moved it, and
    /// with `keep_caret`, also when its row was on screen before.
    fn commit_inner(
        &mut self,
        before: Option<Caret>,
        imports: Option<(flowie_core::store::ImportState, flowie_core::store::ImportState)>,
        keep_caret: bool,
        cx: &mut Context<Self>,
    ) {
        for n in self.ws.tree.journal_removed() {
            self.ghosts.insert(n.id, n);
        }
        let after = self.caret();
        let follow = after != before || (keep_caret && before.is_some_and(|c| self.row_on_screen(c.node)));
        self.ws.commit_with_imports(before, after, imports);
        self.refresh(true);
        if follow {
            self.follow_edit();
        }
        cx.notify();
    }

    pub(super) fn commit_typing(&mut self, id: NodeId, before: Option<Caret>, cx: &mut Context<Self>) {
        self.see_around(id);
        let after = self.caret();
        self.ws.commit_typing(id, before, after);
        if !self.query.is_empty() {
            self.refresh(false);
        }
        cx.notify();
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.ws.persist();
        self.remember_scroll();
        if let Some(s) = self.page_scrolls.get(&self.zoom) {
            self.set_meta("scroll", &s.encode(self.zoom));
        }
        if let Some(place) = &self.window_place {
            self.set_meta("window", &place.encode());
        }
        self.ws.flush()
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
        if let Some(caret) = self.ws.undo() {
            self.after_history(caret, cx);
        }
    }

    pub fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(caret) = self.ws.redo() {
            self.after_history(caret, cx);
        }
    }

    /// Update the view after the workspace has undone or redone a step.
    fn after_history(&mut self, caret: Option<Caret>, cx: &mut Context<Self>) {
        if !self.ws.tree.contains(self.zoom) {
            self.zoom = ROOT;
        }
        self.block = None;
        self.edit = caret.filter(|c| self.ws.tree.contains(c.node)).map(|c| {
            let len = self.ws.tree.node(c.node).text.len();
            Edit::new(Target::Text(c.node), c.offset.min(len))
        });
        // Make sure the caret's row is visible.
        if let Some(Target::Text(id)) = self.edit_target() {
            for a in self.ws.tree.ancestors(id) {
                if self.ws.tree.node(a).collapsed && self.ws.tree.is_ancestor(self.zoom, a) {
                    self.ws.tree.set_collapsed(a, false);
                    self.ws.tree.take_journal();
                    self.ws.persist();
                }
            }
        }
        self.refresh(true);
        self.follow_edit();
        cx.notify();
    }
}
