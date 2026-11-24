//! Pickers: a modal palette (jump to a bullet, move to, link to, mirror)
//! and an inline popup at the caret (slash commands, tag/link completion).

use flowie_core::{markup, BlockKind, NodeId, ROOT};
use gpui::{div, prelude::*, px, AnyElement, Context, MouseButton, Window};

use crate::i18n::t;
use crate::outline::{Edit, Outline, Target};

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// ⌘P: go to a bullet (starred first) or run a saved search.
    Jump,
    /// ⌘⇧M: move these bullets under the chosen one.
    MoveTo(Vec<NodeId>),
    /// Mirror these bullets under the chosen one.
    MirrorTo(Vec<NodeId>),
    /// Insert a link to the chosen bullet into `node` at `at`.
    LinkTo { node: NodeId, at: usize },
    /// Create a mirror of the chosen bullet after `after`.
    Mirror { after: NodeId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    Slash,
    /// `#` or `@` completion.
    Tag(char),
    /// `[[` link completion.
    Link,
}

/// An inline popup tied to text typed in a bullet: the trigger starts at `start`.
#[derive(Debug, Clone, PartialEq)]
pub struct Inline {
    pub kind: InlineKind,
    pub node: NodeId,
    pub start: usize,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Node(NodeId),
    /// A new top-level bullet (page) with this text, then the mode's action on it.
    NewPage(String),
    Search(String),
    Kind(BlockKind),
    Wrap(&'static str, &'static str),
    Color(&'static str),
    InsertText(String),
    Complete,
    Note,
    OpenLink,
    OpenMirror,
    OpenMove,
    ExportPackage,
    Today,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub label: String,
    pub detail: String,
    pub icon: &'static str,
    pub action: Action,
}

pub struct Palette {
    pub mode: Mode,
    pub query: String,
    pub selected: usize,
    pub entries: Vec<Entry>,
    /// Where the caret was before opening, restored on close.
    pub return_to: Option<(Target, usize)>,
}

fn path_label(o: &Outline, id: NodeId) -> String {
    let parts: Vec<String> = o
        .tree
        .ancestors(id)
        .into_iter()
        .map(|a| markup::plain_text(&o.tree.node(o.tree.source(a)).text).chars().take(24).collect::<String>())
        .collect();
    if parts.is_empty() {
        t!("common.home").into()
    } else {
        parts.join(" › ")
    }
}

pub fn node_label(o: &Outline, id: NodeId) -> String {
    if id == ROOT {
        return t!("common.home").into();
    }
    let t = markup::plain_text(&o.tree.node(o.tree.source(id)).text);
    if t.trim().is_empty() {
        t!("common.untitled").into()
    } else {
        t
    }
}

impl Outline {
    /// "New page “q”" when `q` is not empty and no bullet's text is exactly `q`.
    pub fn new_page_entry(&self, q: &str) -> Option<Entry> {
        let name = q.trim();
        if name.is_empty() {
            return None;
        }
        let lower = name.to_lowercase();
        let exists = self.tree.nodes().any(|n| n.mirror == ROOT && markup::plain_text(&n.text).trim().to_lowercase() == lower);
        (!exists).then(|| Entry {
            label: t!("palette.new_page", name = name),
            detail: t!("palette.on_home").into(),
            icon: "plus",
            action: Action::NewPage(name.to_string()),
        })
    }

    /// Append a bullet named `name` to Home (not committed).
    pub fn create_page(&mut self, name: &str) -> NodeId {
        let n = self.tree.children(ROOT).len();
        self.tree.create_at(ROOT, n, name)
    }

    /// Link to a new page: one commit for the page, then the link text.
    pub fn link_to_new_page(&mut self, node: NodeId, at: usize, name: &str, cx: &mut Context<Self>) {
        let page = self.create_page(name);
        self.commit_structure(None, cx);
        self.insert_link_to(node, at, page, cx);
        self.toast(t!("toast.page_created", name = node_label(self, page)));
    }

    /// Bullets matching `query` (every word must appear), best matches first.
    pub fn search_nodes(&self, query: &str, limit: usize, exclude: &[NodeId]) -> Vec<NodeId> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
        let mut scored: Vec<(i64, NodeId)> = Vec::new();
        for n in self.tree.nodes() {
            if n.mirror != ROOT || exclude.iter().any(|e| *e == n.id || self.tree.is_ancestor(*e, n.id)) {
                continue;
            }
            if n.text.is_empty() {
                continue;
            }
            let text = n.text.to_lowercase();
            if !words.iter().all(|w| text.contains(w.as_str())) {
                continue;
            }
            let mut score = 0i64;
            if self.starred.contains(&n.id) {
                score -= 1_000_000;
            }
            if let Some(first) = words.first() {
                if text.starts_with(first.as_str()) {
                    score -= 10_000;
                }
            }
            score += n.text.len() as i64 + self.tree.ancestors(n.id).len() as i64 * 8;
            score -= (n.modified / 3_600_000) % 1000; // mild recency bias
            scored.push((score, n.id));
        }
        scored.sort_unstable();
        scored.into_iter().take(limit).map(|(_, id)| id).collect()
    }

