use std::sync::Arc;

use gpui::{
    px, size, App, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, SharedString,
    Style, TextRun, Window,
};

use crate::MathLayout;

/// A GPUI element that renders one formula. On parse errors it shows the
/// source in red so the user can see what to fix.
pub struct MathElement {
    source: SharedString,
    display: bool,
    font_size: Option<Pixels>,
    color: Option<Hsla>,
}

pub fn math(source: impl Into<SharedString>, display: bool) -> MathElement {
    MathElement { source: source.into(), display, font_size: None, color: None }
}

impl MathElement {
    pub fn font_size(mut self, size: Pixels) -> Self {
        self.font_size = Some(size);
        self
    }
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

pub enum MathState {
    Ok(Arc<MathLayout>, Pixels),
    Err(gpui::ShapedLine),
}

impl IntoElement for MathElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for MathElement {
    type RequestLayoutState = MathState;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, MathState) {
        let text_style = window.text_style();
        let fs = self.font_size.unwrap_or_else(|| text_style.font_size.to_pixels(window.rem_size()));
        let mut style = Style::default();
        let state = match crate::layout(&self.source, self.display) {
            Ok(layout) => {
                style.size = size(px(layout.width * f32::from(fs)), px(layout.total_height() * f32::from(fs))).map(Into::into);
                MathState::Ok(layout, fs)
            }
            Err(_) => {
                let run = TextRun {
                    len: self.source.len(),
                    font: text_style.font(),
                    color: gpui::red(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(self.source.clone(), fs, &[run], None);
                style.size = size(line.width, window.line_height()).map(Into::into);
                MathState::Err(line)
            }
        };
        (window.request_layout(style, [], cx), state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut MathState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut MathState,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let color = self.color.unwrap_or_else(|| window.text_style().color);
        match state {
            MathState::Ok(layout, fs) => crate::paint(layout, bounds.origin, *fs, color, window),
            MathState::Err(line) => {
                let _ = line.paint(bounds.origin, window.line_height(), gpui::TextAlign::Left, None, window, cx);
            }
        }
    }
}
