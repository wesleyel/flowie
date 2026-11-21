//! The tutorial outline: seeded on first launch and reopened from
//! Help → Flowie Tutorial. Written as Markdown lists in `tutorial/<locale>.md`
//! with a few directives in `{{…}}` at the end of a line:
//!
//! - `{{#name}}` names the bullet; `[label](flowie:@name)` links to it
//! - `{{mirror name}}` (alone on a bullet) becomes a mirror of that bullet
//! - `{{collapsed}}`, `{{text}}`, `{{code}}`, `{{star}}`
//! - `{{today}}` is replaced with today's date

use std::collections::HashMap;

use flowie_core::{io::parse_text, BlockKind, NodeId, ROOT};
use gpui::{Context, Window};

use crate::outline::{OpenTutorial, Outline};

const TUTORIALS: &[(&str, &str)] = &[("en", include_str!("../tutorial/en.md")), ("zh-CN", include_str!("../tutorial/zh-CN.md"))];

/// The saved search the "Finding things" lesson points at.
const SAVED_SEARCH: &str = "is:todo is:open";

fn source() -> &'static str {
    let code = crate::i18n::active();
    TUTORIALS.iter().find(|(c, _)| *c == code).unwrap_or(&TUTORIALS[0]).1
}

fn adapt_for_platform(text: String) -> String {
    if cfg!(target_os = "macos") {
        return text;
    }
    adapt_non_macos(text)
}

fn adapt_non_macos(text: String) -> String {
    let exe = std::env::current_exe()
        .ok()
        .map(|p| {
            let s = p.display().to_string();
            if s.contains(' ') {
                format!("\"{s}\"")
            } else {
                s
            }
        })
        .unwrap_or_else(|| if cfg!(target_os = "windows") { "Flowie.exe".into() } else { "flowie".into() });
    text.replace("⌘⌥", "Ctrl+Alt+")
        .replace("⌘⇧", "Ctrl+Shift+")
        .replace(['⌘', '⌃'], "Ctrl+")
        .replace('⌥', "Alt")
        .replace("on your Mac only", "on this computer only")
        .replace("你的 Mac 上", "这台电脑上")
        .replace("/Applications/Flowie.app/Contents/MacOS/Flowie", &exe)
}

impl Outline {
    /// Insert the tutorial at `index` under Home; returns its top bullet.
    /// Not committed: the caller decides whether it is an undo step.
    pub fn insert_tutorial(&mut self, index: usize) -> NodeId {
        let text = adapt_for_platform(source().replace("{{today}}", &flowie_core::search::today()));
        let items = parse_text(&text);
        let top = self.tree.insert_items(ROOT, index, &items);
        let mut all: Vec<NodeId> = Vec::new();
        for &t in &top {
            all.push(t);
            all.extend(self.tree.descendants(t));
        }
        // Pass 1: directives on each bullet.
        let mut names: HashMap<String, NodeId> = HashMap::new();
        let mut mirrors: Vec<(NodeId, String)> = Vec::new();
        for &id in &all {
            let mut node = self.tree.node(id).clone();
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
                        if !self.starred.contains(&id) {
                            self.starred.push(id);
                        }
                    }
                    d => {
                        if let Some(name) = d.strip_prefix('#') {
                            names.insert(name.to_string(), id);
                        } else if let Some(name) = d.strip_prefix("mirror ") {
                            mirrors.push((id, name.trim().to_string()));
                        }
                    }
                }
            }
            node.text = text.trim_end().to_string();
            self.tree.put(node);
        }
        // Pass 2: links and mirrors to named bullets.
        for &id in &all {
            let text = &self.tree.node(id).text;
            if text.contains("(flowie:@") {
                let mut text = text.clone();
                for (name, target) in &names {
                    text = text.replace(&format!("(flowie:@{name})"), &format!("(flowie:{target})"));
                }
                self.tree.set_text(id, &text);
            }
        }
        for (id, name) in mirrors {
            let Some(&src) = names.get(&name) else { continue };
            let (parent, at) = (self.tree.parent(id), self.tree.index_in_parent(id));
            self.tree.delete(id);
            self.tree.create_mirror(src, parent, at);
        }
        let starred = self.starred.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(",");
        self.set_meta("starred", &starred);
        if !self.saved_searches.iter().any(|s| s == SAVED_SEARCH) {
            self.saved_searches.push(SAVED_SEARCH.to_string());
            self.set_meta("saved_searches", &self.saved_searches.join("\n"));
        }
        top.first().copied().unwrap_or(ROOT)
    }

    /// Help → Flowie Tutorial: a fresh copy at the end of Home, zoomed into.
    pub fn open_tutorial(&mut self, _: &OpenTutorial, _: &mut Window, cx: &mut Context<Self>) {
        let before = None;
        let id = self.insert_tutorial(self.tree.children(ROOT).len());
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
            for (i, _) in md.match_indices("{{mirror ") {
                let rest = &md[i + 9..];
                let name = &rest[..rest.find("}}").unwrap()];
                assert!(names.contains(&name), "{code}: mirror of unknown {name}");
            }
        }
        assert_eq!(TUTORIALS[0].1.lines().count(), TUTORIALS[1].1.lines().count(), "translations have the same lines");
    }

    #[test]
    fn non_macos_tutorial_has_no_macos_glyphs() {
        for (code, md) in TUTORIALS {
            let adapted = super::adapt_non_macos((*md).to_string());
            for ch in ['⌘', '⌃', '⌥', '⇧'] {
                assert!(!adapted.contains(ch), "{code}: adapted tutorial still contains {ch}");
            }
            assert!(!adapted.contains("on your Mac"), "{code}: adapted tutorial still mentions Mac");
            assert!(!adapted.contains("你的 Mac"), "{code}: adapted tutorial still mentions Mac");
            assert!(!adapted.contains("/Applications/Flowie.app"), "{code}: adapted tutorial still has macOS app path");
        }
    }
}