    pub fn open_palette(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let return_to = self.edit.as_ref().map(|e| (e.target, e.caret()));
        self.palette = Some(Palette { mode, query: String::new(), selected: 0, entries: Vec::new(), return_to });
        self.inline = None;
        self.block_saved = self.block.take();
        self.edit = Some(Edit::new(Target::Palette, 0));
        self.refresh_palette();
        cx.notify();
    }

    pub fn refresh_palette(&mut self) {
        let Some(p) = self.palette.as_ref() else { return };
        let q = p.query.clone();
        let mode = p.mode.clone();
        let mut entries = Vec::new();
        match &mode {
            Mode::Jump => {
                if q.trim().is_empty() {
                    entries.push(Entry {
                        label: t!("common.home").into(),
                        detail: String::new(),
                        icon: "house",
                        action: Action::Node(ROOT),
                    });
                    for &id in &self.starred {
                        if self.tree.contains(id) {
                            entries.push(Entry {
                                label: node_label(self, id),
                                detail: path_label(self, id),
                                icon: "star",
                                action: Action::Node(id),
                            });
                        }
                    }
                    for s in &self.saved_searches {
                        entries.push(Entry {
                            label: s.clone(),
                            detail: t!("palette.saved_search").into(),
                            icon: "search",
                            action: Action::Search(s.clone()),
                        });
                    }
                } else {
                    for s in self.saved_searches.iter().filter(|s| s.to_lowercase().contains(&q.to_lowercase())) {
                        entries.push(Entry {
                            label: s.clone(),
                            detail: t!("palette.saved_search").into(),
                            icon: "search",
                            action: Action::Search(s.clone()),
                        });
                    }
                    for id in self.search_nodes(&q, 40, &[]) {
                        let icon = if self.starred.contains(&id) { "star" } else { "dot" };
                        entries.push(Entry { label: node_label(self, id), detail: path_label(self, id), icon, action: Action::Node(id) });
                    }
                    entries.extend(self.new_page_entry(&q));
                    entries.push(Entry {
                        label: t!("palette.search_for", query = q),
                        detail: t!("palette.filter_outline").into(),
                        icon: "search",
                        action: Action::Search(q.clone()),
                    });
                }
            }
            Mode::MoveTo(ids) | Mode::MirrorTo(ids) => {
                if q.trim().is_empty() || ("home".contains(&q.to_lowercase()) || t!("common.home").contains(q.trim())) {
                    entries.push(Entry {
                        label: t!("common.home").into(),
                        detail: t!("palette.top_level").into(),
                        icon: "house",
                        action: Action::Node(ROOT),
                    });
                }
                for id in self.search_nodes(&q, 40, ids) {
                    entries.push(Entry {
                        label: node_label(self, id),
                        detail: path_label(self, id),
                        icon: "corner-down-right",
                        action: Action::Node(id),
                    });
                }
                entries.extend(self.new_page_entry(&q));
            }
            Mode::LinkTo { .. } | Mode::Mirror { .. } => {
                let exclude: Vec<NodeId> = match &mode {
                    Mode::Mirror { after } => self.tree.ancestors(*after),
                    _ => Vec::new(),
                };
                let ids = if q.trim().is_empty() {
                    self.starred.iter().copied().filter(|id| self.tree.contains(*id)).collect()
                } else {
                    self.search_nodes(&q, 40, &[])
                };
                for id in ids.into_iter().filter(|id| !exclude.contains(id)) {
                    entries.push(Entry {
                        label: node_label(self, id),
                        detail: path_label(self, id),
                        icon: "link",
                        action: Action::Node(id),
                    });
                }
                if matches!(mode, Mode::LinkTo { .. }) {
                    entries.extend(self.new_page_entry(&q));
                }
            }
        }
        let p = self.palette.as_mut().unwrap();
        p.selected = p.selected.min(entries.len().saturating_sub(1));
        p.entries = entries;
        self.popup_scroll.scroll_to_item(p.selected);
    }

    pub fn close_palette(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.palette.take() {
            self.edit = p
                .return_to
                .filter(|(t, _)| match t {
                    Target::Text(id) | Target::Note(id) => self.tree.contains(*id),
                    _ => true,
                })
                .map(|(t, off)| Edit::new(t, off));
            if self.edit.is_none() {
                self.block = self.block_saved.take();
            }
        }
        cx.notify();
    }

    pub fn palette_move(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        if let Some(p) = self.palette.as_mut() {
            let n = p.entries.len() as isize;
            if n > 0 {
                p.selected = ((p.selected as isize + delta).rem_euclid(n)) as usize;
            }
            self.popup_scroll.scroll_to_item(p.selected);
            cx.notify();
            return true;
        }
        if let Some(i) = self.inline.as_mut() {
            let n = self.inline_entries_len as isize;
            if n > 0 {
                i.selected = ((i.selected as isize + delta).rem_euclid(n)) as usize;
            }
            self.popup_scroll.scroll_to_item(i.selected);
            cx.notify();
            return true;
        }
        false
    }

