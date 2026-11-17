//! Rendering of the outline window.

use flowie_core::{markup, NodeId, ROOT};
use flowie_math::math;
use gpui::{
    div, list, prelude::*, px, radians, relative, svg, AnyElement, Context, FontWeight, KeyContext, MouseButton,
    MouseDownEvent, MouseMoveEvent, SharedString, StyledText, Transformation, Window,
};

use crate::anim::{self, ease_in_out, ease_out, Channel};
use crate::outline::*;
use crate::rich::{self, Click, Piece, Style};

pub const FONT_SIZE: f32 = 15.0;
pub const LINE_H: f32 = 24.0;
pub const TITLE_SIZE: f32 = 26.0;
/// KaTeX glyphs look small next to the UI font at the same size.
pub const MATH_SCALE: f32 = 1.1;

struct DragPreview {
    label: SharedString,
    theme: crate::theme::Theme,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(self.theme.bg)
            .border_1()
            .border_color(self.theme.border)
            .shadow_md()
            .text_color(self.theme.text)
            .text_size(px(FONT_SIZE))
            .child(self.label.clone())
    }
}

impl Outline {
    fn base_style(&self, window: &Window, completed: bool) -> (gpui::Font, gpui::Hsla) {
        let font = window.text_style().font();
        let color = if completed { self.theme.muted } else { self.theme.text };
        (font, color)
    }

