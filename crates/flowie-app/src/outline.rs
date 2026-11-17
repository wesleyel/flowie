//! The outline view's state and every user command. Rendering lives in
//! `view.rs`; text input plumbing (IME, caret) in `editor.rs`.

use std::ops::Range;

use flowie_core::{
    merge_leaving, store::Writer, Caret, DisplayRow, History, HistoryKind, NodeId, Query, Row,
    Tree, ViewOptions, ROOT,
};
use gpui::{
    actions, px, Bounds, ClipboardItem, Context, FocusHandle, ListAlignment, ListState, Pixels, TextLayout,
    Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::anim::{self, Anims, Channel, Tween};
use crate::theme::Theme;

actions!(
    flowie,
    [
        Newline,
        ToggleNote,
        Indent,
        Outdent,
        Backspace,
        Delete,
        DeleteWordLeft,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectUp,
        SelectDown,
        Home,
        End,
        SelectHome,
        SelectEnd,
        SelectAll,
        ToggleComplete,
        Collapse,
        Expand,
        ZoomIn,
        ZoomOut,
        ZoomHome,
        MoveUp,
        MoveDown,
        DeleteNode,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        Bold,
        Italic,
        InlineCode,
        WrapMath,
        InsertLink,
        Find,
        Escape,
        ToggleCompletedVisible,
        Quit,
        Jump,
        MoveTo,
        LinkToBullet,
        MirrorBullet,
        ToggleStar,
        ExpandAll,
        Level1,
        Level2,
        Level3,
        Level4,
        Import,
        ExportOpml,
        ExportMarkdown,
        ExportText,
        ShowTrash,
        SaveSearch,
        Highlight,
        KindBullet,
        KindH1,
        KindH2,
        KindH3,
        KindParagraph,
        KindTodo,
        KindNumbered,
        KindQuote,
        KindCode,
        Today,
        NextMatch,
        PrevMatch,
        ShowCalendar,
        OpenSettings,
        OpenAbout,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Text(NodeId),
    Note(NodeId),
    Search,
    Palette,
}

pub struct Edit {
    pub target: Target,
    pub sel: Range<usize>,
    pub reversed: bool,
    pub marked: Option<Range<usize>>,
    /// Layout of the editor's text from the last paint (caret mapping, IME).
    pub layout: Option<TextLayout>,
    pub bounds: Option<Bounds<Pixels>>,
    pub selecting_with_mouse: bool,
}

impl Edit {
    pub fn new(target: Target, offset: usize) -> Edit {
        Edit {
            target,
            sel: offset..offset,
            reversed: false,
            marked: None,
            layout: None,
            bounds: None,
            selecting_with_mouse: false,
        }
    }

    pub fn caret(&self) -> usize {
        if self.reversed {
            self.sel.start
        } else {
            self.sel.end
        }
    }
}

/// Block selection of whole rows (Esc from the editor, Shift-↑/↓).
#[derive(Debug, Clone, Copy)]
pub struct BlockSel {
    pub anchor: NodeId,
    pub head: NodeId,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropTarget {
    /// Show the insertion line above this node (or below it if `below`).
    pub node: NodeId,
    pub below: bool,
    pub depth: u16,
    pub parent: NodeId,
    pub index: usize,
}

#[derive(Clone)]
pub struct DragNode {
    pub id: NodeId,
    pub label: String,
}

pub struct Outline {
    pub focus: FocusHandle,
    pub tree: Tree,
    pub history: History,
    pub writer: Option<Writer>,
    pub zoom: NodeId,
    pub hide_completed: bool,
    pub query: String,
    pub rows: Vec<Row>,
    pub display: Vec<DisplayRow>,
    pub list: ListState,
    pub edit: Option<Edit>,
    pub block: Option<BlockSel>,
    pub anims: Anims,
    pub zoom_tween: Option<Tween>,
    pub theme: Theme,
    pub drop: Option<DropTarget>,
    pub highlight_terms: Vec<String>,
    pub palette: Option<crate::palette::Palette>,
    pub inline: Option<crate::palette::Inline>,
    pub inline_entries_len: usize,
    pub block_saved: Option<BlockSel>,
    pub starred: Vec<NodeId>,
    pub saved_searches: Vec<String>,
    /// Trash page contents while it is open.
    pub trash: Option<Vec<flowie_core::store::TrashEntry>>,
    pub db_path: Option<std::path::PathBuf>,
    saved_next_id: NodeId,
    pub calendar: Option<crate::calendar::Calendar>,
    pub journal_id: Option<NodeId>,
    /// A block selection is being extended by dragging the mouse.
    pub mouse_block: bool,
    /// Snapshots of deleted nodes, so their rows can animate out.
    pub ghosts: std::collections::HashMap<NodeId, flowie_core::Node>,
    /// Scroll position of the open popup list (palette or inline menu).
    pub popup_scroll: gpui::ScrollHandle,
    pub settings: Option<crate::settings::SettingsUi>,
    pub settings_return: Option<(Target, usize)>,
    pub config: crate::settings::Config,
    pub config_path: std::path::PathBuf,
    /// Selectable read-only labels, keyed by name.
    pub labels: std::collections::HashMap<&'static str, gpui::Entity<crate::selectable::SelectableText>>,
    pub toast: Option<(String, Tween)>,
}

pub const INDENT: f32 = 24.0;
pub const PAD_X: f32 = 36.0;

impl Outline {
    pub fn new(
        mut tree: Tree,
        writer: Option<Writer>,
        db_path: Option<std::path::PathBuf>,
        meta: Vec<(String, String)>,
        config: crate::settings::Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let db_path_for_config = db_path.clone();
        let mut zoom = ROOT;
        let mut hide_completed = false;
        let mut starred = Vec::new();
        let mut saved_searches = Vec::new();
        for (k, v) in &meta {
            match k.as_str() {
                "zoom" => zoom = v.parse().unwrap_or(ROOT),
                "hide_completed" => hide_completed = v == "1",
                "next_id" => tree.ensure_next_id(v.parse().unwrap_or(0)),
                "starred" => starred = v.split(',').filter_map(|x| x.parse().ok()).collect(),
                "saved_searches" => saved_searches = v.split('\n').filter(|x| !x.is_empty()).map(String::from).collect(),
                _ => {}
            }
        }
        let saved_next_id = tree.next_id();
        if !tree.contains(zoom) {
            zoom = ROOT;
        }
        let mut this = Outline {
            focus: cx.focus_handle(),
            tree,
            history: History::new(),
            writer,
            zoom,
            hide_completed,
            query: String::new(),
            rows: Vec::new(),
            display: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            edit: None,
            block: None,
            anims: Anims::default(),
            zoom_tween: None,
            theme: Theme::build(window.appearance(), &config),
            drop: None,
            highlight_terms: Vec::new(),
            palette: None,
            inline: None,
            inline_entries_len: 0,
            block_saved: None,
            starred,
            saved_searches,
            trash: None,
            db_path,
            saved_next_id,
            calendar: None,
            journal_id: meta.iter().find(|(k, _)| k == "journal").and_then(|(_, v)| v.parse().ok()),
            toast: None,
            mouse_block: false,
            ghosts: Default::default(),
            popup_scroll: gpui::ScrollHandle::new(),
            settings: None,
            settings_return: None,
            config_path: crate::settings::Config::path(db_path_for_config.as_ref()),
            labels: Default::default(),
            config,
        };
        if this.tree.children(ROOT).is_empty() {
            this.seed_welcome();
        }
        this.refresh(false);
        if let Some(first) = this.rows.first() {
            this.edit = Some(Edit::new(Target::Text(first.id), this.tree.node(first.id).text.len()));
        }
        cx.observe_window_appearance(window, |this, window, cx| {
            this.apply_theme(window);
            cx.notify();
        })
        .detach();
        this
    }

    fn seed_welcome(&mut self) {
        let text = "Welcome to Flowie\n\
            \tEnter adds a bullet, Tab / Shift-Tab indents and outdents\n\
            \tClick a bullet (or ⌘]) to zoom in, ⌘[ to zoom out\n\
            \t⌘↑ / ⌘↓ collapse and expand, ⌘↩ completes\n\
            \t\t**Bold**, *italic*, `code`, [links](https://example.com), #tags and @people\n\
            \t\tMath: $e^{i\\pi}+1=0$ and $\\int_0^1 x^2\\,dx=\\frac{1}{3}$\n\
            \t⌘F searches everything instantly\n\
            \tEsc selects whole bullets; Shift-↑/↓ extends; drag a bullet to move it\n";
        let top = self.tree.insert_indented_text(ROOT, 0, text);
        let _ = top;
        self.tree.take_journal();
        self.persist();
    }

    // ---------------------------------------------------------------- rows

    pub fn view_options(&self) -> ViewOptions {
        let q = Query::parse(&self.query);
        ViewOptions { hide_completed: self.hide_completed, query: (!q.is_empty()).then_some(q) }
    }

    /// Recompute visible rows. With `animate`, rows that appear grow in and
    /// rows that disappear shrink out.
    pub fn refresh(&mut self, animate: bool) {
        let opts = self.view_options();
        self.highlight_terms = opts.query.as_ref().map(|q| q.highlight_terms()).unwrap_or_default();
        let (new_rows, virtuals) = flowie_core::flatten_mapped(&self.tree, self.zoom, &opts);
        self.tree.add_virtuals(virtuals);
        let old_display = std::mem::take(&mut self.display);
        let new_display: Vec<DisplayRow> = if animate {
            let old: Vec<Row> = old_display
                .iter()
                .filter(|d| !d.leaving || !self.anims.is_leaving_done(d.row.id))
                .filter(|d| self.tree.contains(d.row.id) || self.ghosts.contains_key(&self.tree.resolve(d.row.id)) || d.leaving)
                .map(|d| d.row)
                .collect();
            let old_ids: std::collections::HashSet<NodeId> = old_display.iter().filter(|d| !d.leaving).map(|d| d.row.id).collect();
            let merged = merge_leaving(&old, &new_rows);
            for d in &merged {
                if d.leaving {
                    if self.anims.t(d.row.id, Channel::Leave).is_none() {
                        self.anims.start(d.row.id, Channel::Leave, anim::ROW_OUT);
                    }
                } else {
                    self.anims.clear_leave(d.row.id);
                    if !old_ids.contains(&d.row.id) {
                        self.anims.start(d.row.id, Channel::Enter, anim::ROW_IN);
                    }
                }
            }
            // Leaving rows of deleted nodes render from their snapshot.
            merged
                .into_iter()
                .filter(|d| !d.leaving || self.tree.contains(d.row.id) || self.ghosts.contains_key(&self.tree.resolve(d.row.id)))
                .collect()
        } else {
            new_rows.iter().map(|&row| DisplayRow { row, leaving: false }).collect()
        };
        self.rows = new_rows;
        self.splice_list(&old_display, &new_display);
        self.display = new_display;
    }

    /// Tell the list which items changed, preserving scroll position.
    fn splice_list(&self, old: &[DisplayRow], new: &[DisplayRow]) {
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let old_end = old.len() - suffix;
        let new_count = new.len() - suffix - prefix;
        // The list has one extra item at the end: the click-to-add footer.
        if self.list.item_count() != old.len() + 1 {
            self.list.reset(new.len() + 1);
        } else if old_end > prefix || new_count > 0 {
            self.list.splice(prefix..old_end, new_count);
        }
    }

    /// Called each frame while animating: drop rows that finished leaving.
    pub fn tick(&mut self) {
        if self.display.iter().any(|d| d.leaving && self.anims.is_leaving_done(d.row.id)) {
            let old = self.display.clone();
            let ids: Vec<NodeId> = old
                .iter()
                .filter(|d| d.leaving && self.anims.is_leaving_done(d.row.id))
                .map(|d| d.row.id)
                .collect();
            self.display.retain(|d| !(d.leaving && self.anims.is_leaving_done(d.row.id)));
            for id in ids {
                self.anims.clear_leave(id);
                let real = self.tree.resolve(id);
                if !self.tree.contains(real) {
                    self.ghosts.remove(&real);
                }
            }
            let new = self.display.clone();
            self.splice_list(&old, &new);
        }
        self.anims.gc();
        if !self.ghosts.is_empty() && !self.display.iter().any(|d| d.leaving) {
            self.ghosts.clear();
        }
        if self.zoom_tween.is_some_and(|t| t.done()) {
            self.zoom_tween = None;
        }
    }

    pub fn is_animating(&self) -> bool {
        self.anims.any_running()
            || self.zoom_tween.is_some()
            || self.display.iter().any(|d| d.leaving)
    }

    pub fn row_index(&self, id: NodeId) -> Option<usize> {
        self.rows.iter().position(|r| r.id == id)
    }

    pub fn display_index(&self, id: NodeId) -> Option<usize> {
        self.display.iter().position(|d| d.row.id == id && !d.leaving)
    }

    // ------------------------------------------------------------ commits

    fn caret(&self) -> Option<Caret> {
        match self.edit.as_ref()?.target {
            Target::Text(id) | Target::Note(id) => Some(Caret { node: id, offset: self.edit.as_ref()?.caret() }),
            Target::Search | Target::Palette => None,
        }
    }

    fn persist(&mut self) {
        let (up, del) = self.tree.take_dirty();
        if let Some(w) = &self.writer {
            w.write(up, del);
            if self.tree.next_id() != self.saved_next_id {
                self.saved_next_id = self.tree.next_id();
                w.set_meta("next_id", &self.saved_next_id.to_string());
            }
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Tween::new(std::time::Duration::from_millis(2600))));
    }

    /// Finish a structural command: journal for undo, persist, re-flatten with animation.
    pub fn commit_structure(&mut self, before: Option<Caret>, cx: &mut Context<Self>) {
        for n in self.tree.journal_removed() {
            self.ghosts.insert(n.id, n);
        }
        let after = self.caret();
        self.history.commit(&mut self.tree, HistoryKind::Structure, before, after);
        self.persist();
        self.refresh(true);
        self.reveal_edit();
        cx.notify();
    }

    fn commit_typing(&mut self, id: NodeId, before: Option<Caret>, cx: &mut Context<Self>) {
        let after = self.caret();
        self.history.commit(&mut self.tree, HistoryKind::Typing(id), before, after);
        self.persist();
        if !self.query.is_empty() {
            self.refresh(false);
        }
        cx.notify();
    }

    pub fn reveal_edit(&self) {
        if let Some(Target::Text(id) | Target::Note(id)) = self.edit.as_ref().map(|e| e.target) {
            if let Some(ix) = self.display_index(id) {
                self.list.scroll_to_reveal_item(ix);
            }
        }
    }

    pub fn set_meta(&self, key: &str, value: &str) {
        if let Some(w) = &self.writer {
            w.set_meta(key, value);
        }
    }

    pub fn flush(&mut self) {
        self.persist();
        if let Some(w) = &self.writer {
            w.flush();
        }
    }

    // ---------------------------------------------------------- editing

    pub fn edit_text(&self) -> &str {
        match self.edit.as_ref().map(|e| e.target) {
            Some(Target::Text(id)) => self.tree.get(self.tree.source(id)).map(|n| n.text.as_str()).unwrap_or(""),
            Some(Target::Note(id)) => self.tree.get(self.tree.source(id)).map(|n| n.note.as_str()).unwrap_or(""),
            Some(Target::Search) => &self.query,
            Some(Target::Palette) => self.palette.as_ref().map(|p| p.query.as_str()).unwrap_or(""),
            None => "",
        }
    }

    /// Replace `range` of the edited text with `new`, leaving the caret after it.
    pub fn replace_edit_text(&mut self, range: Range<usize>, new: &str, mark: bool, cx: &mut Context<Self>) {
        let Some(target) = self.edit.as_ref().map(|e| e.target) else { return };
        let before = self.caret();
        let old = self.edit_text().to_string();
        let range = range.start.min(old.len())..range.end.min(old.len());
        // Bullet text is single-line: newlines typed or pasted become spaces.
        let new = match target {
            Target::Note(_) => new.to_string(),
            _ => new.replace(['\n', '\r'], " "),
        };
        let text = format!("{}{}{}", &old[..range.start], new, &old[range.end..]);
        let caret = range.start + new.len();
        if let Some(e) = self.edit.as_mut() {
            e.sel = caret..caret;
            e.reversed = false;
            e.marked = (mark && !new.is_empty()).then(|| range.start..caret);
        }
        match target {
            Target::Text(id) => {
                let src = self.tree.source(id);
                self.tree.set_text(src, &text);
                self.commit_typing(src, before, cx);
                if !mark {
                    self.check_inline(&new);
                }
            }
            Target::Palette => {
                if let Some(p) = self.palette.as_mut() {
                    p.query = text;
                    p.selected = 0;
                }
                self.refresh_palette();
                cx.notify();
            }
            Target::Note(id) => {
                let src = self.tree.source(id);
                self.tree.set_note(src, &text);
                self.commit_typing(src, before, cx);
            }
            Target::Search => {
                self.query = text;
                self.refresh(false);
                self.list.scroll_to(gpui::ListOffset { item_ix: 0, offset_in_item: px(0.) });
                cx.notify();
            }
        }
    }

    pub fn start_edit(&mut self, target: Target, offset: usize, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            if e.target != target {
                self.history.seal();
            }
        }
        self.block = None;
        self.inline = None;
        let len = match target {
            Target::Text(id) => self.tree.get(self.tree.source(id)).map_or(0, |n| n.text.len()),
            Target::Note(id) => self.tree.get(self.tree.source(id)).map_or(0, |n| n.note.len()),
            Target::Search => self.query.len(),
            Target::Palette => self.palette.as_ref().map_or(0, |p| p.query.len()),
        };
        self.edit = Some(Edit::new(target, offset.min(len)));
        self.reveal_edit();
        cx.notify();
    }

    pub fn editing_node(&self) -> Option<NodeId> {
        match self.edit.as_ref()?.target {
            Target::Text(id) | Target::Note(id) => Some(id),
            Target::Search | Target::Palette => None,
        }
    }

    fn edit_target(&self) -> Option<Target> {
        self.edit.as_ref().map(|e| e.target)
    }

    fn move_caret(&mut self, offset: usize, select: bool, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_mut() else { return };
        if select {
            if e.reversed {
                e.sel.start = offset;
            } else {
                e.sel.end = offset;
            }
            if e.sel.end < e.sel.start {
                e.reversed = !e.reversed;
                e.sel = e.sel.end..e.sel.start;
            }
        } else {
            e.sel = offset..offset;
            e.reversed = false;
        }
        cx.notify();
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        self.edit_text()
            .grapheme_indices(true)
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        let text = self.edit_text();
        text.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(text.len())
    }

    fn prev_word(&self, offset: usize) -> usize {
        let text = self.edit_text();
        text.split_word_bound_indices()
            .rev()
            .find(|(i, w)| *i < offset && !w.trim().is_empty())
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next_word(&self, offset: usize) -> usize {
        let text = self.edit_text();
        text.split_word_bound_indices()
            .find(|(i, w)| i + w.len() > offset && !w.trim().is_empty())
            .map(|(i, w)| i + w.len())
            .unwrap_or(text.len())
    }

    /// The row above/below the given node in display order (None at the edges).
    fn neighbor(&self, id: NodeId, delta: isize) -> Option<NodeId> {
        let ix = self.row_index(id)? as isize + delta;
        (ix >= 0).then(|| self.rows.get(ix as usize).map(|r| r.id)).flatten()
    }

    fn char_col(&self) -> usize {
        let Some(e) = &self.edit else { return 0 };
        self.edit_text()[..e.caret().min(self.edit_text().len())].chars().count()
    }

    fn offset_for_col(text: &str, col: usize) -> usize {
        text.char_indices().nth(col).map(|(i, _)| i).unwrap_or(text.len())
    }

    // ---------------------------------------------------------- actions

    pub fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_confirm(window, cx) {
            return;
        }
        match self.edit_target() {
            Some(Target::Text(id)) => {
                if id == self.zoom {
                    // Enter in the title adds a first child.
                    let before = self.caret();
                    let child = self.tree.create_at(id, 0, "");
                    self.edit = Some(Edit::new(Target::Text(child), 0));
                    self.commit_structure(before, cx);
                    return;
                }
                let before = self.caret();
                if self.tree.node(id).mirror != ROOT {
                    // Enter on a mirror adds a plain sibling after it.
                    let n = self.tree.create_after(id, "");
                    self.edit = Some(Edit::new(Target::Text(n), 0));
                    self.commit_structure(before, cx);
                    return;
                }
                let caret = self.edit.as_ref().unwrap().caret();
                if let Some(e) = self.edit.as_ref() {
                    if !e.sel.is_empty() {
                        let r = e.sel.clone();
                        let text = self.edit_text().to_string();
                        self.tree.set_text(id, &format!("{}{}", &text[..r.start], &text[r.end..]));
                    }
                }
                let caret = self.edit.as_ref().map(|e| e.sel.start).unwrap_or(caret);
                let (focus, off) = self.tree.split(id, caret);
                self.edit = Some(Edit::new(Target::Text(focus), off));
                self.commit_structure(before, cx);
            }
            Some(Target::Note(_)) => {
                let r = self.edit.as_ref().unwrap().sel.clone();
                self.replace_edit_text(r, "\n", false, cx);
            }
            Some(Target::Search) => {
                if let Some(first) = self.rows.first().map(|r| r.id) {
                    let len = self.tree.node(first).text.len();
                    self.start_edit(Target::Text(first), len, cx);
                }
            }
            Some(Target::Palette) => {}
            None => self.enter_from_idle(cx),
        }
    }

    fn enter_from_idle(&mut self, cx: &mut Context<Self>) {
        if let Some(b) = self.block {
            let len = self.tree.node(b.head).text.len();
            self.start_edit(Target::Text(b.head), len, cx);
        } else if let Some(first) = self.rows.first().map(|r| r.id) {
            self.start_edit(Target::Text(first), 0, cx);
        } else {
            self.append_child_and_edit(cx);
        }
    }

    pub fn append_child_and_edit(&mut self, cx: &mut Context<Self>) {
        // Reuse a trailing empty bullet instead of piling up blanks.
        if let Some(&last) = self.tree.children(self.zoom).last() {
            if self.tree.node(last).text.is_empty() && !self.tree.has_children(last) {
                self.start_edit(Target::Text(last), 0, cx);
                return;
            }
        }
        let before = self.caret();
        let n = self.tree.children(self.zoom).len();
        let id = self.tree.create_at(self.zoom, n, "");
        self.edit = Some(Edit::new(Target::Text(id), 0));
        self.commit_structure(before, cx);
    }

    pub fn toggle_note(&mut self, _: &ToggleNote, _: &mut Window, cx: &mut Context<Self>) {
        match self.edit_target() {
            Some(Target::Text(id)) if id != self.zoom || self.zoom != ROOT => {
                let len = self.tree.node(id).note.len();
                self.start_edit(Target::Note(id), len, cx);
            }
            Some(Target::Note(id)) => {
                let len = self.tree.node(id).text.len();
                self.start_edit(Target::Text(id), len, cx);
            }
            _ => {}
        }
    }

    pub fn indent(&mut self, _: &Indent, window: &mut Window, cx: &mut Context<Self>) {
        if self.inline.is_some() && self.inline_confirm(None, window, cx) {
            return;
        }
        if self.palette.is_some() {
            return;
        }
        let before = self.caret();
        if let Some(ids) = self.block_roots() {
            let mut ok = false;
            for id in ids {
                ok |= self.tree.indent(id);
            }
            if ok {
                self.commit_structure(before, cx);
            }
            return;
        }
        if let Some(Target::Text(id) | Target::Note(id)) = self.edit_target() {
            if id != self.zoom && self.tree.indent(id) {
                self.commit_structure(before, cx);
            }
        }
    }

    pub fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        if let Some(ids) = self.block_roots() {
            let mut ok = false;
            for id in ids.into_iter().rev() {
                ok |= self.tree.outdent(id, self.zoom);
            }
            if ok {
                self.commit_structure(before, cx);
            }
            return;
        }
        if let Some(Target::Text(id) | Target::Note(id)) = self.edit_target() {
            if id != self.zoom && self.tree.outdent(id, self.zoom) {
                self.commit_structure(before, cx);
            }
        }
    }

    pub fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.block.is_some() {
            self.delete_node(&DeleteNode, window, cx);
            return;
        }
        let Some(e) = self.edit.as_ref() else { return };
        if !e.sel.is_empty() {
            let r = e.sel.clone();
            self.replace_edit_text(r, "", false, cx);
            return;
        }
        let caret = e.caret();
        if caret > 0 {
            let start = self.prev_boundary(caret);
            self.replace_edit_text(start..caret, "", false, cx);
            return;
        }
        // At the very start: merge into the row above.
        if let Some(Target::Text(id)) = self.edit_target() {
            if id == self.zoom {
                return;
            }
            let before = self.caret();
            let Some(prev) = self.neighbor(id, -1) else { return };
            let node = self.tree.node(id);
            if node.text.is_empty() && !self.tree.has_children(id) {
                self.tree.delete(id);
                let len = self.tree.node(prev).text.len();
                self.edit = Some(Edit::new(Target::Text(prev), len));
                self.commit_structure(before, cx);
                return;
            }
            if self.tree.has_children(id) && (prev == self.tree.parent(id) || self.tree.has_children(prev)) {
                return;
            }
            if let Some(off) = self.tree.merge_into(id, prev) {
                self.edit = Some(Edit::new(Target::Text(prev), off));
                self.commit_structure(before, cx);
            }
        } else if let Some(Target::Note(id)) = self.edit_target() {
            if self.tree.node(id).note.is_empty() {
                let len = self.tree.node(id).text.len();
                self.start_edit(Target::Text(id), len, cx);
            }
        }
    }

    pub fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.block.is_some() {
            self.delete_node(&DeleteNode, window, cx);
            return;
        }
        let Some(e) = self.edit.as_ref() else { return };
        if !e.sel.is_empty() {
            let r = e.sel.clone();
            self.replace_edit_text(r, "", false, cx);
            return;
        }
        let caret = e.caret();
        if caret < self.edit_text().len() {
            let end = self.next_boundary(caret);
            self.replace_edit_text(caret..end, "", false, cx);
            return;
        }
        if let Some(Target::Text(id)) = self.edit_target() {
            let Some(next) = self.neighbor(id, 1) else { return };
            if self.tree.has_children(next) {
                return;
            }
            let before = self.caret();
            if let Some(off) = self.tree.merge_into(next, id) {
                self.edit = Some(Edit::new(Target::Text(id), off));
                self.commit_structure(before, cx);
            }
        }
    }

    pub fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        let caret = e.caret();
        let start = if e.sel.is_empty() { self.prev_word(caret) } else { e.sel.start };
        let end = if e.sel.is_empty() { caret } else { e.sel.end };
        self.replace_edit_text(start..end, "", false, cx);
    }

    pub fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        if !e.sel.is_empty() {
            let s = e.sel.start;
            self.move_caret(s, false, cx);
            return;
        }
        let caret = e.caret();
        if caret == 0 {
            if let Some(Target::Text(id)) = self.edit_target() {
                if let Some(prev) = self.neighbor(id, -1) {
                    let len = self.tree.node(prev).text.len();
                    self.start_edit(Target::Text(prev), len, cx);
                }
            }
            return;
        }
        let to = self.prev_boundary(caret);
        self.move_caret(to, false, cx);
    }

    pub fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        if !e.sel.is_empty() {
            let s = e.sel.end;
            self.move_caret(s, false, cx);
            return;
        }
        let caret = e.caret();
        if caret >= self.edit_text().len() {
            if let Some(Target::Text(id)) = self.edit_target() {
                if let Some(next) = self.neighbor(id, 1) {
                    self.start_edit(Target::Text(next), 0, cx);
                }
            }
            return;
        }
        let to = self.next_boundary(caret);
        self.move_caret(to, false, cx);
    }

    pub fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.prev_word(e.caret());
            self.move_caret(to, false, cx);
        }
    }

    pub fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.next_word(e.caret());
            self.move_caret(to, false, cx);
        }
    }

    pub fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.prev_boundary(e.caret());
            self.move_caret(to, true, cx);
        }
    }

    pub fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.next_boundary(e.caret());
            self.move_caret(to, true, cx);
        }
    }

    pub fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.prev_word(e.caret());
            self.move_caret(to, true, cx);
        }
    }

    pub fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            let to = self.next_word(e.caret());
            self.move_caret(to, true, cx);
        }
    }

    pub fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(0, false, cx);
    }

    pub fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.edit_text().len();
        self.move_caret(len, false, cx);
    }

    pub fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(0, true, cx);
    }

    pub fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.edit_text().len();
        self.move_caret(len, true, cx);
    }

    pub fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = self.edit.as_mut() {
            let len = match e.target {
                Target::Text(id) => self.tree.node(self.tree.source(id)).text.len(),
                Target::Note(id) => self.tree.node(self.tree.source(id)).note.len(),
                Target::Search => self.query.len(),
                Target::Palette => self.palette.as_ref().map_or(0, |p| p.query.len()),
            };
            if e.sel == (0..len) && matches!(e.target, Target::Text(_)) {
                // Second ⌘A selects every visible bullet.
                if let (Some(first), Some(last)) = (self.rows.first(), self.rows.last()) {
                    self.block = Some(BlockSel { anchor: first.id, head: last.id });
                    self.edit = None;
                }
            } else {
                e.sel = 0..len;
                e.reversed = false;
            }
            cx.notify();
        } else if let (Some(first), Some(last)) = (self.rows.first(), self.rows.last()) {
            self.block = Some(BlockSel { anchor: first.id, head: last.id });
            cx.notify();
        }
    }

    /// Visual line movement inside the editor; leaves the row at its edges.
    fn vertical(&mut self, dir: isize, select: bool, cx: &mut Context<Self>) -> bool {
        let Some(e) = self.edit.as_ref() else { return false };
        let (Some(layout), Some(bounds)) = (e.layout.clone(), e.bounds) else { return false };
        let caret = e.caret().min(self.edit_text().len());
        let Some(pos) = layout.position_for_index(caret) else { return false };
        let lh = layout.line_height();
        let target_y = if dir < 0 { pos.y - lh * 0.5 } else { pos.y + lh * 1.5 };
        if target_y < bounds.top() || target_y > bounds.bottom() {
            return false;
        }
        let ix = match layout.index_for_position(gpui::point(pos.x, target_y)) {
            Ok(i) | Err(i) => i,
        };
        self.move_caret(ix.min(self.edit_text().len()), select, cx);
        true
    }

    pub fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_move(-1, cx) {
            return;
        }
        if let Some(b) = self.block {
            if let Some(prev) = self.neighbor(b.head, -1) {
                self.block = Some(BlockSel { anchor: prev, head: prev });
                self.reveal(prev);
                cx.notify();
            }
            return;
        }
        if self.vertical(-1, false, cx) {
            return;
        }
        let col = self.char_col();
        match self.edit_target() {
            Some(Target::Text(id)) => {
                let prev = self.neighbor(id, -1).or_else(|| {
                    // From the first row, go up into the zoomed title.
                    (self.zoom != ROOT && self.rows.first().map(|r| r.id) == Some(id)).then_some(self.zoom)
                });
                if let Some(prev) = prev {
                    let off = Self::offset_for_col(&self.tree.node(prev).text, col);
                    self.start_edit(Target::Text(prev), off, cx);
                }
            }
            Some(Target::Note(id)) => self.start_edit(Target::Text(id), 0, cx),
            None => self.enter_from_idle(cx),
            _ => {}
        }
    }

    pub fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_move(1, cx) {
            return;
        }
        if let Some(b) = self.block {
            if let Some(next) = self.neighbor(b.head, 1) {
                self.block = Some(BlockSel { anchor: next, head: next });
                self.reveal(next);
                cx.notify();
            }
            return;
        }
        if self.vertical(1, false, cx) {
            return;
        }
        let col = self.char_col();
        match self.edit_target() {
            Some(Target::Text(id)) => {
                let next = if id == self.zoom { self.rows.first().map(|r| r.id) } else { self.neighbor(id, 1) };
                if let Some(next) = next {
                    let off = Self::offset_for_col(&self.tree.node(next).text, col);
                    self.start_edit(Target::Text(next), off, cx);
                }
            }
            Some(Target::Note(id)) => {
                if let Some(next) = self.neighbor(id, 1) {
                    self.start_edit(Target::Text(next), 0, cx);
                }
            }
            Some(Target::Search) => {
                if let Some(first) = self.rows.first().map(|r| r.id) {
                    self.start_edit(Target::Text(first), 0, cx);
                }
            }
            Some(Target::Palette) => {}
            None => self.enter_from_idle(cx),
        }
    }

    fn reveal(&self, id: NodeId) {
        if let Some(ix) = self.display_index(id) {
            self.list.scroll_to_reveal_item(ix);
        }
    }

    pub fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_block(-1, cx);
    }

    pub fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.extend_block(1, cx);
    }

    fn extend_block(&mut self, dir: isize, cx: &mut Context<Self>) {
        if let Some(b) = self.block {
            if let Some(n) = self.neighbor(b.head, dir) {
                self.block = Some(BlockSel { anchor: b.anchor, head: n });
                self.reveal(n);
                cx.notify();
            }
            return;
        }
        if self.vertical(dir, true, cx) {
            return;
        }
        let Some(e) = self.edit.as_ref() else { return };
        let Target::Text(id) = e.target else { return };
        let len = self.edit_text().len();
        let at_edge = if dir < 0 { e.sel.start == 0 && e.sel.end == e.caret() && e.caret() == 0 } else { e.caret() == len };
        if !at_edge && !(dir < 0 && e.caret() == 0) {
            let to = if dir < 0 { 0 } else { len };
            self.move_caret(to, true, cx);
            return;
        }
        if id == self.zoom {
            return;
        }
        let head = self.neighbor(id, dir).unwrap_or(id);
        self.block = Some(BlockSel { anchor: id, head });
        self.edit = None;
        cx.notify();
    }

    /// Selected row ids in display order.
    pub fn block_ids(&self) -> Vec<NodeId> {
        let Some(b) = self.block else { return Vec::new() };
        let (Some(a), Some(h)) = (self.row_index(b.anchor), self.row_index(b.head)) else { return Vec::new() };
        let (lo, hi) = (a.min(h), a.max(h));
        self.rows[lo..=hi].iter().map(|r| r.id).collect()
    }

    /// Selected rows without those whose ancestor is also selected.
    fn block_roots(&self) -> Option<Vec<NodeId>> {
        self.block?;
        let ids = self.block_ids();
        let set: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
        Some(ids.into_iter().filter(|id| !self.tree.ancestors(*id).iter().any(|a| set.contains(a))).collect())
    }

    pub fn toggle_complete(&mut self, _: &ToggleComplete, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        let ids = self.block_roots().or_else(|| self.editing_node().map(|id| vec![id])).unwrap_or_default();
        if ids.is_empty() {
            return;
        }
        let done = !ids.iter().all(|id| self.tree.node(*id).completed);
        for id in &ids {
            self.tree.set_completed(*id, done);
            self.anims.start(*id, Channel::Complete, anim::FLAG);
        }
        // When completing the edited row with completed rows hidden, move on to the next row.
        if done && self.hide_completed {
            if let Some(Target::Text(id)) = self.edit_target() {
                let next = self.neighbor(id, 1).or_else(|| self.neighbor(id, -1));
                self.edit = next.map(|n| Edit::new(Target::Text(n), 0));
            }
            self.block = None;
        }
        self.commit_structure(before, cx);
    }

    pub fn collapse(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        self.set_fold(true, cx);
    }

    pub fn expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        self.set_fold(false, cx);
    }

    fn set_fold(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        let ids = self.block_roots().or_else(|| self.editing_node().map(|id| vec![id])).unwrap_or_default();
        let before = self.caret();
        let mut changed = false;
        for id in ids {
            if id != self.zoom && self.tree.has_children(self.tree.source(id)) && self.tree.node(id).collapsed != collapsed {
                self.tree.set_collapsed(id, collapsed);
                self.anims.start(id, Channel::Fold, anim::ROW_IN);
                changed = true;
            }
        }
        if changed {
            self.commit_structure(before, cx);
        }
    }

    pub fn toggle_fold(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if !self.tree.has_children(self.tree.source(id)) {
            return;
        }
        let before = self.caret();
        let collapsed = !self.tree.node(id).collapsed;
        self.tree.set_collapsed(id, collapsed);
        self.anims.start(id, Channel::Fold, anim::ROW_IN);
        // Don't leave the caret inside a hidden row.
        if collapsed {
            if let Some(eid) = self.editing_node() {
                if self.tree.is_ancestor(id, eid) {
                    self.edit = Some(Edit::new(Target::Text(id), self.tree.node(id).text.len()));
                }
            }
        }
        self.commit_structure(before, cx);
    }

    pub fn zoom_to(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let id = if self.tree.contains(id) { id } else { ROOT };
        let came_from = self.zoom;
        self.zoom = id;
        self.block = None;
        self.history.seal();
        self.anims.clear();
        self.refresh(false);
        self.list.reset(self.display.len() + 1);
        self.zoom_tween = Some(Tween::new(anim::ZOOM));
        self.set_meta("zoom", &id.to_string());
        // Put the caret somewhere sensible: on the node we came out of, or the first row.
        let target = if self.tree.is_ancestor(id, came_from) && self.row_index(came_from).is_some() {
            Some(came_from)
        } else {
            self.rows.first().map(|r| r.id)
        };
        match target {
            Some(t) => {
                let len = self.tree.node(t).text.len();
                self.edit = Some(Edit::new(Target::Text(t), len));
                self.reveal(t);
            }
            None if id != ROOT => {
                let len = self.tree.node(id).text.len();
                self.edit = Some(Edit::new(Target::Text(id), len));
            }
            None => self.edit = None,
        }
        cx.notify();
    }

    pub fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        let id = self.block.map(|b| b.head).or_else(|| self.editing_node());
        if let Some(id) = id {
            if id != self.zoom {
                self.zoom_to(id, cx);
            }
        }
    }

    pub fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        if self.zoom != ROOT {
            let parent = self.tree.parent(self.zoom);
            self.zoom_to(parent, cx);
        }
    }

    pub fn zoom_home(&mut self, _: &ZoomHome, _: &mut Window, cx: &mut Context<Self>) {
        if self.zoom != ROOT {
            self.zoom_to(ROOT, cx);
        }
    }

    pub fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        let ids = self.block_roots().or_else(|| self.editing_node().filter(|id| *id != self.zoom).map(|id| vec![id])).unwrap_or_default();
        let mut ok = false;
        for id in ids {
            ok |= self.tree.move_up(id, self.zoom);
        }
        if ok {
            self.commit_structure(before, cx);
        }
    }

    pub fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        let ids = self.block_roots().or_else(|| self.editing_node().filter(|id| *id != self.zoom).map(|id| vec![id])).unwrap_or_default();
        let mut ok = false;
        for id in ids.into_iter().rev() {
            ok |= self.tree.move_down(id, self.zoom);
        }
        if ok {
            self.commit_structure(before, cx);
        }
    }

    pub fn delete_node(&mut self, _: &DeleteNode, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        let ids = self.block_roots().or_else(|| self.editing_node().filter(|id| *id != self.zoom).map(|id| vec![id])).unwrap_or_default();
        let Some(&first) = ids.first() else { return };
        let fallback = self.neighbor(first, -1).or_else(|| {
            let last = *ids.last().unwrap();
            let mut n = self.neighbor(last, 1);
            while let Some(x) = n {
                if ids.iter().any(|d| *d == x || self.tree.is_ancestor(*d, x)) {
                    n = self.neighbor(x, 1);
                } else {
                    break;
                }
            }
            n
        });
        for id in &ids {
            self.send_to_trash(*id);
            self.tree.delete(*id);
        }
        self.block = None;
        self.edit = fallback.filter(|f| self.tree.contains(*f)).map(|f| Edit::new(Target::Text(f), self.tree.node(f).text.len()));
        self.commit_structure(before, cx);
    }

    pub fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(caret) = self.history.undo(&mut self.tree) {
            self.after_history(caret, cx);
        }
    }

    pub fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(caret) = self.history.redo(&mut self.tree) {
            self.after_history(caret, cx);
        }
    }

    fn after_history(&mut self, caret: Option<Caret>, cx: &mut Context<Self>) {
        self.persist();
        if !self.tree.contains(self.zoom) {
            self.zoom = ROOT;
        }
        self.block = None;
        self.edit = caret.filter(|c| self.tree.contains(c.node)).map(|c| {
            let len = self.tree.node(c.node).text.len();
            Edit::new(Target::Text(c.node), c.offset.min(len))
        });
        // Make sure the caret's row is visible.
        if let Some(Target::Text(id)) = self.edit_target() {
            for a in self.tree.ancestors(id) {
                if self.tree.node(a).collapsed && self.tree.is_ancestor(self.zoom, a) {
                    self.tree.set_collapsed(a, false);
                    self.tree.take_journal();
                    self.persist();
                }
            }
        }
        self.refresh(true);
        self.reveal_edit();
        cx.notify();
    }

    pub fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(roots) = self.block_roots() {
            self.copy_items(&roots, cx);
        } else if let Some(e) = &self.edit {
            if !e.sel.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(self.edit_text()[e.sel.clone()].to_string()));
            }
        }
    }

    pub fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&Copy, window, cx);
        if self.block.is_some() {
            self.delete_node(&DeleteNode, window, cx);
            self.toast("Cut — also kept in Trash");
        } else if let Some(e) = &self.edit {
            if !e.sel.is_empty() {
                let r = e.sel.clone();
                self.replace_edit_text(r, "", false, cx);
            }
        }
    }

    pub fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        let Some(text) = item.text() else { return };
        // Bullets copied from Flowie carry their full structure as metadata.
        let items: Vec<flowie_core::Item> = item
            .entries()
            .iter()
            .find_map(|e| match e {
                gpui::ClipboardEntry::String(s) => s.metadata_json::<Vec<flowie_core::Item>>(),
                _ => None,
            })
            .unwrap_or_else(|| {
                if text.trim_end_matches('\n').contains('\n') {
                    flowie_core::io::parse_any(&text)
                } else {
                    Vec::new()
                }
            });
        let multiline = !items.is_empty();
        match self.edit_target() {
            Some(Target::Text(id)) if multiline && id != self.zoom => {
                let before = self.caret();
                let parent = self.tree.parent(id);
                let empty = self.tree.node(id).text.is_empty() && !self.tree.has_children(id);
                let index = self.tree.index_in_parent(id) + if empty { 0 } else { 1 };
                let top = self.tree.insert_items(parent, index, &items);
                if empty {
                    self.tree.delete(id);
                }
                if let Some(&last) = top.last() {
                    let len = self.tree.node(last).text.len();
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
                    let parent = self.tree.parent(b.head);
                    let index = self.tree.index_in_parent(b.head) + 1;
                    let top = if items.is_empty() { self.tree.insert_indented_text(parent, index, &text) } else { self.tree.insert_items(parent, index, &items) };
                    self.block = top.first().zip(top.last()).map(|(a, h)| BlockSel { anchor: *a, head: *h });
                    self.commit_structure(before, cx);
                }
            }
        }
    }

    /// Wrap the selection in `open`/`close`, or insert the pair around the caret.
    fn wrap(&mut self, open: &str, close: &str, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        if matches!(e.target, Target::Search | Target::Palette) {
            return;
        }
        let r = e.sel.clone();
        let inner = self.edit_text()[r.clone()].to_string();
        // Toggle off when the selection is already wrapped.
        let text = self.edit_text();
        if !inner.is_empty() && r.start >= open.len() && text[..r.start].ends_with(open) && text[r.end..].starts_with(close) {
            let outer = r.start - open.len()..r.end + close.len();
            self.replace_edit_text(outer.clone(), &inner, false, cx);
            if let Some(e) = self.edit.as_mut() {
                e.sel = outer.start..outer.start + inner.len();
            }
            return;
        }
        self.replace_edit_text(r.clone(), &format!("{open}{inner}{close}"), false, cx);
        if let Some(e) = self.edit.as_mut() {
            let s = r.start + open.len();
            e.sel = s..s + inner.len();
        }
        cx.notify();
    }

    pub fn bold(&mut self, _: &Bold, _: &mut Window, cx: &mut Context<Self>) {
        self.wrap("**", "**", cx);
    }

    pub fn italic(&mut self, _: &Italic, _: &mut Window, cx: &mut Context<Self>) {
        self.wrap("*", "*", cx);
    }

    pub fn inline_code(&mut self, _: &InlineCode, _: &mut Window, cx: &mut Context<Self>) {
        self.wrap("`", "`", cx);
    }

    pub fn wrap_math(&mut self, _: &WrapMath, _: &mut Window, cx: &mut Context<Self>) {
        self.wrap("$", "$", cx);
    }

    pub fn insert_link(&mut self, _: &InsertLink, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        let r = e.sel.clone();
        let label = self.edit_text()[r.clone()].to_string();
        let label = if label.is_empty() { "link".to_string() } else { label };
        let insert = format!("[{label}](https://)");
        self.replace_edit_text(r.clone(), &insert, false, cx);
        if let Some(e) = self.edit.as_mut() {
            let s = r.start + label.len() + 3;
            e.sel = s..s + "https://".len();
        }
        cx.notify();
    }

    pub fn find(&mut self, _: &Find, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.query.len();
        self.start_edit(Target::Search, len, cx);
        if let Some(e) = self.edit.as_mut() {
            e.sel = 0..len;
        }
    }

    /// Clear the search and keep the caret in the (now empty) search box.
    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.query.clear();
        self.refresh(false);
        self.list.reset(self.display.len() + 1);
        self.start_edit(Target::Search, 0, cx);
    }

    pub fn search_for(&mut self, q: &str, cx: &mut Context<Self>) {
        self.query = q.to_string();
        self.refresh(false);
        self.list.reset(self.display.len() + 1);
        self.start_edit(Target::Search, self.query.len(), cx);
    }

    pub fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(cx);
            return;
        }
        if self.inline.take().is_some() {
            cx.notify();
            return;
        }
        if self.calendar.take().is_some() {
            cx.notify();
            return;
        }
        if self.trash.take().is_some() {
            cx.notify();
            return;
        }
        match self.edit_target() {
            Some(Target::Search) => {
                self.query.clear();
                self.refresh(false);
                self.list.reset(self.display.len() + 1);
                self.edit = self.rows.first().map(|r| Edit::new(Target::Text(r.id), 0));
            }
            Some(Target::Note(id)) => {
                let len = self.tree.node(id).text.len();
                self.start_edit(Target::Text(id), len, cx);
            }
            Some(Target::Text(id)) => {
                if id != self.zoom {
                    self.history.seal();
                    self.block = Some(BlockSel { anchor: id, head: id });
                    self.edit = None;
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.refresh(false);
                }
            }
            Some(Target::Palette) => {}
            None => {
                if self.block.take().is_none() && !self.query.is_empty() {
                    self.query.clear();
                    self.refresh(false);
                    self.list.reset(self.display.len() + 1);
                }
            }
        }
        cx.notify();
    }

    pub fn toggle_completed_visible(&mut self, _: &ToggleCompletedVisible, _: &mut Window, cx: &mut Context<Self>) {
        self.hide_completed = !self.hide_completed;
        self.set_meta("hide_completed", if self.hide_completed { "1" } else { "0" });
        if let Some(id) = self.editing_node() {
            if self.hide_completed && self.tree.node(id).completed {
                self.edit = None;
            }
        }
        self.refresh(true);
        cx.notify();
    }

    // -------------------------------------------------------- mouse/drag

    /// Shift-click: select rows from the current row to `id`.
    pub fn shift_click(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let anchor = self.block.map(|b| b.anchor).or_else(|| self.editing_node()).unwrap_or(id);
        if anchor == self.zoom {
            return;
        }
        self.block = Some(BlockSel { anchor, head: id });
        self.edit = None;
        cx.notify();
    }

    /// Work out where a dragged node would land for a pointer at `pos` over row `id`.
    pub fn update_drop(&mut self, over: NodeId, bounds: Bounds<Pixels>, pos: gpui::Point<Pixels>, dragged: NodeId, cx: &mut Context<Self>) {
        let Some(ix) = self.row_index(over) else { return };
        let below_half = pos.y > bounds.center().y;
        let gap = if below_half { ix + 1 } else { ix };
        let above = gap.checked_sub(1).and_then(|i| self.rows.get(i)).copied();
        let below = self.rows.get(gap).copied();
        let max_d = above.map_or(0, |a| a.depth + 1);
        let min_d = below.map_or(0, |b| b.depth).min(max_d);
        let x = f32::from(pos.x - bounds.left()) - PAD_X;
        let want = (x / INDENT).floor().max(0.0) as u16;
        let depth = want.clamp(min_d, max_d);
        let (parent, index) = match above {
            None => (self.zoom, 0),
            Some(a) if depth == a.depth + 1 => (a.id, 0),
            Some(a) => {
                let mut x = a.id;
                for _ in 0..(a.depth - depth) {
                    x = self.tree.parent(x);
                }
                (self.tree.parent(x), self.tree.index_in_parent(x) + 1)
            }
        };
        // Can't drop a node into itself.
        let invalid = parent == dragged || self.tree.is_ancestor(dragged, parent);
        let target = (!invalid).then(|| match below {
            Some(b) if !below_half || above.is_none() => DropTarget { node: b.id, below: false, depth, parent, index },
            _ => DropTarget { node: above.map_or(over, |a| a.id), below: true, depth, parent, index },
        });
        if self.drop != target {
            self.drop = target;
            cx.notify();
        }
    }

    pub fn perform_drop(&mut self, dragged: NodeId, cx: &mut Context<Self>) {
        let Some(t) = self.drop.take() else { return };
        let before = self.caret();
        if self.tree.move_to(dragged, t.parent, t.index) {
            self.tree.set_collapsed(t.parent, false);
            self.anims.start(dragged, Channel::Enter, anim::ROW_IN);
            self.commit_structure(before, cx);
        }
        cx.notify();
    }
}
