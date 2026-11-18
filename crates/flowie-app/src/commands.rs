//! Commands added on top of basic editing: trash, move to, mirrors, stars,
//! levels, block types, import/export, saved searches, journal.

use std::path::PathBuf;

use flowie_core::{io, markup, search::today, store, BlockKind, NodeId, Query, ROOT};
use gpui::{ClipboardItem, Context, PathPromptOptions, Window};

use crate::anim::{self, Channel};
use crate::i18n::t;
use crate::outline::*;
use crate::palette::{node_label, Mode};

impl Outline {
    pub fn send_to_trash(&mut self, id: NodeId) {
        if let Some(w) = &self.writer {
            let label: String = node_label(self, id).chars().take(80).collect();
            w.trash(id, &label, self.tree.subtree_nodes(id));
        }
    }

    fn targets(&self) -> Vec<NodeId> {
        if self.block.is_some() {
            let ids = self.block_ids();
            let set: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
            return ids.into_iter().filter(|id| !self.tree.ancestors(*id).iter().any(|a| set.contains(a))).collect();
        }
        self.editing_node().filter(|id| *id != self.zoom).into_iter().collect()
    }

    // --------------------------------------------------------- palette

    pub fn jump(&mut self, _: &Jump, _: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(Mode::Jump, cx);
    }

    pub fn move_to_action(&mut self, _: &MoveTo, _: &mut Window, cx: &mut Context<Self>) {
        let ids = self.targets();
        if !ids.is_empty() {
            self.open_palette(Mode::MoveTo(ids), cx);
        }
    }

