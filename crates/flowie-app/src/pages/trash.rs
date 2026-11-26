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
            let unrestorable = e.corrupted || e.nodes.is_empty();
            let count = e.nodes.len();
            let meta = if unrestorable {
                t!("trash.unrestorable", ago = ago(e.deleted_at))
            } else {
                t!("trash.entry", count = count, ago = ago(e.deleted_at))
            };
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
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(if unrestorable { theme.danger } else { theme.muted })
                        .whitespace_nowrap()
                        .child(meta),
                )
                .when(!unrestorable, |row| {
                    row.child(
                        button(theme, ("restore", seq as usize), Variant::Link)
                            .child(crate::ui::icons::icon("rotate-ccw", crate::ui::icons::SM, Variant::Link.fg(theme)))
                            .child(t!("trash.restore"))
                            .on_click(cx.listener(move |this, _, _, cx| this.restore_trash(seq, cx))),
                    )
                })
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
        let entry = self.trash_entry(id);
        self.write_trash(vec![entry]);
    }

    /// What Trash keeps of a bullet about to be deleted: its id, a label and
    /// its subtree. Taken before the delete; written with `write_trash`.
    pub fn trash_entry(&self, id: NodeId) -> (NodeId, String, Vec<flowie_core::Node>) {
        (id, node_label(self, id).chars().take(80).collect(), self.ws.tree.subtree_nodes(id))
    }

    pub fn write_trash(&self, entries: Vec<(NodeId, String, Vec<flowie_core::Node>)>) {
        if let Some(w) = &self.ws.writer {
            for (id, label, nodes) in entries {
                w.trash(id, &label, nodes);
            }
        }
    }

    pub fn show_trash(&mut self, _: &ShowTrash, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(writer) = &self.ws.writer {
            if let Err(error) = writer.flush() {
                self.toast(t!("outline.save_error", error = error));
                return cx.notify();
            }
        }
        self.close_auxiliary_page(true, cx);
        let entries =
            self.db_path.as_ref().and_then(|p| store::open(p).ok()).and_then(|c| store::list_trash(&c, 30).ok()).unwrap_or_default();
        // Hide entries whose bullet came back (undo or restore). Restoring
        // keeps the entry, so undoing the restore brings it back here; show
        // only the newest entry for each bullet.
        let mut shown = std::collections::HashSet::new();
        let entries = entries.into_iter().filter(|e| !self.ws.tree.contains(e.root) && shown.insert(e.root)).collect();
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
        if entry.corrupted || entry.nodes.is_empty() {
            self.toast(t!("toast.trash_unrestorable"));
            cx.notify();
            return;
        }
        // The entry stays in the database: undoing the restore deletes the
        // bullets again, and they must still be in Trash then.
        if let Some(root) = self.ws.tree.restore(&entry.nodes) {
            self.toast(t!("toast.restored", name = entry.label));
            self.close_trash(false, cx);
            let parent = self.ws.tree.parent(root);
            self.commit_structure(None, cx);
            self.zoom_to(parent, cx);
            let len = self.ws.tree.node(root).text.len();
            self.start_edit(Target::Text(root), len, cx);
        } else {
            self.toast(t!("toast.trash_unrestorable"));
            cx.notify();
        }
    }

    pub fn purge_trash(&mut self, seq: i64, cx: &mut Context<Self>) {
        let Some(root) = self.trash.as_ref().and_then(|trash| trash.entries.iter().find(|e| e.seq == seq)).map(|e| e.root) else { return };
        // Older entries for the same bullet go too, or they would take its place.
        if let Some(w) = &self.ws.writer {
            w.untrash_root(root);
        }
        if let Some(page) = self.trash.as_mut() {
            page.entries.retain(|e| e.root != root);
        }
        cx.notify();
    }
}
