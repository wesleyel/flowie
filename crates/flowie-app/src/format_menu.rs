//! Right-click menu on a text selection in the bullet being edited: inline
//! styles (with their shortcuts) and a row of text colors. Each entry
//! toggles, like the shortcuts: applying it to already-styled text removes it.

use flowie_core::markup::COLORS;
use gpui::{anchored, deferred, div, point, prelude::*, px, AnyElement, Context, MouseButton, MouseDownEvent, Pixels, Point, Window};

use crate::i18n::{t, text};
use crate::outline::{InsertLink, Outline};

#[derive(Clone, Copy)]
enum Item {
    Wrap(&'static str, &'static str),
    Link,
}

/// (label key, icon, shortcut id, what it does).
const ITEMS: &[(&str, &str, Option<&str>, Item)] = &[
    ("slash.bold", "bold", Some("bold"), Item::Wrap("**", "**")),
    ("slash.italic", "italic", Some("italic"), Item::Wrap("*", "*")),
    ("slash.strikethrough", "strikethrough", None, Item::Wrap("~~", "~~")),
    ("slash.highlight", "highlighter", Some("highlight"), Item::Wrap("==", "==")),
    ("slash.inline_code", "code", Some("code"), Item::Wrap("`", "`")),
    ("slash.math", "sigma", Some("math"), Item::Wrap("$", "$")),
    ("shortcut.link", "link", Some("link"), Item::Link),
];

impl Outline {
    /// Open the menu at `at` for the current selection (if there is one).
    pub fn open_format_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.inline = None;
        self.popover.close();
        self.format_menu = Some(at);
        cx.notify();
    }

    fn run_format_item(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.format_menu = None;
        match item {
            Item::Wrap(open, close) => self.wrap(open, close, cx),
            Item::Link => self.insert_link(&InsertLink, window, cx),
        }
        cx.notify();
    }

    fn run_format_color(&mut self, name: &str, cx: &mut Context<Self>) {
        self.format_menu = None;
        self.wrap(&format!("{{{name}}}"), "{/}", cx);
        cx.notify();
    }

    pub fn render_format_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let at = self.format_menu?;
        // Only while there is a selection to style.
        if self.edit.as_ref().is_none_or(|e| e.sel.is_empty()) {
            self.format_menu = None;
            return None;
        }
        let theme = self.theme;
        let rows = ITEMS.iter().enumerate().map(|(i, &(label, icon, keys, item))| {
            let chip = keys
                .and_then(|id| crate::keymap::keys_for(id, &self.config.keys).into_iter().next())
                .map(|k| crate::icons::key_chip(&k, theme.muted, theme.bg, theme.border));
            div()
                .id(("format-item", i))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .h(px(28.))
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(theme.hover))
                .child(crate::icons::slot(icon, crate::icons::MD, theme.muted))
                .child(div().flex_1().whitespace_nowrap().child(text(label)))
                .children(chip)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.run_format_item(item, window, cx)))
        });
        let swatches = COLORS.iter().enumerate().map(|(i, &name)| {
            div()
                .id(("format-color", i))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .hover(|s| s.bg(theme.hover))
                .child(div().size(px(12.)).rounded_full().bg(theme.colors[i]))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| this.run_format_color(name, cx)))
        });
        let card = div()
            .id("format-menu")
            .occlude()
            .w(px(220.))
            .p_1()
            .rounded_md()
            .bg(theme.bg)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                if this.format_menu.take().is_some() {
                    cx.notify();
                }
            }))
            .children(rows)
            .child(div().h(px(1.)).my_1().bg(theme.border))
            .child(div().px_2().pt_1().text_size(px(11.)).text_color(theme.muted).child(t!("format_menu.color")))
            .child(div().flex().justify_between().px_1().py_1().children(swatches));
        Some(
            deferred(anchored().position(at + point(px(2.), px(2.))).snap_to_window_with_margin(px(8.)).child(card))
                .with_priority(2)
                .into_any_element(),
        )
    }
}
