//! Floating surfaces: popovers, sheets over a scrim, and toasts.

use gpui::{div, prelude::*, px, Div, ElementId, MouseButton, SharedString, Stateful};

use crate::ui::theme::{Theme, R_XL};

/// A popover or floating panel (calendar, link popover, quick open).
pub fn popover(theme: Theme, id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .occlude()
        .rounded(px(R_XL))
        .bg(theme.bg)
        .border_1()
        .border_color(theme.border)
        .shadow(theme.popover_shadow())
        .text_size(px(13.))
        .text_color(theme.text)
}

/// The dimmed layer behind a sheet; it takes the clicks outside it.
pub fn scrim(theme: Theme) -> Div {
    div().absolute().inset_0().flex().justify_center().bg(theme.scrim).occlude()
}

/// A sheet on a scrim; clicks inside it don't reach the scrim.
pub fn sheet(theme: Theme, id: impl Into<ElementId>) -> Stateful<Div> {
    popover(theme, id).overflow_hidden().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A transient message at the bottom of the window.
pub fn toast(theme: Theme, message: SharedString, opacity: f32) -> Div {
    div().absolute().bottom(px(22.)).left_0().right_0().flex().justify_center().child(
        div()
            .px(px(14.))
            .py(px(8.))
            .rounded(px(16.))
            .bg(theme.bg)
            .border_1()
            .border_color(theme.border)
            .shadow(theme.popover_shadow())
            .text_color(theme.text)
            .text_size(px(13.))
            .opacity(opacity)
            .child(message),
    )
}
