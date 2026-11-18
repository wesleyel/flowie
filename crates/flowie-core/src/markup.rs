//! Inline markup inside a bullet's text. The stored text stays plain
//! markdown-lite; this parser splits it into runs for rendering.
//!
//! Supported: `**bold**`, `*italic*` / `_italic_`, `~~strike~~`, `` `code` ``,
//! `[label](url)` (`flowie:ID` urls link to other bullets), bare
//! `http(s)://` URLs, `#tag`, `@mention`, dates `2026-11-17`,
//! `==highlight==`, `{red}colored text{/}`, `$inline math$`,
//! `$$display math$$`, and `\` escapes for `$ * _ ` [ # = {`.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Text,
    /// Syntax characters (`**`, `[`, `](url)`, …): hidden when rendered, dimmed when editing.
    Marker,
    Code,
    /// Whole `$…$` / `$$…$$` span; `source` is the LaTeX inside.
    Math {
        source: Range<usize>,
        display: bool,
    },
    Link {
        url: String,
    },
    Tag,
    Mention,
    /// An ISO date (`2026-11-17`).
    Date,
}

/// Text colors usable as `{name}…{/}`.
pub const COLORS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub range: Range<usize>,
    pub kind: Kind,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub highlight: bool,
    /// Index into `COLORS`.
    pub color: Option<u8>,
}

pub fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '/'
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

struct Parser<'a> {
    s: &'a str,
    runs: Vec<Run>,
    bold: bool,
    italic: bool,
    strike: bool,
    highlight: bool,
    color: Option<u8>,
    text_start: usize,
}

impl<'a> Parser<'a> {
    fn flush(&mut self, upto: usize) {
        if upto > self.text_start {
            self.push(self.text_start..upto, Kind::Text);
        }
        self.text_start = upto;
    }

    fn push(&mut self, range: Range<usize>, kind: Kind) {
        if range.is_empty() {
            return;
        }
        // Merge adjacent plain text runs with identical style.
        if let Some(last) = self.runs.last_mut() {
            if last.kind == Kind::Text
                && kind == Kind::Text
                && last.range.end == range.start
                && (last.bold, last.italic, last.strike, last.highlight, last.color)
                    == (self.bold, self.italic, self.strike, self.highlight, self.color)
            {
                last.range.end = range.end;
                return;
            }
        }
        self.runs.push(Run {
            range,
            kind,
            bold: self.bold,
            italic: self.italic,
            strike: self.strike,
            highlight: self.highlight,
            color: self.color,
        });
    }

    fn emit(&mut self, at: usize, range: Range<usize>, kind: Kind) {
        self.flush(at);
        let end = range.end;
        self.push(range, kind);
        self.text_start = end;
    }

    fn prev_char(&self, i: usize) -> Option<char> {
        self.s[..i].chars().next_back()
    }
}

