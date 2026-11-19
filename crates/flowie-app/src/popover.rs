//! The one floating popover of the outline view (who links here, hidden path
//! levels). It replaces GPUI hover tooltips, which only open after a fixed
//! 500 ms hover, ignore clicks, snapshot their content when they open and
//! can't nest (a tooltip inside a tooltip replaces it).
//!
//! An anchor opens it on press at once (pinned until an outside press, Esc,
//! a page change or a scroll) or on hover after a short delay (closes again
//! shortly after the pointer leaves both the anchor and the popover). Its
//! content is rendered from live data every frame.

use std::time::Duration;

use flowie_core::markup;
use flowie_core::tree::{NodeId, ROOT};
use gpui::{
    anchored, deferred, div, point, prelude::*, px, AnyElement, Context, MouseButton, MouseDownEvent, Pixels, Point, Stateful, Task, Window,
};

use crate::i18n::t;
use crate::outline::Outline;

const HOVER_OPEN: Duration = Duration::from_millis(280);
const HOVER_CLOSE: Duration = Duration::from_millis(220);

#[derive(Clone, Debug, PartialEq)]
pub enum PopoverKind {
    /// Bullets linking to this node.
    References(NodeId),
    /// Path levels to list (hidden by an ellipsis, or a truncated label).
    Path(Vec<NodeId>),
}

struct Open {
    kind: PopoverKind,
    at: Point<Pixels>,
    pinned: bool,
    over_anchor: bool,
    over_popup: bool,
}

#[derive(Default)]
pub struct PopoverState {
    open: Option<Open>,
    /// Pending hover-open or hover-close; dropping it cancels.
    timer: Option<Task<()>>,
    /// The press that just closed a pinned popover from outside, so the same
    /// press on its own anchor toggles it off instead of reopening it.
    closed_by: Option<(PopoverKind, Point<Pixels>)>,
}

impl PopoverState {
    /// Close; true if something was open.
    pub fn close(&mut self) -> bool {
        self.timer = None;
        self.open.take().is_some()
    }

    pub fn is_open(&self, kind: &PopoverKind) -> bool {
        self.open.as_ref().is_some_and(|o| &o.kind == kind)
    }
}

