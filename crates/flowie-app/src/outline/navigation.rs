//! Zoom, page history, scroll restoration, and search.

use flowie_core::{NodeId, Query, ViewOptions, ROOT};
use gpui::{div, prelude::*, px, AnyElement, Context, ListState, MouseButton, Pixels, Window};

use super::*;
use crate::app::i18n::t;
use crate::overlays::palette::{node_label, Mode};
use crate::ui::anim;

pub(super) struct PageLocation {
    zoom: NodeId,
    query: String,
    top_row: Option<NodeId>,
    scroll: gpui::ListOffset,
}

/// Where a page was scrolled to: the row at the top and how far into it.
#[derive(Clone, Copy)]
pub(super) struct PageScroll {
    top_row: NodeId,
    offset: Pixels,
}

impl PageScroll {
    pub(super) fn encode(self, zoom: NodeId) -> String {
        format!("{zoom}:{}:{}", self.top_row, f32::from(self.offset))
    }

    pub(super) fn decode(v: &str) -> Option<(NodeId, Self)> {
        let mut it = v.split(':');
        let zoom = it.next()?.parse().ok()?;
        let top_row = it.next()?.parse().ok()?;
        let offset = px(it.next()?.parse().ok()?);
        Some((zoom, PageScroll { top_row, offset }))
    }
}

impl Outline {
    pub fn page_back(&mut self, _: &PageBack, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_page(gpui::NavigationDirection::Back, cx);
    }

    pub fn page_forward(&mut self, _: &PageForward, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_page(gpui::NavigationDirection::Forward, cx);
    }

    fn page_location(&self) -> PageLocation {
        let scroll = self.list.logical_scroll_top();
        PageLocation {
            zoom: self.zoom,
            query: self.query.clone(),
            top_row: scroll.item_ix.checked_sub(HEAD).and_then(|ix| self.display.get(ix)).map(|d| d.row.id),
            scroll,
        }
    }

    pub fn navigate_page(&mut self, direction: gpui::NavigationDirection, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        // Back from Trash or What's New returns to where the user was.
        if self.is_auxiliary_page_open() {
            if direction == gpui::NavigationDirection::Back {
                self.close_auxiliary_page(true, cx);
            }
            return;
        }
        let pages = match direction {
            gpui::NavigationDirection::Back => &mut self.back_pages,
            gpui::NavigationDirection::Forward => &mut self.forward_pages,
        };
        let location = loop {
            let Some(location) = pages.pop() else { return };
            if self.ws.tree.contains(location.zoom) {
                break location;
            }
        };
        let current = self.page_location();
        match direction {
            gpui::NavigationDirection::Back => self.forward_pages.push(current),
            gpui::NavigationDirection::Forward => self.back_pages.push(current),
        }
        self.query = location.query;
        self.palette = None;
        self.inline = None;
        self.calendar = None;
        self.show_page(location.zoom, cx);
        let item_ix = location
            .top_row
            .and_then(|id| self.display_index(id))
            .map(|ix| ix + HEAD)
            .unwrap_or(location.scroll.item_ix.min(self.display.len() + HEAD));
        self.list.scroll_to(gpui::ListOffset { item_ix, offset_in_item: location.scroll.offset_in_item });
    }

    pub fn zoom_to(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let id = if self.ws.tree.contains(id) { id } else { ROOT };
        if id != self.zoom {
            self.back_pages.push(self.page_location());
            self.forward_pages.clear();
        }
        self.show_page(id, cx);
    }

    /// Note where the current page is scrolled to (not while searching: that list is a different one).
    pub(super) fn remember_scroll(&mut self) {
        if !self.query.is_empty() {
            return;
        }
        let scroll = self.list.logical_scroll_top();
        match scroll.item_ix.checked_sub(HEAD).and_then(|ix| self.display.get(ix)) {
            Some(d) => {
                self.page_scrolls.insert(self.zoom, PageScroll { top_row: d.row.id, offset: scroll.offset_in_item });
            }
            None => {
                self.page_scrolls.remove(&self.zoom);
            }
        }
    }

    /// Scroll the current page back to where it was last left; returns the row now at the top.
    pub(super) fn restore_scroll(&mut self) -> Option<NodeId> {
        if !self.query.is_empty() {
            return None;
        }
        let s = *self.page_scrolls.get(&self.zoom)?;
        let row = self.display_index(s.top_row)?;
        if row == 0 && s.offset <= px(0.) {
            return None;
        }
        self.list.scroll_to(gpui::ListOffset { item_ix: row + HEAD, offset_in_item: s.offset });
        Some(s.top_row)
    }

