//! Every keyboard shortcut in one table, so the settings panel can list
//! and rebind them. User overrides (id -> keystrokes) come from settings.

use std::collections::HashMap;

use gpui::{App, KeyBinding, Menu, MenuItem};

use crate::i18n::{self, t};
use crate::outline::*;

pub struct Binding {
    /// Stable id used for overrides. Entries sharing an id are rebound together.
    pub id: &'static str,
    /// Group id; the heading is `group.<id>` and the label `shortcut.<id>` in the locale files.
    pub group: &'static str,
    pub keys: &'static str,
    context: Option<&'static str>,
    make: fn(&str, Option<&str>) -> KeyBinding,
}

macro_rules! b {
    ($id:literal, $group:literal, $keys:literal, $ctx:expr, $action:expr) => {
        Binding { id: $id, group: $group, keys: $keys, context: $ctx, make: |k, c| KeyBinding::new(k, $action, c) }
    };
}

const E: Option<&str> = Some("editing");
const S: Option<&str> = Some("selecting");
const O: Option<&str> = Some("Outline");

pub const GROUPS: [&str; 6] = ["editing", "formatting", "outline", "navigation", "search", "app"];

pub static BINDINGS: &[Binding] = &[
    // Editing
    b!("newline", "editing", "enter", E, Newline),
    b!("newline", "editing", "enter", S, Newline),
    b!("note", "editing", "shift-enter", E, ToggleNote),
    b!("indent", "editing", "tab", O, Indent),
    b!("outdent", "editing", "shift-tab", O, Outdent),
    b!("backspace", "editing", "backspace", O, Backspace),
    b!("delete", "editing", "delete", O, Delete),
    b!("delete-word", "editing", "alt-backspace", E, DeleteWordLeft),
    b!("left", "editing", "left", E, Left),
    b!("right", "editing", "right", E, Right),
    b!("up", "editing", "up", O, Up),
    b!("down", "editing", "down", O, Down),
    b!("word-left", "editing", "alt-left", E, WordLeft),
    b!("word-right", "editing", "alt-right", E, WordRight),
    b!("select-left", "editing", "shift-left", E, SelectLeft),
    b!("select-right", "editing", "shift-right", E, SelectRight),
    b!("select-word-left", "editing", "alt-shift-left", E, SelectWordLeft),
    b!("select-word-right", "editing", "alt-shift-right", E, SelectWordRight),
    b!("select-up", "editing", "shift-up", O, SelectUp),
    b!("select-down", "editing", "shift-down", O, SelectDown),
    b!("home", "editing", "cmd-left", E, Home),
    b!("home", "editing", "home", E, Home),
    b!("end", "editing", "cmd-right", E, End),
    b!("end", "editing", "end", E, End),
    b!("select-home", "editing", "cmd-shift-left", E, SelectHome),
    b!("select-end", "editing", "cmd-shift-right", E, SelectEnd),
    b!("select-all", "editing", "cmd-a", O, SelectAll),
    b!("copy", "editing", "cmd-c", O, Copy),
    b!("cut", "editing", "cmd-x", O, Cut),
    b!("paste", "editing", "cmd-v", O, Paste),
    b!("undo", "editing", "cmd-z", O, Undo),
    b!("redo", "editing", "cmd-shift-z", O, Redo),
    // Formatting
    b!("bold", "formatting", "cmd-b", E, Bold),
    b!("italic", "formatting", "cmd-i", E, Italic),
    b!("code", "formatting", "cmd-e", E, InlineCode),
    b!("math", "formatting", "cmd-m", E, WrapMath),
    b!("link", "formatting", "cmd-k", E, InsertLink),
    b!("highlight", "formatting", "cmd-shift-y", E, Highlight),
    b!("kind-bullet", "formatting", "cmd-alt-0", O, KindBullet),
    b!("kind-h1", "formatting", "cmd-alt-1", O, KindH1),
    b!("kind-h2", "formatting", "cmd-alt-2", O, KindH2),
    b!("kind-h3", "formatting", "cmd-alt-3", O, KindH3),
    b!("kind-text", "formatting", "cmd-alt-4", O, KindParagraph),
    b!("kind-todo", "formatting", "cmd-alt-5", O, KindTodo),
    b!("kind-numbered", "formatting", "cmd-alt-6", O, KindNumbered),
    b!("kind-quote", "formatting", "cmd-alt-7", O, KindQuote),
    b!("kind-code", "formatting", "cmd-alt-8", O, KindCode),
    // Outline
    b!("complete", "outline", "cmd-enter", O, ToggleComplete),
    b!("collapse", "outline", "cmd-up", O, Collapse),
    b!("expand", "outline", "cmd-down", O, Expand),
    b!("expand-all", "outline", "ctrl-0", O, ExpandAll),
    b!("level-1", "outline", "ctrl-1", O, Level1),
    b!("level-2", "outline", "ctrl-2", O, Level2),
    b!("level-3", "outline", "ctrl-3", O, Level3),
    b!("level-4", "outline", "ctrl-4", O, Level4),
    b!("move-up", "outline", "cmd-shift-up", O, MoveUp),
    b!("move-down", "outline", "cmd-shift-down", O, MoveDown),
    b!("move-to", "outline", "cmd-shift-m", O, MoveTo),
    b!("mirror", "outline", "cmd-alt-m", O, MirrorBullet),
    b!("link-to", "outline", "cmd-shift-k", E, LinkToBullet),
    b!("delete-bullet", "outline", "cmd-shift-backspace", O, DeleteNode),
    b!("toggle-completed", "outline", "cmd-o", O, ToggleCompletedVisible),
    // Navigation
    b!("page-back", "navigation", "mouse-back", O, PageBack),
    b!("page-back", "navigation", "cmd-alt-left", O, PageBack),
    b!("page-forward", "navigation", "mouse-forward", O, PageForward),
    b!("page-forward", "navigation", "cmd-alt-right", O, PageForward),
    b!("zoom-in", "navigation", "cmd-]", O, ZoomIn),
    b!("zoom-in", "navigation", "alt-right", S, ZoomIn),
    b!("zoom-out", "navigation", "cmd-[", O, ZoomOut),
    b!("zoom-out", "navigation", "alt-left", S, ZoomOut),
    b!("home-page", "navigation", "cmd-shift-h", O, ZoomHome),
    b!("jump", "navigation", "cmd-p", O, Jump),
    b!("star", "navigation", "cmd-shift-s", O, ToggleStar),
    b!("today", "navigation", "cmd-shift-d", O, Today),
    b!("calendar", "navigation", "cmd-shift-c", O, ShowCalendar),
    // Search
    b!("find", "search", "cmd-f", O, Find),
    b!("next-match", "search", "cmd-g", O, NextMatch),
    b!("prev-match", "search", "cmd-shift-g", O, PrevMatch),
    b!("escape", "search", "escape", O, Escape),
    // App
    b!("import", "app", "cmd-shift-i", O, Import),
    b!("export-md", "app", "cmd-shift-e", O, ExportMarkdown),
    b!("trash", "app", "cmd-shift-delete", O, ShowTrash),
    b!("settings", "app", "cmd-,", None, OpenSettings),
    b!("quit", "app", "cmd-q", None, Quit),
];

