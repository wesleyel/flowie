//! Settings (⌘,): keyboard shortcuts, appearance and about. Stored as JSON
//! next to the database, separate from the outline data.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, SharedString, Window};
use serde::{Deserialize, Serialize};

use crate::i18n::{self, t};
use crate::keymap;
use crate::outline::{OpenAbout, OpenPrivacyPolicy, OpenSettings, OpenTerms, Outline, SendFeedback};
use crate::theme::{Theme, ACCENTS};

const ABOUT_BOX_W: f32 = 500.0;
const ABOUT_PAD: f32 = 12.0;
const ABOUT_KEY_W: f32 = 90.0;
const ABOUT_GAP: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub theme: ThemeMode,
    /// Accent name from `ACCENTS`; empty = default blue.
    #[serde(default)]
    pub accent: String,
    /// Shortcut overrides: binding id -> shortcuts (empty = unbound).
    #[serde(default)]
    pub keys: HashMap<String, Vec<String>>,
    /// What AI agents may do through `flowie mcp`.
    #[serde(default)]
    pub agent: crate::agent::AgentMode,
    /// UI language: a locale code from `locales/`; empty = follow macOS.
    #[serde(default)]
    pub language: String,
    /// Animate mouse-wheel scrolling of the outline (trackpads already scroll smoothly). Not in
    /// the Settings window yet; set `"smooth_scroll": false` in settings.json to turn it off.
    #[serde(default = "yes")]
    pub smooth_scroll: bool,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: ThemeMode::default(),
            accent: String::new(),
            keys: HashMap::new(),
            agent: Default::default(),
            language: String::new(),
            smooth_scroll: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

#[derive(Clone, Copy)]
pub struct Recording {
    id: &'static str,
    index: Option<usize>,
}

impl Config {
    pub fn path(db: Option<&PathBuf>) -> PathBuf {
        match db.and_then(|p| p.parent()) {
            Some(dir) => dir.join("settings.json"),
            None => crate::data_path().with_file_name("settings.json"),
        }
    }

    pub fn load(path: &PathBuf) -> Config {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &PathBuf) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Shortcuts,
    Appearance,
    Agent,
    About,
}

pub struct SettingsUi {
    pub tab: Tab,
    /// Binding id waiting for a new keystroke.
    pub recording: Option<Recording>,
    /// The current page's scroll position.
    pub scroll: gpui::ScrollHandle,
}

