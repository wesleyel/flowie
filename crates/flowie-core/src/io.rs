//! Import and export: indented text / Markdown and OPML (including
//! Workflowy's OPML exports). Everything goes through `Item`, a detached
//! subtree without ids.

use serde::{Deserialize, Serialize};

use crate::tree::{BlockKind, NodeId, Tree};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub text: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub kind: BlockKind,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub children: Vec<Item>,
    /// An image: its stored name, or (from Markdown) the path it was written with.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
}

impl Item {
    pub fn new(text: &str) -> Item {
        Item { text: text.to_string(), ..Default::default() }
    }

    pub fn count(items: &[Item]) -> usize {
        items.iter().map(|i| 1 + Item::count(&i.children)).sum()
    }
}

impl Tree {
    /// Detached copies of `ids` and their subtrees. Mirrors are resolved to
    /// their source's content.
    pub fn to_items(&self, ids: &[NodeId]) -> Vec<Item> {
        ids.iter().filter(|id| self.contains(**id)).map(|&id| self.item(id, 0)).collect()
    }

    fn item(&self, id: NodeId, guard: usize) -> Item {
        let n = self.node(id);
        let src = self.source(id);
        let s = self.node(src);
        let children = if guard > 200 { Vec::new() } else { self.children(src).iter().map(|&c| self.item(c, guard + 1)).collect() };
        Item {
            text: s.text.clone(),
            note: s.note.clone(),
            completed: s.completed,
            kind: n.kind,
            collapsed: n.collapsed,
            children,
            image: s.image.clone(),
        }
    }

    /// Insert items at `index` under `parent`; returns the new top-level ids.
    pub fn insert_items(&mut self, parent: NodeId, index: usize, items: &[Item]) -> Vec<NodeId> {
        let mut top = Vec::new();
        for (i, item) in items.iter().enumerate() {
            top.push(self.insert_item(parent, index + i, item));
        }
        top
    }

    fn insert_item(&mut self, parent: NodeId, index: usize, item: &Item) -> NodeId {
        let id = self.create_at(parent, index, &item.text);
        let mut node = self.node(id).clone();
        node.note = item.note.clone();
        node.completed = item.completed;
        node.kind = item.kind;
        node.collapsed = item.collapsed && !item.children.is_empty();
        node.image = if item.kind == BlockKind::Image { item.image.clone() } else { String::new() };
        self.put(node);
        for (i, kid) in item.children.iter().enumerate() {
            self.insert_item(id, i, kid);
        }
        id
    }
}

// ------------------------------------------------------------------ text

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

// ------------------------------------------------------------------ OPML

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn to_opml(title: &str, items: &[Item]) -> String {
    fn walk(it: &Item, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth + 2);
        out.push_str(&format!("{pad}<outline text=\"{}\"", xml_escape(&it.text)));
        if !it.note.is_empty() {
            out.push_str(&format!(" _note=\"{}\"", xml_escape(&it.note)));
        }
        if it.completed {
            out.push_str(" _complete=\"true\"");
        }
        if it.kind != BlockKind::Bullet {
            out.push_str(&format!(" _flowieKind=\"{}\"", it.kind.name()));
        }
        if it.children.is_empty() {
            out.push_str("/>\n");
        } else {
            out.push_str(">\n");
            for k in &it.children {
                walk(k, depth + 1, out);
            }
            out.push_str(&format!("{pad}</outline>\n"));
        }
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n  <head>\n");
    out.push_str(&format!("    <title>{}</title>\n  </head>\n  <body>\n", xml_escape(title)));
    for it in items {
        walk(it, 0, &mut out);
    }
    out.push_str("  </body>\n</opml>\n");
    out
}

fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let Some(end) = rest.find(';').filter(|e| *e < 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_attrs(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim().rsplit(|c: char| c.is_whitespace()).next().unwrap_or("").to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let body = &after[1..];
        let Some(close) = body.find(q) else { break };
        out.push((name, xml_unescape(&body[..close])));
        rest = &body[close + 1..];
    }
    out
}

/// Workflowy puts light HTML in OPML text; turn it into Flowie markup.
fn html_to_markup(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    let mut link_stack: Vec<String> = Vec::new();
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            out.push_str(&rest[lt..]);
            rest = "";
            break;
        };
        let tag = &rest[lt + 1..lt + gt];
        let lower = tag.to_ascii_lowercase();
        let name = lower.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_string();
        let closing = lower.starts_with('/');
        match name.as_str() {
            "b" | "strong" => out.push_str("**"),
            "i" | "em" => out.push('*'),
            "s" | "strike" | "del" => out.push_str("~~"),
            "code" => out.push('`'),
            "mark" => out.push_str("=="),
            "a" if !closing => {
                let href = parse_attrs(tag).into_iter().find(|(k, _)| k == "href").map(|(_, v)| v).unwrap_or_default();
                out.push('[');
                link_stack.push(href);
            }
            "a" => {
                let href = link_stack.pop().unwrap_or_default();
                out.push_str(&format!("]({href})"));
            }
            "br" => out.push(' '),
            "span" | "u" | "font" => {}
            // Not markup we know: keep it literally.
            _ => out.push_str(&rest[lt..lt + gt + 1]),
        }
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    out
}

pub fn parse_opml(xml: &str) -> Vec<Item> {
    let body = xml.find("<body").map(|i| &xml[i..]).unwrap_or(xml);
    let mut roots: Vec<Item> = Vec::new();
    let mut stack: Vec<Item> = Vec::new();
    let mut rest = body;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if let Some(t) = tag.strip_prefix("outline") {
            let self_closing = t.trim_end().ends_with('/');
            let attrs = parse_attrs(t.trim_end_matches('/'));
            let mut it = Item::default();
            for (k, v) in attrs {
                match k.as_str() {
                    "text" => it.text = html_to_markup(&v).replace('\n', " "),
                    "_note" => it.note = html_to_markup(&v),
                    "_complete" => it.completed = v == "true",
                    "_flowieKind" => it.kind = BlockKind::from_name(&v).unwrap_or_default(),
                    _ => {}
                }
            }
            if self_closing {
                match stack.last_mut() {
                    Some(p) => p.children.push(it),
                    None => roots.push(it),
                }
            } else {
                stack.push(it);
            }
        } else if tag.starts_with("/outline") {
            if let Some(done) = stack.pop() {
                match stack.last_mut() {
                    Some(p) => p.children.push(done),
                    None => roots.push(done),
                }
            }
        }
    }
    while let Some(done) = stack.pop() {
        match stack.last_mut() {
            Some(p) => p.children.push(done),
            None => roots.push(done),
        }
    }
    roots
}

/// Guess the format from the content.
pub fn parse_any(content: &str) -> Vec<Item> {
    let head = content.trim_start();
    if head.starts_with("<?xml") || head.starts_with("<opml") {
        parse_opml(content)
    } else {
        parse_text(content)
    }
}
