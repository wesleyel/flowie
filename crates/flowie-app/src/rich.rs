//! Turns a bullet's raw text into styled runs. Display mode hides markup
//! characters and renders math; edit mode shows everything raw but styled.

use std::ops::Range;

use flowie_core::markup::{self, Kind};
use flowie_core::search::find_highlights;
use gpui::{Font, FontStyle, FontWeight, Hsla, SharedString, StrikethroughStyle, TextRun, UnderlineStyle};

use crate::theme::Theme;

#[derive(Clone, Debug)]
pub enum Click {
    Tag(String),
    Link(String),
}

/// A piece of displayed content.
pub enum Piece {
    Text {
        text: String,
        runs: Vec<TextRun>,
        /// (display offset, raw offset, len) for mapping clicks back to the raw text.
        map: Vec<(usize, usize, usize)>,
        clicks: Vec<(Range<usize>, Click)>,
    },
    Math {
        source: SharedString,
        display: bool,
    },
}

pub struct Style<'a> {
    pub theme: &'a Theme,
    pub font: Font,
    pub color: Hsla,
    pub completed: bool,
    pub highlights: &'a [String],
}

fn code_font() -> Font {
    gpui::font("Menlo")
}

struct Builder<'a> {
    style: &'a Style<'a>,
    raw: &'a str,
    hits: Vec<Range<usize>>,
    pieces: Vec<Piece>,
    text: String,
    runs: Vec<TextRun>,
    map: Vec<(usize, usize, usize)>,
    clicks: Vec<(Range<usize>, Click)>,
}

impl<'a> Builder<'a> {
    /// Append raw[range] with the given run template, split at search hits.
    fn push(&mut self, range: Range<usize>, template: &TextRun, click: Option<Click>) {
        if range.is_empty() {
            return;
        }
        let disp_start = self.text.len();
        self.text.push_str(&self.raw[range.clone()]);
        self.map.push((disp_start, range.start, range.len()));
        if let Some(c) = click {
            self.clicks.push((disp_start..self.text.len(), c));
        }
        let mut cuts = vec![range.start, range.end];
        for h in &self.hits {
            for p in [h.start, h.end] {
                if p > range.start && p < range.end {
                    cuts.push(p);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (s, e) = (w[0], w[1]);
            let hit = self.hits.iter().any(|h| h.start <= s && e <= h.end);
            let mut run = template.clone();
            run.len = e - s;
            if hit {
                run.background_color = Some(self.style.theme.search_hit);
            }
            self.runs.push(run);
        }
    }

    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.pieces.push(Piece::Text {
                text: std::mem::take(&mut self.text),
                runs: std::mem::take(&mut self.runs),
                map: std::mem::take(&mut self.map),
                clicks: std::mem::take(&mut self.clicks),
            });
        }
    }
}

fn template(style: &Style, bold: bool, italic: bool, strike: bool) -> TextRun {
    let mut font = style.font.clone();
    if bold {
        font.weight = FontWeight::BOLD;
    }
    if italic {
        font.style = FontStyle::Italic;
    }
    TextRun {
        len: 0,
        font,
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: (strike || style.completed).then(|| StrikethroughStyle { thickness: gpui::px(1.), color: Some(style.color) }),
    }
}

/// Build the pieces for `raw`. In `edit` mode the result is a single text
/// piece covering every byte of `raw` (so caret offsets map 1:1).
pub fn build(raw: &str, edit: bool, style: &Style) -> Vec<Piece> {
    let theme = style.theme;
    let mut b = Builder {
        style,
        raw,
        hits: find_highlights(raw, style.highlights),
        pieces: Vec::new(),
        text: String::new(),
        runs: Vec::new(),
        map: Vec::new(),
        clicks: Vec::new(),
    };
    for run in markup::parse(raw) {
        let mut t = template(style, run.bold, run.italic, run.strike);
        match &run.kind {
            Kind::Text => b.push(run.range, &t, None),
            Kind::Marker => {
                if edit {
                    t.color = theme.marker;
                    b.push(run.range, &t, None);
                }
            }
            Kind::Code => {
                t.font = code_font();
                t.background_color = Some(theme.code_bg);
                b.push(run.range, &t, None);
            }
            Kind::Math { source, display } => {
                if edit {
                    t.font = code_font();
                    t.color = theme.accent;
                    b.push(run.range.clone(), &t, None);
                } else {
                    b.flush_text();
                    b.pieces.push(Piece::Math { source: raw[source.clone()].to_string().into(), display: *display });
                }
            }
            Kind::Link { url } => {
                t.color = theme.link;
                t.underline = Some(UnderlineStyle { thickness: gpui::px(1.), color: Some(theme.link.opacity(0.4)), wavy: false });
                b.push(run.range, &t, Some(Click::Link(url.clone())));
            }
            Kind::Tag | Kind::Mention => {
                t.color = theme.tag;
                t.background_color = Some(theme.tag_bg);
                let tag = raw[run.range.clone()].to_string();
                b.push(run.range, &t, Some(Click::Tag(tag)));
            }
        }
    }
    b.flush_text();
    if edit && b.pieces.is_empty() {
        b.pieces.push(Piece::Text { text: String::new(), runs: Vec::new(), map: Vec::new(), clicks: Vec::new() });
    }
    b.pieces
}

/// Map a display offset inside a text piece back to a raw offset.
pub fn display_to_raw(map: &[(usize, usize, usize)], disp: usize) -> usize {
    for &(d, r, len) in map {
        if disp >= d && disp <= d + len {
            return r + (disp - d);
        }
    }
    map.last().map(|&(_, r, len)| r + len).unwrap_or(0)
}

/// Split a styled text into word-ish tokens (for wrapping around inline math):
/// runs of non-space characters plus trailing spaces; each CJK character alone.
pub fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (i, c) in text.char_indices() {
        let cjk = is_cjk(c);
        if i > start && (cjk || (in_space && !c.is_whitespace())) {
            out.push(start..i);
            start = i;
        }
        in_space = c.is_whitespace();
        if cjk {
            let end = i + c.len_utf8();
            out.push(start..end);
            start = end;
            in_space = false;
        }
    }
    if start < text.len() {
        out.push(start..text.len());
    }
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
}

/// Slice runs to a sub-range of their text.
pub fn slice_runs(runs: &[TextRun], range: Range<usize>) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut pos = 0;
    for r in runs {
        let (s, e) = (pos, pos + r.len);
        pos = e;
        let (a, b) = (s.max(range.start), e.min(range.end));
        if a < b {
            let mut r = r.clone();
            r.len = b - a;
            out.push(r);
        }
    }
    out
}
