//! Lucide icons (ISC license, see assets/icons/LICENSE), embedded at build
//! time. Every icon sits in a fixed square slot so lists line up.

use gpui::{div, prelude::*, px, svg, Hsla, SharedString, Svg};

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// Icon sizes. Lists, menus and toolbars use `MD`; dense UI and key caps `SM`.
pub const MD: f32 = 16.0;
pub const SM: f32 = 13.0;
pub const KEY: f32 = 11.0;

pub fn path(name: &str) -> SharedString {
    format!("icons/{name}.svg").into()
}

/// A bare icon at `size`.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(path(name)).size(px(size)).flex_none().text_color(color)
}

/// An icon centered in a square slot (slot = size + 4), for aligned lists.
pub fn slot(name: &str, size: f32, color: Hsla) -> gpui::Div {
    div().flex_none().size(px(size + 4.0)).flex().items_center().justify_center().child(icon(name, size, color))
}
