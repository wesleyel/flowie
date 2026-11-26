//! Clipboard commands for text, outline subtrees, and images.

use flowie_core::{io, NodeId, Tree};
use gpui::{ClipboardItem, Context, Window};

use super::{BlockSel, Copy, Cut, DeleteNode, Edit, Outline, Paste, Target};

fn items_to_clipboard(tree: &Tree, ids: &[NodeId]) -> ClipboardItem {
    let items = tree.to_items(ids);
    let text = io::to_markdown(&items);
    ClipboardItem::new_string_with_json_metadata(text, items)
}

impl Outline {
    /// Copy bullets with full fidelity for pasting inside Flowie, plus
    /// Markdown text for other apps.
    pub fn copy_items(&self, ids: &[NodeId], cx: &mut Context<Self>) {
        cx.write_to_clipboard(items_to_clipboard(&self.ws.tree, ids));
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
            self.copy_items(&roots, cx);
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
            if let Some(text) = item.text().filter(|_| matches!(self.edit_target(), Some(Target::Field(_)))) {
                self.replace_edit_text(self.edit.as_ref().unwrap().sel.clone(), &text, false, cx);
            }
            return;
        }
        if self.paste_images(&item, cx) {
            return;
        }
        let Some(text) = item.text() else { return };
        // Bullets copied from Flowie carry their full structure as metadata.
        let mut items: Vec<flowie_core::Item> = item
            .entries()
            .iter()
            .find_map(|e| match e {
                gpui::ClipboardEntry::String(s) => s.metadata_json::<Vec<flowie_core::Item>>(),
                _ => None,
            })
            .unwrap_or_else(|| if text.trim_end_matches('\n').contains('\n') { flowie_core::io::parse_any(&text) } else { Vec::new() });
        self.resolve_item_images(&mut items, None);
        let multiline = !items.is_empty();
        match self.edit_target() {
            Some(Target::Text(id)) if multiline && id != self.zoom => {
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
            Some(_) => {
                let r = self.edit.as_ref().unwrap().sel.clone();
                self.replace_edit_text(r, &text, false, cx);
            }
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
    use flowie_core::{BlockKind, ROOT};

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

        let clipboard = items_to_clipboard(&tree, &[image]);
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
