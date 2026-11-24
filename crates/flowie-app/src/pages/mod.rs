//! Temporary pages that replace the outline and restore its selection on close.

mod trash;
pub(crate) mod whats_new;

use gpui::{AnyElement, Context, ScrollHandle, Window};

use crate::app::i18n::t;
use crate::outline::{BlockSel, Edit, Outline, Target};
use trash::TrashPage;
use whats_new::WhatsNewPage;

enum VolatilePage {
    Trash(TrashPage),
    WhatsNew(Box<WhatsNewPage>),
}

pub struct PageSession {
    page: VolatilePage,
    scroll: ScrollHandle,
    return_edit: Option<Edit>,
    return_block: Option<BlockSel>,
}

impl PageSession {
    pub(crate) fn title(&self) -> &'static str {
        match self.page {
            VolatilePage::Trash(_) => t!("trash.title"),
            VolatilePage::WhatsNew(_) => t!("whats_new.title"),
        }
    }
}

impl Outline {
    fn enter_volatile_page(&mut self, page: VolatilePage, cx: &mut Context<Self>) {
        self.palette = None;
        self.inline = None;
        self.calendar = None;
        self.menu = None;
        self.popover.close();
        self.drop = None;
        self.mouse_block = false;
        let edit = self.edit.take();
        let block = self.block.take();
        let (return_edit, return_block) = self.page.take().map_or((edit, block), |session| (session.return_edit, session.return_block));
        self.page = Some(PageSession { page, scroll: ScrollHandle::new(), return_edit, return_block });
        cx.notify();
    }

    pub fn leave_volatile_page(&mut self, restore: bool, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.page.take() else { return false };
        self.edit = None;
        self.block = None;
        if restore {
            if let Some(mut edit) = session.return_edit {
                let text = match edit.target {
                    Target::Text(id) if self.tree.contains(id) => Some(self.tree.node(self.tree.source(id)).text.as_str()),
                    Target::Note(id) if self.tree.contains(id) => Some(self.tree.node(self.tree.source(id)).note.as_str()),
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
            self.block = session.return_block.filter(|block| self.tree.contains(block.anchor) && self.tree.contains(block.head));
        }
        cx.notify();
        true
    }

    pub(crate) fn render_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match self.page.as_ref()?.page {
            VolatilePage::WhatsNew(_) => self.render_whats_new(window, cx),
            VolatilePage::Trash(_) => self.render_trash(cx),
        })
    }
}
