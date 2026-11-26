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
use std::collections::HashMap;
use std::rc::Rc;

use flowie_core::{markup, BlockKind, NodeId};
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
    if width <= 1. {
        return 1;
    }
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
                if *x > 0. && *x + w > width {
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

/// One row's height as last worked out, and what it depends on.
struct Cached {
    key: (usize, usize, BlockKind, u16, u32),
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
    /// Measured header and footer heights.
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
        if (column - self.heights.column).abs() >= 1. {
            self.heights.column = column;
            self.heights.rows.clear();
            self.heights.edges = (None, None);
            self.heights.dirty = true;
        }
        if self.heights.dirty {
            self.heights.dirty = false;
            let mut values = Vec::with_capacity(self.display.len() + HEAD + 1);
            values.push(self.heights.edges.0.unwrap_or(if self.zoom == flowie_core::ROOT { 0. } else { 64. }));
            for ix in 0..self.display.len() {
                values.push(self.row_height(ix, window));
            }
            values.push(self.heights.edges.1.unwrap_or(40.));
            self.heights.map.borrow_mut().reset(values);
        }
        self.measure_drawn_items();
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
                if let Some(cached) = self.heights.rows.get_mut(&d.row.id) {
                    cached.measured = Some(h);
                }
            }
            self.heights.map.borrow_mut().set(ix, h);
            ix += 1;
        }
    }

    /// The height of display row `ix`: measured if it was drawn as it is now, else predicted.
    fn row_height(&mut self, ix: usize, window: &Window) -> f64 {
        let d = self.display[ix];
        if d.leaving {
            return 0.;
        }
        let tree = &self.ws.tree;
        let Some(node) = tree.get(d.row.id) else { return 0. };
        let src = tree.node(tree.source(d.row.id));
        let key = (src.text.len(), src.note.len(), node.kind, d.row.depth, self.heights.column.to_bits());
        if let Some(cached) = self.heights.rows.get(&d.row.id).filter(|c| c.key == key) {
            return cached.measured.unwrap_or(cached.predicted);
        }
        let predicted = self.predict_row_height(d.row.id, d.row.depth, window);
        self.heights.rows.insert(d.row.id, Cached { key, predicted, measured: None });
        predicted
    }

    /// A row's height worked out from its text, without drawing it (see `rows::render_row`
    /// for the geometry this follows).
    fn predict_row_height(&mut self, id: NodeId, depth: u16, window: &Window) -> f64 {
        let tree = &self.ws.tree;
        let kind = tree.node(id).kind;
        let src = tree.node(tree.source(id));
        let (font_size, line_h, top_pad) = row_metrics(kind);
        let code = kind == BlockKind::Code;
        // Row padding, indent, bullet (18) and the gap after it (6), then the block's own padding.
        let mut width = self.heights.column - 2. * PAD_X - depth as f32 * INDENT - 24.;
        width -= match kind {
            BlockKind::Quote => 12.,
            BlockKind::Code => 16.,
            _ => 0.,
        };
        let text = if code { src.text.clone() } else { markup::plain_text(&src.text) };
        let note = (!src.note.is_empty()).then(|| markup::plain_text(&src.note));
        let image = self.row_image(id).map_or(0., |(_, info)| Self::image_box(info).1 + 6.);
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
}
