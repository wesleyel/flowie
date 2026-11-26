//! Trash, release notes, and their shared outline chrome.

pub(crate) mod trash;
pub(crate) mod whats_new;

use gpui::{div, prelude::*, px, Context, Div, FontWeight};

use crate::app::i18n::t;
use crate::outline::{BlockSel, Edit, Outline, Target, TITLE_SIZE};
use crate::ui::button::{button, Variant};

struct OutlineSelection {
    edit: Option<Edit>,
    block: Option<BlockSel>,
}

impl Outline {
    pub(crate) fn is_auxiliary_page_open(&self) -> bool {
        self.trash.is_some() || self.volatile_page.is_some()
    }

    pub(crate) fn close_auxiliary_page(&mut self, restore: bool, cx: &mut Context<Self>) -> bool {
        self.close_whats_new(restore, cx) || self.close_trash(restore, cx)
    }

    fn suspend_outline_selection(&mut self) -> OutlineSelection {
        self.palette = None;
        self.inline = None;
        self.calendar = None;
        self.menu = None;
        self.popover.close();
        self.drop = None;
        self.mouse_block = false;
        OutlineSelection { edit: self.edit.take(), block: self.block.take() }
    }

    fn finish_auxiliary_page(&mut self, selection: OutlineSelection, restore: bool, cx: &mut Context<Self>) {
        self.edit = None;
        self.block = None;
        if restore {
            if let Some(mut edit) = selection.edit {
                let text = match edit.target {
                    Target::Text(id) if self.ws.tree.contains(id) => Some(self.ws.tree.node(self.ws.tree.source(id)).text.as_str()),
                    Target::Note(id) if self.ws.tree.contains(id) => Some(self.ws.tree.node(self.ws.tree.source(id)).note.as_str()),
                    Target::Search => Some(self.query.as_str()),
                    _ => None,
                };
                if let Some(text) = text {
                    edit.sel =
                        text.floor_char_boundary(edit.sel.start.min(text.len()))..text.floor_char_boundary(edit.sel.end.min(text.len()));
                    edit.marked = None;
                    edit.layout = None;
                    edit.bounds = None;
                    edit.selecting_with_mouse = false;
                    self.edit = Some(edit);
                }
            }
            self.block = selection.block.filter(|block| self.ws.tree.contains(block.anchor) && self.ws.tree.contains(block.head));
        }
        cx.notify();
    }

    fn render_auxiliary_title(&self, title: &'static str, back_id: &'static str, cx: &Context<Self>) -> Div {
        let theme = self.theme;
        div()
            .flex()
            .items_center()
            .justify_between()
            .pt_2()
            .pb_3()
            .child(div().text_size(px(TITLE_SIZE)).font_weight(FontWeight::BOLD).child(title))
            .child(
                button(theme, back_id, Variant::Plain)
                    .child(crate::ui::icons::icon("arrow-left", crate::ui::icons::SM, Variant::Plain.fg(theme)))
                    .child(t!("common.back"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_auxiliary_page(true, cx);
                    })),
            )
    }
}
