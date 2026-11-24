//! Indented text and Markdown outline documents.

use super::Item;
use crate::BlockKind;

/// Markdown-compatible indented list: two spaces per level, notes as
/// indented lines without a bullet marker.
pub fn to_markdown(items: &[Item]) -> String {
    let mut out = String::new();
    for it in items {
        write_md(it, 0, &mut out);
    }
    out
}

fn write_md(it: &Item, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    out.push_str(&pad);
    let text = &it.text;
    match it.kind {
        BlockKind::H1 => out.push_str(&format!("- # {text}")),
        BlockKind::H2 => out.push_str(&format!("- ## {text}")),
        BlockKind::H3 => out.push_str(&format!("- ### {text}")),
        BlockKind::Todo => out.push_str(&format!("- [{}] {text}", if it.completed { "x" } else { " " })),
        BlockKind::Quote => out.push_str(&format!("- > {text}")),
        BlockKind::Numbered => out.push_str(&format!("1. {text}")),
        BlockKind::Image => out.push_str(&format!("- ![{}]({})", text.replace(['[', ']'], ""), image_path(&it.image))),
        _ if it.completed && !text.is_empty() => out.push_str(&format!("- ~~{text}~~")),
        _ => out.push_str(&format!("- {text}")),
    }
    out.push('\n');
    for line in it.note.lines() {
        out.push_str(&pad);
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    for kid in &it.children {
        write_md(kid, depth + 1, out);
    }
}

/// Where Markdown export puts an image: `images/<name>` beside the file.
pub fn image_path(name: &str) -> String {
    if crate::images::is_name(name) {
        format!("images/{name}")
    } else {
        name.to_string()
    }
}

/// Plain text: indentation with tabs, no markers.
pub fn to_plain_text(items: &[Item]) -> String {
    fn walk(it: &Item, depth: usize, out: &mut String) {
        out.push_str(&"\t".repeat(depth));
        out.push_str(&crate::markup::plain_text(&it.text));
        out.push('\n');
        for k in &it.children {
            walk(k, depth + 1, out);
        }
    }
    let mut out = String::new();
    for it in items {
        walk(it, 0, &mut out);
    }
    out
}

fn strip_marker(s: &str) -> Option<&str> {
    for m in ["- ", "* ", "+ ", "• "] {
        if let Some(rest) = s.strip_prefix(m) {
            return Some(rest);
        }
    }
    if s == "-" || s == "*" {
        return Some("");
    }
    // "1. " / "12) "
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits < 4 {
        let rest = &s[digits..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(r);
        }
    }
    None
}

/// Parse indented text or Markdown. If any line has a list marker, lines
/// without one that are indented under an item become its note; otherwise
/// every line is an item and indentation alone gives the structure.
pub fn parse_text(text: &str) -> Vec<Item> {
    let lines: Vec<(usize, &str)> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut indent = 0;
            for c in l.chars() {
                match c {
                    ' ' => indent += 1,
                    '\t' => indent += 4,
                    _ => break,
                }
            }
            (indent, l.trim())
        })
        .collect();
    let has_markers = lines.iter().any(|(_, l)| strip_marker(l).is_some());
    let min = lines.iter().map(|(i, _)| *i).min().unwrap_or(0);

    // Build with a stack of (indent, path into the tree).
    let mut roots: Vec<Item> = Vec::new();
    let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();
    fn at<'a>(roots: &'a mut [Item], path: &[usize]) -> &'a mut Item {
        let mut cur = &mut roots[path[0]];
        for &i in &path[1..] {
            cur = &mut cur.children[i];
        }
        cur
    }
    for (indent, line) in lines {
        let indent = indent - min;
        let (is_item, content, numbered) = match strip_marker(line) {
            Some(rest) => (true, rest, line.starts_with(|c: char| c.is_ascii_digit())),
            None if line.starts_with('#') => (true, line, false),
            None => (!has_markers, line, false),
        };
        if !is_item {
            if let Some((item_indent, path)) = stack.last() {
                if indent > *item_indent || has_markers {
                    let item = at(&mut roots, path);
                    if !item.note.is_empty() {
                        item.note.push('\n');
                    }
                    item.note.push_str(line);
                    continue;
                }
            }
        }
        let mut item = parse_line(content);
        if numbered && item.kind == BlockKind::Bullet {
            item.kind = BlockKind::Numbered;
        }
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            stack.pop();
        }
        let path = match stack.last() {
            Some((_, parent_path)) => {
                let parent = at(&mut roots, parent_path);
                parent.children.push(item);
                let mut p = parent_path.clone();
                p.push(parent.children.len() - 1);
                p
            }
            None => {
                roots.push(item);
                vec![roots.len() - 1]
            }
        };
        stack.push((indent, path));
    }
    roots
}

fn parse_line(s: &str) -> Item {
    let mut it = Item::default();
    // `![caption](path)` alone on the line: an image (the app reads the file).
    if let Some(rest) = s.strip_prefix("![") {
        if let Some((caption, after)) = rest.split_once("](") {
            if let Some(path) = after.strip_suffix(')').filter(|p| !p.is_empty() && !p.contains(char::is_whitespace)) {
                it.kind = BlockKind::Image;
                it.text = caption.to_string();
                it.image = path.to_string();
                return it;
            }
        }
    }
    let mut s = s;
    for (prefix, kind) in [("### ", BlockKind::H3), ("## ", BlockKind::H2), ("# ", BlockKind::H1), ("> ", BlockKind::Quote)] {
        if let Some(rest) = s.strip_prefix(prefix) {
            it.kind = kind;
            s = rest;
            break;
        }
    }
    if let Some(rest) = s.strip_prefix("[ ] ") {
        it.kind = BlockKind::Todo;
        s = rest;
    } else if let Some(rest) = s.strip_prefix("[x] ").or_else(|| s.strip_prefix("[X] ")) {
        it.kind = BlockKind::Todo;
        it.completed = true;
        s = rest;
    }
    if s.len() > 4 && s.starts_with("~~") && s.ends_with("~~") && !s[2..s.len() - 2].contains("~~") {
        it.completed = true;
        s = &s[2..s.len() - 2];
    }
    it.text = s.to_string();
    it
}
