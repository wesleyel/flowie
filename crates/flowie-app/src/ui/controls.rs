//! Small controls: the to-do checkbox, key caps and the search field.

use gpui::{div, prelude::*, px, radians, AnyElement, Div, ElementId, Hsla, Stateful, Transformation};

use crate::app::platform::{self, Os};
use crate::ui::icons::{icon, KEY};
use crate::ui::theme::{Theme, R_MD, R_SM};

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

/// One token of a shortcut: an icon name (key glyph) or a text label.
enum KeyToken {
    Icon(&'static str, f32),
    Text(String),
}

/// The pieces of one keystroke: macOS's key glyphs as icons there, the
/// platform's key names elsewhere (see `platform::key_name`).
fn tokens(stroke: &str) -> Vec<KeyToken> {
    use KeyToken::{Icon, Text};
    if stroke.starts_with("mouse-") {
        return vec![Text(platform::key_name(stroke))];
    }
    let mac = platform::OS == Os::Mac;
    let (mods, key) = platform::split_stroke(stroke);
    let mut out: Vec<KeyToken> = mods
        .iter()
        .map(|m| match (mac, *m) {
            (true, "ctrl") => Icon("chevron-up", 0.0),
            (true, "alt") => Icon("option", 0.0),
            (true, "shift") => Icon("arrow-big-up", 0.0),
            (true, "cmd") => Icon("command", 0.0),
            (_, m) => Text(platform::modifier_name(m).into()),
        })
        .collect();
    out.push(match (mac, key) {
        (_, "up") => Icon("arrow-up", 0.0),
        (_, "down") => Icon("arrow-down", 0.0),
        (_, "left") => Icon("arrow-left", 0.0),
        (_, "right") => Icon("arrow-right", 0.0),
        (true, "enter") => Icon("corner-down-left", 0.0),
        (true, "backspace") => Icon("delete", 0.0),
        (true, "delete") => Icon("delete", 180.0),
        (true, "tab") => Icon("arrow-right-to-line", 0.0),
        (true, "space") => Icon("space", 0.0),
        (true, "escape") => Text("esc".into()),
        (true, k) if k.len() > 1 => Text(k.to_string()),
        (_, k) => Text(platform::key_name(k)),
    });
    out
}

/// A shortcut ("cmd-shift-up") as the platform writes it: bare glyphs on
/// macOS, as menus show it; "Ctrl + Shift + ↑" elsewhere.
/// With `hover`, the glyphs take that color while the named group is hovered.
pub fn shortcut(keys: &str, color: Hsla, hover: Option<(&'static str, Hsla)>) -> Div {
    let strokes = keys.split_whitespace().map(|stroke| {
        let items = tokens(stroke);
        let sep = platform::OS != Os::Mac;
        let mut children: Vec<AnyElement> = Vec::with_capacity(items.len() * 2);
        for (ix, t) in items.into_iter().enumerate() {
            if sep && ix > 0 {
                children.push(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .line_height(px(KEY + 2.0))
                        .text_color(color.opacity(0.6))
                        .when_some(hover, |d, (group, c)| d.group_hover(group, |s| s.text_color(c)))
                        .child("+")
                        .into_any_element(),
                );
            }
            children.push(match t {
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
                    .text_size(px(if sep { 11. } else { 12. }))
                    .line_height(px(KEY + 2.0))
                    .text_color(color)
                    .when_some(hover, |d, (group, c)| d.group_hover(group, |s| s.text_color(c)))
                    .child(t)
                    .into_any_element(),
            });
        }
        div().flex().items_center().gap(px(if sep { 2. } else { 1. })).children(children)
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
