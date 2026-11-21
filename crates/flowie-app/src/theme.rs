//! Colors, shadows, radii and fonts. Values follow the macOS system palette
//! (labels, separators, fills and system colors), as gpui-macos does; the
//! controls built from them live in `crate::ui`.

use gpui::{hsla, point, px, BoxShadow, Hsla, Pixels, WindowAppearance};

/// The UI font (San Francisco) and the code font.
pub const UI_FONT: &str = ".SystemUIFont";
pub const MONO_FONT: &str = "Menlo";

/// Corner radii: key caps and checkboxes; rows, buttons and fields; sheets'
/// inner cards; menus, popovers and sheets.
pub const R_SM: f32 = 4.;
pub const R_MD: f32 = 6.;
pub const R_LG: f32 = 10.;
pub const R_XL: f32 = 12.;

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub bg: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub bullet: Hsla,
    pub bullet_ring: Hsla,
    pub guide: Hsla,
    pub accent: Hsla,
    /// Text and icons drawn on an accent (or danger) fill.
    pub on_accent: Hsla,
    pub caret: Hsla,
    pub selection: Hsla,
    pub row_selected: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
    /// Track of segmented controls and fill of push buttons.
    pub control_fill: Hsla,
    /// The selected segment of a segmented control.
    pub control_selected: Hsla,
    pub code_bg: Hsla,
    pub tag: Hsla,
    pub tag_bg: Hsla,
    pub link: Hsla,
    pub search_hit: Hsla,
    pub border: Hsla,
    pub divider: Hsla,
    /// Inset fields (search) and key caps.
    pub input_bg: Hsla,
    pub marker: Hsla,
    /// Text colors for `{red}…{/}` etc., in `markup::COLORS` order.
    pub colors: [Hsla; 7],
    pub danger: Hsla,
    pub star: Hsla,
    pub mark: Hsla,
    pub quote_bar: Hsla,
    pub date: Hsla,
    pub date_bg: Hsla,
    /// Dims the window behind a sheet.
    pub scrim: Hsla,
}

