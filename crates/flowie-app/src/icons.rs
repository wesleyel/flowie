//! Lucide icons (ISC license, see assets/icons/LICENSE), embedded at build
//! time. Every icon sits in a fixed square slot so lists line up.

use gpui::{div, prelude::*, px, radians, svg, AnyElement, Hsla, SharedString, Svg, Transformation};

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
    div()
        .flex_none()
        .size(px(size + 4.0))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(name, size, color))
}

/// One token of a shortcut: an icon name (macOS key glyph) or a text label.
enum KeyToken {
    Icon(&'static str, f32),
    Text(String),
}

fn tokens(stroke: &str) -> Vec<KeyToken> {
    match stroke {
        "mouse-back" => return vec![KeyToken::Text("Mouse Back".into())],
        "mouse-forward" => return vec![KeyToken::Text("Mouse Forward".into())],
        _ => {}
    }
    let parts: Vec<&str> = stroke.split('-').collect();
    let (mods, key) = if stroke.ends_with("--") {
        (&parts[..parts.len().saturating_sub(2)], "-")
    } else {
        (&parts[..parts.len() - 1], *parts.last().unwrap_or(&""))
    };
    let mut out = Vec::new();
    // macOS order: ⌃ ⌥ ⇧ ⌘
    for (m, icon) in [("ctrl", "chevron-up"), ("alt", "option"), ("shift", "arrow-big-up"), ("cmd", "command")] {
        if mods.contains(&m) {
            out.push(KeyToken::Icon(icon, 0.0));
        }
    }
    if mods.contains(&"fn") {
        out.push(KeyToken::Text("fn".into()));
    }
    out.push(match key {
        "enter" => KeyToken::Icon("corner-down-left", 0.0),
        "backspace" => KeyToken::Icon("delete", 0.0),
        "delete" => KeyToken::Icon("delete", 180.0),
        "tab" => KeyToken::Icon("arrow-right-to-line", 0.0),
        "space" => KeyToken::Icon("space", 0.0),
        "up" => KeyToken::Icon("arrow-up", 0.0),
        "down" => KeyToken::Icon("arrow-down", 0.0),
        "left" => KeyToken::Icon("arrow-left", 0.0),
        "right" => KeyToken::Icon("arrow-right", 0.0),
        "escape" => KeyToken::Text("esc".into()),
        "home" => KeyToken::Text("home".into()),
        "end" => KeyToken::Text("end".into()),
        k if k.len() > 1 => KeyToken::Text(k.to_string()),
        k => KeyToken::Text(k.to_uppercase()),
    });
    out
}

/// A key cap showing a shortcut ("cmd-shift-up") with macOS key icons.
pub fn key_chip(keys: &str, color: Hsla, bg: Hsla, border: Hsla) -> AnyElement {
    let strokes: Vec<AnyElement> = keys
        .split_whitespace()
        .map(|stroke| {
            div()
                .flex()
                .items_center()
                .gap(px(1.))
                .children(tokens(stroke).into_iter().map(|t| match t {
                    KeyToken::Icon(name, angle) => div()
                        .flex_none()
                        .size(px(KEY + 2.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(name, KEY, color).with_transformation(Transformation::rotate(radians(angle.to_radians()))))
                        .into_any_element(),
                    KeyToken::Text(t) => div()
                        .flex_none()
                        .min_w(px(KEY + 2.0))
                        .flex()
                        .justify_center()
                        .text_size(px(11.))
                        .line_height(px(KEY + 2.0))
                        .text_color(color)
                        .child(t)
                        .into_any_element(),
                }))
                .into_any_element()
        })
        .collect();
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .h(px(20.))
        .px(px(5.))
        .rounded(px(5.))
        .bg(bg)
        .border_1()
        .border_color(border)
        .children(strokes)
        .into_any_element()
}
