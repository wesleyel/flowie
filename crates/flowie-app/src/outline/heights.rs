//! The outline's height map, for the scrollbar.
//!
//! GPUI's list only knows the heights of the rows it has drawn, so a scrollbar built on
//! its total jumps as rows scroll into view and a drag can't land where it points. This
//! keeps a height for every list item instead, the way editors keep a height map
//! (CodeMirror's `HeightMap`), and maps the scrollbar to it:
//!
//! - **Predicted** heights for rows not drawn yet: the text's display width is summed from
//!   glyph advances measured once per character, and broken into lines greedily at word
//!   boundaries (the approach of Pretext: measure once, then lay out by arithmetic).
//! - **Measured** heights replace them as rows are drawn, cached per row until its text,
//!   kind, depth or the column width changes.
//! - A **Fenwick tree** sums them, so the offset of an item and the item at an offset are
//!   O(log n), and one row's new height is an O(log n) update.
//!
//! The scrollbar reads its position from here and, when dragged, scrolls the list to the
//! item and offset this map gives, so where the thumb points is where it lands. While a
//! drag is on, new measurements wait, so the map holds still under the pointer.

use std::cell::RefCell;
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use flowie_core::{markup, BlockKind, NodeId, Tree, ROOT};
use gpui::{px, ListOffset, ListState, Pixels, Window};

use super::{row_metrics, Outline, HEAD, INDENT, PAD_X};
use crate::ui::anim::Channel;

/// Prefix sums over item heights.
#[derive(Default)]
pub struct Fenwick {
    tree: Vec<f64>,
    values: Vec<f64>,
}

impl Fenwick {
    pub fn new(values: Vec<f64>) -> Fenwick {
        let mut tree = values.clone();
        for i in 0..tree.len() {
            let j = i | (i + 1);
            if j < tree.len() {
                tree[j] += tree[i];
            }
        }
        Fenwick { tree, values }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn get(&self, i: usize) -> f64 {
        self.values.get(i).copied().unwrap_or(0.)
    }

    pub fn set(&mut self, i: usize, value: f64) {
        let Some(old) = self.values.get_mut(i) else { return };
        let delta = value - *old;
        *old = value;
        let mut j = i;
        while j < self.tree.len() {
            self.tree[j] += delta;
            j |= j + 1;
        }
    }

    /// The sum of the first `i` values: where item `i` starts.
    pub fn prefix(&self, i: usize) -> f64 {
        let mut sum = 0.;
        let mut j = i.min(self.tree.len());
        while j > 0 {
            sum += self.tree[j - 1];
            j &= j - 1;
        }
        sum
    }

    pub fn total(&self) -> f64 {
        self.prefix(self.len())
    }

    /// The item at offset `y` and how far into it `y` is. Past the end: the last item.
    pub fn find(&self, y: f64) -> (usize, f64) {
        if self.len() == 0 {
            return (0, 0.);
        }
        // Walk down the implicit tree: the largest `i` whose prefix is ≤ y.
        let (mut i, mut rest) = (0usize, y.max(0.));
        let mut step = self.len().next_power_of_two();
        while step > 0 {
            let next = i + step;
            if next <= self.len() && self.tree[next - 1] <= rest {
                i = next;
                rest -= self.tree[next - 1];
            }
            step /= 2;
        }
        if i >= self.len() {
            (self.len() - 1, self.get(self.len() - 1))
        } else {
            (i, rest)
        }
    }
}

/// Item heights in list order, with measurements held back during a drag.
#[derive(Default)]
pub struct HeightMap {
    sums: Fenwick,
    frozen: bool,
    pending: Vec<(usize, f64)>,
}

impl HeightMap {
    fn reset(&mut self, values: Vec<f64>) {
        self.sums = Fenwick::new(values);
        self.pending.clear();
    }

    fn set(&mut self, i: usize, value: f64) {
        if (self.sums.get(i) - value).abs() < 0.5 {
            return;
        }
        if self.frozen {
            self.pending.push((i, value));
        } else {
            self.sums.set(i, value);
        }
    }