fn rgb(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

fn black(a: f32) -> Hsla {
    hsla(0., 0., 0., a)
}

fn white(a: f32) -> Hsla {
    hsla(0., 0., 1., a)
}

/// Accent choices, the macOS accent colors: (name, light hex, dark hex).
pub const ACCENTS: [(&str, u32, u32); 6] = [
    ("Blue", 0x007aff, 0x0a84ff),
    ("Purple", 0xaf52de, 0xbf5af2),
    ("Green", 0x34c759, 0x30d158),
    ("Orange", 0xff9500, 0xff9f0a),
    ("Pink", 0xff2d55, 0xff375f),
    ("Graphite", 0x8e8e93, 0x98989d),
];

impl Theme {
    /// Theme for the window's appearance and the user's settings.
    pub fn build(a: WindowAppearance, config: &crate::settings::Config) -> Theme {
        use crate::settings::ThemeMode;
        let dark = match config.theme {
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
            ThemeMode::System => match std::env::var("FLOWIE_THEME").as_deref() {
                Ok("dark") => true,
                Ok("light") => false,
                _ => matches!(a, WindowAppearance::Dark | WindowAppearance::VibrantDark),
            },
        };
        let mut t = if dark { Theme::dark() } else { Theme::light() };
        if let Some((_, l, d)) = ACCENTS.iter().find(|(n, _, _)| *n == config.accent) {
            t.set_accent(rgb(if dark { *d } else { *l }));
        }
        t
    }

    fn set_accent(&mut self, c: Hsla) {
        self.accent = c;
        self.caret = c;
        self.link = c;
        self.selection = c.opacity(if self.dark { 0.45 } else { 0.3 });
        self.row_selected = c.opacity(if self.dark { 0.25 } else { 0.12 });
    }

    pub fn light() -> Theme {
        let bg = rgb(0xffffff);
        let mut t = Theme {
            dark: false,
            bg,
            text: black(216. / 255.),
            muted: black(127. / 255.),
            faint: black(0.26),
            bullet: black(0.62),
            bullet_ring: black(0.11),
            guide: black(0.07),
            accent: rgb(ACCENTS[0].1),
            on_accent: white(1.),
            caret: rgb(ACCENTS[0].1),
            selection: black(0.),
            row_selected: black(0.),
            hover: black(0.07),
            pressed: black(0.12),
            control_fill: bg.blend(black(0.11)),
            control_selected: bg,
            code_bg: rgb(0xf2f2f2),
            tag: black(127. / 255.),
            tag_bg: black(0.06),
            link: rgb(ACCENTS[0].1),
            search_hit: hsla(48. / 360., 1.0, 0.55, 0.45),
            border: black(0.12),
            divider: black(25. / 255.),
            input_bg: rgb(0xf7f7f7),
            marker: black(0.26),
            // Red, orange, yellow, green, blue, purple, gray. Orange and yellow are
            // darkened to read as text on white, but kept apart in hue (about 30°
            // and 45°) so neither turns brown next to red.
            colors: [rgb(0xff3b30), rgb(0xdc6f00), rgb(0xc28f00), rgb(0x248a3d), rgb(0x007aff), rgb(0xaf52de), rgb(0x8e8e93)],
            danger: rgb(0xff3b30),
            star: rgb(0xf4b400),
            mark: hsla(50. / 360., 1.0, 0.6, 0.45),
            quote_bar: black(0.15),
            date: rgb(0x0f766e),
            date_bg: rgb(0xe6f4f1),
            scrim: black(0.18),
        };
        t.set_accent(t.accent);
        t
    }

    pub fn dark() -> Theme {
        let bg = rgb(0x1e1e1e);
        let mut t = Theme {
            dark: true,
            bg,
            text: white(216. / 255.),
            muted: white(140. / 255.),
            faint: white(0.25),
            bullet: white(0.7),
            bullet_ring: white(0.13),
            guide: white(0.08),
            accent: rgb(ACCENTS[0].2),
            on_accent: white(1.),
            caret: rgb(ACCENTS[0].2),
            selection: black(0.),
            row_selected: black(0.),
            hover: white(0.07),
            pressed: white(0.12),
            control_fill: bg.blend(white(0.11)),
            control_selected: bg.blend(white(0.27)),
            code_bg: rgb(0x2a2a2a),
            tag: white(140. / 255.),
            tag_bg: white(0.08),
            link: rgb(ACCENTS[0].2),
            search_hit: hsla(48. / 360., 0.9, 0.5, 0.35),
            border: white(0.13),
            divider: white(25. / 255.),
            input_bg: rgb(0x262626),
            marker: white(0.25),
            colors: [rgb(0xff453a), rgb(0xff9f0a), rgb(0xffd60a), rgb(0x30d158), rgb(0x0a84ff), rgb(0xbf5af2), rgb(0x98989d)],
            danger: rgb(0xff453a),
            star: rgb(0xffd60a),
            mark: hsla(50. / 360., 0.9, 0.5, 0.35),
            quote_bar: white(0.2),
            date: rgb(0x5eead4),
            date_bg: rgb(0x16302d),
            scrim: black(0.4),
        };
        t.set_accent(t.accent);
        t
    }

    /// Menus, popovers, sheets and toasts.
    pub fn popover_shadow(&self) -> Vec<BoxShadow> {
        vec![shadow(0., 8., 28., black(if self.dark { 0.4 } else { 0.16 })), shadow(0., 1., 4., black(0.09))]
    }

    /// The selected segment of a segmented control.
    pub fn control_shadow(&self) -> Vec<BoxShadow> {
        vec![shadow(0., 1., 2., black(if self.dark { 0.2 } else { 0.08 }))]
    }

    /// Keyboard focus around a field.
    pub fn focus_ring(&self) -> Vec<BoxShadow> {
        vec![BoxShadow { spread_radius: px(3.), ..shadow(0., 0., 0., self.accent.opacity(0.35)) }]
    }
}

fn shadow(x: f32, y: f32, blur: f32, color: Hsla) -> BoxShadow {
    BoxShadow { color, offset: point(px(x), px(y)), blur_radius: px(blur), spread_radius: Pixels::ZERO, inset: false }
}
