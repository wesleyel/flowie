//! KaTeX-compatible math for Flowie: RaTeX lays formulas out, GPUI paints them
//! natively (KaTeX glyphs through the GPU glyph atlas, rules as quads, radicals
//! and big delimiters as paths). Layouts are cached per (source, display).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{
    point, px, App, Bounds, FontStyle, FontWeight, GlyphId, Hsla, PathBuilder, Pixels, Point,
    SharedString, TextRun, Window,
};
use ratex_types::{Color, DisplayItem, MathStyle, PathCommand};

const FONT_FILES: &[&str] = &[
    "KaTeX_AMS-Regular.ttf",
    "KaTeX_Caligraphic-Bold.ttf",
    "KaTeX_Caligraphic-Regular.ttf",
    "KaTeX_Fraktur-Bold.ttf",
    "KaTeX_Fraktur-Regular.ttf",
    "KaTeX_Main-Bold.ttf",
    "KaTeX_Main-BoldItalic.ttf",
    "KaTeX_Main-Italic.ttf",
    "KaTeX_Main-Regular.ttf",
    "KaTeX_Math-BoldItalic.ttf",
    "KaTeX_Math-Italic.ttf",
    "KaTeX_SansSerif-Bold.ttf",
    "KaTeX_SansSerif-Italic.ttf",
    "KaTeX_SansSerif-Regular.ttf",
    "KaTeX_Script-Regular.ttf",
    "KaTeX_Size1-Regular.ttf",
    "KaTeX_Size2-Regular.ttf",
    "KaTeX_Size3-Regular.ttf",
    "KaTeX_Size4-Regular.ttf",
    "KaTeX_Typewriter-Regular.ttf",
];

/// Register the embedded KaTeX fonts with GPUI. Call once at startup.
pub fn load_fonts(cx: &App) {
    let fonts = FONT_FILES
        .iter()
        .filter_map(|name| ratex_katex_fonts::ttf_bytes(name))
        .collect::<Vec<_>>();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        eprintln!("flowie-math: failed to load KaTeX fonts: {err}");
    }
}

/// A laid-out formula in em units (multiply by font size for pixels).
/// Origin is top-left; the baseline sits at `height`.
#[derive(Debug)]
pub struct MathLayout {
    pub width: f32,
    pub height: f32,
    pub depth: f32,
    items: Vec<DisplayItem>,
}

impl MathLayout {
    pub fn total_height(&self) -> f32 {
        self.height + self.depth
    }
}

type LayoutCache = Mutex<HashMap<(String, bool), Result<Arc<MathLayout>, String>>>;

fn layout_cache() -> &'static LayoutCache {
    static CACHE: OnceLock<LayoutCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Parse and lay out `source` (without `$` delimiters). Cached; cheap to call per frame.
pub fn layout(source: &str, display: bool) -> Result<Arc<MathLayout>, String> {
    let key = (source.to_string(), display);
    if let Some(hit) = layout_cache().lock().unwrap().get(&key) {
        return hit.clone();
    }
    let result = layout_uncached(source, display);
    let mut cache = layout_cache().lock().unwrap();
    if cache.len() > 4096 {
        cache.clear();
    }
    cache.insert(key, result.clone());
    result
}

fn layout_uncached(source: &str, display: bool) -> Result<Arc<MathLayout>, String> {
    let nodes = ratex_parser::parse(source).map_err(|e| e.to_string())?;
    let options = ratex_layout::LayoutOptions {
        style: if display { MathStyle::Display } else { MathStyle::Text },
        ..Default::default()
    };
    let root = ratex_layout::layout(&nodes, &options);
    ratex_layout::validate_layout(&root)?;
    let list = ratex_layout::to_display_list(&root);
    Ok(Arc::new(MathLayout {
        width: list.width as f32,
        height: list.height as f32,
        depth: list.depth as f32,
        items: list.items,
    }))
}

fn font_for(name: &str) -> gpui::Font {
    let (family, style) = name.split_once('-').unwrap_or((name, "Regular"));
    let mut font = match family {
        "CJK" | "Emoji" => gpui::font(".SystemUIFont"),
        _ => gpui::font(SharedString::from(format!("KaTeX_{family}"))),
    };
    if style.contains("Bold") {
        font.weight = FontWeight::BOLD;
    }
    if style.contains("Italic") {
        font.style = FontStyle::Italic;
    }
    font
}

#[derive(Clone)]
enum GlyphSource {
    /// Rasterized by the platform text system through GPUI's glyph atlas (crisp, hinted).
    Atlas(gpui::FontId, GlyphId),
    /// Vector outline in em units (y down, origin on the baseline). Used for KaTeX faces
    /// GPUI refuses to load (it skips fonts without an `m` glyph: AMS, Size1–4, …).
    Outline(Arc<Vec<PathCommand>>),
}

type GlyphCache = Mutex<HashMap<(String, u32), Option<GlyphSource>>>;

fn glyph_cache() -> &'static GlyphCache {
    static CACHE: OnceLock<GlyphCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn katex_face(font: &str) -> Option<ttf_parser::Face<'static>> {
    let bytes = match ratex_katex_fonts::ttf_bytes(&format!("KaTeX_{font}.ttf"))? {
        std::borrow::Cow::Borrowed(b) => b,
        std::borrow::Cow::Owned(_) => return None,
    };
    ttf_parser::Face::parse(bytes, 0).ok()
}

