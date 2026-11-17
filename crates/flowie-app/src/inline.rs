//! Inline popup at the caret: `/` commands, `#tag` / `@mention` completion
//! and `[[` links to other bullets. Also the shared command executor.

use std::collections::HashMap;

use flowie_core::{markup, search::today, BlockKind, NodeId};
use gpui::{div, point, prelude::*, px, AnyElement, Context, MouseButton, Window};

use crate::outline::{Edit, Outline, Target};
use crate::palette::{node_label, Action, Entry, Inline, InlineKind, Mode};

struct Cmd {
    label: &'static str,
    aliases: &'static str,
    icon: &'static str,
    action: fn() -> Action,
}

const SLASH: &[Cmd] = &[
    Cmd { label: "Bullet", aliases: "list item normal", icon: "list", action: || Action::Kind(BlockKind::Bullet) },
    Cmd { label: "Text", aliases: "paragraph plain", icon: "pilcrow", action: || Action::Kind(BlockKind::Paragraph) },
    Cmd { label: "Heading 1", aliases: "h1 title", icon: "heading-1", action: || Action::Kind(BlockKind::H1) },
    Cmd { label: "Heading 2", aliases: "h2", icon: "heading-2", action: || Action::Kind(BlockKind::H2) },
    Cmd { label: "Heading 3", aliases: "h3", icon: "heading-3", action: || Action::Kind(BlockKind::H3) },
    Cmd { label: "To-do", aliases: "todo checkbox task", icon: "square-check", action: || Action::Kind(BlockKind::Todo) },
    Cmd { label: "Numbered", aliases: "ordered list number", icon: "list-ordered", action: || Action::Kind(BlockKind::Numbered) },
    Cmd { label: "Quote", aliases: "blockquote", icon: "quote", action: || Action::Kind(BlockKind::Quote) },
    Cmd { label: "Code block", aliases: "pre monospace", icon: "square-code", action: || Action::Kind(BlockKind::Code) },
    Cmd { label: "Highlight", aliases: "mark", icon: "highlighter", action: || Action::Wrap("==", "==") },
    Cmd { label: "Bold", aliases: "strong", icon: "bold", action: || Action::Wrap("**", "**") },
    Cmd { label: "Italic", aliases: "emphasis", icon: "italic", action: || Action::Wrap("*", "*") },
    Cmd { label: "Strikethrough", aliases: "strike", icon: "strikethrough", action: || Action::Wrap("~~", "~~") },
    Cmd { label: "Inline code", aliases: "code", icon: "code", action: || Action::Wrap("`", "`") },
    Cmd { label: "Math", aliases: "latex formula equation katex", icon: "sigma", action: || Action::Wrap("$", "$") },
    Cmd { label: "Display math", aliases: "latex block equation", icon: "square-sigma", action: || Action::Wrap("$$", "$$") },
    Cmd { label: "Red", aliases: "color", icon: "circle", action: || Action::Color("red") },
    Cmd { label: "Orange", aliases: "color", icon: "circle", action: || Action::Color("orange") },
    Cmd { label: "Yellow", aliases: "color", icon: "circle", action: || Action::Color("yellow") },
    Cmd { label: "Green", aliases: "color", icon: "circle", action: || Action::Color("green") },
    Cmd { label: "Blue", aliases: "color", icon: "circle", action: || Action::Color("blue") },
    Cmd { label: "Purple", aliases: "color", icon: "circle", action: || Action::Color("purple") },
    Cmd { label: "Gray", aliases: "color grey", icon: "circle", action: || Action::Color("gray") },
    Cmd { label: "Today", aliases: "date now", icon: "calendar-days", action: || Action::Today },
    Cmd { label: "Link to bullet", aliases: "internal reference", icon: "link", action: || Action::OpenLink },
    Cmd { label: "Mirror", aliases: "clone live copy", icon: "copy", action: || Action::OpenMirror },
    Cmd { label: "Move to…", aliases: "relocate", icon: "corner-down-right", action: || Action::OpenMove },
    Cmd { label: "Complete", aliases: "done check", icon: "check", action: || Action::Complete },
    Cmd { label: "Add note", aliases: "description", icon: "notebook-pen", action: || Action::Note },
];

fn trigger_len(kind: InlineKind) -> usize {
    match kind {
        InlineKind::Slash | InlineKind::Tag(_) => 1,
        InlineKind::Link => 2,
    }
}

impl Outline {
    /// All `#tags` / `@mentions` in the outline with use counts.
    fn tags(&self, sigil: char) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for n in self.tree.nodes() {
            if !n.text.contains(sigil) {
                continue;
            }
            for r in markup::parse(&n.text) {
                if matches!(r.kind, markup::Kind::Tag | markup::Kind::Mention) && n.text[r.range.clone()].starts_with(sigil) {
                    *counts.entry(n.text[r.range].to_string()).or_default() += 1;
                }
            }
        }
        let mut v: Vec<_> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    fn inline_query(&self) -> Option<String> {
        let i = self.inline.as_ref()?;
        let e = self.edit.as_ref()?;
        let text = self.tree.get(i.node)?.text.as_str();
        let from = i.start + trigger_len(i.kind);
        let caret = e.caret();
        (caret >= from && caret <= text.len()).then(|| text[from..caret].to_string())
    }

