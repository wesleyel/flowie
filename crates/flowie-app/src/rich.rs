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
    Node(flowie_core::NodeId),
    Date(String),
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
    /// Code blocks: no markup, monospace.
    pub code: bool,
}

fn code_font() -> Font {
    gpui::font(crate::platform::MONO_FONT)
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

fn template(style: &Style, run: &markup::Run) -> TextRun {
    let (bold, italic, strike) = (run.bold, run.italic, run.strike);
    let mut font = style.font.clone();
    if bold {
        font.weight = FontWeight::BOLD;
    }
    if italic {
        font.style = FontStyle::Italic;
    }
    let color = match run.color {
        Some(c) if !style.completed => style.theme.colors[c as usize % 7],
        _ => style.color,
    };
    TextRun {
        len: 0,
        font,
        color,
        background_color: run.highlight.then_some(style.theme.mark),
        underline: None,
        strikethrough: (strike || style.completed).then(|| StrikethroughStyle { thickness: gpui::px(1.), color: Some(style.color) }),
    }
}

/// Internal links (`[label](flowie:ID)`) in `raw`: the whole link and its label.
pub fn internal_links(raw: &str) -> Vec<(Range<usize>, Range<usize>)> {
    if !raw.contains("](flowie:") {
        return Vec::new();
    }
    let runs = markup::parse(raw);
    let mut out = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let Kind::Link { url } = &run.kind else { continue };
        let internal = url.strip_prefix("flowie:").is_some_and(|id| id.parse::<flowie_core::NodeId>().is_ok());
        let (Some(open), Some(close)) = (i.checked_sub(1).and_then(|j| runs.get(j)), runs.get(i + 1)) else { continue };
        if internal && open.kind == Kind::Marker && close.kind == Kind::Marker && open.range.end == run.range.start {
            out.push((open.range.start..close.range.end, run.range.clone()));
        }
    }
    out
}

/// How the editor's raw text maps to what it shows. Everything shows as
/// typed, except internal links: only their label shows (the 32-digit id
/// would be noise), and the caret treats each link as one character.
#[derive(Clone, Debug, Default)]
pub struct EditMap {
    /// Visible raw ranges as (display offset, raw offset, len).
    segs: Vec<(usize, usize, usize)>,
    /// Each link's raw range and its label's display range.
    atoms: Vec<(Range<usize>, Range<usize>)>,
}

impl EditMap {
    pub fn new(raw: &str, code: bool) -> EditMap {
        let links = if code { Vec::new() } else { internal_links(raw) };
        let mut segs = Vec::new();
        let mut atoms = Vec::new();
        let (mut at, mut disp) = (0, 0);
        let mut visible = |start: usize, end: usize, segs: &mut Vec<(usize, usize, usize)>| {
            if end > start {
                segs.push((disp, start, end - start));
                disp += end - start;
            }
            disp
        };
        for (link, label) in links {
            visible(at, label.start.min(link.start), &mut segs);
            let d0 = visible(label.start, label.start, &mut segs);
            let d1 = visible(label.start, label.end, &mut segs);
            atoms.push((link.clone(), d0..d1));
            at = link.end;
        }
        visible(at, raw.len(), &mut segs);
        if segs.is_empty() {
            segs.push((0, 0, 0));
        }
        EditMap { segs, atoms }
    }

    /// Where raw offset `raw` shows (hidden link syntax: at the label's edge).
    pub fn to_display(&self, raw: usize) -> usize {
        if let Some((link, label)) = self.atoms.iter().find(|(l, _)| l.start < raw && raw < l.end) {
            // Inside a link's hidden syntax or label: clamp into the label.
            let first = self.segs.iter().find(|&&(d, _, _)| d == label.start).map_or(link.start + 1, |&(_, r, _)| r);
            return label.start + raw.saturating_sub(first).min(label.len());
        }
        self.segs.iter().rev().find(|&&(_, r, _)| r <= raw).map_or(0, |&(d, r, len)| d + (raw - r).min(len))
    }

    /// The raw offset for display offset `disp`, never inside a link: a point
    /// on a label goes to whichever end of the link is nearer.
    pub fn to_raw(&self, disp: usize) -> usize {
        if let Some((link, label)) = self.atoms.iter().find(|(_, d)| d.start <= disp && disp <= d.end) {
            return if disp - label.start <= label.end - disp { link.start } else { link.end };
        }
        self.segs
            .iter()
            .find(|&&(d, _, len)| d <= disp && disp <= d + len)
            .map_or_else(|| self.segs.last().map_or(0, |&(_, r, len)| r + len), |&(d, r, _)| r + (disp - d))
    }

