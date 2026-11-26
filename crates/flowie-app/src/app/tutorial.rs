//! The tutorial outline: seeded on first launch and reopened from
//! Help → Flowie Tutorial. Written as Markdown lists in `tutorial/<locale>.md`
//! with a few directives in `{{…}}` at the end of a line:
//!
//! - `{{#name}}` names the bullet; `[label](flowie:@name)` links to it
//! - `{{mirror name}}` (alone on a bullet) becomes a mirror of that bullet
//! - `{{collapsed}}`, `{{text}}`, `{{code}}`, `{{star}}`
//! - `{{image <name>}}` makes the bullet an image bullet showing `IMAGES[name]`,
//!   with the bullet's text as its caption
//!
//! and a few placeholders anywhere, filled in before parsing:
//!
//! - `{{key:<shortcut id>}}`: that shortcut's current keys, as this platform writes them
//! - `{{mod:<modifier>}}`: a modifier's name ("⌥" on macOS, "Alt" elsewhere)
//! - `{{mcp-command}}`: the `claude mcp add` command for this install
//! - `{{today}}`: today's date

use std::collections::HashMap;

use flowie_core::{io::parse_text, BlockKind, NodeId, ROOT};
use gpui::{Context, Window};

use crate::outline::{OpenTutorial, Outline};

const TUTORIALS: &[(&str, &str)] = &[("en", include_str!("../../tutorial/en.md")), ("zh-CN", include_str!("../../tutorial/zh-CN.md"))];

/// Pictures the tutorial shows, by `{{image <name>}}` name.
const IMAGES: &[(&str, &[u8])] = &[("icon", include_bytes!("../../../../assets/icon.png"))];

/// The saved search the "Finding things" lesson points at.
const SAVED_SEARCH: &str = "is:todo is:open";

/// The meta key holding the id of the tutorial's top bullet on this device.
pub const TUTORIAL_META: &str = "tutorial";

fn source() -> &'static str {
    let code = crate::app::i18n::active();
    TUTORIALS.iter().find(|(c, _)| *c == code).unwrap_or(&TUTORIALS[0]).1
}