/// The keys a binding id currently uses (empty = unbound).
pub fn keys_for(id: &str, overrides: &HashMap<String, Vec<String>>) -> Vec<String> {
    if let Some(k) = overrides.get(id) {
        return k.clone();
    }
    let mut out: Vec<String> = BINDINGS.iter().filter(|b| b.id == id).map(|b| b.keys.to_string()).collect();
    out.dedup();
    out
}

/// (Re)install every binding, applying overrides. An override replaces all
/// default keys of that id, in each of its contexts.
pub fn install(cx: &mut App, overrides: &HashMap<String, Vec<String>>) {
    cx.clear_key_bindings();
    let mut out = Vec::new();
    for b in BINDINGS {
        if let Some(keys) = overrides.get(b.id) {
            let first_in_ctx = BINDINGS.iter().position(|x| x.id == b.id && x.context == b.context)
                == BINDINGS.iter().position(|x| std::ptr::eq(x, b));
            if first_in_ctx {
                for key in keys.iter().filter(|k| !k.starts_with("mouse-")) {
                    out.push((b.make)(key, b.context));
                }
            }
        } else if !b.keys.starts_with("mouse-") {
            out.push((b.make)(b.keys, b.context));
        }
    }
    // Read-only selectable labels (About panel).
    out.push(KeyBinding::new("cmd-c", Copy, Some("Selectable")));
    out.push(KeyBinding::new("cmd-a", SelectAll, Some("Selectable")));
    cx.bind_keys(out);
    set_menus(cx);
}