    pub fn inline_entries(&self) -> Vec<Entry> {
        let Some(i) = self.inline.as_ref() else { return Vec::new() };
        let q = self.inline_query().unwrap_or_default().to_lowercase();
        match i.kind {
            InlineKind::Slash => SLASH
                .iter()
                .filter(|c| q.is_empty() || c.label.to_lowercase().contains(&q) || c.aliases.contains(&q))
                .map(|c| Entry { label: c.label.into(), detail: String::new(), icon: c.icon, action: (c.action)() })
                .collect(),
            InlineKind::Tag(sigil) => self
                .tags(sigil)
                .into_iter()
                .filter(|(t, _)| t[1..].to_lowercase().starts_with(&q) && t.len() > q.len() + 1)
                .take(8)
                .map(|(t, n)| Entry { label: t.clone(), detail: format!("{n}"), icon: if sigil == '@' { "at-sign" } else { "hash" }, action: Action::InsertText(format!("{t} ")) })
                .collect(),
            InlineKind::Link => self
                .search_nodes(&q, 9, &[])
                .into_iter()
                .filter(|id| *id != i.node)
                .take(8)
                .map(|id| Entry { label: node_label(self, id), detail: String::new(), icon: "link", action: Action::Node(id) })
                .collect(),
        }
    }

    /// After each edit of a bullet's text: open, update or close the popup.
    pub fn check_inline(&mut self, inserted: &str) {
        let Some(Edit { target: Target::Text(node), .. }) = self.edit.as_ref() else {
            self.inline = None;
            return;
        };
        let node = *node;
        let caret = self.edit.as_ref().unwrap().caret();
        let text = self.tree.node(node).text.clone();
        if let Some(i) = &self.inline {
            let from = i.start + trigger_len(i.kind);
            let valid = i.node == node
                && caret >= from
                && caret <= text.len()
                && text.get(i.start..from).is_some_and(|t| match i.kind {
                    InlineKind::Slash => t == "/",
                    InlineKind::Tag(c) => t.starts_with(c),
                    InlineKind::Link => t == "[[",
                })
                && match i.kind {
                    InlineKind::Link => !text[from..caret].contains(']'),
                    _ => !text[from..caret].contains(char::is_whitespace),
                };
            if valid {
                if let Some(i) = self.inline.as_mut() {
                    i.selected = 0;
                }
                self.popup_scroll.scroll_to_item(0);
                self.inline_entries_len = self.inline_entries().len();
                if self.inline_entries_len == 0 && !matches!(self.inline.as_ref().map(|i| i.kind), Some(InlineKind::Link)) {
                    self.inline = None;
                }
                return;
            }
            self.inline = None;
        }
        if inserted.chars().count() != 1 || caret == 0 || caret > text.len() {
            return;
        }
        let before = &text[..caret];
        let at_word_start = |pos: usize| pos == 0 || text[..pos].ends_with(char::is_whitespace);
        let kind = if before.ends_with("[[") {
            Some((InlineKind::Link, caret - 2))
        } else if before.ends_with('/') && at_word_start(caret - 1) {
            Some((InlineKind::Slash, caret - 1))
        } else if before.ends_with('#') && at_word_start(caret - 1) {
            Some((InlineKind::Tag('#'), caret - 1))
        } else if before.ends_with('@') && at_word_start(caret - 1) {
            Some((InlineKind::Tag('@'), caret - 1))
        } else {
            None
        };
        if let Some((kind, start)) = kind {
            self.inline = Some(Inline { kind, node, start, selected: 0 });
            self.popup_scroll.scroll_to_item(0);
            self.inline_entries_len = self.inline_entries().len();
            if self.inline_entries_len == 0 && kind != InlineKind::Link {
                self.inline = None;
            }
        }
    }

    pub fn inline_confirm(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(i) = self.inline.clone() else { return false };
        let entries = self.inline_entries();
        let Some(entry) = entries.get(ix.unwrap_or(i.selected)).cloned() else {
            self.inline = None;
            cx.notify();
            return false;
        };
        let caret = self.edit.as_ref().map(|e| e.caret()).unwrap_or(i.start);
        self.inline = None;
        // Remove the trigger and query text.
        self.replace_edit_text(i.start..caret, "", false, cx);
        match (i.kind, entry.action) {
            (InlineKind::Link, Action::Node(target)) => self.insert_link_to(i.node, i.start, target, cx),
            (_, action) => self.run_action(action, window, cx),
        }
        true
    }

