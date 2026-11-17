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
}

fn rgb(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

impl Theme {
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
        }
    }
}
