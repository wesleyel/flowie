//! Text styles shared by the panels.

use gpui::{div, prelude::*, px, Div, FontWeight, SharedString};

use crate::theme::Theme;

/// A small section label above a group (palette title, settings sidebar).
pub fn overline(theme: Theme, label: impl Into<SharedString>) -> Div {
    div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.muted).child(label.into())
}

/// A settings page's title, with an optional hint under it.
pub fn page_title(theme: Theme, title: impl Into<SharedString>, hint: Option<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title.into()))
        .children(hint.map(|h| div().text_size(px(12.)).text_color(theme.muted).child(h)))
}
