//! Rendering of the outline window.

use flowie_core::{markup, BlockKind, NodeId, ROOT};
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
    /// Mouse handling for the active editor: click to place the caret,
    /// drag / shift-click to select, double-click a word, triple-click all.
    pub fn editor_mouse(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .w_full()
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    if let Some(ix) = this.index_for_point(ev.position) {
                        let text = this.edit_text().to_string();
                        if let Some(e) = this.edit.as_mut() {
                            if ev.click_count >= 2 {
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
    }

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
        let kind = self.tree.node(id).kind;
        let node = self.tree.node(self.tree.source(id));
        let raw = if note { node.note.clone() } else { node.text.clone() };
        let completed = node.completed;
        let (mut font, mut color) = self.base_style(window, completed);
        let heading = !note && matches!(kind, BlockKind::H1 | BlockKind::H2 | BlockKind::H3);
        if bold_title || heading {
            font.weight = FontWeight::BOLD;
        }
        if note {
            color = self.theme.muted;
        } else if kind == BlockKind::Quote && !completed {
            color = self.theme.muted;
        }
        let target = if note { Target::Note(id) } else { Target::Text(id) };
        let editing = self.edit.as_ref().is_some_and(|e| e.target == target);
        let terms = self.highlight_terms.clone();
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
                    .children(formulas.into_iter().map(|(src, display)| math(src, display).font_size(px(FONT_SIZE * MATH_SCALE)).color(theme.text)))
            });
            return self
                .editor_mouse(cx)
                .child(editor)
                .children(preview)
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
                                    Click::Node(n) => {
                                        if this.tree.contains(*n) {
                                            return this.zoom_to(*n, cx);
                                        }
                                        this.toast("That bullet was deleted (see Trash)");
                                        return cx.notify();
                                    }
                                    Click::Date(date) => return this.search_for(&format!("date:{date}"), cx),
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

    fn render_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let links = if self.zoom != ROOT { self.tree.backlinks(self.zoom) } else { Vec::new() };
        let mirrors = if self.zoom != ROOT { self.tree.mirrors_of(self.tree.source(self.zoom)) } else { Vec::new() };
        let mut section = Vec::new();
        for (title, ids) in [("Linked from", links), ("Mirrored in", mirrors)] {
            if ids.is_empty() {
                continue;
            }
            section.push(
                div()
                    .pt_4()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted)
                    .child(format!("{title} · {}", ids.len()))
                    .into_any_element(),
            );
            for id in ids {
                let label = crate::palette::node_label(self, id);
                let target = if title == "Mirrored in" { self.tree.parent(id) } else { id };
                let path = crate::palette::node_label(self, self.tree.parent(id));
                section.push(
                    div()
                        .id(("backlink", id as usize))
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(26.))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .hover(|s| s.text_color(theme.link))
                        .child(crate::icons::slot(if title == "Mirrored in" { "copy" } else { "link" }, crate::icons::SM, theme.faint))
                        .child(div().overflow_hidden().whitespace_nowrap().child(label))
                        .child(div().text_color(theme.faint).whitespace_nowrap().child(format!("in {path}")))
                        .on_click(cx.listener(move |this, _, _, cx| this.zoom_to(target, cx)))
                        .into_any_element(),
                );
            }
        }
        div()
            .id("footer")
            .w_full()
            .min_h(px(240.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(PAD_X + 24.))
                    .children(section),
            )
            .child(div().flex_1().min_h(px(200.)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.append_child_and_edit(cx);
                }),
            )
            .into_any_element()
    }

    /// The marker left of a bullet's text: dot, checkbox, number, …
    fn render_marker(&mut self, id: NodeId, line_h: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let node = self.tree.node(id);
        let kind = node.kind;
        let is_mirror = node.mirror != ROOT;
        let src = self.tree.node(self.tree.source(id));
        let completed = src.completed;
        let has_kids = self.tree.has_children(self.tree.source(id));
        let collapsed = node.collapsed;
        let label: String = markup::plain_text(&src.text).chars().take(60).collect();
        let base = div()
            .id(("bullet", id as usize))
            .flex_none()
            .w(px(18.))
            .h(px(line_h))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer();
        let inner = match kind {
            BlockKind::Todo => {
                return base
                    .child(
                        div()
                            .size(px(14.))
                            .rounded(px(3.5))
                            .border_1()
                            .border_color(if completed { theme.accent } else { theme.muted })
                            .when(completed, |d| d.bg(theme.accent))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(completed, |d| d.child(crate::icons::icon("check", 11., theme.bg))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_checkbox(id, cx)))
                    .on_drag(DragNode { id, label }, move |drag, _, _, cx| cx.new(|_| DragPreview { label: drag.label.clone().into(), theme }))
                    .into_any_element();
            }
            BlockKind::Numbered => {
                let parent = self.tree.parent(id);
                let sibs = self.tree.children(parent);
                let ix = sibs.iter().position(|s| *s == id).unwrap_or(0);
                let mut n = 1;
                for s in sibs[..ix].iter().rev() {
                    if self.tree.node(*s).kind == BlockKind::Numbered {
                        n += 1;
                    } else {
                        break;
                    }
                }
                div().text_size(px(13.)).text_color(theme.muted).child(format!("{n}.")).into_any_element()
            }
            BlockKind::Paragraph => div()
                .size(px(5.))
                .rounded_full()
                .bg(theme.faint)
                .opacity(0.)
                .group_hover("row", |s| s.opacity(1.))
                .into_any_element(),
            _ => div()
                .size(px(17.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(collapsed && has_kids, |d| d.bg(theme.bullet_ring))
                .when(is_mirror, |d| d.border_1().border_color(theme.accent.opacity(0.7)))
                .hover(|s| s.bg(theme.bullet_ring))
                .child(div().size(px(6.)).rounded_full().bg(if completed { theme.faint } else { theme.bullet }))
                .into_any_element(),
        };
        base.child(inner)
            .on_click(cx.listener(move |this, _, _, cx| this.zoom_to(id, cx)))
            .on_drag(DragNode { id, label }, move |drag, _, _, cx| cx.new(|_| DragPreview { label: drag.label.clone().into(), theme }))
            .into_any_element()
    }

    /// A deleted row shrinking out (no interaction).
    fn render_ghost(&mut self, ix: usize) -> AnyElement {
        let d = self.display[ix];
        let real = self.tree.resolve(d.row.id);
        let Some(node) = self.ghosts.get(&real) else { return div().into_any_element() };
        let theme = self.theme;
        let p = 1.0 - ease_out(self.anims.t(d.row.id, Channel::Leave).unwrap_or(1.0));
        let left = PAD_X + d.row.depth as f32 * INDENT;
        let est_h = LINE_H * (1.0 + (node.text.len() as f32 / 90.0).floor()) + 6.0;
        div()
            .w_full()
            .opacity(p)
            .max_h(px(est_h * p))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_start()
                    .pl(px(left))
                    .pr(px(PAD_X))
                    .py(px(3.))
                    .child(div().w(px(18.)).h(px(LINE_H)).flex().items_center().justify_center().child(div().size(px(6.)).rounded_full().bg(theme.faint)))
                    .child(div().pl(px(6.)).text_color(theme.muted).child(markup::plain_text(&node.text))),
            )
            .into_any_element()
    }

    fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if ix >= self.display.len() {
            return self.render_footer(cx);
        }
        let d = self.display[ix];
        let id = d.row.id;
        let Some(node) = self.tree.get(id) else { return self.render_ghost(ix) };
        let theme = self.theme;
        let depth = d.row.depth as f32;
        let kind = node.kind;
        let collapsed = node.collapsed;
        let src_id = self.tree.source(id);
        let has_kids = self.tree.has_children(src_id);
        let src = self.tree.node(src_id);
        let completed = src.completed;
        let has_note = !src.note.is_empty();
        let text_len = src.text.len();
        let editing_note = self.edit.as_ref().is_some_and(|e| e.target == Target::Note(id));
        let selected = self.block.is_some() && self.block_ids().contains(&id);
        let (font_size, line_h) = match kind {
            BlockKind::H1 => (22.0, 32.0),
            BlockKind::H2 => (18.5, 28.0),
            BlockKind::H3 => (16.5, 26.0),
            BlockKind::Code => (13.0, 21.0),
            _ => (FONT_SIZE, LINE_H),
        };
        let top_pad = match kind {
            BlockKind::H1 => 8.0,
            BlockKind::H2 => 6.0,
            BlockKind::H3 => 4.0,
            _ => 3.0,
        };

        // Row grow/shrink animation.
        let mut p = 1.0;
        if d.leaving {
            p = 1.0 - ease_out(self.anims.t(id, Channel::Leave).unwrap_or(1.0));
        } else if let Some(t) = self.anims.t(id, Channel::Enter) {
            p = ease_out(t);
        }
        let est_lines = 1.0 + (text_len as f32 / 90.0).floor();
        let est_h = line_h * est_lines + top_pad + 3.0 + if has_note { 20.0 } else { 0.0 };

        // Completed fade.
        let done_p = self.anims.t(id, Channel::Complete).map(ease_out).unwrap_or(1.0);
        let text_opacity = if completed { 1.0 - 0.45 * done_p } else { 0.55 + 0.45 * done_p };

        // Chevron rotation (0 = pointing right / collapsed, 90° = expanded).
        let fold_p = self.anims.t(id, Channel::Fold).map(ease_in_out).unwrap_or(1.0);
        let angle = if collapsed { 90.0 * (1.0 - fold_p) } else { 90.0 * fold_p };

        let left = PAD_X + depth * INDENT;
        let text_el = self.render_text(id, false, window, cx);
        let note_el = (has_note || editing_note).then(|| self.render_text(id, true, window, cx));
        let marker = self.render_marker(id, line_h, cx);

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

        let chevron = div()
            .id(("chevron", id as usize))
            .absolute()
            .left(px(left - 20.))
            .top(px(top_pad))
            .w(px(18.))
            .h(px(line_h))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .opacity(0.)
            .when(has_kids, |d| d.group_hover("row", |s| s.opacity(1.)))
            .child(
                svg()
                    .path(crate::icons::path("chevron-right"))
                    .size(px(10.))
                    .text_color(theme.muted)
                    .with_transformation(Transformation::rotate(radians(angle.to_radians()))),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_fold(id, cx);
            }));

        let content = div()
            .flex_1()
            .min_w_0()
            .pl(px(6.))
            .opacity(text_opacity)
            .flex()
            .flex_col()
            .text_size(px(font_size))
            .line_height(px(line_h))
            .when(kind == BlockKind::Quote, |d| d.border_l_2().border_color(theme.quote_bar).pl(px(10.)))
            .when(kind == BlockKind::Code, |d| d.bg(theme.code_bg).rounded_md().px_2().py(px(4.)))
            .child(text_el)
            .children(note_el.map(|n| div().text_size(px(13.)).line_height(px(20.)).child(n)));

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
                    .pt(px(top_pad))
                    .pb(px(3.))
                    .when(selected, |d| d.bg(theme.row_selected))
                    .when_some(self.anims.t(id, Channel::Flash), |d, t| d.bg(theme.accent.opacity(0.18 * (1.0 - ease_in_out(t)))))
                    .child(chevron)
                    .child(marker)
                    .child(content),
            )
            .children(drop_line)
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                // Dragging a text selection onto another row selects whole bullets.
                if ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                if let Some(e) = this.edit.as_ref() {
                    if e.selecting_with_mouse {
                        if let Target::Text(from) = e.target {
                            if from != id && from != this.zoom && this.row_index(from).is_some() {
                                this.block = Some(BlockSel { anchor: from, head: id });
                                this.edit = None;
                                this.mouse_block = true;
                                cx.notify();
                            }
                        }
                    }
                } else if this.mouse_block {
                    if let Some(b) = this.block.as_mut() {
                        if b.head != id {
                            b.head = id;
                            cx.notify();
                        }
                    }
                }
            }))
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
                    .when(i + 1 < n || zoomed, |d| d.child(crate::icons::icon("chevron-right", crate::icons::SM, theme.faint)))
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
            let ed = self.editor_element(q, vec![run], theme.caret, theme.selection, cx);
            self.editor_mouse(cx).child(ed).into_any_element()
        } else if self.query.is_empty() {
            let find_keys = crate::keymap::keys_for("find", &self.config.keys).into_iter().next().unwrap_or_default();
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_color(theme.faint)
                .child("Search")
                .child(crate::icons::key_chip(&find_keys, theme.faint, theme.input_bg, theme.input_bg))
                .into_any_element()
        } else {
            div().text_color(theme.text).child(self.query.clone()).into_any_element()
        };
        div()
            .id("search")
            .w(px(240.))
            .h(px(28.))
            .flex()
            .items_center()
            .gap_2()
            .pl_2()
            .pr_2()
            .rounded_md()
            .bg(theme.input_bg)
            .text_size(px(13.))
            .cursor_text()
            .when(editing, |d| d.border_1().border_color(theme.accent.opacity(0.5)))
            .when(!editing, |d| d.border_1().border_color(gpui::transparent_black()))
            .child(crate::icons::icon("search", crate::icons::SM, theme.muted))
            .child(div().flex_1().min_w_0().child(inner))
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
        // While settings are open, only global shortcuts apply (so keys can be recorded).
        if self.settings.is_none() {
            kc.add("Outline");
            if self.edit.is_some() {
                kc.add("editing");
            }
            if self.block.is_some() {
                kc.add("selecting");
            }
        }

        let title = window_title(self);
        window.set_window_title(&title);

        let zoom_p = self.zoom_tween.map(|t| ease_out(t.t())).unwrap_or(1.0);
        let starred = self.zoom != ROOT && self.starred.contains(&self.zoom);
        // Toolbar buttons: a 28px square with a 16px icon.
        let tool = |id: &'static str, icon: &'static str, color: gpui::Hsla| {
            div()
                .id(id)
                .flex_none()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .cursor_pointer()
                .hover(|s| s.bg(theme.hover))
                .child(crate::icons::icon(icon, crate::icons::MD, color))
        };
        let saved = !self.query.trim().is_empty() && self.saved_searches.iter().any(|q| q == self.query.trim());
        let tools = div()
            .flex()
            .items_center()
            .gap_1()
            .when(self.zoom != ROOT, |d| {
                d.child(
                    tool("star", "star", if starred { theme.colors[2] } else { theme.muted })
                        .on_click(cx.listener(|this, _, _, cx| {
                            let z = this.zoom;
                            this.star(z, cx);
                        })),
                )
            })
            .when(!self.query.trim().is_empty(), |d| {
                d.child(
                    tool("save-search", if saved { "bookmark-check" } else { "bookmark" }, if saved { theme.accent } else { theme.muted })
                        .on_click(cx.listener(|this, _, window, cx| this.save_search(&SaveSearch, window, cx))),
                )
            })
            .child(tool("calendar", "calendar", theme.muted).on_click(cx.listener(|this, _, window, cx| this.show_calendar(&ShowCalendar, window, cx))))
            .child(div().w(px(6.)))
            .child(self.render_search(window, cx));
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(PAD_X + 40.))
            .pr(px(PAD_X))
            .pt(px(10.))
            .pb(px(6.))
            .child(self.render_breadcrumbs(cx))
            .child(tools);
        let title_el = if self.trash.is_some() { None } else { self.render_title(window, cx) };

        let entity = cx.entity();
        let body: AnyElement = if self.trash.is_some() {
            self.render_trash(cx)
        } else {
            list(self.list.clone(), move |ix, window, cx| entity.update(cx, |this, cx| this.render_row(ix, window, cx)))
                .size_full()
                .into_any_element()
        };
        let palette = self.render_palette(window, cx);
        let settings = self.render_settings(window, cx);
        let inline = self.render_inline(cx);
        let calendar = self.render_calendar(cx);
        let toast = self.toast.as_ref().filter(|(_, t)| !t.done()).map(|(msg, t)| {
            let tt = t.t();
            let o = if tt < 0.08 { tt / 0.08 } else if tt > 0.85 { (1.0 - tt) / 0.15 } else { 1.0 };
            div()
                .absolute()
                .bottom(px(22.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px_3()
                        .py(px(6.))
                        .rounded_lg()
                        .bg(theme.text)
                        .text_color(theme.bg)
                        .text_size(px(13.))
                        .shadow_md()
                        .opacity(o)
                        .child(msg.clone()),
                )
        });

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
            .on_action(cx.listener(Self::jump))
            .on_action(cx.listener(Self::move_to_action))
            .on_action(cx.listener(Self::link_to_bullet))
            .on_action(cx.listener(Self::mirror_bullet))
            .on_action(cx.listener(Self::toggle_star))
            .on_action(cx.listener(Self::expand_all))
            .on_action(cx.listener(Self::level1))
            .on_action(cx.listener(Self::level2))
            .on_action(cx.listener(Self::level3))
            .on_action(cx.listener(Self::level4))
            .on_action(cx.listener(Self::import))
            .on_action(cx.listener(Self::export_opml))
            .on_action(cx.listener(Self::export_markdown))
            .on_action(cx.listener(Self::export_text))
            .on_action(cx.listener(Self::show_trash))
            .on_action(cx.listener(Self::save_search))
            .on_action(cx.listener(Self::highlight))
            .on_action(cx.listener(Self::kind_bullet))
            .on_action(cx.listener(Self::kind_h1))
            .on_action(cx.listener(Self::kind_h2))
            .on_action(cx.listener(Self::kind_h3))
            .on_action(cx.listener(Self::kind_paragraph))
            .on_action(cx.listener(Self::kind_todo))
            .on_action(cx.listener(Self::kind_numbered))
            .on_action(cx.listener(Self::kind_quote))
            .on_action(cx.listener(Self::kind_code))
            .on_action(cx.listener(Self::today))
            .on_action(cx.listener(Self::next_match))
            .on_action(cx.listener(Self::prev_match))
            .on_action(cx.listener(Self::show_calendar))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::open_about))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                if this.settings.is_some() {
                    this.settings_key(ev, window, cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| {
                this.mouse_block = false;
                if let Some(e) = this.edit.as_mut() {
                    e.selecting_with_mouse = false;
                }
            }))
            .relative()
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
            .children(inline)
            .children(calendar)
            .children(palette)
            .children(settings)
            .children(toast)
    }
}

