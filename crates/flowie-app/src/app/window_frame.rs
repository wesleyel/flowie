//! Window decorations on Linux. Flowie asks for server-side decorations, which KDE, sway,
//! Hyprland and X11 window managers draw. GNOME doesn't offer them on Wayland, so there the
//! window draws its own (client-side decorations): a border and shadow with resize edges
//! around the window (`frame`), and the in-window menu bar doubles as the title bar, which
//! moves the window, maximizes it on a double click and carries the window buttons
//! (`title_bar_handlers`, `window_buttons`). On macOS and Windows none of this applies.

use std::cell::Cell;

use gpui::{
    div, point, prelude::*, px, size, AnyElement, App, BoxShadow, CursorStyle, Decorations, Edges, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, ResizeEdge, Size, Stateful, Tiling, Window, WindowButton, WindowButtonLayout,
};

use crate::app::platform::{Os, OS};
use crate::ui::theme::Theme;

/// The shadow around a client-decorated window, outside its visible frame. The compositor
/// treats this band as outside the window (`Window::set_client_inset`).
const SHADOW: f32 = 10.;
/// The visible frame's border.
const BORDER: f32 = 1.;
/// How far either side of the visible edge a press starts resizing.
const RESIZE_BAND: f32 = 5.;

/// The window's tiling when Flowie draws the decorations itself; `None` when the platform
/// or the window manager draws them.
pub fn client_tiling(window: &Window) -> Option<Tiling> {
    if OS != Os::Linux {
        return None;
    }
    match window.window_decorations() {
        Decorations::Client { tiling } => Some(tiling),
        Decorations::Server => None,
    }
}

/// The shadow band on each side (none against a tiled edge, or a maximized window).
fn shadow_insets(tiling: Tiling) -> Edges<Pixels> {
    let side = |tiled: bool| px(if tiled { 0. } else { SHADOW });
    Edges { top: side(tiling.top), right: side(tiling.right), bottom: side(tiling.bottom), left: side(tiling.left) }
}

/// Where the content starts below the window's top edge: the shadow and border above it.
pub fn content_top(window: &Window) -> f32 {
    match client_tiling(window) {
        Some(t) if !t.top => SHADOW + BORDER,
        _ => 0.,
    }
}

/// `content` inside the window's frame: as is, unless Flowie draws the decorations.
pub fn frame(content: impl IntoElement, theme: Theme, window: &mut Window) -> AnyElement {
    if OS != Os::Linux {
        return content.into_any_element();
    }
    // The same root either way, so element state survives a change of decorations.
    let root = div().id("window-frame").size_full();
    let Some(tiling) = client_tiling(window) else {
        return root.child(content).into_any_element();
    };
    // Keep the inset while maximized: clearing it makes the next restore count the shadow twice.
    window.set_client_inset(px(SHADOW));
    let insets = shadow_insets(tiling);
    let active = window.is_window_active();
    let shadow = |alpha: f32, blur: f32, y: f32| BoxShadow {
        color: gpui::black().opacity(if active { alpha } else { alpha * 0.6 }),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    };
    let visible = div()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .bg(theme.bg)
        .border_color(if theme.dark { theme.border } else { theme.divider })
        .when(!tiling.top, |d| d.border_t(px(BORDER)))
        .when(!tiling.bottom, |d| d.border_b(px(BORDER)))
        .when(!tiling.left, |d| d.border_l(px(BORDER)))
        .when(!tiling.right, |d| d.border_r(px(BORDER)))
        .when(tiling != Tiling::tiled(), |d| d.shadow(vec![shadow(0.22, SHADOW - 2., 2.), shadow(0.18, 3., 1.)]))
        .child(content);
    let zones = if window.is_resizable() { resize_zones(window.viewport_size(), insets, tiling) } else { Vec::new() };
    root.flex()
        .flex_col()
        .pt(insets.top)
        .pr(insets.right)
        .pb(insets.bottom)
        .pl(insets.left)
        .child(visible)
        .children(zones)
        .into_any_element()
}

