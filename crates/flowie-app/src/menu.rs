//! Right-click menus. There are two, split by what they act on:
//!
//! - the text menu (`format_menu.rs`): the selected text in the bullet being
//!   edited — cut / copy / paste, inline styles and colors;
//! - the bullet menu (here): whole bullets — open, fold, complete, star,
//!   block type, copy, duplicate, move, mirror, export, delete.
//!
//! Only one is open at a time, both are built from `ui::menu` (card, rows,
//! separators), and both close on Esc, an outside click or scrolling.

use std::ops::Range;

use flowie_core::{BlockKind, NodeId};
use gpui::{div, prelude::*, px, AnyElement, ClipboardItem, Context, MouseDownEvent, Pixels, Point, Window};

use crate::i18n::t;
use crate::outline::*;
use crate::palette::{node_label, Mode};
pub use crate::ui::menu::{card, heading, place_menu, row, separator};

pub enum ContextMenu {
    /// On a text selection in the edited bullet.
    Text { at: Point<Pixels> },
    /// On a bullet's marker, for `ids` (the selection, or just that bullet).
    Bullet { at: Point<Pixels>, ids: Vec<NodeId>, restore: Option<Restore> },
}

/// What to bring back when a bullet menu opened on an unselected bullet
/// closes: the menu selects that bullet only while it is open.
pub struct Restore {
    temp: BlockSel,
    edit: Option<(Target, Range<usize>, bool)>,
    block: Option<BlockSel>,
}

#[derive(Clone, Copy)]
enum Item {
    ZoomIn,
    ExpandAll,
    CollapseAll,
    Complete,
    Star,
    Kind(BlockKind),
    Copy,
    CopyLink,
    Duplicate,
    MoveTo,
    MirrorTo,
    Export,
    Delete,
}

/// "3 min ago", from a millisecond timestamp.
pub fn ago(ms: i64) -> String {
    let m = (flowie_core::tree::now_millis() - ms) / 60_000;
    if m < 1 {
        t!("time.just_now").to_string()
    } else if m < 60 {
        t!("time.minutes_ago", count = m)
    } else if m < 60 * 24 {
        t!("time.hours_ago", count = m / 60)
    } else {
        t!("time.days_ago", count = m / 60 / 24)
    }
}

