//! OPML import and export, including Workflowy HTML and Flowie extensions.

use super::Item;
use crate::{BlockKind, NodeId, ROOT};

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
