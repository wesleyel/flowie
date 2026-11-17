//! Settings (⌘,): keyboard shortcuts, appearance and about. Stored as JSON
//! next to the database, separate from the outline data.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, SharedString, Window};
use serde::{Deserialize, Serialize};

use crate::keymap;
use crate::outline::{Outline, OpenAbout, OpenSettings};
use crate::theme::{Theme, ACCENTS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub theme: ThemeMode,
    /// Accent name from `ACCENTS`; empty = default blue.
    #[serde(default)]
    pub accent: String,
    /// Shortcut overrides: binding id -> keystrokes ("" = unbound).
    #[serde(default)]
    pub keys: HashMap<String, String>,
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
    About,
}

pub struct SettingsUi {
    pub tab: Tab,
    /// Binding id waiting for a new keystroke.
    pub recording: Option<&'static str>,
}

impl Outline {
    pub fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.show_settings(Tab::Shortcuts, cx);
    }

    pub fn open_about(&mut self, _: &OpenAbout, _: &mut Window, cx: &mut Context<Self>) {
        self.show_settings(Tab::About, cx);
    }

    fn show_settings(&mut self, tab: Tab, cx: &mut Context<Self>) {
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
        self.settings = Some(SettingsUi { tab, recording: None });
        cx.notify();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Focus may be on a selectable label inside the panel; give it back.
        window.focus(&self.focus);
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
            Some(id) => {
                let m = &ks.modifiers;
                let plain = !(m.platform || m.control || m.alt || m.shift);
                if plain && ks.key == "escape" {
                    ui.recording = None;
                } else if plain && ks.key == "backspace" {
                    ui.recording = None;
                    self.config.keys.insert(id.to_string(), String::new());
                    self.rebind(cx);
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
                    let default: Vec<String> = keymap::keys_for(id, &HashMap::new());
                    if default == vec![stroke.clone()] {
                        self.config.keys.remove(id);
                    } else {
                        self.config.keys.insert(id.to_string(), stroke);
                    }
                    self.rebind(cx);
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
        let tabs = [(Tab::Shortcuts, "Shortcuts", "keyboard"), (Tab::Appearance, "Appearance", "sun-moon"), (Tab::About, "About", "info")];
        let sidebar = div()
            .w(px(170.))
            .flex_none()
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .border_r_1()
            .border_color(theme.border)
            .child(div().px_2().pt_1().pb_2().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.muted).child("SETTINGS"))
            .children(tabs.into_iter().map(|(t, label, icon)| {
                div()
                    .id(label)
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .h(px(30.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(t == tab, |d| d.bg(theme.row_selected).text_color(theme.text))
                    .when(t != tab, |d| d.text_color(theme.muted))
                    .hover(|s| s.bg(theme.hover))
                    .child(crate::icons::slot(icon, crate::icons::MD, if t == tab { theme.text } else { theme.muted }))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(s) = this.settings.as_mut() {
                            s.tab = t;
                            s.recording = None;
                        }
                        cx.notify();
                    }))
            }));
        let content = match tab {
            Tab::Shortcuts => self.render_shortcuts(cx),
            Tab::Appearance => self.render_appearance(window, cx),
            Tab::About => self.render_about(cx),
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::black().opacity(0.18))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
                .child(
                    div()
                        .id("settings-card")
                        .w(px(760.))
                        .h(px(540.))
                        .max_h(gpui::relative(0.9))
                        .flex()
                        .bg(theme.bg)
                        .border_1()
                        .border_color(theme.border)
                        .rounded_xl()
                        .shadow_lg()
                        .overflow_hidden()
                        .text_size(px(13.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(sidebar)
                        .child(div().flex_1().min_w_0().flex().flex_col().child(content)),
                )
                .into_any_element(),
        )
    }

    fn render_shortcuts(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
                    .child(group.to_uppercase())
                    .into_any_element(),
            );
            for row in keymap::rows().into_iter().filter(|r| r.group == group) {
                let id = row.id;
                let keys = keymap::keys_for(id, &overrides);
                let custom = overrides.contains_key(id);
                let conflict = conflicts.contains(id);
                let is_rec = recording == Some(id);
                let chips: Vec<AnyElement> = if is_rec {
                    vec![div().h(px(20.)).flex().items_center().px_2().rounded(px(5.)).border_1().border_color(theme.accent).text_size(px(12.)).text_color(theme.accent).child("Press keys…").into_any_element()]
                } else if keys.is_empty() {
                    vec![div().px_2().text_color(theme.faint).child("—").into_any_element()]
                } else {
                    keys.iter()
                        .map(|k| crate::icons::key_chip(k, theme.text, theme.code_bg, if conflict { theme.colors[0].opacity(0.6) } else { theme.border }))
                        .collect()
                };
                sections.push(
                    div()
                        .id(SharedString::from(format!("key-{id}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .h(px(30.))
                        .rounded_md()
                        .cursor_pointer()
                        .when(is_rec, |d| d.bg(theme.row_selected))
                        .hover(|s| s.bg(theme.hover))
                        .child(div().flex_1().child(row.label))
                        .when(conflict && !is_rec, |d| d.child(div().text_size(px(11.)).text_color(theme.colors[0]).child("conflict")))
                        .children(chips)
                        .child(
                            div()
                                .id(SharedString::from(format!("reset-{id}")))
                                .flex_none()
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .when(custom, |d| d.cursor_pointer().hover(|s| s.bg(theme.hover)).child(crate::icons::icon("rotate-ccw", crate::icons::SM, theme.muted)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.reset_binding(id, cx);
                                })),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(s) = this.settings.as_mut() {
                                s.recording = if s.recording == Some(id) { None } else { Some(id) };
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
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Keyboard shortcuts"))
                            .child(div().text_size(px(12.)).text_color(theme.muted).child(if recording.is_some() {
                                "Press the new shortcut · Esc cancels · ⌫ removes it"
                            } else {
                                "Click a shortcut to change it"
                            })),
                    )
                    .when(any_custom, |d| {
                        d.child(
                            div()
                                .id("reset-all")
                                .px_2()
                                .py(px(3.))
                                .rounded_md()
                                .cursor_pointer()
                                .text_color(theme.link)
                                .hover(|s| s.bg(theme.hover))
                                .child("Reset all")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.config.keys.clear();
                                    this.rebind(cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(div().id("shortcut-list").flex_1().min_h_0().overflow_y_scroll().px_3().pb_3().children(sections))
            .into_any_element()
    }

    fn render_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let mode = self.config.theme;
        let accent = if self.config.accent.is_empty() { "Blue".to_string() } else { self.config.accent.clone() };
        let dark = matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark);
        let modes = [(ThemeMode::System, "System"), (ThemeMode::Light, "Light"), (ThemeMode::Dark, "Dark")];
        let segmented = div()
            .flex()
            .p(px(2.))
            .rounded_lg()
            .bg(theme.input_bg)
            .children(modes.into_iter().map(|(m, label)| {
                div()
                    .id(label)
                    .px_4()
                    .py(px(4.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(m == mode, |d| d.bg(theme.bg).shadow_sm().font_weight(FontWeight::SEMIBOLD))
                    .when(m != mode, |d| d.text_color(theme.muted))
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
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
                .child(
                    div()
                        .size(px(26.))
                        .rounded_full()
                        .bg(color)
                        .border_2()
                        .border_color(if selected { theme.text } else { gpui::transparent_black() }),
                )
                .child(div().text_size(px(11.)).text_color(if selected { theme.text } else { theme.muted }).child(name))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.config.accent = name.to_string();
                    this.save_config();
                    this.apply_theme(window);
                    cx.notify();
                }))
        }));
        let field = |label: &'static str, hint: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .py_3()
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex().flex_col().child(div().font_weight(FontWeight::SEMIBOLD).child(label)).child(div().text_size(px(12.)).text_color(theme.muted).child(hint)))
                .child(control)
        };
        let preview = div()
            .mt_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap_1()
            .child(div().flex().items_center().gap_2().child(div().size(px(6.)).rounded_full().bg(theme.bullet)).child("A bullet with a ").child(div().text_color(theme.link).child("link")).child(" and a ").child(div().px_1().rounded_sm().bg(theme.tag_bg).text_color(theme.tag).child("#tag")))
            .child(div().flex().items_center().gap_2().pl(px(20.)).child(div().size(px(6.)).rounded_full().bg(theme.bullet)).child(div().px_1().bg(theme.selection).child("selected text")).child(div().w(px(2.)).h(px(16.)).bg(theme.caret)));
        div()
            .id("appearance")
            .size_full()
            .overflow_y_scroll()
            .px_4()
            .pt_3()
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).pb_1().child("Appearance"))
            .child(field("Theme", "Follow macOS, or always use light or dark.", segmented.into_any_element()))
            .child(field("Accent color", "Caret, selection, links and highlights.", swatches.into_any_element()))
            .child(preview)
            .into_any_element()
    }

    fn render_about(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let version = env!("CARGO_PKG_VERSION");
        let commit = option_env!("FLOWIE_COMMIT").unwrap_or("dev");
        let db = self.db_path.clone();
        let db_label = db.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "In memory (not saved)".into());
        let nodes = self.tree.len();
        let version_label = self.label("version", format!("Version {version} ({commit})"), cx);
        let values = [
            ("Bullets", self.label("bullets", format!("{nodes}"), cx)),
            ("Data", self.label("data", db_label, cx)),
            ("Settings", self.label("settings", self.config_path.display().to_string(), cx)),
            ("Built with", self.label("built", "GPUI · RaTeX (KaTeX-compatible math) · SQLite".into(), cx)),
            ("License", self.label("license", "MIT".into(), cx)),
        ];
        let line = |k: &'static str, v: gpui::Entity<crate::selectable::SelectableText>| {
            div().flex().gap_3().py(px(3.)).child(div().w(px(90.)).flex_none().text_color(theme.muted).child(k)).child(div().flex_1().min_w_0().child(v))
        };
        div()
            .id("about")
            .size_full()
            .overflow_y_scroll()
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
                            .child(div().size(px(5.)).rounded_full().bg(gpui::white()))
                            .child(div().w(px(w)).h(px(4.)).rounded_full().bg(gpui::white().opacity(0.9)))
                    })),
            )
            .child(div().flex_none().pt_3().text_size(px(22.)).font_weight(FontWeight::BOLD).child("Flowie"))
            .child(div().flex_none().text_color(theme.muted).child(version_label))
            .child(div().flex_none().pt_2().pb_4().text_color(theme.muted).child("A fast outliner for thinking in lists."))
            .child(
                div()
                    .w_full()
                    .flex_none()
                    .max_w(px(500.))
                    .p_3()
                    .rounded_lg()
                    .bg(theme.input_bg)
                    .children(values.into_iter().map(|(k, v)| line(k, v))),
            )
            .when_some(db, |d, path| {
                d.child(
                    div()
                        .id("reveal")
                        .flex_none()
                        .mt_3()
                        .px_3()
                        .py(px(5.))
                        .rounded_md()
                        .cursor_pointer()
                        .border_1()
                        .border_color(theme.border)
                        .hover(|s| s.bg(theme.hover))
                        .child("Show data in Finder")
                        .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&path))),
                )
            })
            .into_any_element()
    }
}