fn date_time(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

// ------------------------------------------------------------- open/close

impl Outline {
    /// Close whichever menu is open; true if one was.
    pub fn close_menu(&mut self) -> bool {
        match self.menu.take() {
            None => false,
            Some(ContextMenu::Text { .. }) => true,
            Some(ContextMenu::Bullet { restore, .. }) => {
                if let Some(r) = restore {
                    self.apply_restore(r);
                }
                true
            }
        }
    }

    /// Undo the menu's temporary selection, unless a command changed it.
    fn apply_restore(&mut self, r: Restore) {
        if self.palette.is_some() || self.block != Some(r.temp) {
            return;
        }
        self.block = r.block;
        self.edit = r.edit.and_then(|(target, sel, reversed)| {
            let node = match target {
                Target::Text(id) | Target::Note(id) => Some(id),
                _ => None,
            };
            // Only back into a row that still exists and is visible.
            if node.is_some_and(|id| !self.tree.contains(id) || self.row_index(id).is_none()) {
                return None;
            }
            let mut e = Edit::new(target, 0);
            e.sel = sel;
            e.reversed = reversed;
            Some(e)
        });
        if self.edit.is_some() {
            self.block = None;
        }
    }

    /// Right-click on a bullet's marker. Inside the selection it acts on the
    /// whole selection; elsewhere on that bullet alone.
    pub fn open_bullet_menu(&mut self, id: NodeId, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu();
        self.inline = None;
        self.popover.close();
        if self.palette.is_some() || id == self.zoom {
            return;
        }
        let (ids, restore) = if self.is_block_selected(id) {
            (self.block_roots().unwrap_or_else(|| vec![id]), None)
        } else {
            let temp = BlockSel { anchor: id, head: id };
            let edit = self.edit.as_ref().map(|e| (e.target, e.sel.clone(), e.reversed));
            let r = Restore { temp, edit, block: self.block };
            self.history.seal();
            self.edit = None;
            self.block = Some(temp);
            (vec![id], Some(r))
        };
        window.focus(&self.focus, cx);
        self.menu = Some(ContextMenu::Bullet { at, ids, restore });
        self.menu_scroll = gpui::ScrollHandle::new();
        cx.notify();
    }

    fn run_bullet_item(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ContextMenu::Bullet { ids, restore, .. }) = self.menu.take() else { return };
        let ids: Vec<NodeId> = ids.into_iter().filter(|id| self.tree.contains(*id)).collect();
        let Some(&first) = ids.first() else { return };
        match item {
            Item::ZoomIn => {
                // Navigating away: don't bring the old caret back.
                self.block = None;
                self.zoom_to(first, cx);
                return cx.notify();
            }
            Item::ExpandAll | Item::CollapseAll => {
                for &id in &ids {
                    let src = self.tree.source(id);
                    self.tree.set_collapsed(id, false);
                    self.tree.expand_to_level(src, if matches!(item, Item::ExpandAll) { usize::MAX } else { 1 });
                    self.anims.start(id, crate::anim::Channel::Fold, crate::anim::ROW_IN);
                }
                self.commit_structure(None, cx);
            }
            Item::Complete => self.toggle_complete(&ToggleComplete, window, cx),
            Item::Star => self.star(first, cx),
            Item::Kind(kind) => self.set_kind_of_targets(kind, cx),
            Item::Copy => {
                self.copy_items(&ids, cx);
                self.toast(t!("toast.copied", count = ids.len()));
            }
            Item::CopyLink => {
                let links: Vec<String> = ids
                    .iter()
                    .map(|&id| {
                        let src = self.tree.source(id);
                        let label: String = node_label(self, src).replace(['[', ']'], "").chars().take(48).collect();
                        format!("[{label}](flowie:{src})")
                    })
                    .collect();
                cx.write_to_clipboard(ClipboardItem::new_string(links.join("\n")));
                self.toast(t!("toast.link_copied", count = ids.len()));
            }
            Item::Duplicate => self.duplicate(&ids, cx),
            Item::MoveTo => self.open_palette(Mode::MoveTo(ids), cx),
            Item::MirrorTo => self.open_palette(Mode::MirrorTo(ids), cx),
            Item::Export => self.export_markdown(&ExportMarkdown, window, cx),
            Item::Delete => self.delete_node(&DeleteNode, window, cx),
        }
        if let Some(r) = restore {
            self.apply_restore(r);
        }
        cx.notify();
    }

    /// Copies of `ids` (with their children) right after the last of them,
    /// left selected.
    fn duplicate(&mut self, ids: &[NodeId], cx: &mut Context<Self>) {
        let Some(&last) = ids.last() else { return };
        let items = self.tree.to_items(ids);
        let (parent, index) = (self.tree.parent(last), self.tree.index_in_parent(last) + 1);
        let top = self.tree.insert_items(parent, index, &items);
        self.edit = None;
        self.block = top.first().zip(top.last()).map(|(a, h)| BlockSel { anchor: *a, head: *h });
        self.toast(t!("toast.duplicated", count = top.len()));
        self.commit_structure(None, cx);
    }

    /// Mirrors of `ids` at the end of `target`.
    pub fn mirror_nodes_to(&mut self, ids: &[NodeId], target: NodeId, cx: &mut Context<Self>) {
        let mut made = 0;
        for &id in ids {
            let n = self.tree.children(target).len();
            if self.tree.create_mirror(self.tree.source(id), target, n).is_some() {
                made += 1;
            }
        }
        if made == 0 {
            self.toast(t!("toast.mirror_into_self"));
            return cx.notify();
        }
        self.tree.set_collapsed(target, false);
        self.toast(t!("toast.mirrored", count = made, target = node_label(self, target)));
        self.commit_structure(None, cx);
    }

    // ---------------------------------------------------------- render

    pub fn render_menu(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.menu.as_ref()? {
            ContextMenu::Text { at } => {
                let at = *at;
                self.render_format_menu(at, window, cx)
            }
            ContextMenu::Bullet { at, ids, .. } => {
                let (at, ids) = (*at, ids.clone());
                self.render_bullet_menu(at, ids, window, cx)
            }
        }
    }

    fn render_bullet_menu(&mut self, at: Point<Pixels>, ids: Vec<NodeId>, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ids: Vec<NodeId> = ids.into_iter().filter(|id| self.tree.contains(*id)).collect();
        let Some(&first) = ids.first() else {
            self.menu = None;
            return None;
        };
        let theme = self.theme;
        let single = ids.len() == 1;
        let keys = &self.config.keys;
        let chip = |id: &str| crate::keymap::keys_for(id, keys).into_iter().next();
        let item = |ix: usize, icon: &str, label: String, shortcut: Option<String>, it: Item, cx: &mut Context<Self>| {
            row(theme, ("bullet-menu-item", ix), icon, label, shortcut, matches!(it, Item::Delete))
                .on_click(cx.listener(move |this, _, window, cx| this.run_bullet_item(it, window, cx)))
        };

        let has_kids = ids.iter().any(|id| self.tree.has_children(self.tree.source(*id)));
        let all_done = ids.iter().all(|id| self.tree.node(self.tree.source(*id)).completed);
        let starred = self.starred.contains(&first);
        let kind = self.tree.node(first).kind;
        let same_kind = ids.iter().all(|id| self.tree.node(*id).kind == kind);

        let mut c = card(theme, "bullet-menu").on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
            if this.close_menu() {
                cx.notify();
            }
        }));
        if !single {
            c = c.child(heading(theme, t!("bullet_menu.selected", count = ids.len())));
        }

        // Open / fold
        if single {
            c = c.child(item(0, "corner-down-right", t!("shortcut.zoom-in").into(), chip("zoom-in"), Item::ZoomIn, cx));
        }
        if has_kids {
            c = c.child(item(1, "chevron-down", t!("bullet_menu.expand_all").into(), None, Item::ExpandAll, cx)).child(item(
                2,
                "chevron-right",
                t!("bullet_menu.collapse_all").into(),
                None,
                Item::CollapseAll,
                cx,
            ));
        }
        c = c.child(separator(theme));

        // State
        let done_label = if all_done { t!("bullet_menu.uncomplete") } else { t!("shortcut.complete") };
        c = c.child(item(3, "check", done_label.into(), chip("complete"), Item::Complete, cx));
        if single {
            let star_label = if starred { t!("bullet_menu.unstar") } else { t!("bullet_menu.star") };
            c = c.child(item(4, "star", star_label.into(), chip("star"), Item::Star, cx));
        }

        // Block type: a row of icons, like the color row of the text menu.
        let current = crate::inline::kinds().find(|k| same_kind && k.0 == kind).map(|k| k.2);
        let kind_title = match current {
            Some(name) => t!("bullet_menu.kind_current", kind = name),
            None => t!("bullet_menu.kind").into(),
        };
        let kinds = crate::inline::kinds().enumerate().map(|(i, (k, icon, _))| {
            let on = same_kind && k == kind;
            crate::ui::menu::cell(theme, ("bullet-menu-kind", i), on)
                .child(crate::icons::icon(icon, crate::icons::SM, if on { theme.accent } else { theme.muted }))
                .on_click(cx.listener(move |this, _, window, cx| this.run_bullet_item(Item::Kind(k), window, cx)))
        });
        c = c
            .child(separator(theme))
            .child(heading(theme, kind_title))
            .child(div().flex().justify_between().px_1().py_1().children(kinds))
            .child(separator(theme));

        // Copy
        c = c
            .child(item(5, "copy", t!("bullet_menu.copy").into(), chip("copy"), Item::Copy, cx))
            .child(item(6, "link", t!("bullet_menu.copy_link").into(), None, Item::CopyLink, cx))
            .child(item(7, "plus", t!("bullet_menu.duplicate").into(), None, Item::Duplicate, cx))
            .child(separator(theme));

        // Elsewhere
        c = c
            .child(item(8, "arrow-right", t!("shortcut.move-to").into(), chip("move-to"), Item::MoveTo, cx))
            .child(item(9, "circle", t!("bullet_menu.mirror_to").into(), None, Item::MirrorTo, cx))
            .child(item(10, "arrow-down", t!("shortcut.export-md").into(), chip("export-md"), Item::Export, cx))
            .child(separator(theme));

        // Delete, saying how much goes with it.
        let below: usize = ids.iter().map(|id| self.tree.descendants(self.tree.source(*id)).len()).sum();
        let delete_label = match (single, below) {
            (true, 0) => t!("shortcut.delete-bullet").to_string(),
            (true, n) => t!("bullet_menu.delete_with_children", count = n),
            (false, _) => t!("bullet_menu.delete_count", count = ids.len()),
        };
        c = c.child(item(11, "trash-2", delete_label, chip("delete-bullet"), Item::Delete, cx));

        // About this bullet.
        if single {
            let src = self.tree.source(first);
            let node = self.tree.node(src);
            let mut facts =
                vec![t!("bullet_menu.created", date = date_time(node.created)), t!("bullet_menu.modified", ago = ago(node.modified))];
            let mut counts = Vec::new();
            if below > 0 {
                counts.push(t!("bullet_menu.descendants", count = below));
            }
            let links = self.tree.link_count(src);
            if links > 0 {
                counts.push(t!("bullet_menu.links", count = links));
            }
            let mirrors = self.tree.mirrors_of(src).len();
            if mirrors > 0 {
                counts.push(t!("bullet_menu.mirrors", count = mirrors));
            }
            if !counts.is_empty() {
                facts.push(counts.join(" · "));
            }
            c = c.child(separator(theme)).child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .flex_col()
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(theme.muted)
                    .children(facts.into_iter().map(|f| div().child(f))),
            );
        }
        Some(place_menu(at, c, &self.menu_scroll, theme, window))
    }
}
