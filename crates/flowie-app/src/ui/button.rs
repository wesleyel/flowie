//! Buttons: icon-only toolbar buttons and push buttons with a label.

use gpui::{div, prelude::*, px, Div, ElementId, Hsla, Stateful};

use crate::theme::{Theme, R_MD};

/// A square icon button: 24px with a 14px glyph.
pub fn icon_button(theme: Theme, id: impl Into<ElementId>, icon: &str, color: Hsla) -> Stateful<Div> {
    square(theme, id, 24.).child(crate::icons::icon(icon, 14., color))
}

/// A toolbar button: 28px with a 16px glyph, matching the search field.
pub fn tool_button(theme: Theme, id: impl Into<ElementId>, icon: &str, color: Hsla) -> Stateful<Div> {
    square(theme, id, 28.).child(crate::icons::icon(icon, crate::icons::MD, color))
}

fn square(theme: Theme, id: impl Into<ElementId>, size: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_MD))
        .cursor_pointer()
        .hover(|s| s.bg(theme.hover))
        .active(|s| s.bg(theme.pressed))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Variant {
    /// A bordered push button (About's Reveal / Send Feedback).
    Secondary,
    /// Accent text, no fill until hovered (Restore, Reset All, +).
    Link,
    /// Secondary text, no fill until hovered (Back).
    Plain,
    /// Red text, no fill until hovered (Delete Forever).
    Destructive,
}

impl Variant {
    /// The label and icon color.
    pub fn fg(self, theme: Theme) -> Hsla {
        match self {
            Variant::Secondary => theme.text,
            Variant::Link => theme.link,
            Variant::Plain => theme.muted,
            Variant::Destructive => theme.danger,
        }
    }
}

/// A push button; add an icon (in `variant.fg(theme)`) and a label.
pub fn button(theme: Theme, id: impl Into<ElementId>, variant: Variant) -> Stateful<Div> {
    let b = div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .h(px(24.))
        .px(px(if variant == Variant::Secondary { 12. } else { 8. }))
        .rounded(px(R_MD))
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(variant.fg(theme))
        .whitespace_nowrap();
    match variant {
        Variant::Secondary => b
            .bg(theme.control_selected)
            .border_1()
            .border_color(theme.border)
            .shadow(theme.control_shadow())
            .hover(|s| s.bg(theme.control_selected.blend(theme.hover)))
            .active(|s| s.bg(theme.control_selected.blend(theme.pressed))),
        _ => b.hover(|s| s.bg(theme.hover)).active(|s| s.bg(theme.pressed)),
    }
}