    fn show_page(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.popover.close();
        self.remember_scroll();
        let came_from = self.zoom;
        self.zoom = id;
        self.see_around(id);
        // Going anywhere closes What's New (and Trash) without putting the caret back.
        self.close_auxiliary_page(false, cx);
        self.block = None;
        self.ws.history.seal();
        self.anims.clear();
        self.refresh(false);
        self.list.reset(self.display.len() + HEAD + 1);
        self.zoom_tween = Some(Tween::new(anim::ZOOM));
        self.set_meta("zoom", &id.to_string());
        let top = self.restore_scroll();
        // Put the caret somewhere sensible: on the node we came out of, or the top row.
        let target = if self.ws.tree.is_ancestor(id, came_from) && self.row_index(came_from).is_some() {
            Some(came_from)
        } else {
            top.filter(|&t| self.row_index(t).is_some()).or_else(|| self.rows.first().map(|r| r.id))
        };
        match target {
            Some(t) => {
                let len = self.ws.tree.node(t).text.len();
                self.edit = Some(Edit::new(Target::Text(t), len));
                self.reveal(t);
            }
            None if id != ROOT => {
                let len = self.ws.tree.node(id).text.len();
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
            let parent = self.ws.tree.parent(self.zoom);
            self.zoom_to(parent, cx);
        }
    }

    pub fn zoom_home(&mut self, _: &ZoomHome, _: &mut Window, cx: &mut Context<Self>) {
        if self.zoom != ROOT {
            self.zoom_to(ROOT, cx);
        }
    }

    pub fn jump(&mut self, _: &Jump, _: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(Mode::Jump, cx);
    }

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
        let starred = !self.ws.tree.is_starred(id);
        self.ws.set_starred(id, starred);
        if starred {
            self.toast(t!("toast.starred", name = node_label(self, id), shortcut = crate::app::keymap::hint("jump", &self.config.keys)));
        } else {
            self.toast(t!("toast.unstarred", name = node_label(self, id)));
        }
        cx.notify();
    }

    /// A search query, with what `is:new` means here.
    pub fn parse_query(&self, text: &str) -> Query {
        let mut q = Query::parse(text);
        if text.contains("is:new") {
            q.set_new(self.unread_roots());
        }
        q
    }

    pub fn view_options(&self) -> ViewOptions {
        let q = self.parse_query(&self.query);
        ViewOptions { hide_completed: self.hide_completed, query: (!q.is_empty()).then_some(q) }
    }

    pub fn find(&mut self, _: &Find, _: &mut Window, cx: &mut Context<Self>) {
        self.close_auxiliary_page(false, cx);
        let len = self.query.len();
        self.start_edit(Target::Search, len, cx);
        if let Some(e) = self.edit.as_mut() {
            e.sel = 0..len;
        }
    }

    /// Change the search. Starting one notes where the page was scrolled;
    /// clearing it scrolls the page back there instead of to the top.
    pub(super) fn set_query(&mut self, q: String) {
        if q == self.query {
            return;
        }
        if self.query.is_empty() {
            self.remember_scroll();
        }
        self.query = q;
        self.refresh(false);
        self.list.reset(self.display.len() + HEAD + 1);
        self.restore_scroll();
    }

