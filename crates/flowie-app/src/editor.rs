//! Text input for the single active editor: IME (marked text), caret and
//! selection painting. The text itself is laid out by GPUI's `StyledText`;
//! an overlay canvas reads that layout to place the caret.

use std::ops::Range;

use gpui::{
    canvas, div, fill, point, prelude::*, px, size, AnyElement, Bounds, Context, ElementInputHandler,
    EntityInputHandler, Hsla, Pixels, Point, StyledText, TextRun, UTF16Selection, Window,
};

use crate::outline::Outline;

fn to_utf16(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].encode_utf16().count()
}

fn from_utf16(text: &str, offset: usize) -> usize {
    let mut utf16 = 0;
    for (i, ch) in text.char_indices() {
        if utf16 >= offset {
            return i;
        }
        utf16 += ch.len_utf16();
    }
    text.len()
}

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
    pub fn editor_element(&self, text: String, runs: Vec<TextRun>, caret_color: Hsla, sel_color: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let Some(edit) = self.edit.as_ref() else { return div().into_any_element() };
        let (shown, runs) = if text.is_empty() {
            let mut run = runs.first().cloned().unwrap_or_else(|| TextRun {
                len: 0,
                font: gpui::font(".SystemUIFont"),
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
        let sel = edit.sel.clone();
        let caret = edit.caret();
        let marked = edit.marked.clone();
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
                        let pos = |ix: usize| layout.position_for_index(ix.min(text_len.max(0)));
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

    pub fn index_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let layout = self.edit.as_ref()?.layout.as_ref()?;
        let ix = match layout.index_for_position(p) {
            Ok(i) | Err(i) => i,
        };
        Some(ix.min(self.edit_text().len()))
    }
}

fn paint_selection(a: Point<Pixels>, b: Point<Pixels>, lh: Pixels, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
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
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(e.marked.clone())
            .unwrap_or(e.sel.clone());
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
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(e.marked.clone())
            .unwrap_or(e.sel.clone());
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

    fn bounds_for_range(&mut self, range_utf16: Range<usize>, element_bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let layout = self.edit.as_ref()?.layout.as_ref()?;
        let lh = layout.line_height();
        let start = layout.position_for_index(range.start).unwrap_or(element_bounds.origin);
        let end = layout.position_for_index(range.end).unwrap_or(start);
        let right = if (end.y - start.y).abs() < px(1.) { end.x } else { start.x + px(1.) };
        Some(Bounds::from_corners(start, point(right.max(start.x + px(1.)), start.y + lh)))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let ix = self.index_for_point(p)?;
        Some(to_utf16(self.edit_text(), ix))
    }
}
