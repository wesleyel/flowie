//! Outline row layout, markers, and drag previews.

use flowie_core::{markup, BlockKind, NodeId, ROOT};
use gpui::{
    div, prelude::*, px, radians, svg, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, SharedString,
    Transformation, Window,
};

use super::structure::DragNode;
use super::{BlockSel, Outline, Target, INDENT, PAD_X};
use crate::app::i18n::t;
use crate::overlays::popover::PopoverKind;
use crate::ui::anim::{ease_in_out, ease_out, Channel};

pub const FONT_SIZE: f32 = 15.0;
pub const LINE_H: f32 = 24.0;

pub(crate) fn row_metrics(kind: BlockKind) -> (f32, f32, f32) {
    match kind {
        BlockKind::H1 => (22.0, 32.0, 8.0),
        BlockKind::H2 => (18.5, 28.0, 6.0),
        BlockKind::H3 => (16.5, 26.0, 4.0),
        BlockKind::Code => (13.0, 21.0, 3.0),
        _ => (FONT_SIZE, LINE_H, 3.0),
    }
}

pub(crate) fn bullet_row(depth: u16, top_pad: f32) -> gpui::Div {
    div().relative().flex().items_start().pl(px(PAD_X + depth as f32 * INDENT)).pr(px(PAD_X)).pt(px(top_pad)).pb(px(3.))
}

pub(crate) fn indent_guides(depth: u16, theme: crate::ui::theme::Theme) -> impl Iterator<Item = gpui::Div> {
    (0..depth).map(move |level| div().absolute().top_0().bottom_0().left(px(PAD_X + level as f32 * INDENT + 8.5)).w(px(1.)).bg(theme.guide))
}

struct DragPreview {
    label: SharedString,
    theme: crate::ui::theme::Theme,
}

