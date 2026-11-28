//! macOS overlay scrollbars: a thin thumb at the right edge that shows while
//! the content scrolls or the pointer is over it, then fades. It widens on
//! hover, and it can be dragged or clicked to jump.
//!
//! Put `scrollbar(..)` as the last child of a `relative()` box that has the
//! scrolling area's size (not inside the scrolling element itself).

use std::time::{Duration, Instant};

use gpui::{
    canvas, fill, point, prelude::*, px, size, Bounds, DispatchPhase, ElementId, Hsla, ListState, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, ScrollHandle,
};

use crate::ui::theme::Theme;

/// A scroll position the caller keeps itself (the outline's height map), for content
/// whose full height the scrolling element can't know.
pub trait ScrollModel {
    /// How far the content is scrolled, ≥ 0.
    fn top(&self) -> Pixels;
    /// How far it can scroll.
    fn max(&self) -> Pixels;
    /// Scroll to `top` (clamped to 0…max by the caller).
    fn set_top(&self, top: Pixels);
    /// A thumb drag starts or ends.
    fn drag(&self, _on: bool) {}
}

/// What scrolls: a scrolling div, a `list`, or a model of either.
#[derive(Clone)]
pub enum Source {
    Handle(ScrollHandle),
    List(ListState),
    Model(std::rc::Rc<dyn ScrollModel>),
}

impl From<std::rc::Rc<dyn ScrollModel>> for Source {
    fn from(m: std::rc::Rc<dyn ScrollModel>) -> Self {
        Source::Model(m)
    }
}

impl From<&ScrollHandle> for Source {
    fn from(h: &ScrollHandle) -> Self {
        Source::Handle(h.clone())
    }
}

impl From<&ListState> for Source {
    fn from(l: &ListState) -> Self {
        Source::List(l.clone())
    }
}

impl Source {
    /// How far the content is scrolled, ≥ 0.
    fn top(&self) -> Pixels {
        match self {
            Source::Handle(h) => -h.offset().y,
            Source::List(l) => -l.scroll_px_offset_for_scrollbar().y,
            Source::Model(m) => m.top(),
        }
    }

    fn max(&self) -> Pixels {
        match self {
            Source::Handle(h) => h.max_offset().y,
            Source::List(l) => l.max_offset_for_scrollbar().y,
            Source::Model(m) => m.max(),
        }
    }

    fn set_top(&self, top: Pixels) {
        let max = self.max();
        let top = top.clamp(Pixels::ZERO, max);
        match self {
            Source::Handle(h) => h.set_offset(point(h.offset().x, -top)),
            // A list knows only the heights of the rows it has drawn and estimates the
            // rest, and drawing the rows it scrolls to corrects the total. So the thumb's
            // place is kept as a fraction of the current total (not pixels of the total
            // when the drag began), and its ends are the page's ends: the end is laid out
            // back from the last row, so it is reached however tall the rows turn out.
            Source::List(l) if top >= max - px(0.5) => l.scroll_to_end(),
            Source::List(l) if top <= Pixels::ZERO => l.scroll_to(gpui::ListOffset { item_ix: 0, offset_in_item: Pixels::ZERO }),
            Source::List(l) => l.set_offset_from_scrollbar(point(Pixels::ZERO, -top)),
            Source::Model(m) => m.set_top(top),
        }
    }

    fn drag(&self, on: bool) {
        if let Source::Model(m) = self {
            m.drag(on);
        }
    }
}

/// Width of the strip that takes the pointer.
const STRIP: f32 = 14.;
const INSET: f32 = 3.;
const THIN: f32 = 6.;
const WIDE: f32 = 9.;
const MIN_THUMB: f32 = 24.;
/// How long the thumb stays after scrolling, then how long it fades.
const LINGER: Duration = Duration::from_millis(900);
const FADE: Duration = Duration::from_millis(300);

#[derive(Default)]
struct State {
    last_top: Option<Pixels>,
    shown_at: Option<Instant>,
    hovered: bool,
    /// While dragging: where in the thumb it was grabbed.
    grab: Option<Pixels>,
}

struct Frame {
    state: gpui::Entity<State>,
    /// Thumb top and height, relative to the strip; `None` when nothing overflows.
    thumb: Option<(Pixels, Pixels)>,
    track: Pixels,
    max: Pixels,
}

