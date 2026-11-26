//! Indented text and Markdown outline documents.

use super::Item;
use crate::BlockKind;

/// Markdown-compatible indented list: two spaces per level, notes as
/// indented lines without a bullet marker.
pub fn to_markdown(items: &[Item]) -> String {
    let mut out = String::new();
    write_md_list(items, 0, &mut out);
    out
}

/// A code row that can sit inside a fenced block: nothing a fence can't hold.
fn fenceable(it: &Item) -> bool {
    it.kind == BlockKind::Code && it.note.is_empty() && it.children.is_empty() && !it.completed
}

/// Siblings at `depth`, with each run of plain code rows written as one fenced
/// block, which `parse_text` reads back as code rows.
fn write_md_list(items: &[Item], depth: usize, out: &mut String) {
    let mut ix = 0;
    while ix < items.len() {
        let run = items[ix..].iter().take_while(|it| fenceable(it)).count();
        if run == 0 {
            write_md(&items[ix], depth, out);
            ix += 1;
            continue;
        }
        let lines = &items[ix..ix + run];
        // Longer than any run of backticks inside, so no line can close it early.
        let longest = lines.iter().flat_map(|it| it.text.split(|c| c != '`')).map(str::len).max().unwrap_or(0);
        let fence = "`".repeat((longest + 1).max(3));
        let pad = "  ".repeat(depth);
        out.push_str(&format!("{pad}{fence}\n"));
        for it in lines {
            if !it.text.is_empty() {
                out.push_str(&pad);
                out.push_str(&it.text);
            }
            out.push('\n');
        }
        out.push_str(&format!("{pad}{fence}\n"));
        ix += run;
    }
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
    write_md_list(&it.children, depth + 1, out);
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
        // Code is verbatim: markup characters in it are part of the code.
        if it.kind == BlockKind::Code {
            out.push_str(&it.text);
        } else {
            out.push_str(&crate::markup::plain_text(&it.text));
        }
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

fn indent_of(line: &str) -> usize {
    let mut indent = 0;
    for c in line.chars() {
        match c {
            ' ' => indent += 1,
            '\t' => indent += 4,
            _ => break,
        }
    }
    indent
}

/// Strip up to `cols` columns of leading indentation (tab = 4 columns),
/// keeping any remaining indentation verbatim.
fn strip_indent(line: &str, cols: usize) -> String {
    let mut col = 0;
    let mut rest = line;
    while col < cols {
        if let Some(r) = rest.strip_prefix(' ') {
            col += 1;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('\t') {
            col += 4;
            rest = r;
            if col > cols {
                let mut out = " ".repeat(col - cols);
                out.push_str(rest);
                return out;
            }
        } else {
            break;
        }
    }
    rest.to_string()
}

/// Opening code fence (at least 3 backticks or tildes). Backtick fences cannot
/// have backticks in their info string (which also keeps inline ```` ```x``` ````
/// from opening a block).
fn opening_fence(trimmed: &str) -> Option<(u8, usize)> {
    let ch = *trimmed.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = trimmed.bytes().take_while(|&b| b == ch).count();
    if len < 3 {
        return None;
    }
    if ch == b'`' && trimmed[len..].contains('`') {
        return None;
    }
    Some((ch, len))
}

fn is_closing_fence(trimmed: &str, ch: u8, min_len: usize) -> bool {
    trimmed.len() >= min_len && trimmed.bytes().all(|b| b == ch)
}

enum Entry<'a> {
    Line { indent: usize, text: &'a str, blank_before: bool },
    Code { indent: usize, lines: Vec<String> },
}

struct Fence {
    ch: u8,
    len: usize,
    indent: usize,
    lines: Vec<String>,
}

/// Parse indented text or Markdown.
///
/// - Headings (`#` to `######`, without a list marker) open sections: what
///   follows, down to the next heading of the same or a higher level, goes
///   under them. Levels below 3 become H3 rows.
/// - Fenced code blocks (```` ``` ```` or `~~~`) keep each line verbatim as a
///   `Code` row (indentation relative to the fence), without interpreting
///   headings or list markers inside them.
/// - If any line outside a fenced code block has a list marker or heading, a
///   line without a list marker is a continuation of the item above (its note)
///   when it's indented under it or follows it with no blank line in between;
///   otherwise it starts a paragraph row, which the lines right after it join.
/// - Without list markers or headings every non-code line is an item and
///   indentation alone gives the structure.
pub fn parse_text(text: &str) -> Vec<Item> {
    let mut entries: Vec<Entry<'_>> = Vec::new();
    let mut blank = false;
    let mut fence: Option<Fence> = None;
    for raw in text.lines() {
        let trimmed = raw.trim();
        if let Some(open) = &mut fence {
            if is_closing_fence(trimmed, open.ch, open.len) && indent_of(raw) <= open.indent + 3 {
                let indent = open.indent;
                let lines = std::mem::take(&mut open.lines);
                fence = None;
                entries.push(Entry::Code { indent, lines });
                blank = true;
            } else if trimmed.is_empty() {
                open.lines.push(String::new());
            } else {
                open.lines.push(strip_indent(raw, open.indent));
            }
            continue;
        }
        if trimmed.is_empty() {
            blank = true;
            continue;
        }
        let indent = indent_of(raw);
        if let Some((ch, len)) = opening_fence(trimmed) {
            fence = Some(Fence { ch, len, indent, lines: Vec::new() });
            blank = false;
            continue;
        }
        entries.push(Entry::Line { indent, text: trimmed, blank_before: std::mem::take(&mut blank) });
    }
    if let Some(open) = fence {
        entries.push(Entry::Code { indent: open.indent, lines: open.lines });
    }

    let markdown = entries.iter().any(|e| match e {
        Entry::Line { text, .. } => strip_marker(text).is_some() || heading(text).is_some(),
        Entry::Code { .. } => false,
    });
    let min = entries
        .iter()
        .map(|e| match e {
            Entry::Line { indent, .. } | Entry::Code { indent, .. } => *indent,
        })
        .min()
        .unwrap_or(0);

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
    for entry in entries {
        match entry {
            Entry::Code { indent, lines } => {
                let indent = indent - min;
                while stack.last().is_some_and(|(i, _)| *i >= indent) {
                    stack.pop();
                }
                let parent = stack.last().or(sections.last()).map(|(_, p)| p.clone());
                for line in lines {
                    attach(&mut roots, parent.as_deref(), Item { text: line, kind: BlockKind::Code, ..Default::default() });
                }
                paragraph = None;
            }
            Entry::Line { indent, text: line, blank_before } => {
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
                    None => (!markdown, line, false),
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
                    // A paragraph row of its own, ending any open list.
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
        }
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