/// Effective (keys, context) pairs of a binding id after overrides.
fn effective(id: &str, overrides: &HashMap<String, Vec<String>>) -> Vec<(String, Option<&'static str>)> {
    BINDINGS
        .iter()
        .filter(|b| b.id == id)
        .flat_map(|b| match overrides.get(id) {
            Some(keys) => keys.iter().map(|k| (k.clone(), b.context)).collect::<Vec<_>>(),
            None => vec![(b.keys.to_string(), b.context)],
        })
        .collect()
}

pub fn mouse_action(key: &str, overrides: &HashMap<String, Vec<String>>, editing: bool, selecting: bool) -> Option<Box<dyn gpui::Action>> {
    BINDINGS.iter().rev().find(|b| {
        let active = match b.context {
            Some("editing") => editing,
            Some("selecting") => selecting,
            _ => true,
        };
        active && keys_for(b.id, overrides).iter().any(|k| k == key)
    }).map(|b| (b.make)("space", b.context).action().boxed_clone())
}

/// Whether two binding contexts can be active at the same time
/// ("editing" and "selecting" never are).
fn overlap(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some("editing"), Some("selecting")) | (Some("selecting"), Some("editing")) => false,
        _ => true,
    }
}

/// Ids whose shortcut collides with another action's in an overlapping context.
pub fn conflicts(overrides: &HashMap<String, Vec<String>>) -> std::collections::HashSet<&'static str> {
    let all: Vec<(&'static str, String, Option<&'static str>)> = rows()
        .into_iter()
        .flat_map(|r| effective(r.id, overrides).into_iter().map(move |(k, c)| (r.id, k, c)))
        .collect();
    let mut out = std::collections::HashSet::new();
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            if a.0 != b.0 && a.1 == b.1 && overlap(a.2, b.2) {
                out.insert(a.0);
                out.insert(b.0);
            }
        }
    }
    out
}

impl Binding {
    pub fn label(&self) -> &'static str {
        i18n::text(&format!("shortcut.{}", self.id))
    }
}

/// A shortcut group's heading.
pub fn group_label(group: &str) -> &'static str {
    i18n::text(&format!("group.{group}"))
}