    /// Text of a node, as the editor (when active) or as rendered rich text.
    fn render_text(
        &mut self,
        id: NodeId,
        note: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bold_title = !note && id == self.zoom && id != ROOT;
        let node = self.tree.node(id);
        let raw = if note { node.note.clone() } else { node.text.clone() };
        let completed = node.completed;
        let (mut font, mut color) = self.base_style(window, completed);
        if bold_title {
            font.weight = FontWeight::BOLD;
        }
        if note {
            color = self.theme.muted;
        }
        let target = if note { Target::Note(id) } else { Target::Text(id) };
        let editing = self.edit.as_ref().is_some_and(|e| e.target == target);
        let terms = self.highlight_terms.clone();
        let style = Style { theme: &self.theme, font, color, completed: completed && !note, highlights: &terms };

        if editing {
            let pieces = rich::build(&raw, true, &style);
            let (text, runs) = match pieces.into_iter().next() {
                Some(Piece::Text { text, runs, .. }) => (text, runs),
                _ => (String::new(), Vec::new()),
            };
            let editor = self.editor_element(text, runs, self.theme.caret, self.theme.selection, cx);
            return div()
                .w_full()
                .cursor_text()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        if let Some(ix) = this.index_for_point(ev.position) {
                            let text = this.edit_text().to_string();
                            if let Some(e) = this.edit.as_mut() {
                                if ev.click_count >= 2 {
                                    // Double-click selects a word, triple selects all.
                                    e.sel = if ev.click_count == 2 { word_range(&text, ix) } else { 0..text.len() };
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
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                    let dragging = this.edit.as_ref().is_some_and(|e| e.selecting_with_mouse);
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
                .child(editor)
                .into_any_element();
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
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            if ev.modifiers.shift {
                                this.shift_click(id, cx);
                            } else {
                                this.start_edit(target, 0, cx);
                            }
                            cx.stop_propagation();
                        }),
                    )
                    .into_any_element();
            };
            let styled = StyledText::new(text).with_runs(runs);
            let layout = styled.layout().clone();
            return div()
                .w_full()
                .cursor_text()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if ev.modifiers.shift {
                            this.shift_click(id, cx);
                            return;
                        }
                        let disp = match layout.index_for_position(ev.position) {
                            Ok(i) => Some(i),
                            Err(i) => {
                                let _ = i;
                                None
                            }
                        };
                        if let Some(d) = disp {
                            if let Some((_, c)) = clicks.iter().find(|(r, _)| r.contains(&d)) {
                                match c {
                                    Click::Tag(t) => return this.search_for(t, cx),
                                    Click::Link(u) => return cx.open_url(u),
                                }
                            }
                        }
                        let d = match layout.index_for_position(ev.position) {
                            Ok(i) | Err(i) => i,
                        };
                        let raw_ix = rich::display_to_raw(&map, d).min(raw_len);
                        this.start_edit(target, raw_ix, cx);
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
                Piece::Text { text, runs, .. } => {
                    for tok in rich::tokens(&text) {
                        let t = text[tok.clone()].to_string();
                        let r = rich::slice_runs(&runs, tok);
                        children.push(div().h(px(LINE_H)).child(StyledText::new(t).with_runs(r).into_any_element()).into_any_element());
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
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if ev.modifiers.shift {
                        this.shift_click(id, cx);
                    } else {
                        this.start_edit(target, raw_len, cx);
                    }
                }),
            )
            .children(children)
            .into_any_element()
    }

    fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if ix >= self.display.len() {
            // Footer: clicking below the last bullet adds one.
            return div()
                .id("footer")
                .w_full()
                .h(px(240.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.append_child_and_edit(cx);
                    }),
                )
                .into_any_element();
        }
        let d = self.display[ix];
        let id = d.row.id;
        let Some(node) = self.tree.get(id) else { return div().into_any_element() };
        let theme = self.theme;
        let depth = d.row.depth as f32;
        let has_kids = self.tree.has_children(id);
        let collapsed = node.collapsed;
        let completed = node.completed;
        let has_note = !node.note.is_empty();
        let text_len = node.text.len();
        let label: String = markup::plain_text(&node.text).chars().take(60).collect();
        let editing_note = self.edit.as_ref().is_some_and(|e| e.target == Target::Note(id));
        let selected = self.block.is_some() && self.block_ids().contains(&id);

        // Row grow/shrink animation.
        let mut p = 1.0;
        if d.leaving {
            p = 1.0 - ease_out(self.anims.t(id, Channel::Leave).unwrap_or(1.0));
        } else if let Some(t) = self.anims.t(id, Channel::Enter) {
            p = ease_out(t);
        }
        let est_lines = 1.0 + (text_len as f32 / 90.0).floor();
        let est_h = LINE_H * est_lines + 6.0 + if has_note { 20.0 } else { 0.0 };

        // Completed fade.
        let done_p = self.anims.t(id, Channel::Complete).map(ease_out).unwrap_or(1.0);
        let text_opacity = if completed { 1.0 - 0.45 * done_p } else { 0.55 + 0.45 * done_p };

        // Chevron rotation (0 = pointing right / collapsed, 90° = expanded).
        let fold_p = self.anims.t(id, Channel::Fold).map(ease_in_out).unwrap_or(1.0);
        let angle = if collapsed { 90.0 * (1.0 - fold_p) } else { 90.0 * fold_p };

        let left = PAD_X + depth * INDENT;
        let text_el = self.render_text(id, false, window, cx);
        let note_el = (has_note || editing_note).then(|| self.render_text(id, true, window, cx));

        let mut guides = Vec::new();
        for k in 0..d.row.depth {
            guides.push(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(PAD_X + k as f32 * INDENT + 8.5))
                    .w(px(1.))
                    .bg(theme.guide),
            );
        }

        let drop_line = self.drop.filter(|t| t.node == id).map(|t| {
            let el = div()
                .absolute()
                .left(px(PAD_X + t.depth as f32 * INDENT))
                .right(px(PAD_X))
                .h(px(2.))
                .rounded_full()
                .bg(theme.accent);
            if t.below {
                el.bottom(px(-1.))
            } else {
                el.top(px(-1.))
            }
        });

        let bullet = div()
            .id(("bullet", id as usize))
            .flex_none()
            .w(px(18.))
            .h(px(LINE_H))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(
                div()
                    .size(px(17.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(collapsed && has_kids, |d| d.bg(theme.bullet_ring))
                    .hover(|s| s.bg(theme.bullet_ring))
                    .child(div().size(px(6.)).rounded_full().bg(if completed { theme.faint } else { theme.bullet })),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.zoom_to(id, cx)))
            .on_drag(DragNode { id, label: label.clone() }, move |drag, _, _, cx| {
                cx.new(|_| DragPreview { label: drag.label.clone().into(), theme })
            });

        let chevron = div()
            .id(("chevron", id as usize))
            .absolute()
            .left(px(left - 20.))
            .top_0()
            .w(px(18.))
            .h(px(LINE_H))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .opacity(0.)
            .when(has_kids, |d| d.group_hover("row", |s| s.opacity(1.)))
            .when(has_kids && collapsed, |d| d.opacity(0.0))
            .child(
                svg()
                    .path("icons/chevron.svg")
                    .size(px(10.))
                    .text_color(theme.muted)
                    .with_transformation(Transformation::rotate(radians(angle.to_radians()))),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_fold(id, cx);
            }));

        let row = div()
            .id(("row", id as usize))
            .group("row")
            .relative()
            .w_full()
            .children(guides)
            .child(
                div()
                    .relative()
                    .flex()
                    .items_start()
                    .pl(px(left))
                    .pr(px(PAD_X))
                    .py(px(3.))
                    .when(selected, |d| d.bg(theme.row_selected))
                    .child(chevron)
                    .child(bullet)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl(px(6.))
                            .opacity(text_opacity)
                            .flex()
                            .flex_col()
                            .child(text_el)
                            .children(note_el.map(|n| div().text_size(px(13.)).line_height(px(20.)).child(n))),
                    ),
            )
            .children(drop_line)
            .on_drag_move(cx.listener(move |this, ev: &gpui::DragMoveEvent<DragNode>, _, cx| {
                if ev.bounds.contains(&ev.event.position) {
                    let dragged = ev.drag(cx).id;
                    this.update_drop(id, ev.bounds, ev.event.position, dragged, cx);
                }
            }));

        let row = if p < 1.0 {
            row.opacity(p).max_h(px(est_h * p)).overflow_hidden()
        } else {
            row
        };
        row.into_any_element()
    }

    fn render_breadcrumbs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let mut crumbs: Vec<(NodeId, String)> = vec![(ROOT, "Home".into())];
        if self.zoom != ROOT {
            for a in self.tree.ancestors(self.zoom) {
                let t = markup::plain_text(&self.tree.node(a).text);
                let t: String = t.chars().take(28).collect();
                crumbs.push((a, if t.is_empty() { "Untitled".into() } else { t }));
            }
        }
        let n = crumbs.len();
        let zoomed = self.zoom != ROOT;
        div().flex().items_center().gap_1().text_size(px(13.)).text_color(theme.muted).children(
            crumbs.into_iter().enumerate().map(move |(i, (id, label))| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id(("crumb", id as usize))
                            .px_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.hover).text_color(theme.text))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| this.zoom_to(id, cx))),
                    )
                    .when(i + 1 < n || zoomed, |d| d.child(div().text_color(theme.faint).child("›")))
            }),
        )
    }

    fn render_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let editing = self.edit.as_ref().is_some_and(|e| e.target == Target::Search);
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
            self.editor_element(q, vec![run], theme.caret, theme.selection, cx)
        } else if self.query.is_empty() {
            div().text_color(theme.faint).child("Search   ⌘F").into_any_element()
        } else {
            div().text_color(theme.text).child(self.query.clone()).into_any_element()
        };
        div()
            .id("search")
            .w(px(240.))
            .px_3()
            .py(px(4.))
            .rounded_md()
            .bg(theme.input_bg)
            .text_size(px(13.))
            .cursor_text()
            .when(editing, |d| d.border_1().border_color(theme.accent.opacity(0.5)))
            .when(!editing, |d| d.border_1().border_color(gpui::transparent_black()))
            .child(inner)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.edit.as_ref().map(|e| e.target) != Some(Target::Search) {
                        let len = this.query.len();
                        this.start_edit(Target::Search, len, cx);
                    }
                }),
            )
            .into_any_element()
    }

    fn render_title(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.zoom == ROOT {
            return None;
        }
        let id = self.zoom;
        let text = self.render_text(id, false, window, cx);
        let note = (!self.tree.node(id).note.is_empty() || self.edit.as_ref().is_some_and(|e| e.target == Target::Note(id)))
            .then(|| self.render_text(id, true, window, cx));
        Some(
            div()
                .px(px(PAD_X + 24.))
                .pt_2()
                .pb_3()
                .child(
                    div()
                        .text_size(px(TITLE_SIZE))
                        .line_height(px(TITLE_SIZE * 1.35))
                        .font_weight(FontWeight::BOLD)
                        .child(text),
                )
                .children(note.map(|n| div().pt_1().text_size(px(14.)).child(n)))
                .into_any_element(),
        )
    }
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

