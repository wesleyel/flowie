//! Menus: the right-click menus and the inline completion list.

use gpui::{anchored, deferred, div, point, prelude::*, px, AnyElement, Div, ElementId, Hsla, MouseButton, Pixels, Point, Stateful};

use crate::theme::{Theme, R_MD, R_XL};

pub const MENU_W: f32 = 240.;
/// Rows show their hover through this group, so the icon and shortcut can
/// turn white with the label.
const ROW: &str = "menu-row";
const ICON: f32 = 14.;

/// The floating card a menu sits in.
pub fn card(theme: Theme, id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .occlude()
        .w(px(MENU_W))
        .p(px(6.))
        .flex()
        .flex_col()
        .rounded(px(R_XL))
        .bg(theme.bg)
        .border_1()
        .border_color(theme.border)
        .shadow(theme.popover_shadow())
        .text_size(px(13.))
        .text_color(theme.text)
}

/// One menu item: icon, label, and its shortcut (if bound). Hovered, it
/// fills with the accent (or red, for `danger`) and turns white.
pub fn row(theme: Theme, id: impl Into<ElementId>, icon: &str, label: String, shortcut: Option<String>, danger: bool) -> Stateful<Div> {
    let (fg, icon_fg, fill) = if danger { (theme.danger, theme.danger, theme.danger) } else { (theme.text, theme.muted, theme.accent) };
    let on = theme.on_accent;
    div()
        .id(id)
        .group(ROW)
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(28.))
        .rounded(px(R_MD))
        .cursor_pointer()
        .text_color(fg)
        .hover(|s| s.bg(fill).text_color(on))
        .child(
            div()
                .flex_none()
                .size(px(ICON + 4.))
                .flex()
                .items_center()
                .justify_center()
                .child(crate::icons::icon(icon, ICON, icon_fg).group_hover(ROW, |s| s.text_color(on))),
        )
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(label))
        .children(shortcut.map(|k| super::controls::shortcut(&k, theme.muted.opacity(0.72), Some((ROW, on.opacity(0.85))))))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

pub fn separator(theme: Theme) -> Div {
    div().flex_none().py(px(4.)).px_2().child(div().h(px(1.)).bg(theme.divider))
}

/// A section label inside a menu.
pub fn heading(theme: Theme, label: String) -> Div {
    div()
        .flex_none()
        .px_2()
        .pt_1()
        .pb(px(2.))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme.muted)
        .child(label)
}

/// A small square choice in a row of choices (block types).
pub fn cell(theme: Theme, id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_MD))
        .cursor_pointer()
        .when(selected, |d| d.bg(theme.row_selected))
        .hover(|s| s.bg(theme.hover))
        .active(|s| s.bg(theme.pressed))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A color choice: a 12px dot in a 24px cell.
pub fn swatch(theme: Theme, id: impl Into<ElementId>, color: Hsla) -> Stateful<Div> {
    cell(theme, id, false).rounded_full().child(dot(color))
}

/// The 12px color dot, also used in list rows.
pub fn dot(color: Hsla) -> Div {
    div().size(px(12.)).rounded_full().bg(color)
}

/// Place a menu card at a window position. A menu taller than the window
/// scrolls, with an overlay bar.
pub fn place_menu(at: Point<Pixels>, card: Stateful<Div>, scroll: &gpui::ScrollHandle, theme: Theme, window: &gpui::Window) -> AnyElement {
    let max_h = window.viewport_size().height - px(16.);
    place(at, super::scrollbar::area("menu-bar", scroll, theme, card.max_h(max_h).overflow_y_scroll().track_scroll(scroll)))
}

/// Place a card at a window position, kept inside the window.
pub fn place(at: Point<Pixels>, card: impl IntoElement) -> AnyElement {
    deferred(anchored().position(at + point(px(2.), px(2.))).snap_to_window_with_margin(px(8.)).child(card))
        .with_priority(2)
        .into_any_element()
}