struct OutlineSink(Vec<PathCommand>, f64);

impl ttf_parser::OutlineBuilder for OutlineSink {
    fn move_to(&mut self, x: f32, y: f32) {
        let k = self.1;
        self.0.push(PathCommand::MoveTo { x: x as f64 * k, y: -y as f64 * k });
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let k = self.1;
        self.0.push(PathCommand::LineTo { x: x as f64 * k, y: -y as f64 * k });
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let k = self.1;
        self.0.push(PathCommand::QuadTo { x1: x1 as f64 * k, y1: -y1 as f64 * k, x: x as f64 * k, y: -y as f64 * k });
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let k = self.1;
        self.0.push(PathCommand::CubicTo {
            x1: x1 as f64 * k, y1: -y1 as f64 * k,
            x2: x2 as f64 * k, y2: -y2 as f64 * k,
            x: x as f64 * k, y: -y as f64 * k,
        });
    }
    fn close(&mut self) {
        self.0.push(PathCommand::Close);
    }
}

fn resolve_glyph(font: &str, char_code: u32, window: &Window) -> Option<GlyphSource> {
    let key = (font.to_string(), char_code);
    if let Some(hit) = glyph_cache().lock().unwrap().get(&key) {
        return hit.clone();
    }
    let ch = char::from_u32(char_code);
    let resolved = ch.and_then(|ch| {
        if let Some(face) = katex_face(font) {
            if face.glyph_index('m').is_none() {
                let gid = face.glyph_index(ch)?;
                let mut sink = OutlineSink(Vec::new(), 1.0 / face.units_per_em() as f64);
                face.outline_glyph(gid, &mut sink)?;
                return Some(GlyphSource::Outline(Arc::new(sink.0)));
            }
        }
        let text: SharedString = ch.to_string().into();
        let run = TextRun {
            len: text.len(),
            font: font_for(font),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(text, px(16.), &[run], None);
        let run = line.runs.first()?;
        let glyph = run.glyphs.first()?;
        Some(GlyphSource::Atlas(run.font_id, glyph.id))
    });
    glyph_cache().lock().unwrap().insert(key, resolved.clone());
    resolved
}

fn paint_commands(
    commands: &[PathCommand],
    fill: bool,
    map: impl Fn(f64, f64) -> Point<Pixels>,
    color: Hsla,
    window: &mut Window,
) {
    let mut builder = if fill { PathBuilder::fill() } else { PathBuilder::stroke(px(1.)) };
    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo { x, y } => builder.move_to(map(x, y)),
            PathCommand::LineTo { x, y } => builder.line_to(map(x, y)),
            PathCommand::QuadTo { x1, y1, x, y } => builder.curve_to(map(x, y), map(x1, y1)),
            PathCommand::CubicTo { x1, y1, x2, y2, x, y } => {
                builder.cubic_bezier_to(map(x, y), map(x1, y1), map(x2, y2))
            }
            PathCommand::Close => builder.close(),
        }
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn tint(c: &Color, fallback: Hsla) -> Hsla {
    if c.r == 0.0 && c.g == 0.0 && c.b == 0.0 {
        Hsla { a: fallback.a * c.a, ..fallback }
    } else {
        gpui::Rgba { r: c.r, g: c.g, b: c.b, a: c.a }.into()
    }
}

/// Paint `layout` with its top-left at `origin`, at `font_size` px per em.
/// Must be called during an element's paint phase.
pub fn paint(
    layout: &MathLayout,
    origin: Point<Pixels>,
    font_size: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let fs = f32::from(font_size);
    let at = |x: f64, y: f64| point(origin.x + px(x as f32 * fs), origin.y + px(y as f32 * fs));
    for item in &layout.items {
        match item {
            DisplayItem::GlyphPath { x, y, scale, font, char_code, color: c } => {
                match resolve_glyph(font, *char_code, window) {
                    Some(GlyphSource::Atlas(font_id, glyph_id)) => {
                        let _ = window.paint_glyph(
                            at(*x, *y),
                            font_id,
                            glyph_id,
                            px(fs * *scale as f32),
                            tint(c, color),
                        );
                    }
                    Some(GlyphSource::Outline(cmds)) => {
                        let (gx, gy, k) = (*x, *y, *scale);
                        paint_commands(&cmds, true, |px_, py_| at(gx + px_ * k, gy + py_ * k), tint(c, color), window);
                    }
                    None => {}
                }
            }
            DisplayItem::Line { x, y, width, thickness, color: c, .. } => {
                // Keep rules at least one device pixel thick so fraction bars never vanish.
                let min = 1.0 / window.scale_factor();
                let h = (*thickness as f32 * fs).max(min);
                let top = at(*x, *y) - point(px(0.), px(h / 2.));
                window.paint_quad(gpui::fill(
                    Bounds::new(top, gpui::size(px(*width as f32 * fs), px(h))),
                    tint(c, color),
                ));
            }
            DisplayItem::Rect { x, y, width, height, color: c } => {
                window.paint_quad(gpui::fill(
                    Bounds::new(at(*x, *y), gpui::size(px(*width as f32 * fs), px(*height as f32 * fs))),
                    tint(c, color),
                ));
            }
            DisplayItem::Path { x, y, commands, fill, color: c } => {
                paint_commands(commands, *fill, |px_, py_| at(x + px_, y + py_), tint(c, color), window);
            }
        }
    }
}

mod element;
pub use element::{math, MathElement};
