//! Shortcut recording, overrides, and the Shortcuts tab.

use std::collections::HashMap;

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, SharedString, Window};

use crate::app::i18n::t;
use crate::app::keymap;
use crate::outline::Outline;

#[derive(Clone, Copy)]
pub(super) struct Recording {
    id: &'static str,
    index: Option<usize>,
}

impl Outline {
    pub(super) fn finish_recording(&mut self, recording: Recording, stroke: Option<String>, cx: &mut Context<Self>) {
        let mut keys = keymap::keys_for(recording.id, &self.config.keys);
        if let Some(index) = recording.index {
            if index < keys.len() {
                keys.remove(index);
                if let Some(stroke) = stroke {
                    keys.insert(index, stroke);
                }
            }
        } else if let Some(stroke) = stroke {
            keys.push(stroke);
        }
        let mut seen = std::collections::HashSet::new();
        keys.retain(|key| seen.insert(key.clone()));
        if keys == keymap::keys_for(recording.id, &HashMap::new()) {
            self.config.keys.remove(recording.id);
        } else {
            self.config.keys.insert(recording.id.to_string(), keys);
        }
        self.rebind(cx);
    }

    pub fn mouse_shortcut(&mut self, direction: gpui::NavigationDirection, window: &mut Window, cx: &mut Context<Self>) {
        let key = match direction {
            gpui::NavigationDirection::Back => "mouse-back",
            gpui::NavigationDirection::Forward => "mouse-forward",
        };
        if let Some(ui) = self.settings.as_mut() {
            if let Some(recording) = ui.recording.take() {
                self.finish_recording(recording, Some(key.into()), cx);
                cx.notify();
            }
            return;
        }
        if let Some(action) = keymap::mouse_action(key, &self.config.keys, self.edit.is_some(), self.block.is_some()) {
            window.dispatch_action(action, cx);
        }
    }

    fn rebind(&mut self, cx: &mut Context<Self>) {
        self.save_config();
        keymap::install(cx, &self.config.keys);
    }

    fn reset_binding(&mut self, id: &str, cx: &mut Context<Self>) {
        self.config.keys.remove(id);
        self.rebind(cx);
        cx.notify();
    }

    pub(super) fn render_shortcuts(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.settings.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        let theme = self.theme;
        let recording = self.settings.as_ref().and_then(|s| s.recording);
        let overrides = self.config.keys.clone();
        let conflicts = keymap::conflicts(&overrides);
        let mut sections: Vec<AnyElement> = Vec::new();
        for group in keymap::GROUPS {
            sections.push(
                div()
                    .pt_3()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted)
                    .child(keymap::group_label(group).to_uppercase())
                    .into_any_element(),
            );
            for row in keymap::rows().into_iter().filter(|r| r.group == group) {
                let id = row.id;
                let keys = keymap::keys_for(id, &overrides);
                let custom = overrides.contains_key(id);
                let conflict = conflicts.contains(id);
                let is_rec = recording.is_some_and(|r| r.id == id);
                let mut chips: Vec<AnyElement> = keys
                    .iter()
                    .enumerate()
                    .map(|(index, key)| {
                        let active = recording.is_some_and(|r| r.id == id && r.index == Some(index));
                        div()
                            .id(SharedString::from(format!("key-{id}-{index}")))
                            .cursor_pointer()
                            .child(crate::ui::controls::keycap(
                                theme,
                                if active { t!("settings.shortcuts.press_keys") } else { key },
                                if active {
                                    Some(theme.accent)
                                } else if conflict {
                                    Some(theme.danger.opacity(0.6))
                                } else {
                                    None
                                },
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(s) = this.settings.as_mut() {
                                    s.recording = Some(Recording { id, index: Some(index) });
                                }
                                cx.notify();
                            }))
                            .into_any_element()
                    })
                    .collect();
                chips.push(
                    crate::ui::button::button(theme, SharedString::from(format!("add-{id}")), crate::ui::button::Variant::Link)
                        .h(px(20.))
                        .child(if is_rec && recording.is_some_and(|r| r.index.is_none()) {
                            t!("settings.shortcuts.press_keys")
                        } else {
                            "+"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(s) = this.settings.as_mut() {
                                s.recording = Some(Recording { id, index: None });
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                );
                sections.push(
                    crate::ui::list::hover_row(theme, SharedString::from(format!("key-{id}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .min_h(px(30.))
                        .py_1()
                        .when(is_rec, |d| d.bg(theme.row_selected))
                        .child(div().flex_1().min_w_0().child(row.label()))
                        .when(conflict && !is_rec, |d| {
                            d.child(div().text_size(px(11.)).text_color(theme.danger).child(t!("settings.shortcuts.conflict")))
                        })
                        .child(div().flex().flex_wrap().justify_end().max_w(gpui::relative(0.6)).gap_1().children(chips))
                        .child(
                            // Keeps its place when there is nothing to reset.
                            div()
                                .id(SharedString::from(format!("reset-{id}")))
                                .flex_none()
                                .size(px(24.))
                                .when(custom, |d| {
                                    d.child(crate::ui::button::icon_button(
                                        theme,
                                        SharedString::from(format!("reset-icon-{id}")),
                                        "rotate-ccw",
                                        theme.muted,
                                    ))
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.reset_binding(id, cx);
                                })),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(s) = this.settings.as_mut() {
                                s.recording =
                                    if s.recording.is_some_and(|r| r.id == id) { None } else { Some(Recording { id, index: None }) };
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
        }
        let any_custom = !overrides.is_empty();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .pt_3()
                    .pb_2()
                    .border_b_1()
                    .border_color(theme.divider)
                    .child(crate::ui::text::page_title(
                        theme,
                        t!("settings.shortcuts.title"),
                        Some(if recording.is_some() {
                            t!("settings.shortcuts.recording_hint", backspace = crate::app::platform::key_name("backspace")).into()
                        } else {
                            t!("settings.shortcuts.hint").into()
                        }),
                    ))
                    .when(any_custom, |d| {
                        d.child(
                            crate::ui::button::button(theme, "reset-all", crate::ui::button::Variant::Link)
                                .child(t!("settings.shortcuts.reset_all"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.config.keys.clear();
                                    this.rebind(cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                crate::ui::scrollbar::area(
                    "settings-bar",
                    &scroll,
                    theme,
                    div().id("shortcut-list").size_full().overflow_y_scroll().track_scroll(&scroll).px_3().pb_3().children(sections),
                )
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::Config;

    #[test]
    fn shortcut_settings_store_multiple_bindings() {
        let config: Config =
            serde_json::from_str(r#"{"keys":{"bold":["cmd-b"],"italic":[],"page-back":["mouse-back","cmd-alt-left"]}}"#).unwrap();
        assert_eq!(config.keys["bold"], vec!["cmd-b"]);
        assert!(config.keys["italic"].is_empty());
        assert_eq!(config.keys["page-back"], vec!["mouse-back", "cmd-alt-left"]);
        let saved = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&saved).unwrap().keys, config.keys);
    }
}
