//! Settings → Sync: off, iCloud (macOS) or a self-hosted server (its address and this
//! device's token), and how sync is doing.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, Window};

use crate::app::i18n::t;
use crate::outline::sync::SyncTone;
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
        // The editor wraps (a token is 64 characters), so the field grows with
        // it instead of keeping the search field's fixed height.
        crate::ui::controls::search_field(theme, id, editing)
            .w(px(340.))
            .h_auto()
            .min_h(px(28.))
            .py(px(4.))
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
        let icloud = on && self.config.sync.icloud;
        // Off, iCloud (only the Mac has it) or a server of one's own.
        let mut modes = vec![(false, false, "sync-off", t!("settings.sync.mode.off"))];
        if cfg!(target_os = "macos") {
            modes.push((true, true, "sync-icloud", t!("settings.sync.mode.icloud")));
        }
        modes.push((true, false, "sync-server", t!("settings.sync.mode.server")));
        let current = (on, icloud);
        let toggle = crate::ui::segmented::track(theme).children(modes.into_iter().map(|(enabled, cloud, id, label)| {
            crate::ui::segmented::segment(theme, id, (enabled, cloud) == current).child(label).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.end_field_edit();
                    this.config.sync.enabled = enabled;
                    if enabled {
                        this.config.sync.icloud = cloud;
                    }
                    this.save_config();
                    this.apply_sync_config(cx);
                    cx.notify();
                },
            ))
        }));
        let server = self.sync_field(Field::SyncServer, "sync-server", "https://sync.example.com".into(), false, window, cx);
        let token = self.sync_field(Field::SyncToken, "sync-token", t!("settings.sync.token.placeholder").into(), true, window, cx);
        let status = self.render_sync_status();
        let lost = self.render_lost(cx);
        let field = |label: String, hint: String, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .py_3()
                .border_t_1()
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pb_1()
                    .child(crate::ui::text::page_title(theme, t!("settings.sync.title"), None))
                    .child(
                        div()
                            .px_1p5()
                            .rounded_sm()
                            .border_1()
                            .border_color(theme.accent)
                            .text_color(theme.accent)
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("settings.sync.beta")),
                    ),
            )
            .child(div().text_size(px(12.)).text_color(theme.muted).pb_3().child(t!("settings.sync.intro")))
            // The way to sync, and where it stands right under it.
            .child(div().flex().flex_col().gap_3().pb_3().child(toggle).child(status))
            .when(on && !icloud, |d| {
                d.child(field(t!("settings.sync.server.label").into(), t!("settings.sync.server.hint").into(), server)).child(field(
                    t!("settings.sync.token.label").into(),
                    t!("settings.sync.token.hint").into(),
                    token,
                ))
            })
            .children(lost)
            .when(self.sync.session().is_some() && !icloud, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .py_3()
                        .border_t_1()
                        .border_color(theme.divider)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("settings.sync.start_over.label")))
                                .child(div().text_size(px(12.)).text_color(theme.muted).child(t!("settings.sync.start_over.hint"))),
                        )
                        .child(
                            div().child(
                                crate::ui::button::button(theme, "sync-start-over", crate::ui::button::Variant::Link)
                                    .child(t!("settings.sync.start_over.button"))
                                    .on_click(cx.listener(|this, _, window, cx| this.confirm_start_over(window, cx))),
                            ),
                        ),
                )
            })
            .into_any_element()
    }

    /// Where sync stands, in a box under the way to sync: its cloud, a few words, when it last
    /// synced, and a line on what it goes through or what went wrong.
    fn render_sync_status(&self) -> AnyElement {
        let theme = self.theme;
        let status = self.sync_status();
        let color = match status.tone {
            SyncTone::Idle => theme.muted,
            SyncTone::Active => theme.accent,
            SyncTone::Problem => theme.danger,
        };
        div()
            .flex()
            .items_start()
            .gap_3()
            .px_3()
            .py_2p5()
            .rounded(px(crate::ui::theme::R_LG))
            .bg(theme.code_bg)
            .child(div().flex_none().h(px(20.)).flex().items_center().child(crate::ui::icons::icon(
                status.icon,
                crate::ui::icons::MD,
                color,
            )))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .h(px(20.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if status.tone == SyncTone::Problem { theme.danger } else { theme.text })
                                    .child(status.title),
                            )
                            .children(status.time.map(|time| div().flex_none().text_size(px(12.)).text_color(theme.muted).child(time))),
                    )
                    .children(status.detail.map(|detail| div().text_size(px(12.)).text_color(theme.muted).child(detail))),
            )
            .into_any_element()
    }

    /// Conflict copies, newest first, each with Restore and Dismiss; nothing without any.
    fn render_lost(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme;
        let copies = self.ws.sync.lost();
        if copies.is_empty() {
            return None;
        }
        let rows = copies.iter().enumerate().rev().map(|(ix, copy)| {
            let bullet = self.ws.tree.get(copy.id).map(|n| flowie_core::markup::plain_text(&n.text));
            let whose = match (copy.field.as_str(), bullet) {
                (_, None) => t!("settings.sync.lost.deleted").to_string(),
                ("note", Some(text)) => t!("settings.sync.lost.note_of", bullet = excerpt(&text)),
                (_, Some(text)) => t!("settings.sync.lost.text_of", bullet = excerpt(&text)),
            };
            let when = chrono::DateTime::from_timestamp_millis(copy.at)
                .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default();
            div()
                .flex()
                .flex_col()
                .gap_1()
                .py_2()
                .border_t_1()
                .border_color(theme.divider)
                .child(div().text_size(px(12.)).text_color(theme.muted).child(format!("{whose} · {when}")))
                .child(div().p_2().rounded_md().bg(theme.code_bg).child(excerpt_long(&copy.value)))
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            crate::ui::button::button(theme, ("lost-restore", ix), crate::ui::button::Variant::Link)
                                .child(t!("settings.sync.lost.restore"))
                                .on_click(cx.listener(move |this, _, _, cx| this.restore_lost(ix, cx))),
                        )
                        .child(
                            crate::ui::button::button(theme, ("lost-dismiss", ix), crate::ui::button::Variant::Link)
                                .child(t!("settings.sync.lost.dismiss"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.ws.dismiss_lost(ix);
                                    cx.notify();
                                })),
                        ),
                )
        });
        Some(
            div()
                .flex()
                .flex_col()
                .pt_3()
                .border_t_1()
                .border_color(theme.divider)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("settings.sync.lost.label")))
                .child(div().text_size(px(12.)).text_color(theme.muted).pb_1().child(t!("settings.sync.lost.hint")))
                .children(rows)
                .into_any_element(),
        )
    }
}

/// The start of `text` on one line, for naming a bullet.
fn excerpt(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(40) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

/// Text shown whole up to a few hundred characters.
fn excerpt_long(text: &str) -> String {
    match text.char_indices().nth(400) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}
