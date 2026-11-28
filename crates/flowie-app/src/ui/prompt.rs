//! Alerts (`window.prompt`) on Linux, drawn as a Flowie sheet.
//!
//! macOS and Windows show native alerts. Linux has none, and GPUI's fallback is a fixed
//! 288 px white box whose detail text doesn't wrap, so a long line (the Trash's “can't be
//! brought back” warning in English) was cut off on X11/Openbox (wiki: Linux). This builder
//! draws the same sheet as the close dialog instead: the theme's colors, wrapped text, the
//! cancel button on the left and the default action, in the accent color, on the right.

use std::sync::Mutex;

use gpui::{
    div, prelude::*, px, App, Context, FocusHandle, Focusable, FontWeight, KeyDownEvent, PromptButton, PromptHandle, PromptLevel,
    PromptResponse, RenderablePromptHandle, SharedString, Window,
};

use crate::ui::button::{button, Variant};
use crate::ui::surface::{scrim, sheet};
use crate::ui::theme::Theme;

/// The window's current theme, kept by `apply_theme`: the builder only gets `&mut App`.
static THEME: Mutex<Option<Theme>> = Mutex::new(None);

pub fn set_theme(theme: Theme) {
    if let Ok(mut current) = THEME.lock() {
        *current = Some(theme);
    }
}

fn theme() -> Theme {
    THEME.lock().ok().and_then(|t| *t).unwrap_or_else(Theme::light)
}

/// Use Flowie's sheet for every `window.prompt`. Only on Linux: elsewhere the platform's
/// own alert is better, and a custom builder would replace it.
pub fn install(cx: &mut App) {
    cx.set_prompt_builder(build);
}

fn build(
    _level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    actions: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let view = cx.new(|cx| Alert {
        message: message.to_string().into(),
        detail: detail.map(|d| d.to_string().into()),
        actions: actions.to_vec(),
        focus: cx.focus_handle(),
    });
    handle.with_view(view, window, cx)
}

struct Alert {
    message: SharedString,
    detail: Option<SharedString>,
    actions: Vec<PromptButton>,
    focus: FocusHandle,
}

impl Alert {
    /// Escape answers with the cancel button (or the first one when there is none).
    fn cancel_index(&self) -> usize {
        self.actions.iter().position(PromptButton::is_cancel).unwrap_or(0)
    }

    /// Return answers with the default button: the last one that isn't Cancel.
    fn default_index(&self) -> Option<usize> {
        self.actions.iter().rposition(|a| !a.is_cancel())
    }

    fn key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        if k.modifiers.modified() {
            return;
        }
        match k.key.as_str() {
            "escape" => cx.emit(PromptResponse(self.cancel_index())),
            "enter" => {
                if let Some(ix) = self.default_index() {
                    cx.emit(PromptResponse(ix));
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl gpui::EventEmitter<PromptResponse> for Alert {}

impl Focusable for Alert {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Alert {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme();
        let default = self.default_index();
        let button_for = |ix: usize, action: &PromptButton, cx: &mut Context<Self>| {
            let variant = if Some(ix) == default { Variant::Primary } else { Variant::Secondary };
            button(theme, ("prompt-button", ix), variant).child(action.label().clone()).on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(PromptResponse(ix));
                cx.stop_propagation();
            }))
        };
        let (cancel, others): (Vec<_>, Vec<_>) = self.actions.iter().enumerate().partition(|(_, a)| a.is_cancel());
        let left: Vec<_> = cancel.into_iter().map(|(ix, a)| button_for(ix, a, cx)).collect();
        let right: Vec<_> = others.into_iter().map(|(ix, a)| button_for(ix, a, cx)).collect();

        let body = sheet(theme, "prompt")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .w(px(400.))
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(self.message.clone()))
            .children(self.detail.clone().map(|d| div().text_size(px(13.)).text_color(theme.muted).line_height(px(19.)).child(d)))
            .child(
                div()
                    .pt_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().flex().items_center().gap_2().children(left))
                    .child(div().flex().items_center().gap_2().children(right)),
            );
        // Clicks on the scrim do nothing: an alert needs an answer.
        div().size_full().child(scrim(theme).items_center().child(body))
    }
}
