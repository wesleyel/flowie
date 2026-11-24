//! Settings state, persistence, and the tabbed panel.

mod about;
mod agent;
mod appearance;
mod general;
mod shortcuts;

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{div, prelude::*, px, AnyElement, Context, KeyDownEvent, MouseButton, Window};
use serde::{Deserialize, Serialize};

use crate::app::i18n::t;
use crate::outline::{OpenSettings, Outline};

pub(crate) use appearance::ThemeMode;
pub(crate) use general::Author;
use shortcuts::Recording;

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
    /// Who this user is when sharing.
    #[serde(default)]
    pub author: Author,
    /// What AI agents may do through `flowie mcp`.
    #[serde(default)]
    pub agent: crate::agent::AgentMode,
    /// Jump and scroll to bullets as an AI agent edits them.
    #[serde(default = "yes")]
    pub agent_follow_edits: bool,
    /// UI language: a locale code from `locales/`; empty = follow macOS.
    #[serde(default)]
    pub language: String,
    /// Animate mouse-wheel scrolling of the outline (trackpads already scroll smoothly). Not in
    /// the Settings window yet; set `"smooth_scroll": false` in settings.json to turn it off.
    #[serde(default = "yes")]
    pub smooth_scroll: bool,
    /// The newest version whose What's New the user has had the chance to see.
    #[serde(default)]
    pub seen_version: Option<String>,
    /// Open What's New once after each update.
    #[serde(default = "yes")]
    pub whats_new_after_updates: bool,
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
            author: Default::default(),
            agent: Default::default(),
            agent_follow_edits: true,
            language: String::new(),
            smooth_scroll: true,
            seen_version: None,
            whats_new_after_updates: true,
        }
    }
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
    General,
    Shortcuts,
    Appearance,
    Agent,
    About,
}

pub struct SettingsUi {
    tab: Tab,
    /// Binding id waiting for a new keystroke.
    recording: Option<Recording>,
    /// The current page's scroll position.
    scroll: gpui::ScrollHandle,
}

impl Outline {
    pub fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.show_settings(Tab::General, cx);
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
                if self.field_key(ks, window, cx) {
                    cx.stop_propagation();
                } else if ks.key == "escape" && !ks.modifiers.modified() {
                    self.close_settings(window, cx);
                    cx.stop_propagation();
                }
            }
        }
        cx.notify();
    }

    /// A selectable label entity, created on first use and kept in sync.
    fn label(&mut self, key: &'static str, text: String, cx: &mut Context<Self>) -> gpui::Entity<crate::text::selectable::SelectableText> {
        let color = self.theme.selection;
        let e =
            self.labels.entry(key).or_insert_with(|| cx.new(|cx| crate::text::selectable::SelectableText::new(text.clone(), cx))).clone();
        e.update(cx, |l, _| {
            l.set_text(text);
            l.selection_color = color;
        });
        e
    }

    pub fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ui = self.settings.as_ref()?;
        let theme = self.theme;
        let tab = ui.tab;
        let tabs = [
            (Tab::General, t!("settings.tab.general"), "user"),
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
                    .child(crate::ui::icons::slot(icon, crate::ui::icons::MD, if on { theme.on_accent } else { theme.accent }))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.end_field_edit();
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
            Tab::General => self.render_general(window, cx),
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
}
