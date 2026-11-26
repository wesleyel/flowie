//! The Trash page: deleted subtrees (`Tree::trashed`), restoring them, and deleting them for good.

use flowie_core::{markup, NodeId, Tree, ROOT};
use gpui::{div, prelude::*, px, AnyElement, Context, ScrollHandle, Window};

use super::OutlineSelection;
use crate::app::i18n::t;
use crate::outline::{Outline, ShowTrash, Target, PAD_X};

/// One deleted subtree as the page lists it.
struct TrashEntry {
    root: NodeId,
    label: String,
    /// How many bullets restoring it brings back.
    count: usize,
    deleted_at: i64,
}

pub(crate) struct TrashPage {
    entries: Vec<TrashEntry>,
    scroll: ScrollHandle,
    selection: OutlineSelection,
}

/// What a deleted bullet is called in the Trash: its text (a mirror's source's, if that's
/// still around), cut short.
fn label(tree: &Tree, id: NodeId) -> String {
    let node = tree.node(id);
    let shown = (node.mirror != ROOT).then(|| tree.record(node.mirror)).flatten().unwrap_or(node);
    markup::plain_text(&shown.text).chars().take(80).collect()
}

impl Outline {
    pub(crate) fn render_trash(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::ui::button::{button, Variant};
        let theme = self.theme;
        let trash = self.trash.as_ref()?;
        let ago = crate::overlays::context_menu::ago;
        let rows = trash.entries.iter().enumerate().map(|(ix, e)| {
            let root = e.root;
            div()
                .flex()
                .items_center()
                .gap_3()
                .py(px(6.))
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(if e.label.trim().is_empty() {
                    t!("common.untitled").to_string()
                } else {
                    e.label.clone()
                }))
                .child(div().text_size(px(12.)).text_color(theme.muted).whitespace_nowrap().child(t!(
                    "trash.entry",
                    count = e.count,
                    ago = ago(e.deleted_at)
                )))
                .child(
                    button(theme, ("restore", ix), Variant::Link)
                        .child(crate::ui::icons::icon("rotate-ccw", crate::ui::icons::SM, Variant::Link.fg(theme)))
                        .child(t!("trash.restore"))
                        .on_click(cx.listener(move |this, _, _, cx| this.restore_trash(root, cx))),
                )
                .child(
                    button(theme, ("purge", ix), Variant::Destructive)
                        .child(crate::ui::icons::icon("trash-2", crate::ui::icons::SM, Variant::Destructive.fg(theme)))
                        .child(t!("trash.delete_forever"))
                        .on_click(cx.listener(move |this, _, _, cx| this.purge_trash(root, cx))),
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

    pub fn show_trash(&mut self, _: &ShowTrash, _: &mut Window, cx: &mut Context<Self>) {
        self.close_auxiliary_page(true, cx);
        // Entries past the retention period go now, as they do at launch.
        self.ws.purge_expired();
        let tree = &self.ws.tree;
        let entries = tree
            .trashed()
            .into_iter()
            .map(|root| TrashEntry {
                root,
                label: label(tree, root),
                count: tree.trashed_subtree(root).len(),
                deleted_at: tree.node(root).deleted_at.unwrap_or_default(),
            })
            .collect();
        let selection = self.suspend_outline_selection();
        self.trash = Some(TrashPage { entries, scroll: ScrollHandle::new(), selection });
        cx.notify();
    }

    pub(super) fn close_trash(&mut self, restore: bool, cx: &mut Context<Self>) -> bool {
        let Some(trash) = self.trash.take() else { return false };
        self.finish_auxiliary_page(trash.selection, restore, cx);
        true
    }

    /// Bring an entry back where it was (an undo step), and go to it.
    pub fn restore_trash(&mut self, root: NodeId, cx: &mut Context<Self>) {
        let Some(name) = self.trash.as_ref().and_then(|trash| trash.entries.iter().find(|e| e.root == root)).map(|e| e.label.clone())
        else {
            return;
        };
        if !self.ws.tree.untrash(root) {
            return;
        }
        let name = if name.trim().is_empty() { t!("common.untitled").to_string() } else { name };
        self.toast(t!("toast.restored", name = name));
        self.close_trash(false, cx);
        let parent = self.ws.tree.parent(root);
        self.commit_structure(None, cx);
        self.zoom_to(parent, cx);
        let len = self.ws.tree.node(root).text.len();
        self.start_edit(Target::Text(root), len, cx);
    }

    /// Delete an entry for good (not an undo step).
    pub fn purge_trash(&mut self, root: NodeId, cx: &mut Context<Self>) {
        self.ws.purge(root);
        if let Some(page) = self.trash.as_mut() {
            page.entries.retain(|e| e.root != root);
        }
        cx.notify();
    }
}
