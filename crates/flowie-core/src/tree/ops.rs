//! Outline editing operations. Each mutates the tree through `put`/`remove`,
//! so every operation is journaled for undo and persistence automatically.

use super::{BlockKind, Node, NodeId, Tree, ROOT};

impl Tree {
    /// Create an empty node at `index` among `parent`'s children.
    pub fn create_at(&mut self, parent: NodeId, index: usize, text: &str) -> NodeId {
        // New children of a mirror go into its source.
        let parent = self.source(parent);
        let rank = self.rank_at(parent, index);
        let id = self.alloc_id();
        let mut node = Node::new(id, parent, rank);
        node.text = text.to_string();
        self.put(node);
        id
    }

    pub fn create_after(&mut self, sibling: NodeId, text: &str) -> NodeId {
        let parent = self.parent(sibling);
        let index = self.index_in_parent(sibling) + 1;
        self.create_at(parent, index, text)
    }

    pub fn create_before(&mut self, sibling: NodeId, text: &str) -> NodeId {
        let parent = self.parent(sibling);
        let index = self.index_in_parent(sibling);
        self.create_at(parent, index, text)
    }

    pub fn set_text(&mut self, id: NodeId, text: &str) {
        if self.node(id).text != text {
            let mut node = self.node(id).clone();
            node.text = text.to_string();
            self.put(node);
        }
    }

    pub fn set_note(&mut self, id: NodeId, note: &str) {
        if self.node(id).note != note {
            let mut node = self.node(id).clone();
            node.note = note.to_string();
            self.put(node);
        }
    }

    pub fn set_completed(&mut self, id: NodeId, completed: bool) {
        if self.node(id).completed != completed {
            let mut node = self.node(id).clone();
            node.completed = completed;
            self.put(node);
        }
    }

    pub fn set_collapsed(&mut self, id: NodeId, collapsed: bool) {
        if id != ROOT && self.node(id).collapsed != collapsed {
            let mut node = self.node(id).clone();
            node.collapsed = collapsed;
            self.put(node);
        }
    }

    /// Star `id` at position `index` of the starred list (counted without it;
    /// past the end appends). Only `id` changes: its new key falls between its
    /// neighbours' keys.
    pub fn star_at(&mut self, id: NodeId, index: usize) {
        let id = self.resolve(id);
        if id == ROOT || !self.contains(id) {
            return;
        }
        let keys: Vec<&str> =
            self.index.stars.iter().filter(|(_, s)| *s != id && !self.index.hidden.contains(s)).map(|(k, _)| k.as_str()).collect();
        let index = index.min(keys.len());
        let valid = |k: &&str| crate::rank::is_valid(k);
        let before = keys[..index].iter().rev().copied().find(valid);
        let after = keys[index..].iter().copied().filter(valid).find(|k| before.is_none_or(|b| b < *k));
        let star = crate::rank::between(before, after);
        self.set_star(id, star);
    }

    pub fn unstar(&mut self, id: NodeId) {
        self.set_star(id, String::new());
    }

    /// Set `id`'s star key as is (empty = not starred).
    pub fn set_star(&mut self, id: NodeId, star: String) {
        let id = self.resolve(id);
        if id != ROOT && self.get(id).is_some_and(|n| n.star != star) {
            let mut node = self.node(id).clone();
            node.star = star;
            self.put(node);
        }
    }