    /// Run a command on the bullet being edited (slash menu, palette).
    pub fn run_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Target::Text(id)) = self.edit.as_ref().map(|e| e.target) else { return };
        let caret = self.edit.as_ref().map(|e| e.caret()).unwrap_or(0);
        match action {
            Action::Kind(kind) => {
                let before = Some(flowie_core::Caret { node: id, offset: caret });
                let src = self.tree.source(id);
                let _ = src;
                self.tree.set_kind(id, kind);
                self.commit_structure(before, cx);
            }
            Action::Wrap(open, close) => {
                let sel = self.edit.as_ref().unwrap().sel.clone();
                let inner = self.edit_text()[sel.clone()].to_string();
                self.replace_edit_text(sel.clone(), &format!("{open}{inner}{close}"), false, cx);
                if let Some(e) = self.edit.as_mut() {
                    let s = sel.start + open.len();
                    e.sel = s..s + inner.len();
                }
            }
            Action::Color(name) => {
                let sel = self.edit.as_ref().unwrap().sel.clone();
                let inner = self.edit_text()[sel.clone()].to_string();
                let open = format!("{{{name}}}");
                self.replace_edit_text(sel.clone(), &format!("{open}{inner}{{/}}"), false, cx);
                if let Some(e) = self.edit.as_mut() {
                    let s = sel.start + open.len();
                    e.sel = s..s + inner.len();
                }
            }
            Action::InsertText(t) => {
                let sel = self.edit.as_ref().unwrap().sel.clone();
                self.replace_edit_text(sel, &t, false, cx);
            }
            Action::Today => {
                let sel = self.edit.as_ref().unwrap().sel.clone();
                self.replace_edit_text(sel, &format!("{} ", today()), false, cx);
            }
            Action::Complete => self.toggle_complete(&crate::outline::ToggleComplete, window, cx),
            Action::Note => self.toggle_note(&crate::outline::ToggleNote, window, cx),
            Action::OpenLink => self.open_palette(Mode::LinkTo { node: id, at: caret }, cx),
            Action::OpenMirror => self.open_palette(Mode::Mirror { after: id }, cx),
            Action::OpenMove => self.open_palette(Mode::MoveTo(vec![id]), cx),
            Action::Node(n) => self.zoom_to(n, cx),
            Action::Search(q) => self.search_for(&q, cx),
        }
        cx.notify();
    }

    pub fn insert_link_to(&mut self, node: NodeId, at: usize, target: NodeId, cx: &mut Context<Self>) {
        if !self.tree.contains(node) {
            return;
        }
        let label: String = node_label(self, target).replace(['[', ']'], "").chars().take(48).collect();
        let link = format!("[{label}](flowie:{target})");
        let len = self.tree.node(node).text.len();
        let at = at.min(len);
        self.start_edit(Target::Text(node), at, cx);
        self.replace_edit_text(at..at, &link, false, cx);
    }

    pub fn render_inline(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let i = self.inline.clone()?;
        let e = self.edit.as_ref()?;
        let layout = e.layout.as_ref()?;
        let pos = layout.position_for_index(i.start.min(self.edit_text().len()))?;
        let lh = layout.line_height();
        let entries = self.inline_entries();
        if entries.is_empty() {
            return None;
        }
        let theme = self.theme;
        let at = point(pos.x - px(6.), pos.y + lh + px(4.));
        let items = entries.into_iter().enumerate().map(|(ix, en)| {
            div()
                .id(("inline", ix))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .h(px(28.))
                .rounded_md()
                .cursor_pointer()
                .when(ix == i.selected, |d| d.bg(theme.row_selected))
                .hover(|s| s.bg(theme.hover))
                .child(match &en.action {
                    // Color commands show a filled swatch in that color, same slot size as icons.
                    Action::Color(name) => {
                        let c = flowie_core::markup::COLORS.iter().position(|c| c == name).map(|i| theme.colors[i]).unwrap_or(theme.muted);
                        div()
                            .flex_none()
                            .size(px(crate::icons::MD + 4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(div().size(px(12.)).rounded_full().bg(c))
                    }
                    _ => crate::icons::slot(en.icon, crate::icons::MD, theme.muted),
                })
                .child(div().flex_1().whitespace_nowrap().overflow_hidden().child(en.label))
                .child(div().text_size(px(11.)).text_color(theme.faint).child(en.detail))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.inline_confirm(Some(ix), window, cx);
                }))
        });
        Some(
            div()
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(px(250.))
                .p_1()
                .bg(theme.bg)
                .border_1()
                .border_color(theme.border)
                .rounded_lg()
                .shadow_lg()
                .text_size(px(13.))
                .child(
                    div()
                        .id("inline-list")
                        .max_h(px(300.))
                        .overflow_y_scroll()
                        .track_scroll(&self.popup_scroll)
                        .children(items),
                )
                .into_any_element(),
        )
    }
}
