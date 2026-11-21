//! Outline editing operations. Each mutates the tree through `put`/`remove`,
//! so every operation is journaled for undo and persistence automatically.

use crate::tree::{BlockKind, Node, NodeId, Tree, ROOT};

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
    /// row above). Children of `id` move to the end of `into`'s children.
    /// Returns the caret offset inside `into`.
    pub fn merge_into(&mut self, id: NodeId, into: NodeId) -> Option<usize> {
        if id == into || into == ROOT || self.is_ancestor(id, into) {
            return None;
        }
        let caret = self.node(into).text.len();
        let text = format!("{}{}", self.node(into).text, self.node(id).text);
        self.set_text(into, &text);
        let kids = self.children(id).to_vec();
        for kid in kids {
            let index = self.children(into).len();
            self.move_to(kid, into, index);
        }
        if self.node(into).collapsed && self.has_children(into) {
            self.set_collapsed(into, false);
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

    /// Delete a node and its whole subtree.
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

    /// Copies of `id` and its subtree, root first (for the trash).
    pub fn subtree_nodes(&self, id: NodeId) -> Vec<Node> {
        let mut out = vec![self.node(id).clone()];
        out.extend(self.descendants(id).into_iter().map(|d| self.node(d).clone()));
        out
    }

    /// Put back nodes from the trash. Nodes that still exist are skipped; a
    /// root whose parent is gone goes to the end of the top level.
    pub fn restore(&mut self, nodes: &[Node]) -> Option<NodeId> {
        let root = nodes.first()?.clone();
        if self.contains(root.id) {
            return None;
        }
        let mut r = root.clone();
        if !self.contains(r.parent) {
            r.parent = ROOT;
            r.rank = self.rank_at(ROOT, self.children(ROOT).len());
        }
        self.put(r);
        for n in &nodes[1..] {
            if !self.contains(n.id) && self.contains(n.parent) {
                self.put(n.clone());
            }
        }
        Some(root.id)
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
