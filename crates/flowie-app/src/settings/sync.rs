//! Settings → Sync: a self-hosted server's address, this device's token, and how sync is doing.

use flowie_doc::SyncPhase;
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, Window};

use crate::app::i18n::t;
use crate::outline::{Field, Outline, Target};

impl Outline {
    /// A one-line text field for `field`; `secret` shows dots while it isn't being edited.
    fn sync_field(
        &mut self,
        field: Field,
        id: &'static str,
        placeholder: String,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let editing = self.edit.as_ref().is_some_and(|e| e.target == Target::Field(field));
        let text = self.field_text(field).to_string();
        let inner = if editing {
            let (font, _) = self.base_style(window, false);
            let run =
                gpui::TextRun { len: text.len(), font, color: theme.text, background_color: None, underline: None, strikethrough: None };
            let ed = self.editor_element(text, vec![run], theme.caret, theme.selection, cx);
            self.editor_mouse(cx).child(ed).into_any_element()
        } else if text.trim().is_empty() {
            div().text_color(theme.faint).child(placeholder).into_any_element()
        } else if secret {
            div().text_color(theme.text).child("•".repeat(text.chars().count().min(24))).into_any_element()
        } else {
            div().text_color(theme.text).child(text).into_any_element()
        };
        crate::ui::controls::search_field(theme, id, editing)
            .w(px(340.))
            .child(div().flex_1().min_w_0().overflow_hidden().child(inner))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if this.edit.as_ref().map(|e| e.target) != Some(Target::Field(field)) {
                        let len = this.field_text(field).len();
                        this.start_edit(Target::Field(field), len, cx);
                    }
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.settings.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        let theme = self.theme;
        let on = self.config.sync.enabled;
        let toggle = div()
            .id("sync-enabled")
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .child(crate::ui::controls::checkbox(theme, on))
            .child(t!("settings.sync.enabled"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.end_field_edit();
                this.config.sync.enabled = !on;
                this.save_config();
                this.apply_sync_config(cx);
                cx.notify();
            }));
        let server = self.sync_field(Field::SyncServer, "sync-server", "https://sync.example.com".into(), false, window, cx);
        let token = self.sync_field(Field::SyncToken, "sync-token", t!("settings.sync.token.placeholder").into(), true, window, cx);
        let (icon, color, status) = match self.sync.as_ref().map(|s| (s.phase().clone(), s.last_synced)) {
            None if on => ("cloud-off", theme.muted, t!("settings.sync.status.incomplete").to_string()),
            None => ("cloud-off", theme.muted, t!("settings.sync.status.off").to_string()),
            Some((SyncPhase::Syncing, _)) => ("refresh-cw", theme.accent, t!("settings.sync.status.syncing").to_string()),
            Some((SyncPhase::Synced, at)) => (
                "cloud",
                theme.accent,
                t!("settings.sync.status.synced", time = at.map(|at| at.format("%H:%M").to_string()).unwrap_or_default()),
            ),
            Some((SyncPhase::Offline(error), _)) => ("cloud-off", theme.danger, t!("settings.sync.status.offline", error = error)),
        };
        let status = div()
            .flex()
            .items_start()
            .gap_2()
            .text_size(px(12.))
            .child(crate::ui::icons::icon(icon, crate::ui::icons::SM, color))
            .child(div().flex_1().min_w_0().text_color(theme.muted).child(status));
        let field = |label: String, hint: String, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .py_3()
                .border_b_1()
                .border_color(theme.divider)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                        .child(div().text_size(px(12.)).text_color(theme.muted).child(hint)),
                )
                .child(control)
        };
        div()
            .id("sync")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .px_4()
            .pt_3()
            .pb_4()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.end_field_edit() {
                        this.apply_sync_config(cx);
                        cx.notify();
                    }
                }),
            )
            .child(crate::ui::text::page_title(theme, t!("settings.sync.title"), None).pb_1())
            .child(div().text_size(px(12.)).text_color(theme.muted).pb_3().child(t!("settings.sync.intro")))
            .child(div().flex().flex_col().gap_2().pb_3().border_b_1().border_color(theme.divider).child(toggle).child(status))
            .child(field(t!("settings.sync.server.label").into(), t!("settings.sync.server.hint").into(), server))
            .child(field(t!("settings.sync.token.label").into(), t!("settings.sync.token.hint").into(), token))
            .child(div().pt_3().text_size(px(12.)).text_color(theme.muted).child(t!("settings.sync.backup")))
            .into_any_element()
    }
}
