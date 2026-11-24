//! Import and export: indented text / Markdown and OPML (including
//! Workflowy's OPML exports). Everything goes through `Item`, a detached
//! subtree with optional Flowie OPML identity.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::markup::{rewrite_links, LinkEdit};
use crate::tree::{BlockKind, Node, NodeId, Tree, ROOT};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    #[serde(skip)]
    pub id: Option<NodeId>,
    #[serde(skip)]
    pub mirror_of: Option<NodeId>,
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
            ..Default::default()
        }
    }

    pub fn to_opml_items(&self, ids: &[NodeId]) -> Vec<Item> {
        fn build(tree: &Tree, id: NodeId) -> Item {
            let node = tree.node(id);
            let shown = tree.node(tree.source(id));
            Item {
                id: Some(node.id),
                mirror_of: (node.mirror != ROOT).then_some(node.mirror),
                text: shown.text.clone(),
                note: shown.note.clone(),
                completed: shown.completed,
                kind: node.kind,
                collapsed: node.collapsed,
                children: tree.children(id).iter().map(|&child| build(tree, child)).collect(),
                image: String::new(),
            }
        }
        let selected: HashSet<_> = ids.iter().map(|&id| self.resolve(id)).collect();
        let mut emitted = HashSet::new();
        ids.iter()
            .map(|&id| self.resolve(id))
            .filter(|&id| {
                id != ROOT && self.contains(id) && emitted.insert(id) && !self.ancestors(id).iter().any(|parent| selected.contains(parent))
            })
            .map(|id| build(self, id))
            .collect()
    }

    /// Insert items at `index` under `parent`; returns the new top-level ids.
    pub fn insert_items(&mut self, parent: NodeId, index: usize, items: &[Item]) -> Vec<NodeId> {
        let mut roots = Vec::new();
        let parent = self.source(parent);
        let mut stack: Vec<_> = items.iter().enumerate().rev().map(|(offset, item)| (parent, index + offset, item)).collect();
        let mut imported = Vec::new();
        let mut mapped = HashMap::new();
        let mut ambiguous = HashSet::new();
        while let Some((under, position, item)) = stack.pop() {
            let original = item.id.filter(|id| *id != ROOT && !id.is_virtual());
            let id = original.filter(|id| !self.contains(*id)).unwrap_or_else(|| self.alloc_id());
            if let Some(original) = original {
                if mapped.insert(original, id).is_some() {
                    ambiguous.insert(original);
                }
            }
            let mut node = Node::new(id, under, self.rank_at(under, position));
            node.text = item.text.clone();
            node.note = item.note.clone();
            node.completed = item.completed;
            node.kind = item.kind;
            node.collapsed = item.collapsed && (!item.children.is_empty() || item.mirror_of.is_some());
            if item.kind == BlockKind::Image {
                node.image = item.image.clone();
            }
            self.put(node);
            if under == parent {
                roots.push(id);
            }
            imported.push((id, item.mirror_of));
            stack.extend(item.children.iter().enumerate().rev().map(|(offset, child)| (id, offset, child)));
        }
        let mirrors: HashSet<_> = imported.iter().filter(|(_, source)| source.is_some()).map(|(id, _)| *id).collect();
        let remap = |target: NodeId| mapped.get(&target).copied().unwrap_or(target);
        for (id, mirror_of) in imported {
            let source = mirror_of.filter(|source| !ambiguous.contains(source)).map(remap).filter(|&source| {
                source != ROOT
                    && source != id
                    && !source.is_virtual()
                    && !mirrors.contains(&source)
                    && self.contains(source)
                    && self.node(source).mirror == ROOT
                    && !self.has_children(id)
                    && !self.is_ancestor(source, id)
            });
            if source.is_none() && mapped.is_empty() {
                continue;
            }
            let mut node = self.node(id).clone();
            if let Some(source) = source {
                node.mirror = source;
                node.text.clear();
                node.note.clear();
                node.image.clear();
                node.kind = self.node(source).kind;
            } else {
                let relink = |text: &str| {
                    rewrite_links(text, |url| {
                        let Some(target) = url.strip_prefix("flowie:").and_then(|value| value.parse().ok()) else { return LinkEdit::Keep };
                        if ambiguous.contains(&target) {
                            return LinkEdit::Unlink;
                        }
                        match mapped.get(&target) {
                            Some(&new) if target != new => LinkEdit::Url(format!("flowie:{new}")),
                            _ => LinkEdit::Keep,
                        }
                    })
                };
                if node.kind != BlockKind::Code {
                    node.text = relink(&node.text);
                }
                node.note = relink(&node.note);
            }
            self.put(node);
        }
        roots
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

const OPML_NAMESPACE: &str = "https://github.com/wesleyel/flowie/ns/opml/1";

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#9;"),
            '\r' => out.push_str("&#13;"),
            '\0'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => out.push('\u{fffd}'),
            _ => out.push(c),
        }
    }
    out
}

pub fn to_opml(title: &str, items: &[Item]) -> String {
    fn walk(it: &Item, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth + 2);
        out.push_str(&format!("{pad}<outline text=\"{}\"", xml_escape(&it.text)));
        if let Some(id) = it.id {
            out.push_str(&format!(" flowie:id=\"{id}\""));
        }
        if let Some(source) = it.mirror_of {
            out.push_str(&format!(" flowie:mirrorOf=\"{source}\""));
        }
        if !it.note.is_empty() {
            out.push_str(&format!(" flowie:note=\"{}\"", xml_escape(&it.note)));
        }
        if it.completed {
            out.push_str(" flowie:complete=\"true\"");
        }
        if it.collapsed {
            out.push_str(" flowie:collapsed=\"true\"");
        }
        if it.kind != BlockKind::Bullet {
            out.push_str(&format!(" flowie:kind=\"{}\"", it.kind.name()));
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
    let mut out =
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\" xmlns:flowie=\"{OPML_NAMESPACE}\">\n  <head>\n");
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
    fn children(parent: roxmltree::Node<'_, '_>) -> Vec<Item> {
        parent
            .children()
            .filter(|node| node.has_tag_name("outline") && node.tag_name().namespace().is_none())
            .map(|node| {
                let extension = |name| node.attribute((OPML_NAMESPACE, name));
                let identity =
                    |name| extension(name).and_then(|value| value.parse::<NodeId>().ok()).filter(|id| *id != ROOT && !id.is_virtual());
                let raw = node.attribute("text").unwrap_or_default();
                let text = if node.namespaces().any(|namespace| namespace.uri() == OPML_NAMESPACE) {
                    raw.to_owned()
                } else {
                    html_to_markup(raw).replace('\n', " ")
                };
                Item {
                    text,
                    id: identity("id"),
                    mirror_of: identity("mirrorOf"),
                    note: extension("note")
                        .map(str::to_owned)
                        .unwrap_or_else(|| html_to_markup(node.attribute("_note").unwrap_or_default())),
                    kind: extension("kind").or_else(|| node.attribute("_flowieKind")).and_then(BlockKind::from_name).unwrap_or_default(),
                    completed: extension("complete").or_else(|| node.attribute("_complete")) == Some("true"),
                    collapsed: extension("collapsed") == Some("true"),
                    children: children(node),
                    ..Item::default()
                }
            })
            .collect()
    }
    let Ok(document) = roxmltree::Document::parse(xml.trim_start()) else { return Vec::new() };
    let root = document.root_element();
    if root.tag_name().name() == "opml" {
        if let Some(body) = root.children().find(|node| node.has_tag_name("body") && node.tag_name().namespace().is_none()) {
            return children(body);
        }
    }
    Vec::new()
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
