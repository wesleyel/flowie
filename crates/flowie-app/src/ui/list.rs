//! Rows of a selectable list: quick open, completion, the settings sidebar.

use gpui::{div, prelude::*, px, Div, ElementId, Stateful};

use crate::theme::{Theme, R_MD};

/// A 28px list row. `selected` fills it with the accent and white text,
/// like a focused Mac list; otherwise it shows a hover fill.
pub fn row(theme: Theme, id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(28.))
        .rounded(px(R_MD))
        .cursor_pointer()
        .when(selected, |d| d.bg(theme.accent).text_color(theme.on_accent))
        .when(!selected, |d| d.hover(|s| s.bg(theme.hover)))
}

/// The icon color on a row: white on the accent fill.
pub fn icon_color(theme: Theme, selected: bool) -> gpui::Hsla {
    if selected {
        theme.on_accent
    } else {
        theme.muted
    }
}

/// Secondary text on a row (a path, a hint).
pub fn detail_color(theme: Theme, selected: bool) -> gpui::Hsla {
    if selected {
        theme.on_accent.opacity(0.8)
    } else {
        theme.muted
    }
}

/// A row-shaped region that only highlights on hover (references, shortcuts).
pub fn hover_row(theme: Theme, id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).rounded(px(R_MD)).cursor_pointer().hover(|s| s.bg(theme.hover))
}
