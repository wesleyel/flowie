//! The window's size and place, kept between launches: the display it was on, its frame there,
//! and whether it was full screen. Saved as one meta value, `window`.

use gpui::{point, px, size, App, Bounds, DisplayId, Pixels, Size, Window, WindowBounds};

#[derive(Clone, PartialEq)]
pub struct Place {
    /// The display's UUID: display ids can change between launches, UUIDs don't.
    display: String,
    fullscreen: bool,
    /// Top-left corner on that display and the content size (without the title bar), which is
    /// what opening a window takes.
    rect: Bounds<Pixels>,
}

impl Place {
    pub fn parse(value: &str) -> Option<Place> {
        let mut it = value.split(' ');
        let display = it.next()?.to_string();
        let fullscreen = it.next()? == "fullscreen";
        let n: Vec<f32> = it.map(|x| x.parse().ok()).collect::<Option<_>>()?;
        let [x, y, w, h] = n[..] else { return None };
        Some(Place { display, fullscreen, rect: Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) } })
    }

    pub fn encode(&self) -> String {
        let r = self.rect;
        format!(
            "{} {} {} {} {} {}",
            self.display,
            if self.fullscreen { "fullscreen" } else { "windowed" },
            f32::from(r.origin.x),
            f32::from(r.origin.y),
            f32::from(r.size.width),
            f32::from(r.size.height)
        )
    }

    /// The window's place now. In full screen (or maximized) only that is noted: the frame stays
    /// the last windowed one in `prev`, to return to when leaving full screen next launch.
    pub fn observe(prev: Option<&Place>, window: &Window, cx: &App) -> Option<Place> {
        let display = window.display(cx)?.uuid().ok()?.to_string();
        match window.window_bounds() {
            WindowBounds::Windowed(b) => {
                Some(Place { display, fullscreen: false, rect: Bounds { origin: b.origin, size: window.viewport_size() } })
            }
            WindowBounds::Maximized(_) | WindowBounds::Fullscreen(_) => prev.map(|p| Place { display, fullscreen: true, rect: p.rect }),
        }
    }

    /// Where to open the window: this place, if its display is still attached, moved and shrunk
    /// as needed to lie wholly on that display. `None` (open at the default place) when the
    /// display is gone.
    pub fn open_at(&self, min: Size<Pixels>, cx: &App) -> Option<(DisplayId, WindowBounds)> {
        let display = cx.displays().into_iter().find(|d| d.uuid().is_ok_and(|u| u.to_string() == self.display))?;
        let area = display.bounds();
        let r = self.rect;
        let w = r.size.width.max(min.width).min(area.size.width);
        let h = r.size.height.max(min.height).min(area.size.height);
        let x = r.origin.x.max(area.origin.x).min(area.origin.x + area.size.width - w);
        let y = r.origin.y.max(area.origin.y).min(area.origin.y + area.size.height - h);
        let b = Bounds { origin: point(x, y), size: size(w, h) };
        Some((display.id(), if self.fullscreen { WindowBounds::Fullscreen(b) } else { WindowBounds::Windowed(b) }))
    }
}
