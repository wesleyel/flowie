//! The in-window menu bar, for platforms without a global one (Windows and
//! Linux; see `platform::MENU_BAR_IN_WINDOW`). Built from the same
//! `keymap::app_menus()` table that feeds macOS's native menu bar.

use gpui::{anchored, deferred, div, prelude::*, px, AnyElement, Context, MouseButton, MouseDownEvent, SharedString, Window};

use crate::keymap::{self, AppMenuItem};
use crate::outline::Outline;
use crate::theme::R_SM;

const MENU_BAR_H: f32 = 28.0;

/// Height the menu bar takes at the top of the window (0 with a global menu bar).
pub const fn bar_height() -> f32 {
    if crate::platform::MENU_BAR_IN_WINDOW {
        MENU_BAR_H
    } else {
        0.0
    }
}
const ROW: &str = "app-menu-row";

impl Outline {
    pub fn close_app_menu(&mut self) -> bool {
        self.app_menu.take().is_some()
    }

    /// The menu bar, when this platform draws one in the window.
    pub fn render_app_menu_bar(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !crate::platform::MENU_BAR_IN_WINDOW {
            return None;
        }
        let theme = self.theme;
        let open = self.app_menu;
        let menus = keymap::app_menus();
        let keys = self.config.keys.clone();
        let max_h = (window.viewport_size().height - px(MENU_BAR_H + 16.)).max(px(120.));

        let mut buttons: Vec<AnyElement> = Vec::with_capacity(menus.len());
        for (ix, menu) in menus.into_iter().enumerate() {
            let is_open = open == Some(ix);
            let dropdown = is_open.then(|| {
                let mut rows: Vec<AnyElement> = Vec::with_capacity(menu.items.len());
                for (item_ix, entry) in menu.items.into_iter().enumerate() {
                    match entry {
                        AppMenuItem::Separator => {
                            rows.push(crate::ui::menu::separator(theme).into_any_element());
                        }
                        AppMenuItem::Action { label, shortcut_id, action } => {
                            let shortcut = shortcut_id.and_then(|id| keymap::keys_for(id, &keys).into_iter().next());
                            let on = theme.on_accent;
                            rows.push(
                                div()
                                    .id(("app-menu-item", ix * 100 + item_ix))
                                    .group(ROW)
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_4()
                                    .px_2()
                                    .h(px(26.))
                                    .rounded(px(crate::theme::R_MD))
                                    .cursor_pointer()
                                    .text_color(theme.text)
                                    .hover(|s| s.bg(theme.accent).text_color(on))
                                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(label))
                                    .children(shortcut.map(|k| {
                                        crate::ui::controls::shortcut(&k, theme.muted.opacity(0.75), Some((ROW, on.opacity(0.85))))
                                    }))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.app_menu = None;
                                        window.dispatch_action(action(), cx);
                                        cx.notify();
                                    }))
                                    .into_any_element(),
                            );
                        }
                    }
                }
                let card = crate::ui::menu::card(theme, SharedString::from(format!("app-menu-card-{ix}")))
                    .w(px(250.))
                    .max_h(max_h)
                    .overflow_y_scroll()
                    .track_scroll(&self.app_menu_scroll)
                    .on_mouse_down_out(cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        // Clicks on the top bar itself are handled by the menu buttons.
                        if f32::from(ev.position.y) < MENU_BAR_H {
                            return;
                        }
                        if this.close_app_menu() {
                            cx.notify();
                        }
                    }))
                    .children(rows);
                let scrolled = crate::ui::scrollbar::area("app-menu-scroll-bar", &self.app_menu_scroll, theme, card);
                div()
                    .absolute()
                    .top(px(MENU_BAR_H - 3.))
                    .left_0()
                    .child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(scrolled)).with_priority(3))
                    .into_any_element()
            });

            buttons.push(
                div()
                    .relative()
                    .child(
                        div()
                            .id(("app-menu-btn", ix))
                            .h(px(22.))
                            .px_2()
                            .rounded(px(R_SM))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .when(is_open, |d| d.bg(theme.hover))
                            .hover(|s| s.bg(theme.hover))
                            .child(menu.name)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.close_menu();
                                    this.popover.close();
                                    this.app_menu = if this.app_menu == Some(ix) { None } else { Some(ix) };
                                    this.app_menu_scroll = gpui::ScrollHandle::new();
                                    cx.notify();
                                }),
                            )
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if *hovered && this.app_menu.is_some() && this.app_menu != Some(ix) {
                                    this.app_menu = Some(ix);
                                    this.app_menu_scroll = gpui::ScrollHandle::new();
                                    cx.notify();
                                }
                            })),
                    )
                    .children(dropdown)
                    .into_any_element(),
            );
        }

        Some(
            div()
                .id("app-menu-bar")
                .flex_none()
                .h(px(MENU_BAR_H))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(2.))
                .bg(theme.bg)
                .border_b_1()
                .border_color(theme.divider)
                .text_size(px(12.5))
                .text_color(theme.text)
                .children(buttons)
                .into_any_element(),
        )
    }
}
