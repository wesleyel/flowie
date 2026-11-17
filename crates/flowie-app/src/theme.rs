use gpui::{hsla, Hsla, WindowAppearance};

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub bullet: Hsla,
    pub bullet_ring: Hsla,
    pub guide: Hsla,
    pub accent: Hsla,
    pub caret: Hsla,
    pub selection: Hsla,
    pub row_selected: Hsla,
    pub hover: Hsla,
    pub code_bg: Hsla,
    pub tag: Hsla,
    pub tag_bg: Hsla,
    pub link: Hsla,
    pub search_hit: Hsla,
    pub border: Hsla,
    pub input_bg: Hsla,
    pub marker: Hsla,
    /// Text colors for `{red}…{/}` etc., in `markup::COLORS` order.
    pub colors: [Hsla; 7],
    pub mark: Hsla,
    pub quote_bar: Hsla,
    pub date: Hsla,
    pub date_bg: Hsla,
}

fn rgb(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Accent choices: (name, light hex, dark hex).
pub const ACCENTS: [(&str, u32, u32); 6] = [
    ("Blue", 0x3b82f6, 0x60a5fa),
    ("Purple", 0x8b5cf6, 0xa78bfa),
    ("Green", 0x10b981, 0x34d399),
    ("Orange", 0xf97316, 0xfb923c),
    ("Pink", 0xec4899, 0xf472b6),
    ("Graphite", 0x6b7280, 0x9ca3af),
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
            let c: Hsla = gpui::rgb(if dark { *d } else { *l }).into();
            t.accent = c;
            t.caret = c;
            t.link = c;
            t.selection = c.opacity(if dark { 0.32 } else { 0.25 });
            t.row_selected = c.opacity(if dark { 0.16 } else { 0.12 });
        }
        t
    }

    #[allow(dead_code)]
    pub fn for_appearance(a: WindowAppearance) -> Theme {
        match std::env::var("FLOWIE_THEME").as_deref() {
            Ok("dark") => return Theme::dark(),
            Ok("light") => return Theme::light(),
            _ => {}
        }
        match a {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Theme::dark(),
            _ => Theme::light(),
        }
    }

    pub fn light() -> Theme {
        Theme {
            bg: rgb(0xffffff),
            text: rgb(0x2a3037),
            muted: rgb(0x868c94),
            faint: rgb(0xb7bcc2),
            bullet: rgb(0x4b5159),
            bullet_ring: rgb(0xdfe2e6),
            guide: rgb(0xeceef0),
            accent: rgb(0x3b82f6),
            caret: rgb(0x3b82f6),
            selection: hsla(217. / 360., 0.9, 0.6, 0.25),
            row_selected: hsla(217. / 360., 0.9, 0.6, 0.12),
            hover: hsla(0., 0., 0., 0.035),
            code_bg: rgb(0xf2f3f5),
            tag: rgb(0x6b7280),
            tag_bg: rgb(0xeef0f3),
            link: rgb(0x2f6fd8),
            search_hit: hsla(48. / 360., 1.0, 0.55, 0.45),
            border: rgb(0xe8eaed),
            input_bg: rgb(0xf4f5f7),
            marker: rgb(0xc3c7cd),
            colors: [rgb(0xdc2626), rgb(0xea580c), rgb(0xca8a04), rgb(0x16a34a), rgb(0x2563eb), rgb(0x9333ea), rgb(0x6b7280)],
            mark: hsla(50. / 360., 1.0, 0.6, 0.45),
            quote_bar: rgb(0xd5d9de),
            date: rgb(0x0f766e),
            date_bg: rgb(0xe6f4f1),
        }
    }

    pub fn dark() -> Theme {
        Theme {
            bg: rgb(0x1c1e22),
            text: rgb(0xe3e5e8),
            muted: rgb(0x8d939b),
            faint: rgb(0x5b6068),
            bullet: rgb(0xc9cdd2),
            bullet_ring: rgb(0x3a3e45),
            guide: rgb(0x2c2f35),
            accent: rgb(0x60a5fa),
            caret: rgb(0x60a5fa),
            selection: hsla(217. / 360., 0.9, 0.6, 0.32),
            row_selected: hsla(217. / 360., 0.9, 0.6, 0.16),
            hover: hsla(0., 0., 1., 0.04),
            code_bg: rgb(0x2a2d33),
            tag: rgb(0xa3a9b1),
            tag_bg: rgb(0x2c3036),
            link: rgb(0x7cb0ff),
            search_hit: hsla(48. / 360., 0.9, 0.5, 0.35),
            border: rgb(0x2c2f35),
            input_bg: rgb(0x26292e),
            marker: rgb(0x5b6068),
            colors: [rgb(0xf87171), rgb(0xfb923c), rgb(0xfacc15), rgb(0x4ade80), rgb(0x60a5fa), rgb(0xc084fc), rgb(0x9ca3af)],
            mark: hsla(50. / 360., 0.9, 0.5, 0.35),
            quote_bar: rgb(0x454a52),
            date: rgb(0x5eead4),
            date_bg: rgb(0x16302d),
        }
    }
}
