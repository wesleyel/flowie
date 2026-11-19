//! Small controls: the to-do checkbox, key caps and the search field.

use gpui::{div, prelude::*, px, radians, AnyElement, Div, ElementId, Hsla, Stateful, Transformation};

use crate::icons::{icon, KEY};
use crate::theme::{Theme, R_MD, R_SM};

/// A 16px checkbox.
pub fn checkbox(theme: Theme, checked: bool) -> Div {
    div()
        .flex_none()
        .size(px(16.))
        .rounded(px(R_SM))
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |d| d.bg(theme.accent).child(icon("check", 12., theme.on_accent)))
        .when(!checked, |d| d.bg(theme.control_selected).border_1().border_color(theme.faint).shadow(theme.control_shadow()))
}

/// One token of a shortcut: an icon name (macOS key glyph) or a text label.
enum KeyToken {
    Icon(&'static str, f32),
    Text(String),
}

fn tokens(stroke: &str) -> Vec<KeyToken> {
    match stroke {
        "mouse-back" => return vec![KeyToken::Text(crate::i18n::t!("settings.shortcuts.mouse_back").into())],
        "mouse-forward" => return vec![KeyToken::Text(crate::i18n::t!("settings.shortcuts.mouse_forward").into())],
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

/// A shortcut ("cmd-shift-up") as bare macOS key glyphs, as menus show it.
/// With `hover`, the glyphs take that color while the named group is hovered.
pub fn shortcut(keys: &str, color: Hsla, hover: Option<(&'static str, Hsla)>) -> Div {
    let strokes = keys.split_whitespace().map(|stroke| {
        div().flex().items_center().gap(px(1.)).children(tokens(stroke).into_iter().map(|t| {
            match t {
                KeyToken::Icon(name, angle) => {
                    let mut glyph = icon(name, KEY, color).with_transformation(Transformation::rotate(radians(angle.to_radians())));
                    if let Some((group, c)) = hover {
                        glyph = glyph.group_hover(group, |s| s.text_color(c));
                    }
                    div().flex_none().size(px(KEY + 2.0)).flex().items_center().justify_center().child(glyph).into_any_element()
                }
                KeyToken::Text(t) => div()
                    .flex_none()
                    .min_w(px(KEY + 2.0))
                    .flex()
                    .justify_center()
                    .text_size(px(12.))
                    .line_height(px(KEY + 2.0))
                    .text_color(color)
                    .when_some(hover, |d, (group, c)| d.group_hover(group, |s| s.text_color(c)))
                    .child(t)
                    .into_any_element(),
            }
        }))
    });
    div().flex_none().flex().items_center().gap(px(4.)).children(strokes)
}

/// A key cap: the shortcut's glyphs on an inset cap. `border` marks a
/// state (recording, conflict); `None` uses the plain divider.
pub fn keycap(theme: Theme, keys: &str, border: Option<Hsla>) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .h(px(20.))
        .px(px(4.))
        .rounded(px(R_SM))
        .bg(theme.input_bg)
        .border_1()
        .border_color(border.unwrap_or(theme.border))
        .child(shortcut(keys, theme.text.opacity(0.85), None))
        .into_any_element()
}

/// The search field's frame; `focused` adds the accent border and ring.
pub fn search_field(theme: Theme, id: impl Into<ElementId>, focused: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(px(R_MD))
        .bg(theme.input_bg)
        .border_1()
        .text_size(px(13.))
        .cursor_text()
        .when(focused, |d| d.border_color(theme.accent).shadow(theme.focus_ring()))
        .when(!focused, |d| d.border_color(theme.border))
}