    fn freeze(&mut self, on: bool) {
        self.frozen = on;
        if !on {
            for (i, value) in std::mem::take(&mut self.pending) {
                self.sums.set(i, value);
            }
        }
    }
}

/// How many lines `text` takes in `width` with glyphs as wide as `advance` says, broken
/// greedily at spaces (and between CJK characters); a word longer than a line is broken
/// between characters. Never less than one line.
pub fn wrap_lines(text: &str, width: f32, mut advance: impl FnMut(char) -> f32) -> usize {
    let width = width.max(0.);
    let (mut lines, mut x) = (1usize, 0f32);
    // The word being collected and its width.
    let (mut word, mut word_w) = (Vec::new(), 0f32);
    let place = |word: &mut Vec<f32>, word_w: &mut f32, lines: &mut usize, x: &mut f32| {
        if word.is_empty() {
            return;
        }
        if *x > 0. && *x + *word_w > width {
            *lines += 1;
            *x = 0.;
        }
        if *word_w > width {
            for &w in word.iter() {
                if w > 0. && *x > 0. && *x + w > width {
                    *lines += 1;
                    *x = 0.;
                }
                *x += w;
            }
        } else {
            *x += *word_w;
        }
        word.clear();
        *word_w = 0.;
    };
    for ch in text.chars() {
        if ch == '\n' {
            place(&mut word, &mut word_w, &mut lines, &mut x);
            lines += 1;
            x = 0.;
            continue;
        }
        let w = advance(ch);
        if ch.is_whitespace() {
            place(&mut word, &mut word_w, &mut lines, &mut x);
            // A space at the end of a line hangs past it instead of wrapping.
            x += w;
        } else if is_cjk(ch) {
            place(&mut word, &mut word_w, &mut lines, &mut x);
            word.push(w);
            word_w = w;
            place(&mut word, &mut word_w, &mut lines, &mut x);
        } else {
            word.push(w);
            word_w += w;
        }
    }
    place(&mut word, &mut word_w, &mut lines, &mut x);
    lines
}

/// Characters a line may break before or after (CJK ideographs, kana, hangul, fullwidth forms).
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

/// Minimum text column width assumed under extreme nesting or very narrow windows so
/// prediction keeps wrapping tightly instead of collapsing to one line.
const MIN_TEXT_WIDTH: f32 = 16.;

/// Available text column width for a row at `depth` and `kind` inside `column`.
fn row_text_width(column: f32, depth: u16, kind: BlockKind) -> f32 {
    // Row padding, indent, bullet (18) and the gap after it (6), then the block's own padding.
    let extra = match kind {
        BlockKind::Quote => 12.,
        BlockKind::Code => 16.,
        _ => 0.,
    };
    (column - 2. * PAD_X - depth as f32 * INDENT - 24. - extra).max(MIN_TEXT_WIDTH)
}

/// What a row's height depends on; a change to any of it invalidates the cached height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowKey {
    content: u64,
    image_box: Option<(u32, u32)>,
    kind: BlockKind,
    depth: u16,
    column: u32,
}

impl RowKey {
    fn new(text: &str, note: &str, image: &str, image_box: Option<(f32, f32)>, kind: BlockKind, depth: u16, column: f32) -> Self {
        let mut h = DefaultHasher::new();
        text.hash(&mut h);
        note.hash(&mut h);
        image.hash(&mut h);
        Self { content: h.finish(), image_box: image_box.map(|(w, h)| (w.to_bits(), h.to_bits())), kind, depth, column: column.to_bits() }
    }
}

/// One row's height as last worked out, and what it depends on.
struct Cached {
    key: RowKey,
    predicted: f64,
    measured: Option<f64>,
}

/// The outline's height map and what it's built from.
#[derive(Default)]
pub struct Heights {
    map: Rc<RefCell<HeightMap>>,
    /// Rebuild the map at the next frame (rows changed).
    dirty: bool,
    rows: HashMap<NodeId, Cached>,
    /// The zoom root (and no-match header state) `edges` were measured for.
    zoom: Option<(NodeId, bool)>,
    /// Measured header and footer heights for `zoom`.
    edges: (Option<f64>, Option<f64>),
    /// Glyph advances by (character, size, bold, monospace).
    glyphs: HashMap<(char, u32, bool, bool), f32>,
    /// The text column's width the cached heights are for.
    column: f32,
}

impl Heights {
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Drop cached header/footer measurements whenever the zoom root (or no-match header) changes.
    fn sync_zoom(&mut self, zoom: NodeId, empty_hint: bool) -> bool {
        if self.zoom == Some((zoom, empty_hint)) {
            return false;
        }
        self.zoom = Some((zoom, empty_hint));
        self.edges = (None, None);
        self.dirty = true;
        true
    }

