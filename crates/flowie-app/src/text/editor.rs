//! Text input for the single active editor: IME (marked text), caret and
//! selection painting. The text itself is laid out by GPUI's `StyledText`;
//! an overlay canvas reads that layout to place the caret.

use std::ops::Range;

use gpui::{
    canvas, div, fill, point, prelude::*, px, size, AnyElement, Bounds, Context, ElementInputHandler, EntityInputHandler, Hsla, Pixels,
    Point, StyledText, TextLayout, TextRun, UTF16Selection, Window,
};

use flowie_doc::utf16::{from_utf16, to_utf16};

use crate::outline::Outline;

impl Outline {
    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        let t = self.edit_text();
        to_utf16(t, r.start)..to_utf16(t, r.end)
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        let t = self.edit_text();
        from_utf16(t, r.start)..from_utf16(t, r.end)
    }

    /// The editor for the active target: styled text plus caret/selection overlay.
    /// An empty text still gets one line so the caret has somewhere to sit.
    pub fn editor_element(
        &self,
        text: String,
        runs: Vec<TextRun>,
        caret_color: Hsla,
        sel_color: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(edit) = self.edit.as_ref() else { return div().into_any_element() };
        let (shown, runs) = if text.is_empty() {
            let mut run = runs.first().cloned().unwrap_or_else(|| TextRun {
                len: 0,
                font: gpui::font(crate::ui::theme::UI_FONT),
                color: caret_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
            run.len = " ".len();
            (" ".to_string(), vec![run])
        } else {
            (text.clone(), runs)
        };
        let styled = StyledText::new(shown).with_runs(runs);
        let layout = styled.layout().clone();
        // The editor paints in display offsets; the selection is in raw ones.
        let map = self.edit_map();
        let sel = map.to_display(edit.sel.start)..map.to_display(edit.sel.end);
        let caret = map.to_display(edit.caret());
        let marked = edit.marked.as_ref().map(|m| map.to_display(m.start)..map.to_display(m.end));
        let text_len = text.len();
        let entity = cx.entity();
        let focus = self.focus.clone();
        div()
            .relative()
            .w_full()
            .child(styled)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let lh = layout.line_height();
                        let pos = |ix: usize| layout.position_for_index(ix.min(text_len));
                        // Selection: one rect per visual line between start and end.
                        if !sel.is_empty() {
                            if let (Some(a), Some(b)) = (pos(sel.start), pos(sel.end)) {
                                paint_selection(a, b, lh, bounds, sel_color, window);
                            }
                        }
                        if let Some(m) = &marked {
                            if let (Some(a), Some(b)) = (pos(m.start), pos(m.end)) {
                                paint_selection(a, b, lh, bounds, sel_color.opacity(0.5), window);
                            }
                        }
                        if sel.is_empty() && focus.is_focused(window) {
                            if let Some(p) = pos(caret) {
                                window.paint_quad(fill(
                                    Bounds::new(point(p.x - px(0.5), p.y + lh * 0.12), size(px(2.), lh * 0.76)),
                                    caret_color,
                                ));
                            }
                        }
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
                        entity.update(cx, |this, _| {
                            if let Some(e) = this.edit.as_mut() {
                                e.layout = Some(layout);
                                e.bounds = Some(bounds);
                            }
                        });
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }

    /// Caret index for a mouse position, snapped to the nearest character boundary.
    pub fn index_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let layout = self.edit.as_ref()?.layout.as_ref()?;
        let ix = match caret_index_for_position(layout, p) {
            Ok(i) | Err(i) => i,
        };
        Some(self.edit_map().to_raw(ix).min(self.edit_text().len()))
    }

    /// The bullet the internal link under a point leads to.
    pub fn link_target_at_point(&self, p: Point<Pixels>) -> Option<flowie_core::NodeId> {
        let range = self.link_at_point(p)?;
        let raw = &self.edit_text()[range];
        let id = raw.rsplit_once("](flowie:")?.1.strip_suffix(')')?;
        id.parse().ok()
    }

    /// The internal link under a point, as a raw range (double-click selects it whole).
    pub fn link_at_point(&self, p: Point<Pixels>) -> Option<Range<usize>> {
        let layout = self.edit.as_ref()?.layout.as_ref()?;
        let ix = match layout.index_for_position(p) {
            Ok(i) => i,
            Err(_) => return None,
        };
        let map = self.edit_map();
        map.atom_at_display(ix).or_else(|| map.atom_at_display(ix + 1))
    }

    /// Index of the character under a point (not rounded), as the IME expects.
    fn char_index_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let layout = self.edit.as_ref()?.layout.as_ref()?;
        let ix = match layout.index_for_position(p) {
            Ok(i) | Err(i) => i,
        };
        Some(self.edit_map().to_raw(ix).min(self.edit_text().len()))
    }
}