impl Outline {
    pub fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.show_settings(Tab::Shortcuts, cx);
    }

    pub fn open_about(&mut self, _: &OpenAbout, _: &mut Window, cx: &mut Context<Self>) {
        self.show_settings(Tab::About, cx);
    }

    pub fn send_feedback(&mut self, _: &SendFeedback, _: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(&feedback_url());
    }

    pub fn open_privacy_policy(&mut self, _: &OpenPrivacyPolicy, _: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(&format!("{SITE_URL}/privacy.html"));
    }

    pub fn open_terms(&mut self, _: &OpenTerms, _: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(&format!("{SITE_URL}/terms.html"));
    }

    pub(crate) fn show_settings(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.settings.as_ref().is_some_and(|s| s.tab == tab) {
            self.settings = None;
            if let Some((t, off)) = self.settings_return.take() {
                self.start_edit(t, off, cx);
            }
            cx.notify();
            return;
        }
        if self.settings.is_none() {
            self.palette = None;
            self.inline = None;
            self.calendar = None;
            self.settings_return = self.edit.take().map(|e| (e.target, e.caret()));
        }
        self.settings = Some(SettingsUi { tab, recording: None, scroll: gpui::ScrollHandle::new() });
        cx.notify();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Focus may be on a selectable label inside the panel; give it back.
        window.focus(&self.focus, cx);
        self.settings = None;
        if let Some((t, off)) = self.settings_return.take() {
            self.start_edit(t, off, cx);
        }
        cx.notify();
    }

    fn save_config(&self) {
        self.config.save(&self.config_path);
    }

    pub fn apply_theme(&mut self, window: &Window) {
        self.theme = Theme::build(window.appearance(), &self.config);
    }

    /// Keystrokes while the settings panel is open: record a shortcut or close.
    pub fn settings_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ui) = self.settings.as_mut() else { return };
        let ks = &ev.keystroke;
        match ui.recording {
            Some(recording) => {
                let m = &ks.modifiers;
                let plain = !(m.platform || m.control || m.alt || m.shift);
                if plain && ks.key == "escape" {
                    ui.recording = None;
                } else if plain && ks.key == "backspace" {
                    ui.recording = None;
                    self.finish_recording(recording, None, cx);
                } else if !["shift", "control", "alt", "platform", "function", "fn"].contains(&ks.key.as_str()) {
                    ui.recording = None;
                    let mut stroke = String::new();
                    if m.control {
                        stroke.push_str("ctrl-");
                    }
                    if m.alt {
                        stroke.push_str("alt-");
                    }
                    if m.shift {
                        stroke.push_str("shift-");
                    }
                    if m.platform {
                        stroke.push_str("cmd-");
                    }
                    if m.function {
                        stroke.push_str("fn-");
                    }
                    stroke.push_str(&ks.key);
                    self.finish_recording(recording, Some(stroke), cx);
                }
                cx.stop_propagation();
            }
            None => {
                if ks.key == "escape" && !ks.modifiers.modified() {
                    self.close_settings(window, cx);
                    cx.stop_propagation();
                }
            }
        }
        cx.notify();
    }

    fn finish_recording(&mut self, recording: Recording, stroke: Option<String>, cx: &mut Context<Self>) {
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

    /// A selectable label entity, created on first use and kept in sync.
    fn label(&mut self, key: &'static str, text: String, cx: &mut Context<Self>) -> gpui::Entity<crate::selectable::SelectableText> {
        let color = self.theme.selection;
        let e = self.labels.entry(key).or_insert_with(|| cx.new(|cx| crate::selectable::SelectableText::new(text.clone(), cx))).clone();
        e.update(cx, |l, _| {
            l.set_text(text);
            l.selection_color = color;
        });
        e
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

    // ------------------------------------------------------------ render

    pub fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ui = self.settings.as_ref()?;
        let theme = self.theme;
        let tab = ui.tab;
        let tabs = [
            (Tab::Shortcuts, t!("settings.tab.shortcuts"), "keyboard"),
            (Tab::Appearance, t!("settings.tab.appearance"), "sun-moon"),
            (Tab::Agent, t!("settings.tab.agent"), "bot"),
            (Tab::About, t!("settings.tab.about"), "info"),
        ];
        let sidebar = div()
            .w(px(170.))
            .flex_none()
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .border_r_1()
            .border_color(theme.divider)
            .child(crate::ui::text::overline(theme, t!("settings.heading")).px_2().pt_1().pb_2())
            .children(tabs.into_iter().map(|(t, label, icon)| {
                let on = t == tab;
                crate::ui::list::row(theme, icon, on)
                    .child(crate::icons::slot(icon, crate::icons::MD, if on { theme.on_accent } else { theme.accent }))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(s) = this.settings.as_mut() {
                            s.tab = t;
                            s.recording = None;
                            s.scroll = gpui::ScrollHandle::new();
                        }
                        cx.notify();
                    }))
            }));
        let scroll = ui.scroll.clone();
        let content = match tab {
            // Only its list scrolls; it draws its own bar.
            Tab::Shortcuts => self.render_shortcuts(cx),
            Tab::Appearance => self.render_appearance(window, cx),
            Tab::Agent => self.render_agent(cx),
            Tab::About => self.render_about(cx),
        };
        let content = if tab == Tab::Shortcuts {
            content
        } else {
            crate::ui::scrollbar::area("settings-bar", &scroll, theme, content).size_full().into_any_element()
        };
        Some(
            crate::ui::surface::scrim(theme)
                .items_center()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
                .child(
                    crate::ui::surface::sheet(theme, "settings-card")
                        .w(px(760.))
                        .h(px(540.))
                        .max_h(gpui::relative(0.9))
                        .flex()
                        .child(sidebar)
                        .child(div().flex_1().min_w_0().flex().flex_col().child(content)),
                )
                .into_any_element(),
        )
    }

    fn render_shortcuts(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
                            t!("settings.shortcuts.recording_hint", backspace = keymap::backspace_hint()).into()
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

    fn render_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.settings.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        let theme = self.theme;
        let mode = self.config.theme;
        let accent = if self.config.accent.is_empty() { "Blue".to_string() } else { self.config.accent.clone() };
        let dark = matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark);
        let modes = [
            (ThemeMode::System, "system", t!("settings.appearance.theme.system")),
            (ThemeMode::Light, "light", t!("settings.appearance.theme.light")),
            (ThemeMode::Dark, "dark", t!("settings.appearance.theme.dark")),
        ];
        let segmented = crate::ui::segmented::track(theme).children(modes.into_iter().map(|(m, id, label)| {
            crate::ui::segmented::segment(theme, id, m == mode).child(label).on_click(cx.listener(move |this, _, window, cx| {
                this.config.theme = m;
                this.save_config();
                this.apply_theme(window);
                cx.notify();
            }))
        }));
        let swatches = div().flex().gap_3().children(ACCENTS.iter().map(|(name, light, darkc)| {
            let name: &'static str = name;
            let color: gpui::Hsla = gpui::rgb(if dark { *darkc } else { *light }).into();
            let selected = accent == name;
            div()
                .id(name)
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .cursor_pointer()
                // Selected: the color, a gap in the background color, and a ring
                // in the color, as nested circles (a spread shadow isn't round).
                .child(
                    div().size(px(30.)).rounded_full().flex().items_center().justify_center().when(selected, |d| d.bg(color)).child(
                        div()
                            .size(px(26.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |d| d.bg(theme.bg))
                            .child(div().size(px(if selected { 22. } else { 26. })).rounded_full().bg(color)),
                    ),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(if selected { theme.text } else { theme.muted })
                        .child(i18n::text(&format!("settings.appearance.accent.{}", name.to_lowercase()))),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.config.accent = name.to_string();
                    this.save_config();
                    this.apply_theme(window);
                    cx.notify();
                }))
        }));
        let language = self.config.language.clone();
        let choices = std::iter::once(("", t!("settings.appearance.language.system"))).chain(i18n::languages());
        let languages = crate::ui::segmented::track(theme).children(choices.map(|(code, label)| {
            crate::ui::segmented::segment(theme, SharedString::from(format!("lang-{code}")), code == language).child(label).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.config.language = code.to_string();
                    this.save_config();
                    i18n::set(code);
                    keymap::set_menus(cx);
                    cx.notify();
                }),
            )
        }));
        let field = |label: &'static str, hint: SharedString, control: AnyElement| {
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
        let preview = div()
            .mt_3()
            .p_3()
            .rounded(px(crate::theme::R_LG))
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(6.)).rounded_full().bg(theme.bullet))
                    .child(t!("settings.appearance.preview.before_link"))
                    .child(div().text_color(theme.link).child(t!("settings.appearance.preview.link")))
                    .child(t!("settings.appearance.preview.before_tag"))
                    .child(div().px_1().rounded_sm().bg(theme.tag_bg).text_color(theme.tag).child("#tag")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(20.))
                    .child(div().size(px(6.)).rounded_full().bg(theme.bullet))
                    .child(div().px_1().bg(theme.selection).child(t!("settings.appearance.preview.selected")))
                    .child(div().w(px(2.)).h(px(16.)).bg(theme.caret)),
            );
        let os = os_name();
        div()
            .id("appearance")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .px_4()
            .pt_3()
            .child(crate::ui::text::page_title(theme, t!("settings.appearance.title"), None).pb_1())
            .child(field(
                t!("settings.appearance.theme.label"),
                t!("settings.appearance.theme.hint", os = os).into(),
                segmented.into_any_element(),
            ))
            .child(field(t!("settings.appearance.accent.label"), t!("settings.appearance.accent.hint").into(), swatches.into_any_element()))
            .child(field(
                t!("settings.appearance.language.label"),
                t!("settings.appearance.language.hint", os = os).into(),
                languages.into_any_element(),
            ))
            .child(preview)
            .into_any_element()
    }

    fn render_agent(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.settings.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        use crate::agent::AgentMode;
        let theme = self.theme;
        let mode = self.config.agent;
        let modes = [
            (AgentMode::Off, "off", t!("settings.agent.mode.off")),
            (AgentMode::ReadOnly, "read-only", t!("settings.agent.mode.read_only")),
            (AgentMode::ReadWrite, "read-write", t!("settings.agent.mode.read_write")),
        ];
        let segmented = crate::ui::segmented::track(theme).children(modes.into_iter().map(|(m, id, label)| {
            crate::ui::segmented::segment(theme, id, m == mode).child(label).on_click(cx.listener(move |this, _, _, cx| {
                this.config.agent = m;
                this.save_config();
                cx.notify();
            }))
        }));
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "Flowie".into());
        // Screenshots show where the app is installed, not the build directory.
        #[cfg(feature = "screenshots")]
        let exe = {
            drop(exe);
            "/Applications/Flowie.app/Contents/MacOS/Flowie".to_string()
        };
        let db_env = self
            .db_path
            .as_ref()
            .filter(|p| **p != crate::data_path())
            .map(|p| format!("FLOWIE_DB=\"{}\" ", p.display()))
            .unwrap_or_default();
        let claude = format!("claude mcp add flowie -- {db_env}\"{exe}\" mcp");
        let exe_json = serde_json::to_string(&exe).unwrap_or_else(|_| format!("{exe:?}"));
        let json_cfg = format!("{{\n  \"mcpServers\": {{\n    \"flowie\": {{ \"command\": {exe_json}, \"args\": [\"mcp\"] }}\n  }}\n}}");
        let claude_label = self.label("mcp-claude", claude, cx);
        let json_label = self.label("mcp-json", json_cfg, cx);
        let block = |label: gpui::Entity<crate::selectable::SelectableText>| {
            div().p_2().rounded_md().bg(theme.code_bg).font_family(crate::theme::MONO_FONT).text_size(px(12.)).child(label)
        };
        let desc = match mode {
            AgentMode::Off => t!("settings.agent.desc.off").to_string(),
            AgentMode::ReadOnly => t!("settings.agent.desc.read_only").to_string(),
            AgentMode::ReadWrite => t!("settings.agent.desc.read_write", undo = keymap::primary_hint("Z")),
        };
        div()
            .id("agent")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .px_4()
            .pt_3()
            .pb_4()
            .child(crate::ui::text::page_title(theme, t!("settings.agent.title"), None).pb_1())
            .child(div().text_size(px(12.)).text_color(theme.muted).pb_3().child(t!("settings.agent.intro")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pb_3()
                    .border_b_1()
                    .border_color(theme.divider)
                    .child(segmented)
                    .child(div().text_size(px(12.)).text_color(theme.muted).child(desc)),
            )
            .child(div().pt_3().pb_1().font_weight(FontWeight::SEMIBOLD).child("Claude Code"))
            .child(block(claude_label))
            .child(div().pt_3().pb_1().font_weight(FontWeight::SEMIBOLD).child(t!("settings.agent.other_clients")))
            .child(block(json_label))
            .child(div().pt_3().text_size(px(12.)).text_color(theme.muted).child(t!("settings.agent.tools")))
            .into_any_element()
    }

    fn render_about(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.settings.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        let theme = self.theme;
        let version = env!("CARGO_PKG_VERSION");
        let commit = option_env!("FLOWIE_COMMIT").unwrap_or("dev");
        let db = self.db_path.clone();
        let db_label = db.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| t!("settings.about.in_memory").into());
        let nodes = self.tree.len();
        let version_label = self.label("version", t!("settings.about.version", version = version, commit = commit), cx);
        let values = [
            (t!("settings.about.bullets"), self.label("bullets", format!("{nodes}"), cx)),
            (t!("settings.about.data"), self.label("data", db_label, cx)),
            (t!("settings.about.settings"), self.label("settings", self.config_path.display().to_string(), cx)),
        ];
        let line = |k: &'static str, v: gpui::Entity<crate::selectable::SelectableText>| {
            // Fixed widths so wrapped values are measured at the width they render at.
            div()
                .flex()
                .items_start()
                .gap(px(ABOUT_GAP))
                .py(px(3.))
                .child(div().w(px(ABOUT_KEY_W)).flex_none().text_color(theme.muted).child(k))
                .child(div().w(px(ABOUT_BOX_W - 2. * ABOUT_PAD - ABOUT_KEY_W - ABOUT_GAP)).flex_none().child(v))
        };
        div()
            .id("about")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .flex()
            .flex_col()
            .items_center()
            .pt(px(28.))
            .pb_6()
            .px_6()
            .child(
                div()
                    .flex_none()
                    .size(px(64.))
                    .rounded(px(15.))
                    .bg(theme.accent)
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(5.))
                    .px(px(12.))
                    .children([(0., 30.), (10., 22.), (10., 22.)].into_iter().map(|(indent, w)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .pl(px(indent))
                            .child(div().size(px(5.)).rounded_full().bg(theme.on_accent))
                            .child(div().w(px(w)).h(px(4.)).rounded_full().bg(theme.on_accent.opacity(0.9)))
                    })),
            )
            .child(div().flex_none().pt_3().text_size(px(22.)).font_weight(FontWeight::BOLD).child("Flowie"))
            .child(div().flex_none().text_color(theme.muted).child(version_label))
            .child(div().flex_none().pt_2().pb_4().text_color(theme.muted).child(t!("settings.about.tagline")))
            .child(
                div()
                    .w(px(ABOUT_BOX_W))
                    .flex_none()
                    .p(px(ABOUT_PAD))
                    .rounded(px(crate::theme::R_LG))
                    .bg(theme.input_bg)
                    .children(values.into_iter().map(|(k, v)| line(k, v))),
            )
            .child({
                let button = |id: &'static str| crate::ui::button::button(theme, id, crate::ui::button::Variant::Secondary);
                div()
                    .flex()
                    .flex_none()
                    .gap_2()
                    .mt_3()
                    .when_some(db, |d, path| {
                        d.child(
                            button("reveal")
                                .child(t!("settings.about.reveal", manager = file_manager_name()))
                                .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&path))),
                        )
                    })
                    .child(
                        button("feedback")
                            .child(t!("settings.about.feedback"))
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(&feedback_url()))),
                    )
            })
            .into_any_element()
    }
}