/// One invisible strip per resizable edge and corner, in window coordinates (the root has no
/// border, so its padding box starts at the window's origin). Corners come last, on top.
fn resize_zones(window: Size<Pixels>, insets: Edges<Pixels>, tiling: Tiling) -> Vec<AnyElement> {
    let band = px(RESIZE_BAND);
    let (left, top) = (insets.left - band, insets.top - band);
    let (right, bottom) = (window.width - insets.right - band, window.height - insets.bottom - band);
    let (wide, tall) = (right - left + band * 2., bottom - top + band * 2.);
    let mut zones = Vec::new();
    let mut zone = |edge: ResizeEdge, at: Point<Pixels>, extent: Size<Pixels>| {
        zones.push(
            div()
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(extent.width)
                .h(extent.height)
                .cursor(resize_cursor(edge))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    MOVING.set(false);
                    window.start_window_resize(edge);
                })
                .into_any_element(),
        );
    };
    let edge_h = size(wide, band * 2.);
    let edge_v = size(band * 2., tall);
    let corner = size(band * 3., band * 3.);
    if !tiling.top {
        zone(ResizeEdge::Top, point(left, top), edge_h);
    }
    if !tiling.bottom {
        zone(ResizeEdge::Bottom, point(left, bottom), edge_h);
    }
    if !tiling.left {
        zone(ResizeEdge::Left, point(left, top), edge_v);
    }
    if !tiling.right {
        zone(ResizeEdge::Right, point(right, top), edge_v);
    }
    let far_x = right - band;
    let far_y = bottom - band;
    if !tiling.top && !tiling.left {
        zone(ResizeEdge::TopLeft, point(left, top), corner);
    }
    if !tiling.top && !tiling.right {
        zone(ResizeEdge::TopRight, point(far_x, top), corner);
    }
    if !tiling.bottom && !tiling.left {
        zone(ResizeEdge::BottomLeft, point(left, far_y), corner);
    }
    if !tiling.bottom && !tiling.right {
        zone(ResizeEdge::BottomRight, point(far_x, far_y), corner);
    }
    zones
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

thread_local! {
    /// The title bar was pressed and the pointer hasn't moved yet: the first move hands the
    /// window to the compositor. Waiting for it keeps double clicks and plain clicks working.
    static MOVING: Cell<bool> = const { Cell::new(false) };
}

/// Makes `bar` the window's title bar when Flowie draws the decorations: dragging it moves
/// the window, a double click maximizes or restores it, and a right click opens the window
/// manager's window menu. Controls on the bar stop their presses from reaching it.
pub fn title_bar_handlers(bar: Stateful<gpui::Div>, window: &Window) -> Stateful<gpui::Div> {
    if client_tiling(window).is_none() {
        return bar;
    }
    bar.on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, _| {
        if ev.click_count == 2 {
            MOVING.set(false);
            window.zoom_window();
        } else {
            MOVING.set(true);
        }
    })
    .on_mouse_up(MouseButton::Left, |_, _, _| MOVING.set(false))
    .on_mouse_move(|ev: &MouseMoveEvent, window, _| {
        if ev.pressed_button != Some(MouseButton::Left) {
            MOVING.set(false);
        } else if MOVING.replace(false) {
            window.start_window_move();
        }
    })
    .on_mouse_down(MouseButton::Right, |ev: &MouseDownEvent, window, cx| {
        cx.stop_propagation();
        window.show_window_menu(ev.position);
    })
}

/// The desktop's title bar button layout (GNOME's `button-layout`), else minimize, maximize
/// and close on the right; only the buttons the window manager supports.
fn button_layout(window: &Window, cx: &App) -> WindowButtonLayout {
    let supported = window.window_controls();
    let mut layout = cx.button_layout().unwrap_or(WindowButtonLayout {
        left: Default::default(),
        right: [Some(WindowButton::Minimize), Some(WindowButton::Maximize), Some(WindowButton::Close)],
    });
    for slot in layout.left.iter_mut().chain(layout.right.iter_mut()) {
        let keep = match *slot {
            Some(WindowButton::Minimize) => supported.minimize,
            Some(WindowButton::Maximize) => supported.maximize,
            _ => true,
        };
        if !keep {
            *slot = None;
        }
    }
    layout
}

/// The window buttons for one side of the title bar, when Flowie draws the decorations.
pub fn window_buttons(left: bool, theme: Theme, window: &Window, cx: &App) -> Option<AnyElement> {
    client_tiling(window)?;
    let layout = button_layout(window, cx);
    let side = if left { layout.left } else { layout.right };
    let buttons: Vec<AnyElement> = side.into_iter().flatten().map(|b| window_button(b, theme, window)).collect();
    if buttons.is_empty() {
        return None;
    }
    Some(div().flex_none().flex().items_center().gap(px(6.)).children(buttons).into_any_element())
}

fn window_button(button: WindowButton, theme: Theme, window: &Window) -> AnyElement {
    let (id, icon) = match button {
        WindowButton::Minimize => ("window-minimize", "minus"),
        WindowButton::Maximize if window.is_maximized() => ("window-maximize", "copy"),
        WindowButton::Maximize => ("window-maximize", "square"),
        WindowButton::Close => ("window-close", "x"),
    };
    crate::ui::button::icon_button(theme, id, icon, theme.text)
        .rounded_full()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            match button {
                WindowButton::Minimize => window.minimize_window(),
                WindowButton::Maximize => window.zoom_window(),
                // Like the window manager's close button: respect close-to-tray and unsaved checks.
                WindowButton::Close => window.dispatch_action(Box::new(crate::outline::CloseWindow), cx),
            }
        })
        .into_any_element()
}
