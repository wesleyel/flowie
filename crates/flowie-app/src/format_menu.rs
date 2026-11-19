//! Right-click menu on text in the bullet being edited: cut / copy / paste,
//! inline styles (with their shortcuts) and a row of text colors. Style
//! entries toggle, like the shortcuts: applying one to already-styled text
//! removes it. Whole-bullet commands live in the bullet menu (`menu.rs`).

use flowie_core::markup::COLORS;
use gpui::{div, prelude::*, AnyElement, Context, MouseDownEvent, Pixels, Point, Window};

use crate::i18n::{t, text};
use crate::menu::{card, heading, place, row, separator, ContextMenu};
use crate::outline::{Copy, Cut, InsertLink, Outline, Paste};

#[derive(Clone, Copy)]
enum Item {
    Cut,
    Copy,
    Paste,
    Wrap(&'static str, &'static str),
    Link,
}

/// (label key, icon, shortcut id, what it does).
const CLIPBOARD: &[(&str, &str, &str, Item)] = &[
    ("shortcut.cut", "scissors", "cut", Item::Cut),
    ("shortcut.copy", "copy", "copy", Item::Copy),
    ("shortcut.paste", "corner-down-left", "paste", Item::Paste),
];

const STYLES: &[(&str, &str, Option<&str>, Item)] = &[
    ("slash.bold", "bold", Some("bold"), Item::Wrap("**", "**")),
    ("slash.italic", "italic", Some("italic"), Item::Wrap("*", "*")),
    ("slash.strikethrough", "strikethrough", None, Item::Wrap("~~", "~~")),
    ("slash.highlight", "highlighter", Some("highlight"), Item::Wrap("==", "==")),
    ("slash.inline_code", "code", Some("code"), Item::Wrap("`", "`")),
    ("slash.math", "sigma", Some("math"), Item::Wrap("$", "$")),
    ("shortcut.link", "link", Some("link"), Item::Link),
];

impl Outline {
    /// Open the menu at `at` for the edited bullet's text.
    pub fn open_format_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.close_menu();
        self.inline = None;
        self.popover.close();
        self.menu = Some(ContextMenu::Text { at });
        cx.notify();
    }

    fn run_format_item(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        match item {
            Item::Cut => self.cut(&Cut, window, cx),
            Item::Copy => self.copy(&Copy, window, cx),
            Item::Paste => self.paste(&Paste, window, cx),
            Item::Wrap(open, close) => self.wrap(open, close, cx),
            Item::Link => self.insert_link(&InsertLink, window, cx),
        }
        cx.notify();
    }

    fn run_format_color(&mut self, name: &str, cx: &mut Context<Self>) {
        self.menu = None;
        self.wrap(&format!("{{{name}}}"), "{/}", cx);
        cx.notify();
    }

    pub fn render_format_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) -> Option<AnyElement> {
        // Only while a bullet is being edited.
        let Some(edit) =
            self.edit.as_ref().filter(|e| matches!(e.target, crate::outline::Target::Text(_) | crate::outline::Target::Note(_)))
        else {
            self.menu = None;
            return None;
        };
        let has_sel = !edit.sel.is_empty();
        let theme = self.theme;
        let keys = &self.config.keys;
        let chip = |id: &str| crate::keymap::keys_for(id, keys).into_iter().next();

        // Without a selection only Paste applies.
        let clipboard =
            CLIPBOARD.iter().enumerate().filter(|(_, c)| has_sel || matches!(c.3, Item::Paste)).map(|(i, &(label, icon, keys, item))| {
                row(theme, ("format-clip", i), icon, text(label).into(), chip(keys), false)
                    .on_click(cx.listener(move |this, _, window, cx| this.run_format_item(item, window, cx)))
            });
        let mut c = card(theme, "format-menu")
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                if this.close_menu() {
                    cx.notify();
                }
            }))
            .children(clipboard);
        if has_sel {
            let styles = STYLES.iter().enumerate().map(|(i, &(label, icon, keys, item))| {
                row(theme, ("format-item", i), icon, text(label).into(), keys.and_then(chip), false)
                    .on_click(cx.listener(move |this, _, window, cx| this.run_format_item(item, window, cx)))
            });
            let swatches = COLORS.iter().enumerate().map(|(i, &name)| {
                crate::ui::menu::swatch(theme, ("format-color", i), theme.colors[i])
                    .on_click(cx.listener(move |this, _, _, cx| this.run_format_color(name, cx)))
            });
            c = c
                .child(separator(theme))
                .children(styles)
                .child(separator(theme))
                .child(heading(theme, t!("format_menu.color").into()))
                .child(div().flex().justify_between().px_1().py_1().children(swatches));
        }
        Some(place(at, c))
    }
}
