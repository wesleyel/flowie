//! Trash entries, recovery commands, and the Trash page.

use flowie_core::{
    store::{self, TrashEntry},
    NodeId,
};
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, Window};

use super::VolatilePage;
use crate::app::i18n::t;
use crate::outline::{Outline, ShowTrash, Target, PAD_X, TITLE_SIZE};
use crate::overlays::palette::node_label;

pub(super) struct TrashPage {
    entries: Vec<TrashEntry>,
}

impl Outline {
    pub(super) fn render_trash(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use crate::ui::button::{button, Variant};
        let theme = self.theme;
        let entries = self.trash_entries().unwrap();
        let scroll = self.page.as_ref().unwrap().scroll.clone();
        let ago = crate::overlays::context_menu::ago;
        let rows = entries.iter().map(|e| {
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
        let empty = entries.is_empty();
        let page = div()
            .id("trash")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .px(px(PAD_X + 24.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_2()
                    .pb_3()
                    .child(div().text_size(px(TITLE_SIZE)).font_weight(FontWeight::BOLD).child(t!("trash.title")))
                    .child(
                        button(theme, "trash-back", Variant::Plain)
                            .child(crate::ui::icons::icon("arrow-left", crate::ui::icons::SM, Variant::Plain.fg(theme)))
                            .child(t!("common.back"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.leave_volatile_page(true, cx);
                            })),
                    ),
            )
            .child(div().text_size(px(12.)).text_color(theme.muted).pb_2().child(t!("trash.retention")))
            .when(empty, |d| d.child(div().py_4().text_color(theme.muted).child(t!("trash.empty"))))
            .children(rows);
        crate::ui::scrollbar::area("trash-bar", &scroll, theme, page).size_full().into_any_element()
    }

    pub fn send_to_trash(&mut self, id: NodeId) {
        if let Some(w) = &self.writer {
            let label: String = node_label(self, id).chars().take(80).collect();
            w.trash(id, &label, self.tree.subtree_nodes(id));
        }
    }

    pub fn show_trash(&mut self, _: &ShowTrash, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(w) = &self.writer {
            w.flush();
        }
        let entries =
            self.db_path.as_ref().and_then(|p| store::open(p).ok()).and_then(|c| store::list_trash(&c, 30).ok()).unwrap_or_default();
        // Hide entries whose bullet came back (undo or restore).
        let entries = entries.into_iter().filter(|e| !self.tree.contains(e.root)).collect();
        self.enter_volatile_page(VolatilePage::Trash(TrashPage { entries }), cx);
    }

    pub fn restore_trash(&mut self, seq: i64, cx: &mut Context<Self>) {
        let Some(entry) = self.trash_entries().and_then(|t| t.iter().find(|e| e.seq == seq)).cloned() else { return };
        if let Some(root) = self.tree.restore(&entry.nodes) {
            if let Some(w) = &self.writer {
                w.untrash(seq);
            }
            self.toast(t!("toast.restored", name = entry.label));
            self.leave_volatile_page(false, cx);
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
        if let Some(VolatilePage::Trash(page)) = self.page.as_mut().map(|session| &mut session.page) {
            page.entries.retain(|e| e.seq != seq);
        }
        cx.notify();
    }

    fn trash_entries(&self) -> Option<&[TrashEntry]> {
        match &self.page.as_ref()?.page {
            VolatilePage::Trash(page) => Some(&page.entries),
            _ => None,
        }
    }
}