    /// Enter key. Splits `id` at byte offset `at`; returns the node the caret
    /// should move to and the caret offset inside it.
    ///
    /// - caret at start of non-empty text → empty sibling above, caret stays.
    /// - expanded node with children, caret at end → new first child.
    /// - otherwise → the text after the caret moves to a new sibling below.
    /// - empty to-do or numbered item → it turns back into a plain bullet.
    ///
    /// A new row takes the list kind (to-do, numbered) of the row it sits
    /// next to: the split row for a sibling, the old first child for a child.
    pub fn split(&mut self, id: NodeId, at: usize) -> (NodeId, usize) {
        let text = self.node(id).text.clone();
        let kind = self.node(id).kind;
        let at = at.min(text.len());
        if text.is_empty() && kind.is_list() {
            self.set_kind(id, BlockKind::Bullet);
            return (id, 0);
        }
        if at == 0 && !text.is_empty() {
            let new = self.create_before(id, "");
            self.set_kind(new, kind.list_kind());
            return (id, 0);
        }
        if at == text.len() && self.has_children(id) && !self.node(id).collapsed {
            let first = self.node(self.children(id)[0]).kind;
            let child = self.create_at(id, 0, "");
            self.set_kind(child, first.list_kind());
            return (child, 0);
        }
        let (head, tail) = text.split_at(at);
        let tail = tail.to_string();
        self.set_text(id, head);
        let new = self.create_after(id, &tail);
        self.set_kind(new, kind.list_kind());
        (new, 0)
    }

    /// Backspace at the start of `id`: merge its text into `into` (usually the
    /// row above). Its note goes after `into`'s note, and its children move to
    /// the end of `into`'s children. An image or mirror row is not merged (its
    /// image or mirror link would be lost). Returns the caret offset inside `into`.
    pub fn merge_into(&mut self, id: NodeId, into: NodeId) -> Option<usize> {
        let (id, into) = (self.resolve(id), self.resolve(into));
        let into_src = self.source(into);
        if id == into
            || id == into_src
            || into == ROOT
            || self.is_ancestor(id, into)
            || self.is_ancestor(id, into_src)
            || self.node(id).mirror != ROOT
            || !self.node(id).image.is_empty()
        {
            return None;
        }
        let caret = self.node(into_src).text.len();
        let text = format!("{}{}", self.node(into_src).text, self.node(id).text);
        self.set_text(into_src, &text);
        let note = self.node(id).note.clone();
        if !note.is_empty() {
            let into_note = &self.node(into_src).note;
            let merged = if into_note.is_empty() { note } else { format!("{into_note}\n{note}") };
            self.set_note(into_src, &merged);
        }
        let kids = self.children(id).to_vec();
        for kid in kids {
            let index = self.children(into_src).len();
            self.move_to(kid, into_src, index);
        }
        if self.node(into).collapsed && self.has_children(into_src) {
            self.set_collapsed(into, false);
        }
        if into_src != into && self.node(into_src).collapsed && self.has_children(into_src) {
            self.set_collapsed(into_src, false);
        }
        self.delete(id);
        Some(caret)
    }

    /// Move `id` (with its subtree) to `index` among `new_parent`'s children.
    /// Returns false if the move is impossible (into its own subtree).
    pub fn move_to(&mut self, id: NodeId, new_parent: NodeId, index: usize) -> bool {
        let (id, new_parent) = (self.resolve(id), self.resolve(new_parent));
        if id == ROOT || id == new_parent || self.is_ancestor(id, new_parent) {
            return false;
        }
        // Compute the rank without `id` among the target siblings.
        let mut index = index;
        if self.parent(id) == new_parent {
            let cur = self.index_in_parent(id);
            if cur < index {
                index -= 1;
            }
            if cur == index {
                return true;
            }
        }
        let rank = self.rank_at_excluding(new_parent, index, Some(id));
        let mut node = self.node(id).clone();
        node.parent = new_parent;
        node.rank = rank;
        self.put(node);
        true
    }

    /// Tab: become the last child of the previous sibling.
    pub fn indent(&mut self, id: NodeId) -> bool {
        let Some(prev) = self.prev_sibling(id) else { return false };
        let index = self.children(prev).len();
        self.set_collapsed(prev, false);
        self.move_to(id, prev, index)
    }

    /// Shift-Tab: become the sibling right after the parent. `floor` is the
    /// zoom root; nodes directly under it cannot be outdented.
    pub fn outdent(&mut self, id: NodeId, floor: NodeId) -> bool {
        let parent = self.parent(id);
        if parent == ROOT || parent == floor {
            return false;
        }
        let grand = self.parent(parent);
        let index = self.index_in_parent(parent) + 1;
        self.move_to(id, grand, index)
    }