    /// Header height for `zoom`: measured for this zoom root, or its default estimate.
    fn header_height(&self, zoom: NodeId, empty_hint: bool) -> f64 {
        let measured = if self.zoom == Some((zoom, empty_hint)) { self.edges.0 } else { None };
        measured.unwrap_or(if zoom == ROOT && !empty_hint { 0. } else { 64. })
    }

    /// Drop cached entries for deleted nodes (and stale virtual rows) when the cache outgrows the tree.
    fn prune(&mut self, tree: &Tree, display_pos: &HashMap<NodeId, usize>) {
        let limit = tree.len().saturating_add(display_pos.len()).saturating_add(256);
        if self.rows.len() > limit {
            self.rows.retain(|id, _| if id.is_virtual() { display_pos.contains_key(id) } else { tree.contains(*id) });
        }
    }

    /// Cached height for `id` if it was worked out at the same `depth`.
    pub(super) fn cached_height(&self, id: NodeId, depth: u16) -> Option<f32> {
        self.rows.get(&id).filter(|c| c.key.depth == depth).map(|c| c.measured.unwrap_or(c.predicted) as f32)
    }

    /// Current scroll offset in pixels from this height map, when it is up to date for `zoom`.
    pub(super) fn scroll_top(&self, list: &ListState, zoom: NodeId) -> Option<f32> {
        if self.dirty || self.zoom.is_none_or(|(z, _)| z != zoom) {
            return None;
        }
        let map = self.map.borrow();
        if map.sums.len() == 0 {
            return None;
        }
        let top = list.logical_scroll_top();
        Some((map.sums.prefix(top.item_ix) as f32 + f32::from(top.offset_in_item)).max(0.))
    }

    /// What the scrollbar shows and moves: the list, placed by this map.
    pub fn scroll_model(&self, list: &ListState) -> Rc<dyn crate::ui::scrollbar::ScrollModel> {
        Rc::new(MappedScroll { list: list.clone(), map: self.map.clone() })
    }
}

struct MappedScroll {
    list: ListState,
    map: Rc<RefCell<HeightMap>>,
}

impl crate::ui::scrollbar::ScrollModel for MappedScroll {
    fn top(&self) -> Pixels {
        let top = self.list.logical_scroll_top();
        px((self.map.borrow().sums.prefix(top.item_ix) as f32 + f32::from(top.offset_in_item)).max(0.))
    }

    fn max(&self) -> Pixels {
        let view = f32::from(self.list.viewport_bounds().size.height);
        px((self.map.borrow().sums.total() as f32 - view).max(0.))
    }

    fn set_top(&self, top: Pixels) {
        let max = self.max();
        if top >= max - px(0.5) {
            // The end, laid out back from the last item however tall it turns out.
            self.list.scroll_to_end();
            return;
        }
        let (item_ix, into) = self.map.borrow().sums.find(f64::from(f32::from(top.max(Pixels::ZERO))));
        self.list.scroll_to(ListOffset { item_ix, offset_in_item: px(into as f32) });
    }

    fn drag(&self, on: bool) {
        self.map.borrow_mut().freeze(on);
    }
}

impl Outline {
    /// Bring the height map up to date: rebuild it if the rows or the column changed, then
    /// take the heights of the items drawn in the last frame. Call before drawing the list.
    pub(super) fn update_heights(&mut self, window: &Window) {
        let list_w = f32::from(self.list.viewport_bounds().size.width);
        if list_w <= 0. {
            return;
        }
        let column = self.config.content_width.max_px().map_or(list_w, |max| list_w.min(max));
        let empty_hint = self.rows.is_empty() && !self.query.is_empty();
        let mut stale_bounds = self.heights.sync_zoom(self.zoom, empty_hint);
        if (column - self.heights.column).abs() >= 1. {
            self.heights.column = column;
            self.heights.rows.clear();
            self.heights.edges = (None, None);
            self.heights.dirty = true;
            stale_bounds = true;
        }
        if self.heights.dirty {
            self.heights.dirty = false;
            self.heights.prune(&self.ws.tree, &self.display_pos);
            let mut values = Vec::with_capacity(self.display.len() + HEAD + 1);
            values.push(self.heights.header_height(self.zoom, empty_hint));
            for ix in 0..self.display.len() {
                values.push(self.row_height(ix, window));
            }
            values.push(self.heights.edges.1.unwrap_or(40.));
            self.heights.map.borrow_mut().reset(values);
        }
        if !stale_bounds {
            self.measure_drawn_items();
        }
    }