    /// The link whose label shows at display offset `disp` (strictly inside it).
    pub fn atom_at_display(&self, disp: usize) -> Option<Range<usize>> {
        self.atoms.iter().find(|(_, d)| d.start < disp && disp < d.end).map(|(l, _)| l.clone())
    }

    /// Move a raw offset out of any link: to its start (`forward` false) or end.
    pub fn snap(&self, raw: usize, forward: bool) -> usize {
        match self.atoms.iter().find(|(l, _)| l.start < raw && raw < l.end) {
            Some((l, _)) if forward => l.end,
            Some((l, _)) => l.start,
            None => raw,
        }
    }
}

/// Build the pieces for `raw`. In `edit` mode the result is a single text
/// piece covering every byte of `raw` except the hidden syntax of internal
/// links (see `EditMap`).
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
    if style.code {
        let mut t = template(
            style,
            &markup::Run {
                range: 0..raw.len(),
                kind: Kind::Text,
                bold: false,
                italic: false,
                strike: false,
                highlight: false,
                color: None,
            },
        );
        t.font = code_font();
        b.push(0..raw.len(), &t, None);
        b.flush_text();
        if b.pieces.is_empty() && edit {
            b.pieces.push(Piece::Text { text: String::new(), runs: Vec::new(), map: Vec::new(), clicks: Vec::new() });
        }
        return b.pieces;
    }
    let hidden_links = if edit { internal_links(raw) } else { Vec::new() };
    for run in markup::parse(raw) {
        let mut t = template(style, &run);
        match &run.kind {
            Kind::Text => b.push(run.range, &t, None),
            Kind::Marker => {
                let link_syntax = hidden_links.iter().any(|(l, _)| l.start <= run.range.start && run.range.end <= l.end);
                if edit && !link_syntax {
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
                let internal = url.strip_prefix("flowie:").and_then(|id| id.parse::<flowie_core::NodeId>().ok());
                t.underline = Some(UnderlineStyle {
                    thickness: gpui::px(1.),
                    color: Some(theme.link.opacity(if internal.is_some() { 0.0 } else { 0.4 })),
                    wavy: false,
                });
                if internal.is_some() {
                    t.background_color = Some(theme.link.opacity(0.08));
                }
                let click = match internal {
                    Some(id) => Click::Node(id),
                    None => Click::Link(url.clone()),
                };
                b.push(run.range, &t, Some(click));
            }
            Kind::Date => {
                t.color = theme.date;
                t.background_color = Some(theme.date_bg);
                let d = raw[run.range.clone()].to_string();
                b.push(run.range, &t, Some(Click::Date(d)));
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

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "c239e043175d4aa28788a118eaf4eb92";

    #[test]
    fn internal_links_show_their_label_and_act_as_one_character() {
        let raw = format!("see [alpha](flowie:{ID}) end");
        let link = 4..raw.len() - 4;
        let m = EditMap::new(&raw, false);
        // Shown: "see alpha end".
        assert_eq!(m.to_display(0), 0);
        assert_eq!(m.to_display(4), 4);
        assert_eq!(m.to_display(link.end), 9);
        assert_eq!(m.to_display(raw.len()), "see alpha end".len());
        // Inside the label maps into it; inside the hidden syntax, to its edge.
        assert_eq!(m.to_display(6), 5);
        assert_eq!(m.to_display(link.end - 3), 9);
        // Back to raw: never inside the link.
        assert_eq!(m.to_raw(4), link.start);
        assert_eq!(m.to_raw(5), link.start);
        assert_eq!(m.to_raw(8), link.end);
        assert_eq!(m.to_raw(9), link.end);
        assert_eq!(m.to_raw(10), link.end + 1);
        assert_eq!(m.atom_at_display(6), Some(link.clone()));
        assert_eq!(m.atom_at_display(4), None);
        assert_eq!(m.snap(link.start + 3, false), link.start);
        assert_eq!(m.snap(link.start + 3, true), link.end);
        assert_eq!(m.snap(2, true), 2);
    }

    #[test]
    fn other_text_maps_one_to_one() {
        for raw in ["", "plain **bold** [web](https://example.com)", "[bad](flowie:12)"] {
            let m = EditMap::new(raw, false);
            for i in 0..=raw.len() {
                assert_eq!((m.to_display(i), m.to_raw(i)), (i, i), "{raw:?} at {i}");
            }
        }
        // Code blocks show links raw.
        let raw = format!("[a](flowie:{ID})");
        let m = EditMap::new(&raw, true);
        assert_eq!(m.to_display(raw.len()), raw.len());
    }

    #[test]
    fn link_at_the_end() {
        let raw = format!("[a](flowie:{ID})");
        let m = EditMap::new(&raw, false);
        assert_eq!(m.to_display(raw.len()), 1);
        assert_eq!(m.to_raw(1), raw.len());
        assert_eq!(m.to_raw(0), 0);
    }
}