pub fn parse(s: &str) -> Vec<Run> {
    let mut p = Parser { s, runs: Vec::new(), bold: false, italic: false, strike: false, highlight: false, color: None, text_start: 0 };
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next().unwrap();
        // Escapes: hide the backslash, keep the next char literal.
        if c == '\\' {
            if let Some(n) = rest[1..].chars().next() {
                if "$*_`[]#@~\\={".contains(n) {
                    p.emit(i, i..i + 1, Kind::Marker);
                    i += 1 + n.len_utf8();
                    continue;
                }
            }
        }
        // Display math $$…$$
        if let Some(after) = rest.strip_prefix("$$") {
            if let Some(end) = after.find("$$") {
                if end > 0 {
                    let src = i + 2..i + 2 + end;
                    p.emit(i, i..src.end + 2, Kind::Math { source: src, display: true });
                    i = i + 2 + end + 2;
                    continue;
                }
            }
        }
        // Inline math $…$ (pandoc rules: no space after opening / before closing,
        // closing not followed by a digit — so "$5 and $6" stays text).
        if c == '$' {
            if let Some(end) = find_inline_math_end(s, i) {
                let src = i + 1..end;
                p.emit(i, i..end + 1, Kind::Math { source: src, display: false });
                i = end + 1;
                continue;
            }
        }
        if c == '`' {
            if let Some(end) = rest[1..].find('`') {
                if end > 0 {
                    let start = i;
                    p.emit(start, start..start + 1, Kind::Marker);
                    p.emit(start + 1, start + 1..start + 1 + end, Kind::Code);
                    p.emit(start + 1 + end, start + 1 + end..start + 2 + end, Kind::Marker);
                    i = start + 2 + end;
                    continue;
                }
            }
        }
        if c == '[' {
            if let Some((label_end, url_end)) = find_link(rest) {
                let url = rest[label_end + 2..url_end].to_string();
                p.emit(i, i..i + 1, Kind::Marker);
                p.emit(i + 1, i + 1..i + label_end, Kind::Link { url: url.clone() });
                p.emit(i + label_end, i + label_end..i + url_end + 1, Kind::Marker);
                i += url_end + 1;
                continue;
            }
        }
        if (rest.starts_with("http://") || rest.starts_with("https://")) && p.prev_char(i).is_none_or(|c| !is_word_char(c)) {
            let len = rest.find(|c: char| c.is_whitespace() || "<>\"".contains(c)).unwrap_or(rest.len());
            let mut len = len;
            while len > 0 && ".,;:!?)".contains(rest[..len].chars().next_back().unwrap()) {
                len -= 1;
            }
            let url = rest[..len].to_string();
            p.emit(i, i..i + len, Kind::Link { url });
            i += len;
            continue;
        }
        if (c == '#' || c == '@') && p.prev_char(i).is_none_or(|c| c.is_whitespace() || "([{\"'".contains(c)) {
            let body = rest[1..].find(|c: char| !is_tag_char(c)).unwrap_or(rest.len() - 1);
            if body > 0 {
                let kind = if c == '#' { Kind::Tag } else { Kind::Mention };
                p.emit(i, i..i + 1 + body, kind);
                i += 1 + body;
                continue;
            }
        }
        if rest.starts_with("==") && (p.highlight || rest[2..].contains("==")) {
            p.emit(i, i..i + 2, Kind::Marker);
            p.highlight = !p.highlight;
            i += 2;
            continue;
        }
        if c == '{' {
            if p.color.is_some() && rest.starts_with("{/}") {
                p.emit(i, i..i + 3, Kind::Marker);
                p.color = None;
                i += 3;
                continue;
            }
            if p.color.is_none() {
                if let Some(close) = rest.find('}') {
                    let name = &rest[1..close];
                    if let Some(ix) = COLORS.iter().position(|c| *c == name) {
                        if rest[close..].contains("{/}") {
                            p.emit(i, i..i + close + 1, Kind::Marker);
                            p.color = Some(ix as u8);
                            i += close + 1;
                            continue;
                        }
                    }
                }
            }
        }
        if c.is_ascii_digit() && p.prev_char(i).is_none_or(|ch| !ch.is_alphanumeric()) {
            if let Some(len) = date_len(rest) {
                p.emit(i, i..i + len, Kind::Date);
                i += len;
                continue;
            }
        }
        if rest.starts_with("**") && (p.bold || rest[2..].contains("**")) {
            p.emit(i, i..i + 2, Kind::Marker);
            p.bold = !p.bold;
            i += 2;
            continue;
        }
        if rest.starts_with("~~") && (p.strike || rest[2..].contains("~~")) {
            p.emit(i, i..i + 2, Kind::Marker);
            p.strike = !p.strike;
            i += 2;
            continue;
        }
        if c == '*' || c == '_' {
            let prev = p.prev_char(i);
            let next = rest[1..].chars().next();
            let can_open = !p.italic
                && next.is_some_and(|n| !n.is_whitespace())
                && (c == '*' || prev.is_none_or(|ch| !is_word_char(ch)))
                && rest[1..].contains(c);
            let can_close = p.italic && prev.is_some_and(|ch| !ch.is_whitespace()) && (c == '*' || next.is_none_or(|ch| !is_word_char(ch)));
            if can_open || can_close {
                p.emit(i, i..i + 1, Kind::Marker);
                p.italic = !p.italic;
                i += 1;
                continue;
            }
        }
        let _ = bytes;
        i += c.len_utf8();
    }
    p.flush(s.len());
    p.runs
}

/// Length of an ISO date (`YYYY-MM-DD`) at the start of `s`, if any.
pub fn date_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10)) {
        return None;
    }
    if b.get(10).is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some(10)
}

/// All ISO dates in `s`.
pub fn dates(s: &str) -> Vec<&str> {
    parse(s).into_iter().filter(|r| r.kind == Kind::Date).map(|r| &s[r.range]).collect()
}

fn find_inline_math_end(s: &str, open: usize) -> Option<usize> {
    let after = s[open + 1..].chars().next()?;
    if after.is_whitespace() || after == '$' {
        return None;
    }
    let mut j = open + 1;
    while j < s.len() {
        let rest = &s[j..];
        let c = rest.chars().next()?;
        if c == '\\' {
            j += 1 + rest[1..].chars().next().map_or(0, char::len_utf8);
            continue;
        }
        if c == '$' {
            let before = s[..j].chars().next_back()?;
            let next = rest[1..].chars().next();
            if !before.is_whitespace() && next.is_none_or(|n| !n.is_ascii_digit()) {
                return Some(j);
            }
        }
        j += c.len_utf8();
    }
    None
}

/// For `[label](url)` at the start of `s`: (index of `]`, index of `)`).
fn find_link(s: &str) -> Option<(usize, usize)> {
    let close = s.find(']')?;
    if close < 2 || !s[close..].starts_with("](") {
        return None;
    }
    let url_end = close + 2 + s[close + 2..].find(')')?;
    if url_end == close + 2 || s[close + 2..url_end].contains(char::is_whitespace) {
        return None;
    }
    Some((close, url_end))
}

/// Text with markers removed, e.g. for window titles and breadcrumbs.
pub fn plain_text(s: &str) -> String {
    parse(s).into_iter().filter(|r| r.kind != Kind::Marker).map(|r| &s[r.range]).collect()
}
