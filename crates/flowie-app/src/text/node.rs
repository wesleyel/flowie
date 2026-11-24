//! Render node text and handle pointer gestures in the editor.

use flowie_core::{markup, BlockKind, NodeId, ROOT};
use flowie_math::math;
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, StyledText, Window};

use super::{
    editor::caret_index_for_position,
    rich::{self, Click, Piece, Style},
};
use crate::app::i18n::t;
use crate::outline::{Field, Outline, Target, FONT_SIZE, LINE_H};
use crate::overlays::popover::PopoverKind;

/// KaTeX glyphs look small next to the UI font at the same size.
const MATH_SCALE: f32 = 1.1;

impl Outline {
    /// The single entry point for a left press on displayed (not edited)
    /// text: shift extends the block selection, a press on a link / tag /
    /// date acts on it, anything else starts editing at `raw_ix` with drag
    /// selection armed — so one press, drag and release selects text.
    fn press_text(
        &mut self,
        id: NodeId,
        target: Target,
        raw_ix: usize,
        click: Option<Click>,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let temporary = matches!(target, Target::Field(Field::WhatsNew(_)));
        if ev.modifiers.shift && !temporary {
            return self.shift_click(id, cx);
        }
        match click {
            Some(Click::Link(url)) if temporary && (url.starts_with("https://") || url.starts_with("http://")) => return cx.open_url(&url),
            _ if temporary => {}
            Some(Click::Tag(t)) => return self.search_for(&t, cx),
            Some(Click::Link(u)) => return cx.open_url(&u),
            Some(Click::Node(n)) => {
                if self.tree.contains(n) {
                    return self.zoom_to(n, cx);
                }
                self.toast(t!("toast.bullet_deleted"));
                return cx.notify();
            }
            Some(Click::Date(date)) => return self.search_for(&format!("date:{date}"), cx),
            None => {}
        }
        // The row is under the pointer, so it is visible: don't scroll.
        self.begin_edit(target, raw_ix, false, cx);
        window.focus(&self.focus, cx);
        let text = self.edit_text();
        let sel = match ev.click_count {
            2 => word_range(text, raw_ix),
            3.. => 0..text.len(),
            _ => raw_ix..raw_ix,
        };
        if let Some(edit) = self.edit.as_mut() {
            edit.sel = sel;
            edit.selecting_with_mouse = true;
        }
    }

