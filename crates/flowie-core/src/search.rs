//! Search queries: every whitespace-separated term must match (AND).
//! Terms: plain words (substring, case-insensitive, in text or note),
//! `"quoted phrases"`, `#tag` / `@mention` (whole-token), `-term` (exclude),
//! `is:done` / `is:todo`.

use crate::tree::Node;

#[derive(Debug, Clone, PartialEq)]
enum Term {
    Text(String),
    Tag(String),
    Done(bool),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Query {
    terms: Vec<(bool, Term)>, // (negated, term)
    raw: String,
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
                "is:todo" | "is:open" => Term::Done(false),
                _ if (lower.starts_with('#') || lower.starts_with('@')) && lower.chars().count() > 1 => Term::Tag(lower),
                _ => Term::Text(lower),
            };
            terms.push((neg, term));
        }
        Query { terms, raw: input.to_string() }
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
                Term::Done(_) => None,
            })
            .collect()
    }

    pub fn matches(&self, node: &Node) -> bool {
        if self.terms.is_empty() {
            return true;
        }
        let text = fold(&node.text);
        let note = if node.note.is_empty() { String::new() } else { fold(&node.note) };
        self.terms.iter().all(|(neg, term)| {
            let hit = match term {
                Term::Text(s) => text.contains(s.as_str()) || note.contains(s.as_str()),
                Term::Tag(tag) => has_tag(&text, tag) || has_tag(&note, tag),
                Term::Done(d) => node.completed == *d,
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