impl Render for Outline {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.tick();
        let theme = self.theme;

        let mut kc = KeyContext::new_with_defaults();
        kc.add("Outline");
        if self.edit.is_some() {
            kc.add("editing");
        }
        if self.block.is_some() {
            kc.add("selecting");
        }

        let title = window_title(self);
        window.set_window_title(&title);

        let zoom_p = self.zoom_tween.map(|t| ease_out(t.t())).unwrap_or(1.0);
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(PAD_X + 40.))
            .pr(px(PAD_X))
            .pt(px(10.))
            .pb(px(6.))
            .child(self.render_breadcrumbs(cx))
            .child(self.render_search(window, cx));
        let title_el = self.render_title(window, cx);

        let entity = cx.entity();
        let body = list(self.list.clone(), move |ix, window, cx| entity.update(cx, |this, cx| this.render_row(ix, window, cx)))
            .size_full();

        let empty_hint = (self.rows.is_empty() && !self.query.is_empty()).then(|| {
            div().px(px(PAD_X + 24.)).py_4().text_color(theme.muted).child(format!("No bullets match “{}”", self.query))
        });

        if self.is_animating() {
            window.request_animation_frame();
        }

        div()
            .id("outline")
            .key_context(kc)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .font_family(".SystemUIFont")
            .text_size(px(FONT_SIZE))
            .line_height(px(LINE_H))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::toggle_note))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::toggle_complete))
            .on_action(cx.listener(Self::collapse))
            .on_action(cx.listener(Self::expand))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_home))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::delete_node))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::bold))
            .on_action(cx.listener(Self::italic))
            .on_action(cx.listener(Self::inline_code))
            .on_action(cx.listener(Self::wrap_math))
            .on_action(cx.listener(Self::insert_link))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(Self::toggle_completed_visible))
            .child(header)
            .child(
                div()
                    .id("page")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .opacity(zoom_p)
                    .mt(px((1.0 - zoom_p) * 14.0))
                    .children(title_el)
                    .children(empty_hint)
                    .child(div().flex_1().min_h_0().child(body))
                    .on_drop(cx.listener(|this, drag: &DragNode, _, cx| this.perform_drop(drag.id, cx))),
            )
    }
}

fn window_title(o: &Outline) -> String {
    if o.zoom == ROOT {
        "Flowie".into()
    } else {
        let t = markup::plain_text(&o.tree.node(o.zoom).text);
        if t.is_empty() {
            "Untitled — Flowie".into()
        } else {
            format!("{t} — Flowie")
        }
    }
}

#[allow(dead_code)]
fn unused() {
    let _ = relative(1.);
    let _ = anim::ROW_IN;
}
