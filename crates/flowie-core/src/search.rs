//! Search queries: every whitespace-separated term must match (AND).
//! Terms: plain words (substring, case-insensitive, in text or note),
//! `"quoted phrases"`, `#tag` / `@mention` (whole-token), `-term` (exclude),
//! `is:done` / `is:open`, `is:todo` / `is:heading` / `is:quote` / `is:code`,
//! `has:note`, `is:new` (imported roots not looked at yet), `changed:7d` (also `h`, `m`, `w`), and date filters over the
//! dates written in bullets: `date:2026-11`, `date:today`, `before:2026-12-01`,
//! `after:today`.

use crate::tree::{BlockKind, Node};

#[derive(Debug, Clone, PartialEq)]
enum Term {
    Text(String),
    Tag(String),
    Done(bool),
    Kinds(Vec<BlockKind>),
    HasNote,
    /// An imported root not seen yet (see `Query::set_new`).
    New,
    /// Modified within this many milliseconds.
    Changed(i64),
    DatePrefix(String),
    Before(String),
    After(String),
}

/// Today's local date as `YYYY-MM-DD`.
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn resolve_date(s: &str) -> String {
    use chrono::{Duration, Local};
    match s {
        "today" => today(),
        "tomorrow" => (Local::now() + Duration::days(1)).format("%Y-%m-%d").to_string(),
        "yesterday" => (Local::now() - Duration::days(1)).format("%Y-%m-%d").to_string(),
        _ => s.to_string(),
    }
}

fn parse_span(s: &str) -> Option<i64> {
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    let ms = match unit {
        "m" | "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        _ => return None,
    };
    Some(n * ms)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Query {
    terms: Vec<(bool, Term)>, // (negated, term)
    raw: String,
    /// What `is:new` matches: the tree doesn't know which roots are unread.
    new: std::collections::HashSet<crate::tree::NodeId>,
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut terms = Vec::new();
        let mut chars = input.trim().chars().peekable();
        while chars.peek().is_some() {
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            let mut neg = false;
            if chars.peek() == Some(&'-') {
                neg = true;
                chars.next();
            }
            let mut word = String::new();
            if chars.peek() == Some(&'"') {
                chars.next();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    word.push(c);
                }
            } else {
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() {
                        break;
                    }
                    word.push(c);
                    chars.next();
                }
            }
            if word.is_empty() {
                continue;
            }
            let lower = fold(&word);
            let term = match lower.as_str() {
                "is:done" | "is:completed" => Term::Done(true),
                "is:open" | "is:undone" => Term::Done(false),
                "is:todo" => Term::Kinds(vec![BlockKind::Todo]),
                "is:heading" => Term::Kinds(vec![BlockKind::H1, BlockKind::H2, BlockKind::H3]),
                "is:quote" => Term::Kinds(vec![BlockKind::Quote]),
                "is:code" => Term::Kinds(vec![BlockKind::Code]),
                "is:paragraph" => Term::Kinds(vec![BlockKind::Paragraph]),
                "has:note" => Term::HasNote,
                "is:new" => Term::New,
                _ if lower.starts_with("changed:") && parse_span(&lower[8..]).is_some() => Term::Changed(parse_span(&lower[8..]).unwrap()),
                _ if lower.starts_with("date:") && lower.len() > 5 => Term::DatePrefix(resolve_date(&lower[5..])),
                _ if lower.starts_with("before:") && lower.len() > 7 => Term::Before(resolve_date(&lower[7..])),
                _ if lower.starts_with("after:") && lower.len() > 6 => Term::After(resolve_date(&lower[6..])),
                _ if (lower.starts_with('#') || lower.starts_with('@')) && lower.chars().count() > 1 => Term::Tag(lower),
                _ => Term::Text(lower),
            };
            terms.push((neg, term));
        }
        Query { terms, raw: input.to_string(), new: Default::default() }
    }

    /// The bullets `is:new` matches.
    pub fn set_new(&mut self, ids: std::collections::HashSet<crate::tree::NodeId>) {
        self.new = ids;
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Lowercased plain terms, for highlighting matches.
    pub fn highlight_terms(&self) -> Vec<String> {
        self.terms
            .iter()
            .filter(|(neg, _)| !neg)
            .filter_map(|(_, t)| match t {
                Term::Text(s) | Term::Tag(s) => Some(s.clone()),
                Term::DatePrefix(d) => Some(d.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn matches(&self, node: &Node) -> bool {
        if self.terms.is_empty() {
            return true;
        }
        let text = fold(&node.text);
        let note = if node.note.is_empty() { String::new() } else { fold(&node.note) };
        let now = crate::tree::now_millis();
        let dates = || crate::markup::dates(&node.text);
        self.terms.iter().all(|(neg, term)| {
            let hit = match term {
                Term::Text(s) => text.contains(s.as_str()) || note.contains(s.as_str()),
                Term::Tag(tag) => has_tag(&text, tag) || has_tag(&note, tag),
                Term::Done(d) => node.completed == *d,
                Term::Kinds(k) => k.contains(&node.kind),
                Term::HasNote => !node.note.trim().is_empty(),
                Term::New => self.new.contains(&node.id),
                Term::Changed(ms) => now - node.modified <= *ms,
                Term::DatePrefix(p) => dates().iter().any(|d| d.starts_with(p.as_str())),
                Term::Before(b) => dates().iter().any(|d| *d < b.as_str()),
                Term::After(a) => dates().iter().any(|d| *d > a.as_str()),
            };
            hit != *neg
        })
    }
}

/// Case folding for matching (lowercase; good enough for CJK, which has no case).
pub fn fold(s: &str) -> String {
    s.to_lowercase()
}

fn has_tag(haystack: &str, tag: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(tag) {
        let begin = start + pos;
        let end = begin + tag.len();
        let before_ok = haystack[..begin].chars().next_back().is_none_or(|c| !crate::markup::is_tag_char(c));
        let after_ok = haystack[end..].chars().next().is_none_or(|c| !crate::markup::is_tag_char(c));
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

/// Byte ranges of `terms` inside `text` (case-insensitive), sorted, non-overlapping.
pub fn find_highlights(text: &str, terms: &[String]) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    if terms.is_empty() {
        return out;
    }
    // Lowercasing may change byte lengths for some scripts; map via char boundaries.
    let lower = fold(text);
    if lower.len() != text.len() {
        return out;
    }
    for term in terms {
        let mut start = 0;
        while let Some(pos) = lower[start..].find(term.as_str()) {
            let s = start + pos;
            out.push(s..s + term.len());
            start = s + term.len().max(1);
        }
    }
    out.sort_by_key(|r| r.start);
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
    for r in out {
        if let Some(last) = merged.last_mut() {
            if r.start <= last.end {
                last.end = last.end.max(r.end);
                continue;
            }
        }
        merged.push(r);
    }
    merged
}