/// Like `TextLayout::index_for_position`, but a point past the middle of a
/// glyph snaps to the boundary after it, as macOS text views do. GPUI's own
/// version always returns the boundary before the glyph under the point, so a
/// drag only reaches a character once the pointer has crossed all of it.
/// `Ok` / `Err` keep GPUI's meaning (point over the text or not).
pub(crate) fn caret_index_for_position(layout: &TextLayout, p: Point<Pixels>) -> Result<usize, usize> {
    let hit = layout.index_for_position(p);
    let Ok(ix) = hit else { return hit };
    let (Some(line), Some(left)) = (layout.line_layout_for_index(ix), layout.position_for_index(ix)) else { return hit };
    let line_start = layout.text()[..ix].rfind('\n').map_or(0, |n| n + 1);
    let unwrapped = &line.unwrapped_layout;
    let mut glyphs = unwrapped.runs.iter().flat_map(|r| r.glyphs.iter());
    let Some(glyph) = glyphs.by_ref().find(|g| g.index == ix - line_start) else { return hit };
    let (next_ix, next_x) = glyphs.find(|g| g.index > glyph.index).map_or((unwrapped.len, unwrapped.width), |g| (g.index, g.position.x));
    if p.x - left.x > (next_x - glyph.position.x) / 2. {
        Ok(line_start + next_ix)
    } else {
        hit
    }
}

pub(crate) fn paint_selection(a: Point<Pixels>, b: Point<Pixels>, lh: Pixels, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    if (a.y - b.y).abs() < px(1.) {
        window.paint_quad(fill(Bounds::from_corners(a, point(b.x, b.y + lh)), color));
        return;
    }
    window.paint_quad(fill(Bounds::from_corners(a, point(bounds.right(), a.y + lh)), color));
    if b.y > a.y + lh {
        window.paint_quad(fill(Bounds::from_corners(point(bounds.left(), a.y + lh), point(bounds.right(), b.y)), color));
    }
    window.paint_quad(fill(Bounds::from_corners(point(bounds.left(), b.y), point(b.x, b.y + lh)), color));
}

impl EntityInputHandler for Outline {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        self.edit.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.edit_text()[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let e = self.edit.as_ref()?;
        let sel = e.sel.clone();
        let reversed = e.reversed;
        Some(UTF16Selection { range: self.range_to_utf16(&sel), reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let m = self.edit.as_ref()?.marked.clone()?;
        Some(self.range_to_utf16(&m))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        if let Some(e) = self.edit.as_mut() {
            e.marked = None;
        }
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.edit.as_ref() else { return };
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(e.marked.clone()).unwrap_or(e.sel.clone());
        self.replace_edit_text(range, text, false, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(e) = self.edit.as_ref() else { return };
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(e.marked.clone()).unwrap_or(e.sel.clone());
        self.replace_edit_text(range.clone(), new_text, true, cx);
        if let Some(sel) = new_selected_range_utf16 {
            // The new selection is relative to the inserted text.
            let start = range.start + from_utf16(new_text, sel.start);
            let end = range.start + from_utf16(new_text, sel.end);
            if let Some(e) = self.edit.as_mut() {
                e.sel = start..end;
            }
        }
        if new_text.is_empty() {
            if let Some(e) = self.edit.as_mut() {
                e.marked = None;
            }
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let map = self.edit_map();
        let layout = self.edit.as_ref()?.layout.as_ref()?;
        let lh = layout.line_height();
        let start = layout.position_for_index(map.to_display(range.start)).unwrap_or(element_bounds.origin);
        let end = layout.position_for_index(map.to_display(range.end)).unwrap_or(start);
        let right = if (end.y - start.y).abs() < px(1.) { end.x } else { start.x + px(1.) };
        Some(Bounds::from_corners(start, point(right.max(start.x + px(1.)), start.y + lh)))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let ix = self.char_index_for_point(p)?;
        Some(to_utf16(self.edit_text(), ix))
    }
}
