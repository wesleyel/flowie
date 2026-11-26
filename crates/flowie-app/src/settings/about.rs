//! The About tab and the associated help links.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Window};

use super::Tab;
use crate::app::i18n::t;
use crate::outline::{OpenAbout, OpenPrivacyPolicy, OpenTerms, Outline, SendFeedback};

const ABOUT_BOX_W: f32 = 500.0;
const ABOUT_PAD: f32 = 12.0;
const ABOUT_KEY_W: f32 = 90.0;
const ABOUT_GAP: f32 = 12.0;

impl Outline {
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

    pub(super) fn render_about(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.settings.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        let theme = self.theme;
        let version = env!("CARGO_PKG_VERSION");
        let commit = option_env!("FLOWIE_COMMIT").unwrap_or("dev");
        let db = self.db_path.clone();
        let db_label = db.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| t!("settings.about.in_memory").into());
        let nodes = self.ws.tree.len();
        let version_label = self.label("version", t!("settings.about.version", version = version, commit = commit), cx);
        let values = [
            (t!("settings.about.bullets"), self.label("bullets", format!("{nodes}"), cx)),
            (t!("settings.about.data"), self.label("data", db_label, cx)),
            (t!("settings.about.device"), self.label("device", self.device_id.clone(), cx)),
            (t!("settings.about.settings"), self.label("settings", self.config_path.display().to_string(), cx)),
        ];
        let line = |k: &'static str, v: gpui::Entity<crate::text::selectable::SelectableText>| {
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
                    .rounded(px(crate::ui::theme::R_LG))
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
                                .child(t!("settings.about.reveal", manager = crate::app::platform::file_manager()))
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
const FEEDBACK_EMAIL: &str = "wesleyel@icloud.com";
/// The GitHub Pages site (source in website/): home, privacy.html and terms.html.
const SITE_URL: &str = "https://wesleyel.github.io/flowie";

/// A `mailto:` link with the app version and OS version filled in.
fn feedback_url() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let commit = option_env!("FLOWIE_COMMIT").unwrap_or("dev");
    let os = crate::app::platform::os_version();
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