    /// Mouse handling for the active editor: click to place the caret,
    /// drag / shift-click to select, double-click a word, triple-click all.
    pub fn editor_mouse(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id("editor-text")
            .w_full()
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    if let Some(ix) = this.index_for_point(ev.position) {
                        let text = this.edit_text().to_string();
                        let link = this.link_at_point(ev.position);
                        if let Some(e) = this.edit.as_mut() {
                            if ev.click_count >= 2 {
                                e.sel = match (ev.click_count, link) {
                                    (2, Some(link)) => link,
                                    (2, None) => word_range(&text, ix),
                                    _ => 0..text.len(),
                                };
                                e.reversed = false;
                            } else if ev.modifiers.shift {
                                let anchor = if e.reversed { e.sel.end } else { e.sel.start };
                                e.sel = anchor.min(ix)..anchor.max(ix);
                                e.reversed = ix < anchor;
                            } else {
                                e.sel = ix..ix;
                                e.reversed = false;
                            }
                            e.selecting_with_mouse = true;
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let text = this.edit_text().to_string();
                    let ix = this.index_for_point(ev.position);
                    if let (Some(ix), Some(e)) = (ix, this.edit.as_mut()) {
                        // Outside the selection: select the word under the pointer (as macOS does).
                        if e.sel.is_empty() || ix < e.sel.start || ix > e.sel.end {
                            e.sel = word_range(&text, ix);
                            e.reversed = false;
                        }
                        e.selecting_with_mouse = false;
                    }
                    this.open_format_menu(ev.position, cx);
                    cx.notify();
                }),
            )
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                if !*hovered {
                    this.popover_hover_at(None, window.mouse_position(), cx);
                }
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                let dragging = this.edit.as_ref().is_some_and(|e| e.selecting_with_mouse);
                let link = if dragging || this.is_auxiliary_page_open() { None } else { this.link_target_at_point(ev.position) };
                this.popover_hover_at(link.map(PopoverKind::Link), ev.position, cx);
                if dragging && ev.pressed_button == Some(MouseButton::Left) {
                    if let Some(ix) = this.index_for_point(ev.position) {
                        let e = this.edit.as_mut().unwrap();
                        let anchor = if e.reversed { e.sel.end } else { e.sel.start };
                        e.sel = anchor.min(ix)..anchor.max(ix);
                        e.reversed = ix < anchor;
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if let Some(e) = this.edit.as_mut() {
                        e.selecting_with_mouse = false;
                    }
                }),
            )
    }

    pub(crate) fn base_style(&self, window: &Window, completed: bool) -> (gpui::Font, gpui::Hsla) {
        let font = window.text_style().font();
        let color = if completed { self.theme.muted } else { self.theme.text };
        (font, color)
    }

    /// Text of a node, as the editor (when active) or as rendered rich text.
    pub(crate) fn render_text(&mut self, id: NodeId, note: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let target = if note { Target::Note(id) } else { Target::Text(id) };
        self.render_node_text(id, target, window, cx)
    }

    pub(crate) fn render_node_text(&mut self, id: NodeId, target: Target, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let temporary = matches!(target, Target::Field(Field::WhatsNew(_)));
        let tree = if temporary { &self.notes_page().unwrap().tree } else { &self.tree };
        let note = matches!(target, Target::Note(_));
        let bold_title = !note && id == self.zoom && id != ROOT;
        let kind = tree.node(id).kind;
        let node = tree.node(tree.source(id));
        let raw = if note { node.note.clone() } else { node.text.clone() };
        let completed = node.completed;
        let (mut font, mut color) = self.base_style(window, completed);
        let heading = !note && matches!(kind, BlockKind::H1 | BlockKind::H2 | BlockKind::H3);
        if bold_title || heading {
            font.weight = FontWeight::BOLD;
        }
        if note || (kind == BlockKind::Quote && !completed) {
            color = self.theme.muted;
        }
        let editing = self.edit.as_ref().is_some_and(|e| e.target == target);
        let terms = if temporary { Vec::new() } else { self.highlight_terms.clone() };
        let code = !note && kind == BlockKind::Code;
        let style = Style { theme: &self.theme, font, color, completed: completed && !note, highlights: &terms, code };

        if editing {
            let pieces = rich::build(&raw, true, &style);
            let (text, runs) = match pieces.into_iter().next() {
                Some(Piece::Text { text, runs, .. }) => (text, runs),
                _ => (String::new(), Vec::new()),
            };
            let editor = self.editor_element(text, runs, self.theme.caret, self.theme.selection, cx);
            // Live preview of the formulas in the line being edited.
            let formulas: Vec<(String, bool)> = if code {
                Vec::new()
            } else {
                markup::parse(&raw)
                    .into_iter()
                    .filter_map(|r| match r.kind {
                        markup::Kind::Math { source, display } => Some((raw[source].to_string(), display)),
                        _ => None,
                    })
                    .collect()
            };
            let preview = (!formulas.is_empty()).then(|| {
                let theme = self.theme;
                div()
                    .mt(px(2.))
                    .mb(px(2.))
                    .px_2()
                    .py(px(4.))
                    .rounded_md()
                    .bg(theme.code_bg)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .children(
                        formulas
                            .into_iter()
                            .map(|(src, display)| math(src, display).font_size(px(FONT_SIZE * MATH_SCALE)).color(theme.text)),
                    )
            });
            return self.editor_mouse(cx).child(editor).children(preview).into_any_element();
        }

        let pieces = rich::build(&raw, false, &style);
        let has_math = pieces.iter().any(|p| matches!(p, Piece::Math { .. }));
        let raw_len = raw.len();

        if !has_math {
            let Some(Piece::Text { text, runs, map, clicks }) = pieces.into_iter().next() else {
                // Empty bullet: keep the line height so it is clickable.
                return div()
                    .w_full()
                    .h(px(if note { 0. } else { LINE_H }))
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| this.press_text(id, target, 0, None, ev, window, cx)),
                    )
                    .into_any_element();
            };
            let styled = StyledText::new(text).with_runs(runs);
            let layout = styled.layout().clone();
            let hover = (layout.clone(), clicks.clone());
            return div()
                .id((if note { "note-text" } else { "row-text" }, id.as_usize()))
                .w_full()
                .cursor_text()
                .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                    let link = match hover.0.index_for_position(ev.position) {
                        Ok(d) if !temporary => link_target(&hover.1, d),
                        _ => None,
                    };
                    this.popover_hover_at(link.map(PopoverKind::Link), ev.position, cx);
                }))
                .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                    if !*hovered {
                        this.popover_hover_at(None, window.mouse_position(), cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        let (d, inside) = match layout.index_for_position(ev.position) {
                            Ok(i) => (i, true),
                            Err(i) => (i, false),
                        };
                        let click = inside.then(|| clicks.iter().find(|(r, _)| r.contains(&d)).map(|(_, c)| c.clone())).flatten();
                        let caret = match caret_index_for_position(&layout, ev.position) {
                            Ok(i) | Err(i) => i,
                        };
                        let raw_ix = rich::display_to_raw(&map, caret).min(raw_len);
                        this.press_text(id, target, raw_ix, click, ev, window, cx);
                    }),
                )
                .child(styled)
                .into_any_element();
        }

        // Mixed text and math: lay out tokens in a wrapping flex row.
        let fs = px(FONT_SIZE);
        let text_style = window.text_style();
        let font_id = window.text_system().resolve_font(&text_style.font());
        let ascent = window.text_system().ascent(font_id, fs);
        let descent = window.text_system().descent(font_id, fs);
        let base_from_bottom = (px(LINE_H) - (ascent + descent)) / 2. + descent;
        let mut children: Vec<AnyElement> = Vec::new();
        for piece in pieces {
            match piece {
                Piece::Text { text, runs, map, clicks } => {
                    let map = std::rc::Rc::new(map);
                    let clicks = std::rc::Rc::new(clicks);
                    for tok in rich::tokens(&text) {
                        let t = text[tok.clone()].to_string();
                        let start = tok.start;
                        let r = rich::slice_runs(&runs, tok);
                        let styled = StyledText::new(t).with_runs(r);
                        let layout = styled.layout().clone();
                        let (map, clicks) = (map.clone(), clicks.clone());
                        let hover = (layout.clone(), clicks.clone());
                        children.push(
                            div()
                                .id((if note { "note-token" } else { "row-token" }, id.as_usize() ^ start))
                                .h(px(LINE_H))
                                .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                                    let link = match hover.0.index_for_position(ev.position) {
                                        Ok(i) if !temporary => link_target(&hover.1, start + i),
                                        _ => None,
                                    };
                                    this.popover_hover_at(link.map(PopoverKind::Link), ev.position, cx);
                                }))
                                .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                                    if !*hovered {
                                        this.popover_hover_at(None, window.mouse_position(), cx);
                                    }
                                }))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                        let (d, inside) = match layout.index_for_position(ev.position) {
                                            Ok(i) => (start + i, true),
                                            Err(i) => (start + i, false),
                                        };
                                        let click =
                                            inside.then(|| clicks.iter().find(|(r, _)| r.contains(&d)).map(|(_, c)| c.clone())).flatten();
                                        let caret = start
                                            + match caret_index_for_position(&layout, ev.position) {
                                                Ok(i) | Err(i) => i,
                                            };
                                        let raw_ix = rich::display_to_raw(&map, caret).min(raw_len);
                                        this.press_text(id, target, raw_ix, click, ev, window, cx);
                                    }),
                                )
                                .child(styled)
                                .into_any_element(),
                        );
                    }
                }
                Piece::Math { source, display } => {
                    let layout = flowie_math::layout(&source, display);
                    let mfs = FONT_SIZE * MATH_SCALE;
                    let depth = layout.as_ref().map(|l| px(l.depth * mfs)).unwrap_or(px(0.));
                    let pad = (base_from_bottom - depth).max(px(0.));
                    let el = math(source, display).font_size(px(mfs)).color(color);
                    if display {
                        children.push(div().w_full().flex().justify_center().py_1().child(el).into_any_element());
                    } else {
                        children.push(div().pb(pad).px(px(1.)).child(el).into_any_element());
                    }
                }
            }
        }
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_end()
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| this.press_text(id, target, raw_len, None, ev, window, cx)),
            )
            .children(children)
            .into_any_element()
    }
}

/// The bullet a link at display offset `d` points to.
fn link_target(clicks: &[(std::ops::Range<usize>, Click)], d: usize) -> Option<NodeId> {
    clicks.iter().find_map(|(r, c)| match c {
        Click::Node(n) if r.contains(&d) => Some(*n),
        _ => None,
    })
}

fn word_range(text: &str, ix: usize) -> std::ops::Range<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    for (i, w) in text.split_word_bound_indices() {
        if ix >= i && ix <= i + w.len() && !w.trim().is_empty() {
            return i..i + w.len();
        }
    }
    ix..ix
}