    pub fn palette_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(p) = self.palette.as_ref() {
            let Some(entry) = p.entries.get(p.selected).cloned() else {
                self.close_palette(cx);
                return true;
            };
            let mode = p.mode.clone();
            self.close_palette(cx);
            self.run_palette(mode, entry.action, window, cx);
            return true;
        }
        if self.inline.is_some() {
            return self.inline_confirm(None, window, cx);
        }
        false
    }

    pub fn palette_click(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.palette.as_mut() {
            p.selected = ix;
            self.palette_confirm(window, cx);
        }
    }

    fn run_palette(&mut self, mode: Mode, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match (mode, action) {
            (Mode::Jump, Action::Node(id)) => {
                if id == ROOT || self.tree.has_children(self.tree.source(id)) || self.starred.contains(&id) {
                    self.zoom_to(id, cx);
                } else {
                    // Leaf: show it in its parent with the caret on it.
                    let parent = self.tree.parent(id);
                    self.zoom_to(parent, cx);
                    let len = self.tree.node(id).text.len();
                    self.start_edit(Target::Text(id), len, cx);
                }
            }
            (Mode::Jump, Action::NewPage(name)) => {
                let page = self.create_page(&name);
                let blank = self.tree.create_at(page, 0, "");
                self.commit_structure(None, cx);
                self.zoom_to(page, cx);
                self.start_edit(Target::Text(blank), 0, cx);
            }
            (Mode::MoveTo(ids), Action::NewPage(name)) => {
                let page = self.create_page(&name);
                self.move_nodes_to(&ids, page, cx);
            }
            (Mode::MirrorTo(ids), Action::NewPage(name)) => {
                let page = self.create_page(&name);
                self.mirror_nodes_to(&ids, page, cx);
            }
            (Mode::MirrorTo(ids), Action::Node(target)) => self.mirror_nodes_to(&ids, target, cx),
            (Mode::LinkTo { node, at }, Action::NewPage(name)) => self.link_to_new_page(node, at, &name, cx),
            (_, Action::Search(q)) => self.search_for(&q, cx),
            (Mode::MoveTo(ids), Action::Node(target)) => self.move_nodes_to(&ids, target, cx),
            (Mode::LinkTo { node, at }, Action::Node(target)) => self.insert_link_to(node, at, target, cx),
            (Mode::Mirror { after }, Action::Node(source)) => self.insert_mirror(after, source, cx),
            (_, other) => self.run_action(other, window, cx),
        }
    }

    pub fn render_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.palette.as_ref()?;
        let theme = self.theme;
        let title = match p.mode {
            Mode::Jump => t!("palette.title.jump"),
            Mode::MoveTo(_) => t!("palette.title.move"),
            Mode::MirrorTo(_) => t!("palette.title.mirror_to"),
            Mode::LinkTo { .. } => t!("palette.title.link"),
            Mode::Mirror { .. } => t!("palette.title.mirror"),
        };
        let selected = p.selected;
        let entries = p.entries.clone();
        let query = p.query.clone();
        let (font, _) = (window.text_style().font(), ());
        let run = gpui::TextRun { len: query.len(), font, color: theme.text, background_color: None, underline: None, strikethrough: None };
        let input = self.editor_element(query.clone(), vec![run], theme.caret, theme.selection, cx);
        let placeholder =
            query.is_empty().then(|| div().absolute().top_0().left_0().text_color(theme.faint).child(t!("palette.placeholder")));
        let rows = entries.into_iter().enumerate().map(|(i, e)| {
            let on = i == selected;
            let icon = if e.icon == "star" && !on { theme.star } else { crate::ui::list::icon_color(theme, on) };
            crate::ui::list::row(theme, ("palette", i), on)
                .h(px(32.))
                .child(crate::icons::slot(e.icon, crate::icons::MD, icon))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(e.label))
                .child(
                    div()
                        .max_w(px(220.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(12.))
                        .text_color(crate::ui::list::detail_color(theme, on))
                        .child(e.detail),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.palette_click(i, window, cx)))
        });
        Some(
            crate::ui::surface::scrim(theme)
                .items_start()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_palette(cx)))
                .child(
                    crate::ui::surface::sheet(theme, "palette-card")
                        .mt(px(90.))
                        .w(px(580.))
                        .max_h(px(470.))
                        .flex()
                        .flex_col()
                        .p(px(6.))
                        .child(crate::ui::text::overline(theme, title).px_3().pt_1())
                        .child(div().relative().mx_3().my_2().child(input).children(placeholder))
                        .child(div().h(px(1.)).bg(theme.divider).mb_1())
                        .child(
                            crate::ui::scrollbar::area(
                                "palette-bar",
                                &self.popup_scroll,
                                theme,
                                div().id("palette-list").size_full().overflow_y_scroll().track_scroll(&self.popup_scroll).children(rows),
                            )
                            .flex_1()
                            .min_h_0(),
                        ),
                )
                .into_any_element(),
        )
    }
}