    /// Clear the search and keep the caret in the (now empty) search box.
    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.close_auxiliary_page(false, cx);
        self.set_query(String::new());
        self.start_edit(Target::Search, 0, cx);
    }

    pub fn search_for(&mut self, q: &str, cx: &mut Context<Self>) {
        self.close_auxiliary_page(false, cx);
        self.set_query(q.to_string());
        self.start_edit(Target::Search, self.query.len(), cx);
    }

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
            self.toast(t!("toast.search_saved", query = q, shortcut = crate::app::keymap::hint("jump", &self.config.keys)));
        }
        self.save_searches();
        cx.notify();
    }

    fn jump_match(&mut self, dir: isize, cx: &mut Context<Self>) {
        let q = self.parse_query(&self.query);
        if q.is_empty() {
            return;
        }
        let hits: Vec<NodeId> =
            self.rows.iter().map(|r| r.id).filter(|id| q.matches(self.ws.tree.node(self.ws.tree.source(*id)))).collect();
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
        let len = self.ws.tree.node(self.ws.tree.source(id)).text.len();
        self.start_edit(Target::Text(id), len, cx);
        self.toast(t!("toast.match", index = next + 1, total = hits.len()));
    }

    pub fn next_match(&mut self, _: &NextMatch, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_match(1, cx);
    }

    pub fn prev_match(&mut self, _: &PrevMatch, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_match(-1, cx);
    }

    /// How open the search box is: 0 is its icon, 1 the full box. It opens
    /// while it's being typed in or holds a search, and otherwise rests as an
    /// icon; clicking the icon or the Find shortcut opens it. Call once per
    /// frame before drawing the top bar: it starts the transition.
    pub(super) fn search_openness(&mut self) -> f32 {
        let open = !self.query.is_empty() || self.edit.as_ref().is_some_and(|e| e.target == Target::Search);
        if open != self.search_open {
            let from = self.current_search_openness();
            self.search_open = open;
            self.search_tween = Some((Tween::new(anim::SEARCH), from));
        }
        self.current_search_openness()
    }

    fn current_search_openness(&self) -> f32 {
        let to = if self.search_open { 1. } else { 0. };
        match self.search_tween {
            Some((t, from)) => from + (to - from) * crate::ui::anim::ease_in_out(t.t()),
            None => to,
        }
    }

    /// The search box at `openness` (see `search_openness`): its icon when
    /// closed, the box widening out of it as it opens.
    pub(super) fn render_search(&mut self, openness: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let editing = self.edit.as_ref().is_some_and(|e| e.target == Target::Search);
        if openness <= 0. {
            return crate::ui::button::tool_button(theme, "search-icon", "search", theme.muted)
                .on_click(cx.listener(|this, _, window, cx| this.find(&Find, window, cx)))
                .into_any_element();
        }
        let inner = if editing {
            let (font, _) = self.base_style(window, false);
            let run = gpui::TextRun {
                len: self.query.len(),
                font,
                color: theme.text,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let q = self.query.clone();
            let ed = self.editor_element(q, vec![run], theme.caret, theme.selection, cx);
            self.editor_mouse(cx).child(ed).into_any_element()
        } else if self.query.is_empty() {
            let find_keys = crate::app::keymap::keys_for("find", &self.config.keys).into_iter().next().unwrap_or_default();
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_color(theme.faint)
                .child(t!("common.search"))
                .child(crate::ui::controls::shortcut(&find_keys, theme.faint, None))
                .into_any_element()
        } else {
            div().text_color(theme.text).child(self.query.clone()).into_any_element()
        };
        crate::ui::controls::search_field(theme, "search", editing)
            .w(px(SEARCH_CLOSED_W + (SEARCH_OPEN_W - SEARCH_CLOSED_W) * openness))
            .overflow_hidden()
            .child(crate::ui::icons::icon("search", 14., theme.muted))
            .child(div().flex_1().min_w_0().opacity(openness).child(inner))
            .when(!self.query.is_empty() && openness >= 1., |d| {
                d.child(
                    crate::ui::button::icon_button(theme, "search-clear", "circle-x", theme.muted)
                        .size(px(20.))
                        .rounded_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.clear_search(cx);
                            }),
                        ),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.edit.as_ref().map(|e| e.target) != Some(Target::Search) {
                        this.close_auxiliary_page(false, cx);
                        let len = this.query.len();
                        this.start_edit(Target::Search, len, cx);
                    }
                }),
            )
            .into_any_element()
    }
}

/// The search box's width as an icon and fully open.
pub(super) const SEARCH_CLOSED_W: f32 = 28.;
pub(super) const SEARCH_OPEN_W: f32 = 240.;

/// A mouse wheel scrolls in steps of whole lines; instead of jumping a step at once, the outline
/// eases toward where the wheel has sent it, the way a trackpad scrolls.
#[derive(Default)]
pub(super) struct WheelScroll {
    /// Distance still to scroll, positive = down.
    pending: Pixels,
    last_frame: Option<std::time::Instant>,
}

impl WheelScroll {
    /// How fast the remaining distance shrinks: about 63% of it every `TAU` seconds.
    const TAU: f32 = 0.06;

    pub fn running(&self) -> bool {
        self.pending != Pixels::ZERO
    }

    /// Adds one wheel step; turning the wheel the other way drops what was left.
    pub fn push(&mut self, distance: Pixels) {
        if (distance > Pixels::ZERO) != (self.pending > Pixels::ZERO) {
            self.pending = Pixels::ZERO;
        }
        if !self.running() {
            self.last_frame = Some(std::time::Instant::now());
        }
        self.pending += distance;
    }

    /// Scrolls `list` by this frame's share, stopping at either end.
    pub fn step(&mut self, list: &ListState) {
        if !self.running() {
            return;
        }
        let now = std::time::Instant::now();
        let dt = self.last_frame.map_or(1. / 60., |t| now.duration_since(t).as_secs_f32()).min(0.05);
        self.last_frame = Some(now);
        let mut step = self.pending * (1. - (-dt / Self::TAU).exp());
        if (self.pending - step).abs() < px(0.5) {
            step = self.pending;
        }
        let top = -list.scroll_px_offset_for_scrollbar().y;
        list.set_offset_from_scrollbar(gpui::point(Pixels::ZERO, -(top + step)));
        let moved = -list.scroll_px_offset_for_scrollbar().y - top;
        // Short of the step means an end was reached: nothing more to scroll that way.
        if (moved - step).abs() > px(0.5) {
            self.pending = Pixels::ZERO;
        } else {
            self.pending -= step;
        }
    }
}