/// Fill the placeholders of a tutorial source. Unknown ones are left for
/// the directive pass (and caught by the tests).
fn render(src: &str, keys: &HashMap<String, Vec<String>>, mcp_command: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { break };
        let inner = &rest[start + 2..start + len];
        let value = match inner.split_once(':') {
            Some(("key", id)) => Some(crate::app::keymap::hint(id, keys)),
            Some(("mod", m)) => Some(crate::app::platform::modifier_name(m).to_string()),
            _ => match inner {
                "mcp-command" => Some(mcp_command.to_string()),
                "today" => Some(flowie_core::search::today()),
                _ => None,
            },
        };
        out.push_str(&rest[..start]);
        match value {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[start..start + len + 2]),
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

impl Outline {
    /// Insert the tutorial at `index` under Home; returns its top bullet.
    /// Not committed: the caller decides whether it is an undo step.
    pub fn insert_tutorial(&mut self, index: usize) -> NodeId {
        let mcp_command = crate::agent::mcp::Setup::new(self.db_path.as_deref()).claude_command();
        let text = render(source(), &self.config.keys, &mcp_command);
        let items = parse_text(&text);
        let top = self.ws.tree.insert_items(ROOT, index, &items);
        let mut all: Vec<NodeId> = Vec::new();
        for &t in &top {
            all.push(t);
            all.extend(self.ws.tree.descendants(t));
        }
        // Pass 1: directives on each bullet.
        let mut names: HashMap<String, NodeId> = HashMap::new();
        let mut mirrors: Vec<(NodeId, String)> = Vec::new();
        for &id in &all {
            let mut node = self.ws.tree.node(id).clone();
            let mut text = node.text.clone();
            while let Some(start) = text.rfind("{{") {
                let Some(len) = text[start..].find("}}") else { break };
                let directive = text[start + 2..start + len].trim().to_string();
                text.replace_range(start..start + len + 2, "");
                match directive.as_str() {
                    "collapsed" => node.collapsed = true,
                    "text" => node.kind = BlockKind::Paragraph,
                    "code" => node.kind = BlockKind::Code,
                    "star" => {
                        if node.star.is_empty() {
                            let last = self.ws.tree.starred().last().map(|s| self.ws.tree.node(s).star.clone());
                            node.star = flowie_core::rank::between(last.as_deref(), None);
                        }
                    }
                    d => {
                        if let Some(name) = d.strip_prefix('#') {
                            names.insert(name.to_string(), id);
                        } else if let Some(name) = d.strip_prefix("mirror ") {
                            mirrors.push((id, name.trim().to_string()));
                        } else if let Some(name) = d.strip_prefix("image ") {
                            // Without an image folder (or on a write error) it stays a plain bullet.
                            let bytes = IMAGES.iter().find(|(n, _)| *n == name.trim()).map(|(_, b)| *b);
                            if let Some(stored) = bytes.and_then(|b| self.store_image(b).ok()) {
                                node.kind = BlockKind::Image;
                                node.image = stored;
                            }
                        }
                    }
                }
            }
            node.text = text.trim_end().to_string();
            self.ws.tree.put(node);
        }
        // Pass 2: links and mirrors to named bullets.
        for &id in &all {
            let text = &self.ws.tree.node(id).text;
            if text.contains("(flowie:@") {
                let mut text = text.clone();
                for (name, target) in &names {
                    text = text.replace(&format!("(flowie:@{name})"), &format!("(flowie:{target})"));
                }
                self.ws.tree.set_text(id, &text);
            }
        }
        for (id, name) in mirrors {
            let Some(&src) = names.get(&name) else { continue };
            let (parent, at) = (self.ws.tree.parent(id), self.ws.tree.index_in_parent(id));
            self.ws.tree.delete(id);
            self.ws.tree.create_mirror(src, parent, at);
        }
        if !self.saved_searches.iter().any(|s| s == SAVED_SEARCH) {
            self.saved_searches.push(SAVED_SEARCH.to_string());
            self.save_searches();
        }
        let root = top.first().copied().unwrap_or(ROOT);
        // The tutorial keeps random ids (its text depends on the language and the
        // platform's shortcuts); this device remembers its latest copy by id, so a
        // first sync can tell an untouched tutorial from the user's own outline.
        self.set_meta(TUTORIAL_META, &root.to_string());
        self.tutorial = Some(root);
        root
    }

    /// Help → Flowie Tutorial: a fresh copy at the end of Home, zoomed into.
    pub fn open_tutorial(&mut self, _: &OpenTutorial, _: &mut Window, cx: &mut Context<Self>) {
        let before = None;
        let id = self.insert_tutorial(self.ws.tree.children(ROOT).len());
        self.commit_structure(before, cx);
        self.zoom_to(id, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::TUTORIALS;

    /// Every locale's tutorial parses into one top bullet, and its links and
    /// mirrors name bullets that exist.
    #[test]
    fn tutorials_are_well_formed() {
        for (code, md) in TUTORIALS {
            let items = flowie_core::io::parse_text(md);
            assert_eq!(items.len(), 1, "{code}: one top bullet");
            let names: Vec<&str> = md.match_indices("{{#").map(|(i, _)| &md[i + 3..i + 3 + md[i + 3..].find("}}").unwrap()]).collect();
            for (i, _) in md.match_indices("(flowie:@") {
                let rest = &md[i + 9..];
                let name = &rest[..rest.find(')').unwrap()];
                assert!(names.contains(&name), "{code}: link to unknown {name}");
            }
            for (i, _) in md.match_indices("{{image ") {
                let rest = &md[i + 8..];
                let name = &rest[..rest.find("}}").unwrap()];
                assert!(super::IMAGES.iter().any(|(n, _)| *n == name), "{code}: unknown image {name}");
            }
            for (i, _) in md.match_indices("{{mirror ") {
                let rest = &md[i + 9..];
                let name = &rest[..rest.find("}}").unwrap()];
                assert!(names.contains(&name), "{code}: mirror of unknown {name}");
            }
        }
        assert_eq!(TUTORIALS[0].1.lines().count(), TUTORIALS[1].1.lines().count(), "translations have the same lines");
    }

    #[test]
    fn placeholders_name_real_shortcuts_and_all_get_filled() {
        let ids: std::collections::HashSet<&str> = crate::app::keymap::BINDINGS.iter().map(|b| b.id).collect();
        for (code, md) in TUTORIALS {
            let mut rest = *md;
            while let Some(at) = rest.find("{{key:") {
                let id = &rest[at + 6..at + rest[at..].find("}}").unwrap()];
                assert!(ids.contains(id), "{code}: unknown shortcut id {id:?}");
                rest = &rest[at + 6..];
            }
            let text = super::render(md, &std::collections::HashMap::new(), "claude mcp add flowie -- flowie mcp");
            for leftover in ["{{key:", "{{mod:", "{{mcp-command}}", "{{today}}"] {
                assert!(!text.contains(leftover), "{code}: {leftover} left unfilled");
            }
            assert!(!text.contains("Mac") && !text.contains("/Applications"), "{code}: macOS-only wording");
        }
    }

    #[test]
    fn render_fills_keys_from_overrides() {
        let mut keys = std::collections::HashMap::new();
        keys.insert("jump".to_string(), vec!["f9".to_string()]);
        assert_eq!(super::render("Press {{key:jump}}.", &keys, ""), "Press F9.");
        assert_eq!(super::render("{{collapsed}} stays", &keys, ""), "{{collapsed}} stays");
    }
}
