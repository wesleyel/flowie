//! Commit edits through the workspace, then update the view: rows, animation, caret.

use flowie_core::{Caret, NodeId, ROOT};
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, PromptButton, PromptLevel, Window};

use super::{Edit, Outline, Redo, Target, Undo};
use crate::app::i18n::t;
use crate::app::platform::{Os, OS};
use crate::settings::{decide_close, CloseAction, CloseDecision};

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
        self.pump_sync(cx);
        cx.notify();
    }

    pub(super) fn commit_typing(&mut self, id: NodeId, before: Option<Caret>, cx: &mut Context<Self>) {
        self.see_around(id);
        let after = self.caret();
        self.ws.commit_typing(id, before, after);
        if !self.query.is_empty() {
            self.refresh(false);
        }
        self.pump_sync(cx);
        cx.notify();
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.ws.persist();
        self.remember_scroll();
        self.save_places();
        self.ws.flush()
    }

    /// Save transient UI positions and persist pending edits before hiding the window to the tray.
    pub fn prepare_hide_to_tray(&mut self) {
        self.window_state.take_close_dialog();
        self.menu = None;
        self.app_menu = None;
        self.popover.close();
        self.remember_scroll();
        self.save_places();
        self.ws.persist();
    }

    /// Note this page's scroll position and the window's place in meta.
    fn save_places(&self) {
        if let Some(s) = self.page_scrolls.get(&self.zoom) {
            self.set_meta("scroll", &s.encode(self.zoom));
        }
        if let Some(place) = self.window_state.observed_place() {
            self.set_meta("window", &place.encode());
        }
    }

    /// Handle a window close button (× / Alt+F4). Returns `true` when the window
    /// should close right now (either hiding to tray or quitting).
    pub fn handle_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.close_prompt_open || self.window_state.close_dialog_open() {
            return false;
        }
        match decide_close(OS, self.config.close_action, self.window_state.tray_available()) {
            CloseDecision::HideToTray => {
                self.prepare_hide_to_tray();
                true
            }
            CloseDecision::Ask => {
                self.menu = None;
                self.app_menu = None;
                self.window_state.ask_how_to_close();
                cx.notify();
                false
            }
            CloseDecision::Quit => {
                if self.request_quit(window, cx) {
                    if OS != Os::Mac {
                        cx.quit();
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Flush changes before an explicit quit (or macOS window close); prompts if saving fails.
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
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
            let _ = outline.update_in(cx, |outline, window, cx| {
                outline.close_prompt_open = false;
                if quit {
                    if OS == Os::Mac {
                        window.remove_window();
                    } else {
                        cx.quit();
                    }
                }
            });
        })
        .detach();
        false
    }

    pub fn dismiss_close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_state.take_close_dialog().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub fn close_dialog_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" && !ev.keystroke.modifiers.modified() {
            self.dismiss_close_dialog(window, cx);
        }
        cx.stop_propagation();
    }

    fn finish_close_dialog(&mut self, action: CloseAction, window: &mut Window, cx: &mut Context<Self>) {
        let remember = self.window_state.take_close_dialog().is_some_and(|d| d.remember);
        if remember {
            self.config.close_action = action;
            self.config.save(&self.config_path);
        }
        match action {
            CloseAction::Tray => {
                self.prepare_hide_to_tray();
                window.remove_window();
            }
            CloseAction::Quit => {
                if self.request_quit(window, cx) {
                    cx.quit();
                } else {
                    cx.notify();
                }
            }
            CloseAction::Ask => {}
        }
    }

    pub(crate) fn render_close_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.window_state.close_dialog()?;
        let theme = self.theme;
        let remember = dialog.remember;
        let check = div()
            .id("close-dialog-remember")
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(theme.muted)
            .child(crate::ui::controls::checkbox(theme, remember))
            .child(t!("close_dialog.remember"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.window_state.toggle_remember();
                cx.notify();
            }));
        let sheet = crate::ui::surface::sheet(theme, "close-dialog")
            .w(px(400.))
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("close_dialog.title")))
            .child(div().text_size(px(13.)).text_color(theme.muted).line_height(px(19.)).child(t!("close_dialog.detail")))
            .child(div().pt_1().child(check))
            .child(
                div()
                    .pt_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        crate::ui::button::button(theme, "close-dialog-cancel", crate::ui::button::Variant::Secondary)
                            .child(t!("close_dialog.cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.dismiss_close_dialog(window, cx))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                crate::ui::button::button(theme, "close-dialog-quit", crate::ui::button::Variant::Secondary)
                                    .child(t!("close_dialog.quit"))
                                    .on_click(cx.listener(|this, _, window, cx| this.finish_close_dialog(CloseAction::Quit, window, cx))),
                            )
                            .child(
                                crate::ui::button::button(theme, "close-dialog-tray", crate::ui::button::Variant::Primary)
                                    .child(t!("close_dialog.tray"))
                                    .on_click(cx.listener(|this, _, window, cx| this.finish_close_dialog(CloseAction::Tray, window, cx))),
                            ),
                    ),
            );
        Some(
            crate::ui::surface::scrim(theme)
                .items_center()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.dismiss_close_dialog(window, cx)))
                .child(sheet)
                .into_any_element(),
        )
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