/// Where feedback mail goes.
pub const FEEDBACK_EMAIL: &str = "wesleyel@icloud.com";
/// The GitHub Pages site (source in website/): home, privacy.html and terms.html.
pub const SITE_URL: &str = "https://wesleyel.github.io/flowie";

fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        t!("settings.appearance.theme.system")
    }
}

fn file_manager_name() -> &'static str {
    if cfg!(target_os = "macos") {
        t!("settings.about.file_manager_macos")
    } else if cfg!(target_os = "windows") {
        t!("settings.about.file_manager_windows")
    } else {
        t!("settings.about.file_manager_other")
    }
}

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        let ver = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        format!("macOS {ver}").trim().to_string()
    }
    #[cfg(target_os = "windows")]
    {
        let v = windows_version::OsVersion::current();
        let release = if v.major == 10 && v.build >= 22000 {
            "11"
        } else if v.major == 10 {
            "10"
        } else {
            ""
        };
        if release.is_empty() {
            format!("Windows ({}.{}.{})", v.major, v.minor, v.build)
        } else {
            format!("Windows {release} ({}.{}.{})", v.major, v.minor, v.build)
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::consts::OS.to_string()
    }
}

/// A `mailto:` link with the app version and OS version filled in.
fn feedback_url() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let commit = option_env!("FLOWIE_COMMIT").unwrap_or("dev");
    let os = os_version();
    let subject = format!("Flowie {version} feedback");
    let body = format!("\n\n\n--\nFlowie {version} ({commit}), {os}\n");
    format!("mailto:{FEEDBACK_EMAIL}?subject={}&body={}", percent(&subject), percent(&body))
}

fn percent(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
