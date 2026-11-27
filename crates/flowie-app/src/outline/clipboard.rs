//! Clipboard commands for text, outline subtrees, and images.

use std::collections::HashSet;

use flowie_core::{io, Item, NodeId};
use gpui::{ClipboardItem, Context, Window};

use super::{BlockSel, Copy, Cut, DeleteNode, Edit, Outline, Paste, Target};

fn items_to_clipboard(items: Vec<Item>) -> ClipboardItem {
    let text = io::to_markdown(&items);
    ClipboardItem::new_string_with_json_metadata(text, items)
}

impl Outline {
    /// The code row whose text is being edited (not the page title or a mirror).
    fn editing_code_row(&self) -> Option<NodeId> {
        match self.edit_target() {
            Some(Target::Text(id)) if id != self.zoom => {
                let node = self.ws.tree.get(id)?;
                (node.kind == flowie_core::BlockKind::Code && node.mirror == flowie_core::ROOT).then_some(id)
            }
            _ => None,
        }
    }

    /// With a search filter on, the rows it shows (as real ids); `None` without one.
    pub fn filter_shown(&self) -> Option<HashSet<NodeId>> {
        self.view_options().query.is_some().then(|| self.rows.iter().map(|r| self.ws.tree.resolve(r.id)).collect())
    }

    /// Copy bullets with full fidelity for pasting inside Flowie, plus
    /// Markdown text for other apps. With a search filter on, only the rows
    /// it shows are copied, as they appear.
    pub fn copy_items(&self, ids: &[NodeId], cx: &mut Context<Self>) {
        let items = match self.filter_shown() {
            Some(shown) => self.ws.tree.to_items_where(ids, &|id| shown.contains(&id)),
            None => self.ws.tree.to_items(ids),
        };
        cx.write_to_clipboard(items_to_clipboard(items));
    }

    pub fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.copy_selected_image(cx) {
            return;
        }
        if let Some(roots) = self.block_roots() {
            self.copy_items(&roots, cx);
        } else if let Some(e) = &self.edit {
            if !e.sel.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(self.edit_text()[e.sel.clone()].to_string()));
            }
        }
    }

    pub fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(roots) = self.block_roots() {
            if roots.is_empty() {
                return;
            }
            // Cut moves the bullets, so the clipboard gets everything Delete
            // takes, rows the search filter hides included.
            cx.write_to_clipboard(items_to_clipboard(self.ws.tree.to_items(&roots)));
            self.delete_node(&DeleteNode, window, cx);
            self.toast(crate::app::i18n::t!("toast.cut_kept_in_trash"));
        } else if let Some(e) = &self.edit {
            if !e.sel.is_empty() {
                let r = e.sel.clone();
                self.copy(&Copy, window, cx);
                self.replace_edit_text(r, "", false, cx);
            }
        }
    }

    pub fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        if self.is_auxiliary_page_open() || matches!(self.edit_target(), Some(Target::Field(_))) {
            let field_sel = self.edit.as_ref().filter(|e| matches!(e.target, Target::Field(_))).map(|e| e.sel.clone());
            if let (Some(sel), Some(text)) = (field_sel, item.text()) {
                self.replace_edit_text(sel, &text, false, cx);
            }
            return;
        }
        if self.paste_images(&item, cx) {
            return;
        }
        let Some(text) = item.text() else { return };
        // Bullets copied from Flowie carry their full structure as metadata.
        let copied: Option<Vec<flowie_core::Item>> = item.entries().iter().find_map(|e| match e {
            gpui::ClipboardEntry::String(s) => s.metadata_json::<Vec<flowie_core::Item>>(),
            _ => None,
        });
        // Other text pasted into a code row is code: each line goes in as is.
        if let (None, Some(id), Some(sel)) = (&copied, self.editing_code_row(), self.edit.as_ref().map(|e| e.sel.clone())) {
            if text.trim_end_matches(['\n', '\r']).contains(['\n', '\r']) {
                let before = self.caret();
                let (row, off) = self.ws.tree.paste_code_lines(id, sel, &text);
                self.edit = Some(Edit::new(Target::Text(row), off));
                self.commit_structure(before, cx);
                return;
            }
        }
        let mut items: Vec<flowie_core::Item> =
            copied
                .unwrap_or_else(|| if text.trim_end_matches('\n').contains('\n') { flowie_core::io::parse_any(&text) } else { Vec::new() });
        self.resolve_item_images(&mut items, None);
        let multiline = !items.is_empty();
        match self.edit.as_ref().map(|e| (e.target, e.sel.clone())) {
            Some((Target::Text(id), _)) if multiline && id != self.zoom => {
                let before = self.caret();
                let parent = self.ws.tree.parent(id);
                let empty = self.ws.tree.node(id).text.is_empty() && !self.ws.tree.has_children(id);
                let index = self.ws.tree.index_in_parent(id) + if empty { 0 } else { 1 };
                let top = self.ws.tree.insert_items(parent, index, &items);
                if empty {
                    self.ws.tree.delete(id);
                }
                if let Some(&last) = top.last() {
                    let len = self.ws.tree.node(last).text.len();
                    self.edit = Some(Edit::new(Target::Text(last), len));
                }
                self.commit_structure(before, cx);
            }
            Some((_, sel)) => self.replace_edit_text(sel, &text, false, cx),
            None => {
                if let Some(b) = self.block {
                    let before = self.caret();
                    let parent = self.ws.tree.parent(b.head);
                    let index = self.ws.tree.index_in_parent(b.head) + 1;
                    let top = if items.is_empty() {
                        self.ws.tree.insert_indented_text(parent, index, &text)
                    } else {
                        self.ws.tree.insert_items(parent, index, &items)
                    };
                    self.block = top.first().zip(top.last()).map(|(a, h)| BlockSel { anchor: *a, head: *h });
                    self.commit_structure(before, cx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowie_core::{BlockKind, Tree, ROOT};

    #[test]
    fn cut_image_clipboard_keeps_the_caption_and_subtree() {
        let mut tree = Tree::new();
        let image = tree.create_at(ROOT, 0, "caption");
        let child = tree.create_at(image, 0, "child");
        tree.create_at(child, 0, "grandchild");
        let name = format!("{}.png", "a".repeat(64));
        let mut node = tree.node(image).clone();
        node.kind = BlockKind::Image;
        node.image = name.clone();
        node.collapsed = true;
        tree.put(node);

        let clipboard = items_to_clipboard(tree.to_items(&[image]));
        let [gpui::ClipboardEntry::String(entry)] = clipboard.entries() else { panic!("expected structured text") };
        let items = entry.metadata_json::<Vec<flowie_core::Item>>().unwrap();
        tree.delete(image);
        let [pasted] = tree.insert_items(ROOT, 0, &items)[..] else { panic!("expected one image") };
        let node = tree.node(pasted);
        assert_eq!(
            (node.kind, node.image.as_str(), node.text.as_str(), node.collapsed),
            (BlockKind::Image, name.as_str(), "caption", true)
        );
        let child = tree.children(pasted)[0];
        assert_eq!(tree.node(child).text, "child");
        assert_eq!(tree.node(tree.children(child)[0]).text, "grandchild");
    }
}