    /// Cmd-Shift-Up: swap with the previous sibling; at the top, become the
    /// last child of the parent's previous sibling.
    pub fn move_up(&mut self, id: NodeId, floor: NodeId) -> bool {
        let parent = self.parent(id);
        let index = self.index_in_parent(id);
        if index > 0 {
            return self.move_to(id, parent, index - 1);
        }
        if parent == floor || parent == ROOT {
            return false;
        }
        match self.prev_sibling(parent) {
            Some(uncle) => {
                self.set_collapsed(uncle, false);
                let n = self.children(uncle).len();
                self.move_to(id, uncle, n)
            }
            None => false,
        }
    }

    /// Cmd-Shift-Down: swap with the next sibling; at the bottom, become the
    /// first child of the parent's next sibling.
    pub fn move_down(&mut self, id: NodeId, floor: NodeId) -> bool {
        let parent = self.parent(id);
        let index = self.index_in_parent(id);
        if index + 1 < self.children(parent).len() {
            return self.move_to(id, parent, index + 2);
        }
        if parent == floor || parent == ROOT {
            return false;
        }
        match self.next_sibling(parent) {
            Some(uncle) => {
                self.set_collapsed(uncle, false);
                self.move_to(id, uncle, 0)
            }
            None => false,
        }
    }

    /// Delete a node and its whole subtree right away, not to the Trash (for rows that
    /// merge or get replaced; what the user deletes goes through `trash`).
    pub fn delete(&mut self, id: NodeId) {
        let id = self.resolve(id);
        if id == ROOT || !self.contains(id) {
            return;
        }
        let mut all = self.descendants(id);
        all.reverse();
        for n in all {
            self.remove(n);
        }
        self.remove(id);
    }

    /// Markdown-style indented text of nodes and their subtrees.
    pub fn to_indented_text(&self, ids: &[NodeId]) -> String {
        crate::io::to_markdown(&self.to_items(ids))
    }

    /// Insert indented text / Markdown / OPML as new nodes at `index` under
    /// `parent`. Returns the top-level new ids.
    pub fn insert_indented_text(&mut self, parent: NodeId, index: usize, text: &str) -> Vec<NodeId> {
        let items = crate::io::parse_any(text);
        self.insert_items(parent, index, &items)
    }

    /// The node whose content `id` shows: itself, or the source of a mirror.
    pub fn source(&self, id: NodeId) -> NodeId {
        let id = self.resolve(id);
        match self.get(id) {
            Some(n) if n.mirror != ROOT && self.contains(n.mirror) => n.mirror,
            _ => id,
        }
    }

    pub fn set_kind(&mut self, id: NodeId, kind: BlockKind) {
        if self.node(id).kind != kind {
            let mut node = self.node(id).clone();
            node.kind = kind;
            self.put(node);
        }
    }

    /// Create a mirror of `source` at `index` under `parent`.
    pub fn create_mirror(&mut self, source: NodeId, parent: NodeId, index: usize) -> Option<NodeId> {
        let source = self.source(source);
        if source == ROOT || parent == source || self.is_ancestor(source, parent) {
            return None;
        }
        let id = self.create_at(parent, index, "");
        let mut node = self.node(id).clone();
        node.mirror = source;
        node.kind = self.node(source).kind;
        self.put(node);
        Some(id)
    }

    /// Show `levels` levels below `root`: deeper nodes with children are
    /// collapsed, shallower ones expanded. `usize::MAX` expands everything.
    pub fn expand_to_level(&mut self, root: NodeId, levels: usize) {
        let mut stack: Vec<(NodeId, usize)> = self.children(root).iter().map(|&c| (c, 1)).collect();
        while let Some((id, depth)) = stack.pop() {
            if !self.has_children(id) {
                continue;
            }
            let collapse = depth >= levels;
            self.set_collapsed(id, collapse);
            stack.extend(self.children(id).iter().map(|&c| (c, depth + 1)));
        }
    }

    /// Nodes whose text links to `id` (`[label](flowie:ID)`).
    pub fn backlinks(&self, id: NodeId) -> Vec<NodeId> {
        self.links_to(id).collect()
    }
}