impl Outline {
    /// Make `el` the anchor of a `kind` popover: hovering opens it after a
    /// short delay; with `press`, a press opens (pins) it at once, and a
    /// second press closes it.
    pub fn popover_anchor(&self, el: Stateful<gpui::Div>, kind: PopoverKind, press: bool, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let el = if press {
            let kind = kind.clone();
            el.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let p = &mut this.popover;
                    let toggled_off = p.closed_by.take().is_some_and(|(k, at)| k == kind && at == ev.position);
                    let pinned_here = p.open.as_ref().is_some_and(|o| o.kind == kind && o.pinned);
                    p.timer = None;
                    p.open = (!toggled_off && !pinned_here).then(|| Open {
                        kind: kind.clone(),
                        at: ev.position,
                        pinned: true,
                        over_anchor: true,
                        over_popup: false,
                    });
                    cx.notify();
                }),
            )
        } else {
            el
        };
        el.on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
            let p = &mut this.popover;
            if *hovered {
                if let Some(o) = p.open.as_mut().filter(|o| o.kind == kind) {
                    o.over_anchor = true;
                    p.timer = None;
                    return;
                }
                if p.open.as_ref().is_some_and(|o| o.pinned) {
                    return; // never replace a pinned popover by hovering
                }
                let (kind, at) = (kind.clone(), window.mouse_position());
                p.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(HOVER_OPEN).await;
                    this.update(cx, |this, cx| {
                        this.popover.timer = None;
                        this.popover_scroll = gpui::ScrollHandle::new();
                        this.popover.open = Some(Open { kind, at, pinned: false, over_anchor: true, over_popup: false });
                        cx.notify();
                    })
                    .ok();
                }));
            } else if let Some(o) = p.open.as_mut().filter(|o| o.kind == kind) {
                o.over_anchor = false;
                this.schedule_popover_close(cx);
            } else if p.open.is_none() {
                p.timer = None; // left before the hover delay ran out
            }
        }))
    }

    fn schedule_popover_close(&mut self, cx: &mut Context<Self>) {
        if self.popover.open.as_ref().is_none_or(|o| o.pinned) {
            return;
        }
        self.popover.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_CLOSE).await;
            this.update(cx, |this, cx| {
                let p = &mut this.popover;
                p.timer = None;
                if p.open.as_ref().is_some_and(|o| !o.pinned && !o.over_anchor && !o.over_popup) {
                    p.open = None;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The popover layer, drawn above everything else.
    pub fn render_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let o = self.popover.open.as_ref()?;
        let (kind, at) = (o.kind.clone(), o.at);
        let theme = self.theme;
        let body: Vec<AnyElement> = match &kind {
            PopoverKind::References(id) => {
                let ids: Vec<NodeId> = self.tree.links_to(*id).collect();
                if ids.is_empty() {
                    return None;
                }
                let mut out =
                    vec![div().pb_2().px_2().text_color(theme.muted).child(t!("outline.link_count", count = ids.len())).into_any_element()];
                for src in ids {
                    let (text, note) = self.plain_text_and_note(src);
                    let path = self.render_path(src, 392., false, window, cx);
                    out.push(
                        crate::ui::list::hover_row(theme, ("popover-ref", src as usize))
                            .py_2()
                            .px_2()
                            .child(text)
                            .when(!note.is_empty(), |d| d.child(div().text_color(theme.muted).child(note)))
                            .child(path)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| this.zoom_to(src, cx)))
                            .into_any_element(),
                    );
                }
                out
            }
            PopoverKind::Path(ids) => ids
                .iter()
                .filter(|id| **id == ROOT || self.tree.contains(**id))
                .map(|&id| {
                    crate::ui::list::hover_row(theme, ("popover-crumb", id as usize))
                        .px_2()
                        .py_1()
                        .hover(|s| s.text_color(theme.link))
                        .child(self.crumb_label(id))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| this.zoom_to(id, cx)))
                        .into_any_element()
                })
                .collect(),
        };
        let width = px(440.).min(window.viewport_size().width - px(32.));
        let card = crate::ui::surface::popover(theme, "popover")
            .w(width)
            .p_1()
            .line_height(px(20.))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if let Some(o) = this.popover.open.as_mut() {
                    o.over_popup = *hovered;
                    if *hovered {
                        this.popover.timer = None;
                    } else {
                        this.schedule_popover_close(cx);
                    }
                }
            }))
            .on_mouse_down_out(cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                if let Some(o) = this.popover.open.take() {
                    this.popover.timer = None;
                    this.popover.closed_by = Some((o.kind, ev.position));
                    cx.notify();
                }
            }))
            .child(crate::ui::scrollbar::area(
                "popover-bar",
                &self.popover_scroll,
                theme,
                div().id("popover-scroll").max_h(px(340.)).overflow_y_scroll().track_scroll(&self.popover_scroll).p_1().children(body),
            ));
        Some(
            deferred(anchored().position(at + point(px(-10.), px(14.))).snap_to_window_with_margin(px(8.)).child(card))
                .with_priority(2)
                .into_any_element(),
        )
    }

    /// A node's text and note as plain text ("Untitled" for an empty text).
    pub fn plain_text_and_note(&self, id: NodeId) -> (String, String) {
        let src = self.tree.node(self.tree.source(id));
        let text = markup::plain_text(&src.text);
        (if text.trim().is_empty() { t!("common.untitled").into() } else { text }, markup::plain_text(&src.note))
    }

    /// One path level's label: "Home", or the node's plain text on one line.
    pub fn crumb_label(&self, id: NodeId) -> String {
        if id == ROOT {
            return t!("common.home").into();
        }
        let text = markup::plain_text(&self.tree.node(self.tree.source(id)).text).replace(['\n', '\r'], " ");
        if text.trim().is_empty() {
            t!("common.untitled").into()
        } else {
            text
        }
    }
}