impl Outline {
    fn render_trash(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let entries = self.trash.clone().unwrap_or_default();
        let now = flowie_core::tree::now_millis();
        let ago = |t: i64| {
            let m = (now - t) / 60_000;
            if m < 1 {
                "just now".to_string()
            } else if m < 60 {
                format!("{m} min ago")
            } else if m < 60 * 24 {
                format!("{} h ago", m / 60)
            } else {
                format!("{} days ago", m / 60 / 24)
            }
        };
        let rows = entries.into_iter().map(|e| {
            let seq = e.seq;
            let count = e.nodes.len();
            div()
                .flex()
                .items_center()
                .gap_3()
                .py(px(6.))
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(if e.label.is_empty() { "Untitled".to_string() } else { e.label.clone() }))
                .child(div().text_size(px(12.)).text_color(theme.muted).whitespace_nowrap().child(format!(
                    "{} bullet{} · {}",
                    count,
                    if count == 1 { "" } else { "s" },
                    ago(e.deleted_at)
                )))
                .child(
                    div()
                        .id(("restore", seq as usize))
                        .px_2()
                        .rounded_md()
                        .cursor_pointer()
                        .text_color(theme.link)
                        .hover(|s| s.bg(theme.hover))
                        .flex()
                        .items_center()
                        .gap_1()
                        .h(px(26.))
                        .child(crate::icons::icon("rotate-ccw", crate::icons::SM, theme.link))
                        .child("Restore")
                        .on_click(cx.listener(move |this, _, _, cx| this.restore_trash(seq, cx))),
                )
                .child(
                    div()
                        .id(("purge", seq as usize))
                        .px_2()
                        .rounded_md()
                        .cursor_pointer()
                        .text_color(theme.muted)
                        .hover(|s| s.bg(theme.hover).text_color(theme.colors[0]))
                        .flex()
                        .items_center()
                        .gap_1()
                        .h(px(26.))
                        .child(crate::icons::icon("trash-2", crate::icons::SM, theme.muted))
                        .child("Delete forever")
                        .on_click(cx.listener(move |this, _, _, cx| this.purge_trash(seq, cx))),
                )
        });
        let empty = self.trash.as_ref().is_some_and(|t| t.is_empty());
        div()
            .id("trash")
            .size_full()
            .overflow_y_scroll()
            .px(px(PAD_X + 24.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_2()
                    .pb_3()
                    .child(div().text_size(px(TITLE_SIZE)).font_weight(FontWeight::BOLD).child("Trash"))
                    .child(
                        div()
                            .id("trash-back")
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(theme.muted)
                            .hover(|s| s.bg(theme.hover))
                            .flex()
                            .items_center()
                            .gap_1()
                            .h(px(26.))
                            .child(crate::icons::icon("arrow-left", crate::icons::SM, theme.muted))
                            .child("Back")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.trash = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().text_size(px(12.)).text_color(theme.muted).pb_2().child("Deleted bullets are kept for 30 days."))
            .when(empty, |d| d.child(div().py_4().text_color(theme.muted).child("Trash is empty.")))
            .children(rows)
            .into_any_element()
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