pub fn scrollbar(id: impl Into<ElementId>, source: impl Into<Source>, theme: Theme) -> impl IntoElement {
    let id = id.into();
    let source = source.into();
    let src = source.clone();
    canvas(
        move |bounds, window, cx| {
            let state = window.use_keyed_state(id, cx, |_, _| State::default());
            let (top, max) = (src.top(), src.max());
            state.update(cx, |s, _| {
                if s.last_top.is_some_and(|t| (t - top).abs() > px(0.5)) {
                    s.shown_at = Some(Instant::now());
                }
                s.last_top = Some(top);
            });
            let view = bounds.size.height;
            if max <= px(0.5) || view <= px(2. * INSET + MIN_THUMB) {
                return Frame { state, thumb: None, track: Pixels::ZERO, max };
            }
            let room = view - px(2. * INSET);
            let thumb_h = (room * (view / (view + max))).max(px(MIN_THUMB)).min(room);
            let track = room - thumb_h;
            let y = px(INSET) + track * (top / max).clamp(0., 1.);
            Frame { state, thumb: Some((y, thumb_h)), track, max }
        },
        move |bounds, frame, window, cx| {
            let Some((y, thumb_h)) = frame.thumb else { return };
            let state = frame.state.clone();
            let (hovered, grab, shown_at) = {
                let s = state.read(cx);
                (s.hovered, s.grab, s.shown_at)
            };
            let active = hovered || grab.is_some();
            let alpha = if active {
                1.
            } else {
                match shown_at.map(|t| t.elapsed()) {
                    Some(e) if e < LINGER => 1.,
                    Some(e) if e < LINGER + FADE => 1. - (e - LINGER).as_secs_f32() / FADE.as_secs_f32(),
                    _ => 0.,
                }
            };
            if alpha > 0. && !active {
                window.request_animation_frame();
                #[cfg(feature = "screenshots")]
                crate::dev::shots::animating();
            }
            let fg: Hsla = if theme.dark { gpui::white() } else { gpui::black() };
            if alpha > 0. {
                let w = px(if active { WIDE } else { THIN });
                if active {
                    // The track, as macOS shows it under the pointer.
                    window.paint_quad(fill(bounds, fg.opacity(0.04)));
                    window.paint_quad(fill(Bounds::new(bounds.origin, size(px(1.), bounds.size.height)), fg.opacity(0.08)));
                }
                let thumb = Bounds::new(point(bounds.right() - px(INSET) - w, bounds.top() + y), size(w, thumb_h));
                let a = if active { 0.5 } else { 0.38 };
                window.paint_quad(fill(thumb, fg.opacity(a * alpha)).corner_radii(w / 2.));
            }

            let (track, max, src) = (frame.track, frame.max, source.clone());
            // Where the thumb would be grabbed for a pointer at `py`.
            let top_for = move |py: Pixels, grab: Pixels| {
                if track <= Pixels::ZERO {
                    return Pixels::ZERO;
                }
                max * ((py - bounds.top() - px(INSET) - grab) / track)
            };

            let st = state.clone();
            let visible = alpha > 0.;
            let s2 = src.clone();
            window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left || !bounds.contains(&ev.position) {
                    return;
                }
                if !visible && !st.read(cx).hovered {
                    return;
                }
                let rel = ev.position.y - bounds.top();
                // On the thumb: keep the grab point; on the track: center the thumb there.
                let grab = if rel >= y && rel <= y + thumb_h { rel - y } else { thumb_h / 2. };
                s2.drag(true);
                s2.set_top(top_for(ev.position.y, grab));
                st.update(cx, |s, _| s.grab = Some(grab));
                cx.stop_propagation();
                window.refresh();
            });
            let st = state.clone();
            let s2 = src.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let over = bounds.contains(&ev.position);
                let (was, grab) = {
                    let s = st.read(cx);
                    (s.hovered, s.grab)
                };
                if let Some(grab) = grab {
                    if ev.pressed_button == Some(MouseButton::Left) {
                        s2.set_top(top_for(ev.position.y, grab));
                        window.refresh();
                        cx.stop_propagation();
                        return;
                    }
                }
                if over != was {
                    st.update(cx, |s, _| {
                        s.hovered = over;
                        if !over {
                            // Fade out from here, as after a scroll.
                            s.shown_at = Some(Instant::now());
                        }
                    });
                    window.refresh();
                }
            });
            let s2 = src.clone();
            window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left {
                    return;
                }
                if state.read(cx).grab.is_some() {
                    s2.drag(false);
                    state.update(cx, |s, _| {
                        s.grab = None;
                        s.shown_at = Some(Instant::now());
                    });
                    cx.stop_propagation();
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .right_0()
    .bottom_0()
    .w(px(STRIP))
}

/// A `relative` box holding `scroller` and its bar. Style the box to take
/// the scroller's place; the scroller fills it (or sets its own max height).
pub fn area(id: impl Into<ElementId>, source: impl Into<Source>, theme: Theme, scroller: impl IntoElement) -> gpui::Div {
    gpui::div().relative().child(scroller).child(scrollbar(id, source, theme))
}