pub(crate) fn bullet_dot(theme: crate::ui::theme::Theme, folded: bool, mirror: bool, completed: bool) -> gpui::Div {
    div()
        .size(px(17.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .when(folded, |dot| dot.bg(theme.bullet_ring))
        .when(mirror, |dot| dot.border_1().border_color(theme.accent.opacity(0.7)))
        .hover(|style| style.bg(theme.bullet_ring))
        .child(div().size(px(6.)).rounded_full().bg(if completed { theme.faint } else { theme.bullet }))
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .rounded(px(crate::ui::theme::R_MD))
            .bg(self.theme.bg)
            .border_1()
            .border_color(self.theme.border)
            .shadow(self.theme.popover_shadow())
            .text_color(self.theme.text)
            .text_size(px(FONT_SIZE))
            .child(self.label.clone())
    }
}

impl Outline {
    /// Link count badge (row end / page title); opens who links here.
    pub(super) fn render_reference_count(&self, id: NodeId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.tree.source(id);
        let count = self.tree.link_count(id);
        if count == 0 {
            return None;
        }
        let theme = self.theme;
        let kind = PopoverKind::References(id);
        let open = self.popover.is_open(&kind);
        let badge = div()
            .id(("reference-count", id.as_usize()))
            .flex_none()
            .ml_2()
            .px_1()
            .rounded_sm()
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(12.))
            .text_color(if open { theme.link } else { theme.muted })
            .when(open, |d| d.bg(theme.hover))
            .cursor_pointer()
            .hover(|s| s.bg(theme.hover).text_color(theme.link))
            .child(crate::ui::icons::icon("link", crate::ui::icons::SM, theme.muted))
            .child(count.to_string());
        Some(self.popover_anchor(badge, kind, true, cx).into_any_element())
    }

    pub(super) fn render_footer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let src = self.tree.source(self.zoom);
        let (links, mirrors) = if self.zoom != ROOT { (self.tree.backlinks(src), self.tree.mirrors_of(src)) } else { Default::default() };
        let mut section = Vec::new();
        for (mirror, title, ids) in [(false, t!("outline.linked_from"), links), (true, t!("outline.mirrored_in"), mirrors)] {
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
                let (label, note) = self.plain_text_and_note(id);
                let path = self.render_path(id, f32::from(window.viewport_size().width) - 156., true, window, cx);
                let target = if mirror { self.tree.parent(id) } else { id };
                section.push(
                    div()
                        .id(("backlink", id.as_usize()))
                        .flex()
                        .items_start()
                        .gap_2()
                        .py_2()
                        .w_full()
                        .cursor_pointer()
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .hover(|s| s.text_color(theme.link))
                        .child(crate::ui::icons::slot(if mirror { "copy" } else { "link" }, crate::ui::icons::SM, theme.faint))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(label)
                                .when(!note.is_empty(), |d| d.child(div().text_color(theme.muted).child(note)))
                                .child(div().text_color(theme.muted).child(path)),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
            .child(div().px(px(PAD_X + 24.)).children(section))
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
            .id(("bullet", id.as_usize()))
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
                    .child(crate::ui::controls::checkbox(theme, completed))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_checkbox(id, cx)))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_bullet_menu(id, ev.position, window, cx);
                        }),
                    )
                    .on_drag(DragNode { id, label }, move |drag, _, _, cx| {
                        cx.new(|_| DragPreview { label: drag.label.clone().into(), theme })
                    })
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
            BlockKind::Paragraph => {
                div().size(px(5.)).rounded_full().bg(theme.faint).opacity(0.).group_hover("row", |s| s.opacity(1.)).into_any_element()
            }
            _ => bullet_dot(theme, collapsed && has_kids, is_mirror, completed).into_any_element(),
        };
        // An imported root not looked at yet: an unread dot, like mail.
        let base = match self.unread(id).is_some() {
            true => self.popover_anchor(
                base.relative()
                    .child(div().absolute().top(px(line_h / 2. - 9.)).right(px(-3.)).size(px(7.)).rounded_full().bg(theme.accent)),
                crate::overlays::popover::PopoverKind::Import(id),
                false,
                cx,
            ),
            false => base,
        };
        base.child(inner)
            .on_click(cx.listener(move |this, _, _, cx| this.zoom_to(id, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_bullet_menu(id, ev.position, window, cx);
                }),
            )
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
                    .child(
                        div()
                            .w(px(18.))
                            .h(px(LINE_H))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(div().size(px(6.)).rounded_full().bg(theme.faint)),
                    )
                    .child(div().pl(px(6.)).text_color(theme.muted).child(markup::plain_text(&node.text))),
            )
            .into_any_element()
    }

    pub(super) fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if ix >= self.display.len() {
            return self.render_footer(window, cx);
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
        let selected = self.is_block_selected(id);
        // The row being edited (caret or selection) gets a light tint.
        let editing_row = self.edit.as_ref().is_some_and(|e| matches!(e.target, Target::Text(n) | Target::Note(n) if n == id));
        let (font_size, line_h, top_pad) = row_metrics(kind);

        // Row grow/shrink animation.
        let mut p = 1.0;
        if d.leaving {
            p = 1.0 - ease_out(self.anims.t(id, Channel::Leave).unwrap_or(1.0));
        } else if let Some(t) = self.anims.t(id, Channel::Enter) {
            p = ease_out(t);
        }
        let est_lines = 1.0 + (text_len as f32 / 90.0).floor();
        let image_h = self.row_image(id).map_or(0., |(_, info)| Self::image_box(info).1 + 6.);
        let est_h = line_h * est_lines + top_pad + 3.0 + if has_note { 20.0 } else { 0.0 } + image_h;

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
        let references = self.render_reference_count(id, cx);
        let picture = self.render_row_image(id, cx);

        let drop_line = self.drop.filter(|t| t.node == id).map(|t| {
            let el = div().absolute().left(px(PAD_X + t.depth as f32 * INDENT)).right(px(PAD_X)).h(px(2.)).rounded_full().bg(theme.accent);
            if t.below {
                el.bottom(px(-1.))
            } else {
                el.top(px(-1.))
            }
        });

        let chevron = div()
            .id(("chevron", id.as_usize()))
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
                    .path(crate::ui::icons::path("chevron-right"))
                    .size(px(10.))
                    .text_color(theme.muted)
                    .with_transformation(Transformation::rotate(radians(angle.to_radians()))),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_fold(id, cx);
                }),
            );

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
            .children(picture)
            .child(div().flex().items_start().child(div().flex_1().min_w_0().child(text_el)).children(references))
            .children(note_el.map(|n| div().text_size(px(13.)).line_height(px(20.)).child(n)));

        let row = div()
            .id(("row", id.as_usize()))
            .group("row")
            .relative()
            .w_full()
            .children(indent_guides(d.row.depth, theme))
            .child(
                bullet_row(d.row.depth, top_pad)
                    .when(selected, |d| d.bg(theme.row_selected))
                    .when(editing_row && !selected, |d| d.bg(theme.row_selected.opacity(0.45)))
                    .when_some(self.anims.t(id, Channel::Flash), |d, t| d.bg(theme.accent.opacity(0.18 * (1.0 - ease_in_out(t)))))
                    .child(chevron)
                    .child(marker)
                    .child(content),
            )
            .children(drop_line)
            // Image files dropped on a row go after it.
            .on_drop(cx.listener(move |this, paths: &gpui::ExternalPaths, _, cx| {
                let images: Vec<std::path::PathBuf> = paths.paths().iter().filter(|p| crate::image::is_image_path(p)).cloned().collect();
                let (parent, index) = (this.tree.parent(id), this.tree.index_in_parent(id) + 1);
                this.insert_image_files(&images, Some((parent, index)), cx);
            }))
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

        let row = if p < 1.0 { row.opacity(p).max_h(px(est_h * p)).overflow_hidden() } else { row };
        row.into_any_element()
    }
}
