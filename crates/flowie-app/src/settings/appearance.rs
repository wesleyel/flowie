//! Theme, accent, content width, and language preferences.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, SharedString, Window};
use serde::{Deserialize, Serialize};

use crate::app::i18n::{self, t};
use crate::app::keymap;
use crate::outline::{CycleContentWidth, Outline};
use crate::ui::theme::{Theme, ACCENTS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

/// How wide the outline's text column may grow: three steps. Narrow and Medium
/// center a column of at most that width (bullets and padding included) in a wide
/// window; Full uses the whole window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentWidth {
    Narrow,
    Medium,
    #[default]
    Full,
}

impl ContentWidth {
    pub const ALL: [ContentWidth; 3] = [ContentWidth::Narrow, ContentWidth::Medium, ContentWidth::Full];

    /// The column's largest width in pixels (`None`: the whole window).
    pub fn max_px(self) -> Option<f32> {
        match self {
            ContentWidth::Narrow => Some(680.),
            ContentWidth::Medium => Some(960.),
            ContentWidth::Full => None,
        }
    }

    /// The next step, after Full back to Narrow (the shortcut cycles).
    pub fn next(self) -> ContentWidth {
        match self {
            ContentWidth::Narrow => ContentWidth::Medium,
            ContentWidth::Medium => ContentWidth::Full,
            ContentWidth::Full => ContentWidth::Narrow,
        }
    }

    fn id(self) -> &'static str {
        match self {
            ContentWidth::Narrow => "narrow",
            ContentWidth::Medium => "medium",
            ContentWidth::Full => "full",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContentWidth::Narrow => t!("settings.appearance.width.narrow"),
            ContentWidth::Medium => t!("settings.appearance.width.medium"),
            ContentWidth::Full => t!("settings.appearance.width.full"),
        }
    }
}

impl Outline {
    /// The width of the outline's text column in `window`.
    pub fn content_width(&self, window: &Window) -> f32 {
        let full = f32::from(window.viewport_size().width);
        self.config.content_width.max_px().map_or(full, |max| full.min(max))
    }

    /// One list item (page header, row, footer) placed in the text column.
    pub fn in_content_column(&self, item: AnyElement) -> AnyElement {
        match self.config.content_width.max_px() {
            None => item,
            Some(max) => div().w_full().flex().justify_center().child(div().w_full().max_w(px(max)).child(item)).into_any_element(),
        }
    }

    pub fn set_content_width(&mut self, width: ContentWidth, cx: &mut Context<Self>) {
        self.config.content_width = width;
        self.save_config();
        // Rows wrap at the new width, so every height changes.
        self.list.remeasure();
        cx.notify();
    }

    pub fn cycle_content_width(&mut self, _: &CycleContentWidth, _: &mut Window, cx: &mut Context<Self>) {
        let width = self.config.content_width.next();
        self.set_content_width(width, cx);
        self.toast(t!("toast.content_width", width = width.label()));
    }

    pub fn apply_theme(&mut self, window: &Window) {
        self.theme = Theme::build(window.appearance(), &self.config);
    }

    pub(super) fn render_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
        let width = self.config.content_width;
        let widths = crate::ui::segmented::track(theme).children(ContentWidth::ALL.into_iter().map(|w| {
            crate::ui::segmented::segment(theme, SharedString::from(format!("width-{}", w.id())), w == width)
                .child(w.label())
                .on_click(cx.listener(move |this, _, _, cx| this.set_content_width(w, cx)))
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
            .rounded(px(crate::ui::theme::R_LG))
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
        let os = crate::app::platform::name();
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
                t!("settings.appearance.width.label"),
                t!("settings.appearance.width.hint", shortcut = keymap::hint("content-width", &self.config.keys)).into(),
                widths.into_any_element(),
            ))
            .child(field(
                t!("settings.appearance.language.label"),
                t!("settings.appearance.language.hint", os = os).into(),
                languages.into_any_element(),
            ))
            .child(preview)
            .into_any_element()
    }
}