    /// Record the heights the list drew its items at, from the top of the viewport down.
    fn measure_drawn_items(&mut self) {
        let count = self.display.len() + HEAD + 1;
        let mut ix = self.list.logical_scroll_top().item_ix;
        while ix < count {
            let Some(bounds) = self.list.bounds_for_item(ix) else { break };
            let h = f64::from(f32::from(bounds.size.height));
            if ix == 0 {
                self.heights.edges.0 = Some(h);
            } else if ix == count - 1 {
                self.heights.edges.1 = Some(h);
            } else {
                let d = self.display[ix - HEAD];
                // A row growing in or shrinking out isn't at its height yet.
                let moving = d.leaving || self.anims.t(d.row.id, Channel::Enter).is_some_and(|t| t < 1.);
                if moving {
                    ix += 1;
                    continue;
                }
                if let Some(cached) = self.heights.rows.get(&d.row.id) {
                    if cached.measured.is_none_or(|prev| (prev - h).abs() >= 0.5) {
                        let key = self.row_key(d.row.id, d.row.depth).map(|(k, _)| k);
                        if let Some(cached) = self.heights.rows.get_mut(&d.row.id) {
                            if let Some(key) = key {
                                cached.key = key;
                            }
                            cached.measured = Some(h);
                        }
                    }
                }
            }
            self.heights.map.borrow_mut().set(ix, h);
            ix += 1;
        }
    }

    fn row_key(&self, id: NodeId, depth: u16) -> Option<(RowKey, Option<(f32, f32)>)> {
        let tree = &self.ws.tree;
        let node = tree.get(id)?;
        let src = tree.node(tree.source(id));
        let image_box = self.row_image(id).map(|(_, info)| Self::image_box(info));
        let key = RowKey::new(&src.text, &src.note, &src.image, image_box, node.kind, depth, self.heights.column);
        Some((key, image_box))
    }

    /// The height of display row `ix`: measured if it was drawn as it is now, else predicted.
    fn row_height(&mut self, ix: usize, window: &Window) -> f64 {
        let d = self.display[ix];
        if d.leaving {
            return 0.;
        }
        let Some((key, image_box)) = self.row_key(d.row.id, d.row.depth) else { return 0. };
        if let Some(cached) = self.heights.rows.get(&d.row.id).filter(|c| c.key == key) {
            return cached.measured.unwrap_or(cached.predicted);
        }
        let predicted = self.predict_row_height(d.row.id, d.row.depth, image_box, window);
        self.heights.rows.insert(d.row.id, Cached { key, predicted, measured: None });
        predicted
    }

    /// A row's height worked out from its text, without drawing it (see `rows::render_row`
    /// for the geometry this follows).
    fn predict_row_height(&mut self, id: NodeId, depth: u16, image_box: Option<(f32, f32)>, window: &Window) -> f64 {
        let tree = &self.ws.tree;
        let kind = tree.node(id).kind;
        let src = tree.node(tree.source(id));
        let (font_size, line_h, top_pad) = row_metrics(kind);
        let code = kind == BlockKind::Code;
        let width = row_text_width(self.heights.column, depth, kind);
        let text = if code { src.text.clone() } else { markup::plain_text(&src.text) };
        let note = (!src.note.is_empty()).then(|| markup::plain_text(&src.note));
        let image = image_box.map_or(0., |(_, h)| h + 6.);
        let bold = matches!(kind, BlockKind::H1 | BlockKind::H2 | BlockKind::H3);
        let lines = self.count_lines(&text, width, font_size, bold, code, window);
        let mut h = top_pad + 3. + lines as f32 * line_h + image;
        if code {
            h += 8.;
        }
        if let Some(note) = note {
            h += self.count_lines(&note, width, 13., false, false, window) as f32 * 20.;
        }
        f64::from(h)
    }