    pub fn link_to_bullet(&mut self, _: &LinkToBullet, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            if let Target::Text(id) = e.target {
                let at = e.caret();
                self.open_palette(Mode::LinkTo { node: id, at }, cx);
            }
        }
    }

    pub fn mirror_bullet(&mut self, _: &MirrorBullet, _: &mut Window, cx: &mut Context<Self>) {
        let after = self.editing_node().or(self.block.map(|b| b.head)).unwrap_or(self.zoom);
        self.open_palette(Mode::Mirror { after }, cx);
    }

    pub fn move_nodes_to(&mut self, ids: &[NodeId], target: NodeId, cx: &mut Context<Self>) {
        let before = None;
        let fallback = ids.first().and_then(|f| {
            let ix = self.row_index(*f)?;
            ix.checked_sub(1).and_then(|i| self.rows.get(i)).map(|r| r.id)
        });
        let mut moved = 0;
        for &id in ids {
            let n = self.tree.children(target).len();
            if self.tree.move_to(id, target, n) {
                moved += 1;
            }
        }
        if moved > 0 {
            self.tree.set_collapsed(target, false);
            self.block = None;
            self.edit = fallback.filter(|f| self.tree.contains(*f)).map(|f| Edit::new(Target::Text(f), 0));
            self.toast(t!("toast.moved", count = moved, target = node_label(self, target)));
            self.commit_structure(before, cx);
        }
    }

    pub fn insert_mirror(&mut self, after: NodeId, source: NodeId, cx: &mut Context<Self>) {
        let (parent, index) = if after == self.zoom {
            (self.zoom, self.tree.children(self.zoom).len())
        } else {
            (self.tree.parent(after), self.tree.index_in_parent(after) + 1)
        };
        let before = None;
        match self.tree.create_mirror(source, parent, index) {
            Some(m) => {
                // Replace an empty bullet the mirror was typed into.
                if after != self.zoom
                    && self.tree.node(after).text.is_empty()
                    && !self.tree.has_children(after)
                    && self.tree.node(after).mirror == ROOT
                {
                    self.tree.delete(after);
                }
                self.edit = Some(Edit::new(Target::Text(m), self.tree.node(source).text.len()));
                self.commit_structure(before, cx);
            }
            None => {
                self.toast(t!("toast.mirror_into_self"));
                cx.notify();
            }
        }
    }

    // ----------------------------------------------------------- stars

    pub fn toggle_star(&mut self, _: &ToggleStar, _: &mut Window, cx: &mut Context<Self>) {
        let id = if self.zoom != ROOT && (self.edit.is_none() || self.editing_node() == Some(self.zoom)) {
            self.zoom
        } else {
            match self.editing_node().or(self.block.map(|b| b.head)) {
                Some(id) => id,
                None if self.zoom != ROOT => self.zoom,
                None => return,
            }
        };
        self.star(id, cx);
    }

    pub fn star(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if let Some(pos) = self.starred.iter().position(|s| *s == id) {
            self.starred.remove(pos);
            self.toast(t!("toast.unstarred", name = node_label(self, id)));
        } else {
            self.starred.push(id);
            self.toast(t!("toast.starred", name = node_label(self, id)));
        }
        let v = self.starred.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(",");
        self.set_meta("starred", &v);
        cx.notify();
    }

    // ---------------------------------------------------------- levels

    fn show_levels(&mut self, levels: usize, cx: &mut Context<Self>) {
        let before = None;
        self.tree.expand_to_level(self.zoom, levels);
        if let Some(id) = self.editing_node() {
            if !self.tree.ancestors(id).iter().all(|a| !self.tree.node(*a).collapsed || !self.tree.is_ancestor(self.zoom, *a)) {
                self.edit = None;
            }
        }
        self.commit_structure(before, cx);
    }

    pub fn expand_all(&mut self, _: &ExpandAll, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(usize::MAX, cx);
    }
    pub fn level1(&mut self, _: &Level1, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(1, cx);
    }
    pub fn level2(&mut self, _: &Level2, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(2, cx);
    }
    pub fn level3(&mut self, _: &Level3, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(3, cx);
    }
    pub fn level4(&mut self, _: &Level4, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(4, cx);
    }

    // ----------------------------------------------------- block kinds

    pub fn set_kind_of_targets(&mut self, kind: BlockKind, cx: &mut Context<Self>) {
        let mut ids = self.targets();
        if ids.is_empty() {
            if let Some(id) = self.editing_node() {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return;
        }
        // Toggle back to a bullet when every target already has this kind.
        let kind = if ids.iter().all(|id| self.tree.node(*id).kind == kind) { BlockKind::Bullet } else { kind };
        let before = None;
        for id in ids {
            self.tree.set_kind(id, kind);
        }
        self.commit_structure(before, cx);
    }

    pub fn kind_bullet(&mut self, _: &KindBullet, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Bullet, cx);
    }
    pub fn kind_h1(&mut self, _: &KindH1, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::H1, cx);
    }
    pub fn kind_h2(&mut self, _: &KindH2, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::H2, cx);
    }
    pub fn kind_h3(&mut self, _: &KindH3, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::H3, cx);
    }
    pub fn kind_paragraph(&mut self, _: &KindParagraph, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Paragraph, cx);
    }
    pub fn kind_todo(&mut self, _: &KindTodo, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Todo, cx);
    }
    pub fn kind_numbered(&mut self, _: &KindNumbered, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Numbered, cx);
    }
    pub fn kind_quote(&mut self, _: &KindQuote, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Quote, cx);
    }
    pub fn kind_code(&mut self, _: &KindCode, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Code, cx);
    }

    pub fn highlight(&mut self, _: &Highlight, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(crate::palette::Action::Wrap("==", "=="), window, cx);
    }

    /// Click on a to-do checkbox.
    pub fn toggle_checkbox(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let src = self.tree.source(id);
        let done = !self.tree.node(src).completed;
        self.tree.set_completed(src, done);
        self.anims.start(id, Channel::Complete, anim::FLAG);
        self.commit_structure(None, cx);
    }

    // --------------------------------------------------- import/export

    pub fn import(&mut self, _: &Import, _: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("import.prompt").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let mut items = Vec::new();
            for p in paths {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    items.extend(io::parse_any(&content));
                }
            }
            this.update(cx, |this, cx| this.import_items(items, cx)).ok();
        })
        .detach();
    }

    pub fn import_items(&mut self, items: Vec<io::Item>, cx: &mut Context<Self>) {
        if items.is_empty() {
            self.toast(t!("toast.nothing_to_import"));
            cx.notify();
            return;
        }
        let count = io::Item::count(&items);
        let (parent, index) = match self.editing_node().filter(|id| *id != self.zoom) {
            Some(id) => (self.tree.parent(id), self.tree.index_in_parent(id) + 1),
            None => (self.zoom, self.tree.children(self.zoom).len()),
        };
        let top = self.tree.insert_items(parent, index, &items);
        self.edit = None;
        self.block = top.first().zip(top.last()).map(|(a, h)| BlockSel { anchor: *a, head: *h });
        self.toast(t!("toast.imported", count = count));
        self.commit_structure(None, cx);
    }

    /// What to export: the selected bullets, or everything on the current page.
    fn export_items(&self) -> (String, Vec<io::Item>) {
        let ids = if self.block.is_some() { self.targets() } else { self.tree.children(self.zoom).to_vec() };
        let title = if self.zoom == ROOT { "Flowie".to_string() } else { node_label(self, self.zoom) };
        (title, self.tree.to_items(&ids))
    }

    fn export_with(&mut self, ext: &str, render: fn(&str, &[io::Item]) -> String, cx: &mut Context<Self>) {
        let (title, items) = self.export_items();
        let content = render(&title, &items);
        let safe: String = title.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c }).take(60).collect();
        let dir = std::env::var("HOME").map(|h| PathBuf::from(h).join("Desktop")).unwrap_or_else(|_| PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some(&format!("{safe}.{ext}")));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let msg = match std::fs::write(&path, content) {
                Ok(()) => t!("toast.exported", path = path.display()),
                Err(e) => t!("toast.export_failed", error = e),
            };
            this.update(cx, |this, cx| {
                this.toast(msg);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn export_opml(&mut self, _: &ExportOpml, _: &mut Window, cx: &mut Context<Self>) {
        self.export_with("opml", io::to_opml, cx);
    }

    pub fn export_markdown(&mut self, _: &ExportMarkdown, _: &mut Window, cx: &mut Context<Self>) {
        self.export_with("md", |_, items| io::to_markdown(items), cx);
    }

    pub fn export_text(&mut self, _: &ExportText, _: &mut Window, cx: &mut Context<Self>) {
        self.export_with("txt", |_, items| io::to_plain_text(items), cx);
    }

    /// Copy bullets with full fidelity for pasting inside Flowie, plus
    /// Markdown text for other apps.
    pub fn copy_items(&self, ids: &[NodeId], cx: &mut Context<Self>) {
        let items = self.tree.to_items(ids);
        let text = io::to_markdown(&items);
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(text, items));
    }

    // ----------------------------------------------------------- trash

    pub fn show_trash(&mut self, _: &ShowTrash, _: &mut Window, cx: &mut Context<Self>) {
        self.load_trash();
        self.edit = None;
        self.block = None;
        cx.notify();
    }

    pub fn load_trash(&mut self) {
        if let Some(w) = &self.writer {
            w.flush();
        }
        let entries =
            self.db_path.as_ref().and_then(|p| store::open(p).ok()).and_then(|c| store::list_trash(&c, 30).ok()).unwrap_or_default();
        // Hide entries whose bullet came back (undo or restore).
        self.trash = Some(entries.into_iter().filter(|e| !self.tree.contains(e.root)).collect());
    }

    pub fn restore_trash(&mut self, seq: i64, cx: &mut Context<Self>) {
        let Some(entry) = self.trash.as_ref().and_then(|t| t.iter().find(|e| e.seq == seq)).cloned() else { return };
        if let Some(root) = self.tree.restore(&entry.nodes) {
            if let Some(w) = &self.writer {
                w.untrash(seq);
            }
            self.toast(t!("toast.restored", name = entry.label));
            self.trash = None;
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
        if let Some(t) = self.trash.as_mut() {
            t.retain(|e| e.seq != seq);
        }
        cx.notify();
    }

    // ---------------------------------------------------------- search

    pub fn save_search(&mut self, _: &SaveSearch, _: &mut Window, cx: &mut Context<Self>) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        if let Some(pos) = self.saved_searches.iter().position(|s| *s == q) {
            self.saved_searches.remove(pos);
            self.toast(t!("toast.search_removed", query = q));
        } else {
            self.saved_searches.push(q.clone());
            self.toast(t!("toast.search_saved", query = q));
        }
        self.set_meta("saved_searches", &self.saved_searches.join("\n"));
        cx.notify();
    }

    fn jump_match(&mut self, dir: isize, cx: &mut Context<Self>) {
        let q = Query::parse(&self.query);
        if q.is_empty() {
            return;
        }
        let hits: Vec<NodeId> = self.rows.iter().map(|r| r.id).filter(|id| q.matches(self.tree.node(self.tree.source(*id)))).collect();
        if hits.is_empty() {
            return;
        }
        let cur = self.editing_node().and_then(|id| hits.iter().position(|h| *h == id));
        let next = match cur {
            Some(i) => (i as isize + dir).rem_euclid(hits.len() as isize) as usize,
            None if dir > 0 => 0,
            None => hits.len() - 1,
        };
        let id = hits[next];
        let len = self.tree.node(self.tree.source(id)).text.len();
        self.start_edit(Target::Text(id), len, cx);
        self.toast(t!("toast.match", index = next + 1, total = hits.len()));
    }

    pub fn next_match(&mut self, _: &NextMatch, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_match(1, cx);
    }

    pub fn prev_match(&mut self, _: &PrevMatch, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_match(-1, cx);
    }

    // --------------------------------------------------------- journal

    /// The "Journal" bullet (created on first use).
    pub fn journal_root(&mut self) -> NodeId {
        if let Some(id) = self.journal_id.filter(|id| self.tree.contains(*id)) {
            return id;
        }
        let existing = self.tree.children(ROOT).iter().copied().find(|id| markup::plain_text(&self.tree.node(*id).text) == "Journal");
        let id = existing.unwrap_or_else(|| {
            let n = self.tree.children(ROOT).len();
            self.tree.create_at(ROOT, n, "Journal")
        });
        self.journal_id = Some(id);
        self.set_meta("journal", &id.to_string());
        id
    }

    /// Zoom into the daily note for `date` (YYYY-MM-DD), creating it if needed.
    /// The journal page for `date` (YYYY-MM-DD), created if needed (not committed).
    pub fn ensure_day(&mut self, date: &str, with_blank: bool) -> NodeId {
        let journal = self.journal_root();
        let existing = self.tree.children(journal).iter().copied().find(|id| self.tree.node(*id).text.starts_with(date));
        match existing {
            Some(d) => d,
            None => {
                // Keep days sorted newest first.
                let index = self
                    .tree
                    .children(journal)
                    .iter()
                    .position(|id| self.tree.node(*id).text.as_str() < date)
                    .unwrap_or(self.tree.children(journal).len());
                let d = self.tree.create_at(journal, index, date);
                if with_blank {
                    self.tree.create_at(d, 0, "");
                }
                d
            }
        }
    }

    pub fn open_day(&mut self, date: &str, cx: &mut Context<Self>) {
        let day = self.ensure_day(date, true);
        self.commit_structure(None, cx);
        self.calendar = None;
        self.zoom_to(day, cx);
        if let Some(first) = self.rows.first().map(|r| r.id) {
            let len = self.tree.node(first).text.len();
            self.start_edit(Target::Text(first), len, cx);
        }
    }

    pub fn today(&mut self, _: &Today, _: &mut Window, cx: &mut Context<Self>) {
        let d = today();
        self.open_day(&d, cx);
    }
}
