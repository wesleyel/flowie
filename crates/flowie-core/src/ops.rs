//! Outline editing operations. Each mutates the tree through `put`/`remove`,
//! so every operation is journaled for undo and persistence automatically.

use crate::tree::{Node, NodeId, Tree, ROOT};

impl Tree {
    /// Create an empty node at `index` among `parent`'s children.
    pub fn create_at(&mut self, parent: NodeId, index: usize, text: &str) -> NodeId {
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
    pub fn split(&mut self, id: NodeId, at: usize) -> (NodeId, usize) {
        let text = self.node(id).text.clone();
        let at = at.min(text.len());
        if at == 0 && !text.is_empty() {
            self.create_before(id, "");
            return (id, 0);
        }
        if at == text.len() && self.has_children(id) && !self.node(id).collapsed {
            let child = self.create_at(id, 0, "");
            return (child, 0);
        }
        let (head, tail) = text.split_at(at);
        let tail = tail.to_string();
        self.set_text(id, head);
        let new = self.create_after(id, &tail);
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

    /// Plain-text export of nodes and their subtrees, indented with tabs.
    /// Completed nodes are prefixed with "[x] ", notes go on a `  "` line.
    pub fn to_indented_text(&self, ids: &[NodeId]) -> String {
        let mut out = String::new();
        for &id in ids {
            self.write_indented(id, 0, &mut out);
        }
        out
    }

    fn write_indented(&self, id: NodeId, depth: usize, out: &mut String) {
        let node = self.node(id);
        for _ in 0..depth {
            out.push('\t');
        }
        out.push_str("- ");
        if node.completed {
            out.push_str("[x] ");
        }
        out.push_str(&node.text);
        out.push('\n');
        if !node.note.is_empty() {
            for line in node.note.lines() {
                for _ in 0..=depth {
                    out.push('\t');
                }
                out.push_str("  ");
                out.push_str(line);
                out.push('\n');
            }
        }
        for &kid in self.children(id) {
            self.write_indented(kid, depth + 1, out);
        }
    }

    /// Insert indented text (tabs, or 2/4 spaces, optional "- " / "* " bullets)
    /// as new nodes at `index` under `parent`. Returns the top-level new ids.
    pub fn insert_indented_text(&mut self, parent: NodeId, index: usize, text: &str) -> Vec<NodeId> {
        let lines: Vec<(usize, String)> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let expanded = l.replace('\t', "    ");
                let indent = expanded.len() - expanded.trim_start().len();
                (indent, expanded.trim().to_string())
            })
            .collect();
        if lines.is_empty() {
            return Vec::new();
        }
        let min_indent = lines.iter().map(|(i, _)| *i).min().unwrap_or(0);
        let mut top = Vec::new();
        // Stack of (indent, node id).
        let mut stack: Vec<(usize, NodeId)> = Vec::new();
        let mut next_top_index = index;
        for (indent, raw) in lines {
            let indent = indent - min_indent;
            let mut content = raw.as_str();
            for bullet in ["- ", "* ", "• "] {
                if let Some(rest) = content.strip_prefix(bullet) {
                    content = rest;
                    break;
                }
            }
            let mut completed = false;
            for done in ["[x] ", "[X] "] {
                if let Some(rest) = content.strip_prefix(done) {
                    content = rest;
                    completed = true;
                }
            }
            if let Some(rest) = content.strip_prefix("[ ] ") {
                content = rest;
            }
            while stack.last().is_some_and(|(i, _)| *i >= indent) {
                stack.pop();
            }
            let id = match stack.last() {
                Some(&(_, p)) => {
                    let n = self.children(p).len();
                    self.create_at(p, n, content)
                }
                None => {
                    let id = self.create_at(parent, next_top_index, content);
                    next_top_index += 1;
                    top.push(id);
                    id
                }
            };
            if completed {
                self.set_completed(id, true);
            }
            stack.push((indent, id));
        }
        top
    }
}