    fn count_lines(&mut self, text: &str, width: f32, size: f32, bold: bool, mono: bool, window: &Window) -> usize {
        let mut font = window.text_style().font();
        if bold {
            font.weight = gpui::FontWeight::BOLD;
        }
        if mono {
            font.family = crate::app::platform::MONO_FONT.into();
        }
        let system = window.text_system();
        let font_id = system.resolve_font(&font);
        let glyphs = &mut self.heights.glyphs;
        let size_key = (size * 4.).round() as u32;
        wrap_lines(text, width, |ch| {
            *glyphs
                .entry((ch, size_key, bold, mono))
                .or_insert_with(|| system.advance(font_id, px(size), ch).map_or(size * 0.5, |advance| f32::from(advance.width)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenwick_sums_finds_and_updates() {
        let mut f = Fenwick::new(vec![10., 20., 30., 40.]);
        assert_eq!((f.prefix(0), f.prefix(2), f.total()), (0., 30., 100.));
        assert_eq!(f.find(0.), (0, 0.));
        assert_eq!(f.find(35.), (2, 5.));
        assert_eq!(f.find(99.), (3, 39.));
        assert_eq!(f.find(500.), (3, 40.));
        f.set(1, 5.);
        assert_eq!((f.prefix(2), f.total()), (15., 85.));
        assert_eq!(f.find(16.), (2, 1.));
        // A large tree agrees with plain sums.
        let values: Vec<f64> = (0..1000).map(|i| (i % 7) as f64 * 3. + 1.).collect();
        let f = Fenwick::new(values.clone());
        for i in [0, 1, 17, 500, 999, 1000] {
            assert_eq!(f.prefix(i), values[..i].iter().sum::<f64>());
        }
        let (i, into) = f.find(values[..321].iter().sum::<f64>() + 0.5);
        assert_eq!((i, into), (321, 0.5));
    }

    #[test]
    fn wrapping_breaks_at_spaces_cjk_and_long_words() {
        // Every glyph 1 wide.
        let one = |_| 1.;
        assert_eq!(wrap_lines("", 10., one), 1);
        assert_eq!(wrap_lines("aaaa bbbb", 9., one), 1);
        assert_eq!(wrap_lines("aaaa bbbb", 8., one), 2);
        // A trailing space hangs instead of wrapping.
        assert_eq!(wrap_lines("aaaa ", 4., one), 1);
        // A word longer than the line breaks between characters.
        assert_eq!(wrap_lines("aaaaaaaaaa", 4., one), 3);
        // CJK breaks anywhere.
        assert_eq!(wrap_lines("一二三四五六七", 3., one), 3);
        assert_eq!(wrap_lines("a\nb", 10., one), 2);
    }

    #[test]
    fn frozen_map_holds_still_until_released() {
        let mut m = HeightMap::default();
        m.reset(vec![10., 10., 10.]);
        m.freeze(true);
        m.set(1, 50.);
        assert_eq!(m.sums.total(), 30.);
        m.freeze(false);
        assert_eq!(m.sums.total(), 70.);
    }

    #[test]
    fn row_key_invalidates_on_same_length_text_newline_note_and_image_changes() {
        let base = RowKey::new("iiii", "aaaa", "", None, BlockKind::Bullet, 0, 600.);
        assert_eq!(base, RowKey::new("iiii", "aaaa", "", None, BlockKind::Bullet, 0, 600.));
        // Same-length text edit (narrow -> wide glyphs) and newline insertion.
        assert_ne!(base, RowKey::new("mmmm", "aaaa", "", None, BlockKind::Bullet, 0, 600.));
        let one_line = RowKey::new("ab cd", "", "", None, BlockKind::Bullet, 0, 600.);
        assert_ne!(one_line, RowKey::new("ab\ncd", "", "", None, BlockKind::Bullet, 0, 600.));
        // Same-length note edit.
        assert_ne!(base, RowKey::new("iiii", "a\nbb", "", None, BlockKind::Bullet, 0, 600.));
        // Image presence, dimensions, and blob identity changes.
        let with_img = RowKey::new("iiii", "aaaa", "img1", Some((560., 216.)), BlockKind::Image, 0, 600.);
        let loaded_img = RowKey::new("iiii", "aaaa", "img1", Some((200., 100.)), BlockKind::Image, 0, 600.);
        let swapped_img = RowKey::new("iiii", "aaaa", "img2", Some((200., 100.)), BlockKind::Image, 0, 600.);
        assert_ne!(with_img, loaded_img);
        assert_ne!(loaded_img, swapped_img);
        assert_ne!(base, RowKey::new("iiii", "aaaa", "", None, BlockKind::Code, 0, 600.));
        assert_ne!(base, RowKey::new("iiii", "aaaa", "", None, BlockKind::Bullet, 1, 600.));
        assert_ne!(base, RowKey::new("iiii", "aaaa", "", None, BlockKind::Bullet, 0, 640.));
    }

    #[test]
    fn zoom_change_invalidates_cached_header_height() {
        let mut h = Heights::default();
        let page_a = NodeId::random();
        let page_b = NodeId::random();

        assert!(h.sync_zoom(page_a, false));
        assert_eq!(h.header_height(page_a, false), 64.);
        h.edges = (Some(88.), Some(260.));
        assert_eq!(h.header_height(page_a, false), 88.);
        // Staying on the same zoom keeps measured edge heights.
        assert!(!h.sync_zoom(page_a, false));
        assert_eq!(h.header_height(page_a, false), 88.);

        // Zooming back to ROOT must drop page_a's 88px header instead of reusing it.
        assert!(h.sync_zoom(ROOT, false));
        assert_eq!(h.edges, (None, None));
        assert!(h.dirty);
        assert_eq!(h.header_height(ROOT, false), 0.);

        // Zooming from ROOT (0px measured) to page_b must not reuse ROOT's 0px header.
        h.edges.0 = Some(0.);
        assert!(h.sync_zoom(page_b, false));
        assert_eq!(h.edges, (None, None));
        assert_eq!(h.header_height(page_b, false), 64.);

        // Toggling the no-match search hint on ROOT also invalidates header height.
        h.sync_zoom(ROOT, false);
        h.edges.0 = Some(0.);
        assert!(h.sync_zoom(ROOT, true));
        assert_eq!(h.edges, (None, None));
    }

    #[test]
    fn prune_drops_deleted_and_stale_virtual_rows() {
        let mut tree = Tree::new();
        let live_a = tree.create_at(ROOT, 0, "a");
        let live_b = tree.create_at(ROOT, 1, "b");
        let deleted = tree.create_at(ROOT, 2, "gone");
        tree.delete(deleted);

        let v_live = flowie_core::tree::virtual_id(live_a, live_b);
        let v_stale = flowie_core::tree::virtual_id(live_b, live_a);
        let mut display_pos = HashMap::new();
        display_pos.insert(live_a, 0);
        display_pos.insert(live_b, 1);
        display_pos.insert(v_live, 2);

        let mut h = Heights::default();
        let dummy = || Cached { key: RowKey::new("", "", "", None, BlockKind::Bullet, 0, 600.), predicted: 30., measured: Some(30.) };
        for id in [live_a, live_b, deleted, v_live, v_stale] {
            h.rows.insert(id, dummy());
        }
        // Push past the prune threshold with deleted node ids.
        for _ in 0..300 {
            h.rows.insert(NodeId::random(), dummy());
        }

        h.prune(&tree, &display_pos);

        assert!(h.rows.contains_key(&live_a));
        assert!(h.rows.contains_key(&live_b));
        assert!(h.rows.contains_key(&v_live));
        assert!(!h.rows.contains_key(&deleted));
        assert!(!h.rows.contains_key(&v_stale));
        assert_eq!(h.rows.len(), 3);
    }

    #[test]
    fn narrow_width_wrapping_wraps_tightly() {
        let one = |_| 1.;
        assert_eq!(wrap_lines("", 0., one), 1);
        assert_eq!(wrap_lines("abcd", 0.5, one), 4);
        assert_eq!(wrap_lines("abcd", 0., one), 4);
        assert_eq!(wrap_lines("abcd", -20., one), 4);
        assert_eq!(wrap_lines("a\nb\nc", -10., one), 3);
        assert_eq!(wrap_lines("一二三", 0.5, one), 3);
        // Zero-width combining marks stay with their base character even at width 0.
        assert_eq!(wrap_lines("a\u{0301}b", 0., |c| if c == '\u{0301}' { 0. } else { 1. }), 2);

        // Deep nesting and very narrow windows clamp to a positive minimum width.
        let deep_w = row_text_width(300., 30, BlockKind::Bullet);
        assert!(deep_w >= MIN_TEXT_WIDTH);
        assert!(row_text_width(100., 0, BlockKind::Code) >= MIN_TEXT_WIDTH);
        assert!(wrap_lines("deeply nested paragraph text", deep_w, |_| 8.) > 1);
    }
}
