//! Segmented controls (theme, language, agent access).

use gpui::{div, prelude::*, px, Div, ElementId, Stateful};

use crate::ui::theme::{Theme, R_MD};

/// The track the segments sit in.
pub fn track(theme: Theme) -> Div {
    div().self_start().flex().flex_wrap().p(px(1.)).gap(px(1.)).rounded(px(R_MD)).bg(theme.control_fill).text_size(px(13.))
}

/// One segment; the selected one is raised.
pub fn segment(theme: Theme, id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(22.))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_MD - 1.))
        .cursor_pointer()
        .text_color(theme.text)
        .when(selected, |d| d.bg(theme.control_selected).shadow(theme.control_shadow()).font_weight(gpui::FontWeight::MEDIUM))
        .when(!selected, |d| d.hover(|s| s.bg(theme.hover)).active(|s| s.bg(theme.pressed)))
}
