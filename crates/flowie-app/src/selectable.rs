//! Read-only text that can be selected with the mouse and copied (⌘C),
//! for labels such as the About panel's paths and version.

use std::ops::Range;

use gpui::{
    canvas, div, prelude::*, App, ClipboardItem, Context, FocusHandle, Focusable, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent,
    SharedString, StyledText, TextLayout, Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::outline::{Copy, SelectAll};

pub struct SelectableText {
    focus: FocusHandle,
    text: SharedString,
    sel: Range<usize>,
    anchor: usize,
    dragging: bool,
    layout: Option<TextLayout>,
    pub selection_color: Hsla,
}

impl SelectableText {
    pub fn new(text: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        SelectableText {
            focus: cx.focus_handle(),
            text: text.into(),
            sel: 0..0,
            anchor: 0,
            dragging: false,
            layout: None,
            selection_color: gpui::blue().opacity(0.25),
        }
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>) {
        let text = text.into();
        if text != self.text {
            self.text = text;
            self.sel = 0..0;
        }
    }

    fn index_at(&self, p: gpui::Point<gpui::Pixels>) -> usize {
        match self.layout.as_ref().map(|l| crate::editor::caret_index_for_position(l, p)) {
            Some(Ok(i)) | Some(Err(i)) => i.min(self.text.len()),
            None => 0,
        }
    }

    fn word_at(&self, ix: usize) -> Range<usize> {
        for (i, w) in self.text.split_word_bound_indices() {
            if ix >= i && ix <= i + w.len() && !w.trim().is_empty() {
                return i..i + w.len();
            }
        }
        ix..ix
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.sel.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text[self.sel.clone()].to_string()));
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.sel = 0..self.text.len();
        cx.notify();
    }
}

impl Focusable for SelectableText {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SelectableText {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let styled = StyledText::new(self.text.clone());
        let layout = styled.layout().clone();
        let sel = self.sel.clone();
        let focused = self.focus.is_focused(window);
        let color = if focused { self.selection_color } else { self.selection_color.opacity(0.5) };
        let entity = cx.entity();
        let len = self.text.len();
        div()
            .key_context("Selectable")
            .track_focus(&self.focus)
            .w_full()
            .relative()
            .cursor_text()
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus);
                    let ix = this.index_at(ev.position);
                    if ev.click_count == 2 {
                        this.sel = this.word_at(ix);
                    } else if ev.click_count >= 3 {
                        this.sel = 0..this.text.len();
                    } else if ev.modifiers.shift {
                        this.sel = this.anchor.min(ix)..this.anchor.max(ix);
                    } else {
                        this.anchor = ix;
                        this.sel = ix..ix;
                        this.dragging = true;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.dragging && ev.pressed_button == Some(MouseButton::Left) {
                    let ix = this.index_at(ev.position);
                    this.sel = this.anchor.min(ix)..this.anchor.max(ix);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = false))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = false))
            .child(styled)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        if !sel.is_empty() {
                            let lh = layout.line_height();
                            if let (Some(a), Some(b)) =
                                (layout.position_for_index(sel.start.min(len)), layout.position_for_index(sel.end.min(len)))
                            {
                                crate::editor::paint_selection(a, b, lh, bounds, color, window);
                            }
                        }
                        entity.update(cx, |this, _| this.layout = Some(layout));
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }
}