/// Unique rows for the settings list, in table order.
pub fn rows() -> Vec<&'static Binding> {
    let mut seen = std::collections::HashSet::new();
    BINDINGS.iter().filter(|b| seen.insert(b.id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_bindings_can_be_replaced_and_report_conflicts() {
        let mut overrides = HashMap::new();
        assert!(mouse_action("mouse-back", &overrides, true, false).unwrap().as_any().is::<PageBack>());
        overrides.insert("page-back".into(), vec!["cmd-alt-left".into()]);
        assert!(mouse_action("mouse-back", &overrides, true, false).is_none());
        overrides.insert("page-forward".into(), vec!["mouse-forward".into(), "cmd-alt-left".into()]);
        let conflicts = conflicts(&overrides);
        assert!(conflicts.contains("page-back"));
        assert!(conflicts.contains("page-forward"));
    }
}

pub fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "Flowie".into(),
            items: vec![
                MenuItem::action(t!("menu.app.about"), OpenAbout),
                MenuItem::separator(),
                MenuItem::action(t!("menu.app.settings"), OpenSettings),
                MenuItem::separator(),
                MenuItem::action(t!("menu.app.quit"), Quit),
            ],
        },
        Menu {
            name: t!("menu.file.title").into(),
            items: vec![
                MenuItem::action(t!("menu.file.import"), Import),
                MenuItem::separator(),
                MenuItem::action(t!("menu.file.export_markdown"), ExportMarkdown),
                MenuItem::action(t!("menu.file.export_opml"), ExportOpml),
                MenuItem::action(t!("menu.file.export_text"), ExportText),
                MenuItem::separator(),
                MenuItem::action(t!("menu.file.trash"), ShowTrash),
            ],
        },
        Menu {
            name: t!("menu.edit.title").into(),
            items: vec![
                MenuItem::action(t!("menu.edit.undo"), Undo),
                MenuItem::action(t!("menu.edit.redo"), Redo),
                MenuItem::separator(),
                MenuItem::action(t!("menu.edit.cut"), Cut),
                MenuItem::action(t!("menu.edit.copy"), Copy),
                MenuItem::action(t!("menu.edit.paste"), Paste),
                MenuItem::action(t!("menu.edit.select_all"), SelectAll),
                MenuItem::separator(),
                MenuItem::action(t!("menu.edit.find"), Find),
                MenuItem::action(t!("menu.edit.find_next"), NextMatch),
                MenuItem::action(t!("menu.edit.find_previous"), PrevMatch),
                MenuItem::action(t!("menu.edit.save_search"), SaveSearch),
            ],
        },
        Menu {
            name: t!("menu.format.title").into(),
            items: vec![
                MenuItem::action(t!("menu.format.bold"), Bold),
                MenuItem::action(t!("menu.format.italic"), Italic),
                MenuItem::action(t!("menu.format.code"), InlineCode),
                MenuItem::action(t!("menu.format.math"), WrapMath),
                MenuItem::action(t!("menu.format.highlight"), Highlight),
                MenuItem::action(t!("menu.format.link"), InsertLink),
                MenuItem::action(t!("menu.format.link_to_bullet"), LinkToBullet),
                MenuItem::separator(),
                MenuItem::action(t!("menu.format.bullet"), KindBullet),
                MenuItem::action(t!("menu.format.h1"), KindH1),
                MenuItem::action(t!("menu.format.h2"), KindH2),
                MenuItem::action(t!("menu.format.h3"), KindH3),
                MenuItem::action(t!("menu.format.text"), KindParagraph),
                MenuItem::action(t!("menu.format.todo"), KindTodo),
                MenuItem::action(t!("menu.format.numbered"), KindNumbered),
                MenuItem::action(t!("menu.format.quote"), KindQuote),
                MenuItem::action(t!("menu.format.code_block"), KindCode),
            ],
        },
        Menu {
            name: t!("menu.outline.title").into(),
            items: vec![
                MenuItem::action(t!("menu.outline.complete"), ToggleComplete),
                MenuItem::action(t!("menu.outline.note"), ToggleNote),
                MenuItem::separator(),
                MenuItem::action(t!("menu.outline.indent"), Indent),
                MenuItem::action(t!("menu.outline.outdent"), Outdent),
                MenuItem::action(t!("menu.outline.move_up"), MoveUp),
                MenuItem::action(t!("menu.outline.move_down"), MoveDown),
                MenuItem::action(t!("menu.outline.delete"), DeleteNode),
                MenuItem::action(t!("menu.outline.move_to"), MoveTo),
                MenuItem::action(t!("menu.outline.mirror"), MirrorBullet),
                MenuItem::separator(),
                MenuItem::action(t!("menu.outline.collapse"), Collapse),
                MenuItem::action(t!("menu.outline.expand"), Expand),
                MenuItem::action(t!("menu.outline.zoom_in"), ZoomIn),
                MenuItem::action(t!("menu.outline.zoom_out"), ZoomOut),
                MenuItem::action(t!("menu.outline.home"), ZoomHome),
                MenuItem::action(t!("menu.outline.jump"), Jump),
                MenuItem::action(t!("menu.outline.star"), ToggleStar),
                MenuItem::action(t!("menu.outline.today"), Today),
                MenuItem::action(t!("menu.outline.calendar"), ShowCalendar),
                MenuItem::separator(),
                MenuItem::action(t!("menu.outline.expand_all"), ExpandAll),
                MenuItem::action(t!("menu.outline.level_1"), Level1),
                MenuItem::action(t!("menu.outline.level_2"), Level2),
                MenuItem::action(t!("menu.outline.level_3"), Level3),
                MenuItem::action(t!("menu.outline.level_4"), Level4),
                MenuItem::separator(),
                MenuItem::action(t!("menu.outline.toggle_completed"), ToggleCompletedVisible),
            ],
        },
        Menu {
            name: t!("menu.help.title").into(),
            items: vec![
                MenuItem::action(t!("menu.help.tutorial"), OpenTutorial),
                MenuItem::action(t!("menu.help.feedback"), SendFeedback),
                MenuItem::separator(),
                MenuItem::action(t!("menu.help.privacy"), OpenPrivacyPolicy),
                MenuItem::action(t!("menu.help.terms"), OpenTerms),
            ],
        },
    ]);
}
