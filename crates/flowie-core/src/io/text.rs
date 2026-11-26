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

/// A Markdown heading line (`# Title` to `###### Title`, no list marker): its
/// level and title.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let title = line[level..].strip_prefix([' ', '\t'])?;
    (1..=6).contains(&level).then_some((level, title.trim()))
}

/// Parse indented text or Markdown.
///
/// - Headings (`#` to `######`, without a list marker) open sections: what
///   follows, down to the next heading of the same or a higher level, goes
///   under them. Levels below 3 become H3 rows.
/// - If any line has a list marker, a line without one is a continuation of
///   the item above (its note) when it's indented under it or follows it with
///   no blank line in between; otherwise it starts a paragraph row, which the
///   lines right after it join.
/// - Without list markers every line is an item and indentation alone gives
///   the structure.
pub fn parse_text(text: &str) -> Vec<Item> {
    // (indent, trimmed line, blank line before it)
    let mut lines: Vec<(usize, &str, bool)> = Vec::new();
    let mut blank = false;
    for l in text.lines() {
        if l.trim().is_empty() {
            blank = true;
            continue;
        }
        let mut indent = 0;
        for c in l.chars() {
            match c {
                ' ' => indent += 1,
                '\t' => indent += 4,
                _ => break,
            }
        }
        lines.push((indent, l.trim(), std::mem::take(&mut blank)));
    }
    let has_markers = lines.iter().any(|(_, l, _)| strip_marker(l).is_some());
    let min = lines.iter().map(|(i, _, _)| *i).min().unwrap_or(0);

    // Items hang from `roots` by paths of child indices. `sections` are the open
    // headings (level, path); `stack` the list items (indent, path) inside the
    // innermost one, whose children new top-level items join.
    let mut roots: Vec<Item> = Vec::new();
    let mut sections: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();
    // The paragraph row the last line went into, which a following line joins.
    let mut paragraph: Option<Vec<usize>> = None;
    fn at<'a>(roots: &'a mut [Item], path: &[usize]) -> &'a mut Item {
        let mut cur = &mut roots[path[0]];
        for &i in &path[1..] {
            cur = &mut cur.children[i];
        }
        cur
    }
    fn attach(roots: &mut Vec<Item>, parent: Option<&[usize]>, item: Item) -> Vec<usize> {
        match parent {
            Some(parent_path) => {
                let parent = at(roots, parent_path);
                parent.children.push(item);
                let mut p = parent_path.to_vec();
                p.push(parent.children.len() - 1);
                p
            }
            None => {
                roots.push(item);
                vec![roots.len() - 1]
            }
        }
    }
    for (indent, line, blank_before) in lines {
        let indent = indent - min;
        if strip_marker(line).is_none() {
            if let Some((level, title)) = heading(line) {
                while sections.last().is_some_and(|(l, _)| *l >= level) {
                    sections.pop();
                }
                let mut item = parse_line(title);
                item.kind = [BlockKind::H1, BlockKind::H2, BlockKind::H3][level.min(3) - 1];
                let path = attach(&mut roots, sections.last().map(|(_, p)| p.as_slice()), item);
                sections.push((level, path));
                stack.clear();
                paragraph = None;
                continue;
            }
        }
        let (is_item, content, numbered) = match strip_marker(line) {
            Some(rest) => (true, rest, line.starts_with(|c: char| c.is_ascii_digit())),
            None => (!has_markers, line, false),
        };
        if !is_item {
            if let Some(path) = paragraph.as_ref().filter(|_| !blank_before) {
                let item = at(&mut roots, path);
                item.text.push(' ');
                item.text.push_str(line);
                continue;
            }
            if let Some((item_indent, path)) = stack.last() {
                if indent > *item_indent || !blank_before {
                    let item = at(&mut roots, path);
                    if !item.note.is_empty() {
                        item.note.push('\n');
                    }
                    item.note.push_str(line);
                    continue;
                }
            }
            // A paragraph between lists: a row of its own, and the end of the list.
            let mut item = parse_line(content);
            if item.kind == BlockKind::Bullet {
                item.kind = BlockKind::Paragraph;
            }
            stack.clear();
            paragraph = Some(attach(&mut roots, sections.last().map(|(_, p)| p.as_slice()), item));
            continue;
        }
        let mut item = parse_line(content);
        if numbered && item.kind == BlockKind::Bullet {
            item.kind = BlockKind::Numbered;
        }
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            stack.pop();
        }
        let parent = stack.last().or(sections.last()).map(|(_, p)| p.clone());
        let path = attach(&mut roots, parent.as_deref(), item);
        stack.push((indent, path));
        paragraph = None;
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
    // Flowie has three heading levels: deeper ones become H3.
    for (prefix, kind) in [
        ("###### ", BlockKind::H3),
        ("##### ", BlockKind::H3),
        ("#### ", BlockKind::H3),
        ("### ", BlockKind::H3),
        ("## ", BlockKind::H2),
        ("# ", BlockKind::H1),
        ("> ", BlockKind::Quote),
    ] {
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
