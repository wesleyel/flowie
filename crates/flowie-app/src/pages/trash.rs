//! Trash entries, recovery commands, and the Trash page.

use flowie_core::{
    store::{self, TrashEntry},
    NodeId,
};
use gpui::{div, prelude::*, px, AnyElement, Context, ScrollHandle, Window};

use super::OutlineSelection;
use crate::app::i18n::t;
use crate::outline::{Outline, ShowTrash, Target, PAD_X};
use crate::overlays::palette::node_label;

pub(crate) struct TrashPage {
    entries: Vec<TrashEntry>,
    scroll: ScrollHandle,
    selection: OutlineSelection,
}

impl Outline {
    pub(crate) fn render_trash(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::ui::button::{button, Variant};
        let theme = self.theme;
        let trash = self.trash.as_ref()?;
        let ago = crate::overlays::context_menu::ago;
        let rows = trash.entries.iter().map(|e| {
            let seq = e.seq;
            let count = e.nodes.len();
            div()
                .flex()
                .items_center()
                .gap_3()
                .py(px(6.))
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(if e.label.is_empty() {
                    t!("common.untitled").to_string()
                } else {
                    e.label.clone()
                }))
                .child(div().text_size(px(12.)).text_color(theme.muted).whitespace_nowrap().child(t!(
                    "trash.entry",
                    count = count,
                    ago = ago(e.deleted_at)
                )))
                .child(
                    button(theme, ("restore", seq as usize), Variant::Link)
                        .child(crate::ui::icons::icon("rotate-ccw", crate::ui::icons::SM, Variant::Link.fg(theme)))
                        .child(t!("trash.restore"))
                        .on_click(cx.listener(move |this, _, _, cx| this.restore_trash(seq, cx))),
                )
                .child(
                    button(theme, ("purge", seq as usize), Variant::Destructive)
                        .child(crate::ui::icons::icon("trash-2", crate::ui::icons::SM, Variant::Destructive.fg(theme)))
                        .child(t!("trash.delete_forever"))
                        .on_click(cx.listener(move |this, _, _, cx| this.purge_trash(seq, cx))),
                )
        });
        let page = div()
            .id("trash")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&trash.scroll)
            .px(px(PAD_X + 24.))
            .child(self.render_auxiliary_title(t!("trash.title"), "trash-back", cx))
            .child(div().text_size(px(12.)).text_color(theme.muted).pb_2().child(t!("trash.retention")))
            .when(trash.entries.is_empty(), |d| d.child(div().py_4().text_color(theme.muted).child(t!("trash.empty"))))
            .children(rows);
        Some(crate::ui::scrollbar::area("trash-bar", &trash.scroll, theme, page).size_full().into_any_element())
    }

    pub fn send_to_trash(&mut self, id: NodeId) {
        if let Some(w) = &self.writer {
            let label: String = node_label(self, id).chars().take(80).collect();
            w.trash(id, &label, self.tree.subtree_nodes(id));
        }
    }

    pub fn show_trash(&mut self, _: &ShowTrash, _: &mut Window, cx: &mut Context<Self>) {
        self.close_auxiliary_page(true, cx);
        if let Some(w) = &self.writer {
            w.flush();
        }
        let entries =
            self.db_path.as_ref().and_then(|p| store::open(p).ok()).and_then(|c| store::list_trash(&c, 30).ok()).unwrap_or_default();
        // Hide entries whose bullet came back (undo or restore).
        let entries = entries.into_iter().filter(|e| !self.tree.contains(e.root)).collect();
        let selection = self.suspend_outline_selection();
        self.trash = Some(TrashPage { entries, scroll: ScrollHandle::new(), selection });
        cx.notify();
    }

    pub(super) fn close_trash(&mut self, restore: bool, cx: &mut Context<Self>) -> bool {
        let Some(trash) = self.trash.take() else { return false };
        self.finish_auxiliary_page(trash.selection, restore, cx);
        true
    }

    pub fn restore_trash(&mut self, seq: i64, cx: &mut Context<Self>) {
        let Some(entry) = self.trash.as_ref().and_then(|trash| trash.entries.iter().find(|e| e.seq == seq)).cloned() else { return };
        if let Some(root) = self.tree.restore(&entry.nodes) {
            if let Some(w) = &self.writer {
                w.untrash(seq);
            }
            self.toast(t!("toast.restored", name = entry.label));
            self.close_trash(false, cx);
            let parent = self.tree.parent(root);
            self.commit_structure(None, cx);
            self.zoom_to(parent, cx);
            let len = self.tree.node(root).text.len();
            self.start_edit(Target::Text(root), len, cx);
        }
    }

    pub fn purge_trash(&mut self, seq: i64, cx: &mut Context<Self>) {
        if let Some(w) = &self.writer {
            w.untrash(seq);
        }
        if let Some(page) = self.trash.as_mut() {
            page.entries.retain(|e| e.seq != seq);
        }
        cx.notify();
    }
}
