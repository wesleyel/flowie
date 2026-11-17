//! Every keyboard shortcut in one table, so the settings panel can list
//! and rebind them. User overrides (id -> keystrokes) come from settings.

use std::collections::HashMap;

use gpui::{App, KeyBinding, Menu, MenuItem};

use crate::outline::*;

pub struct Binding {
    /// Stable id used for overrides. Entries sharing an id are rebound together.
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub keys: &'static str,
    context: Option<&'static str>,
    make: fn(&str, Option<&str>) -> KeyBinding,
}

macro_rules! b {
    ($id:literal, $label:literal, $group:literal, $keys:literal, $ctx:expr, $action:expr) => {
        Binding { id: $id, label: $label, group: $group, keys: $keys, context: $ctx, make: |k, c| KeyBinding::new(k, $action, c) }
    };
}

const E: Option<&str> = Some("editing");
const S: Option<&str> = Some("selecting");
const O: Option<&str> = Some("Outline");

pub const GROUPS: [&str; 6] = ["Editing", "Formatting", "Outline", "Navigation", "Search", "App"];

pub static BINDINGS: &[Binding] = &[
    // Editing
    b!("newline", "New bullet / split", "Editing", "enter", E, Newline),
    b!("newline", "New bullet / split", "Editing", "enter", S, Newline),
    b!("note", "Edit note", "Editing", "shift-enter", E, ToggleNote),
    b!("indent", "Indent", "Editing", "tab", O, Indent),
    b!("outdent", "Outdent", "Editing", "shift-tab", O, Outdent),
    b!("backspace", "Delete backward", "Editing", "backspace", O, Backspace),
    b!("delete", "Delete forward", "Editing", "delete", O, Delete),
    b!("delete-word", "Delete word", "Editing", "alt-backspace", E, DeleteWordLeft),
    b!("left", "Caret left", "Editing", "left", E, Left),
    b!("right", "Caret right", "Editing", "right", E, Right),
    b!("up", "Up", "Editing", "up", O, Up),
    b!("down", "Down", "Editing", "down", O, Down),
    b!("word-left", "Word left", "Editing", "alt-left", E, WordLeft),
    b!("word-right", "Word right", "Editing", "alt-right", E, WordRight),
    b!("select-left", "Select left", "Editing", "shift-left", E, SelectLeft),
    b!("select-right", "Select right", "Editing", "shift-right", E, SelectRight),
    b!("select-word-left", "Select word left", "Editing", "alt-shift-left", E, SelectWordLeft),
    b!("select-word-right", "Select word right", "Editing", "alt-shift-right", E, SelectWordRight),
    b!("select-up", "Select up / extend", "Editing", "shift-up", O, SelectUp),
    b!("select-down", "Select down / extend", "Editing", "shift-down", O, SelectDown),
    b!("home", "Line start", "Editing", "cmd-left", E, Home),
    b!("home", "Line start", "Editing", "home", E, Home),
    b!("end", "Line end", "Editing", "cmd-right", E, End),
    b!("end", "Line end", "Editing", "end", E, End),
    b!("select-home", "Select to line start", "Editing", "cmd-shift-left", E, SelectHome),
    b!("select-end", "Select to line end", "Editing", "cmd-shift-right", E, SelectEnd),
    b!("select-all", "Select all", "Editing", "cmd-a", O, SelectAll),
    b!("copy", "Copy", "Editing", "cmd-c", O, Copy),
    b!("cut", "Cut", "Editing", "cmd-x", O, Cut),
    b!("paste", "Paste", "Editing", "cmd-v", O, Paste),
    b!("undo", "Undo", "Editing", "cmd-z", O, Undo),
    b!("redo", "Redo", "Editing", "cmd-shift-z", O, Redo),
    // Formatting
    b!("bold", "Bold", "Formatting", "cmd-b", E, Bold),
    b!("italic", "Italic", "Formatting", "cmd-i", E, Italic),
    b!("code", "Inline code", "Formatting", "cmd-e", E, InlineCode),
    b!("math", "Math", "Formatting", "cmd-m", E, WrapMath),
    b!("link", "Link", "Formatting", "cmd-k", E, InsertLink),
    b!("highlight", "Highlight", "Formatting", "cmd-shift-y", E, Highlight),
    b!("kind-bullet", "Bullet", "Formatting", "cmd-alt-0", O, KindBullet),
    b!("kind-h1", "Heading 1", "Formatting", "cmd-alt-1", O, KindH1),
    b!("kind-h2", "Heading 2", "Formatting", "cmd-alt-2", O, KindH2),
    b!("kind-h3", "Heading 3", "Formatting", "cmd-alt-3", O, KindH3),
    b!("kind-text", "Text", "Formatting", "cmd-alt-4", O, KindParagraph),
    b!("kind-todo", "To-do", "Formatting", "cmd-alt-5", O, KindTodo),
    b!("kind-numbered", "Numbered", "Formatting", "cmd-alt-6", O, KindNumbered),
    b!("kind-quote", "Quote", "Formatting", "cmd-alt-7", O, KindQuote),
    b!("kind-code", "Code block", "Formatting", "cmd-alt-8", O, KindCode),
    // Outline
    b!("complete", "Complete", "Outline", "cmd-enter", O, ToggleComplete),
    b!("collapse", "Collapse", "Outline", "cmd-up", O, Collapse),
    b!("expand", "Expand", "Outline", "cmd-down", O, Expand),
    b!("expand-all", "Expand all", "Outline", "ctrl-0", O, ExpandAll),
    b!("level-1", "Show 1 level", "Outline", "ctrl-1", O, Level1),
    b!("level-2", "Show 2 levels", "Outline", "ctrl-2", O, Level2),
    b!("level-3", "Show 3 levels", "Outline", "ctrl-3", O, Level3),
    b!("level-4", "Show 4 levels", "Outline", "ctrl-4", O, Level4),
    b!("move-up", "Move up", "Outline", "cmd-shift-up", O, MoveUp),
    b!("move-down", "Move down", "Outline", "cmd-shift-down", O, MoveDown),
    b!("move-to", "Move to…", "Outline", "cmd-shift-m", O, MoveTo),
    b!("mirror", "Mirror…", "Outline", "cmd-alt-m", O, MirrorBullet),
    b!("link-to", "Link to bullet…", "Outline", "cmd-shift-k", E, LinkToBullet),
    b!("delete-bullet", "Delete bullet", "Outline", "cmd-shift-backspace", O, DeleteNode),
    b!("toggle-completed", "Show / hide completed", "Outline", "cmd-o", O, ToggleCompletedVisible),
    // Navigation
    b!("zoom-in", "Zoom in", "Navigation", "cmd-]", O, ZoomIn),
    b!("zoom-in", "Zoom in", "Navigation", "alt-right", S, ZoomIn),
    b!("zoom-out", "Zoom out", "Navigation", "cmd-[", O, ZoomOut),
    b!("zoom-out", "Zoom out", "Navigation", "alt-left", S, ZoomOut),
    b!("home-page", "Go home", "Navigation", "cmd-shift-h", O, ZoomHome),
    b!("jump", "Go to…", "Navigation", "cmd-p", O, Jump),
    b!("star", "Star page", "Navigation", "cmd-shift-s", O, ToggleStar),
    b!("today", "Today's journal", "Navigation", "cmd-shift-d", O, Today),
    b!("calendar", "Calendar", "Navigation", "cmd-shift-c", O, ShowCalendar),
    // Search
    b!("find", "Search", "Search", "cmd-f", O, Find),
    b!("next-match", "Next match", "Search", "cmd-g", O, NextMatch),
    b!("prev-match", "Previous match", "Search", "cmd-shift-g", O, PrevMatch),
    b!("escape", "Escape / select bullet", "Search", "escape", O, Escape),
    // App
    b!("import", "Import…", "App", "cmd-shift-i", O, Import),
    b!("export-md", "Export Markdown…", "App", "cmd-shift-e", O, ExportMarkdown),
    b!("trash", "Trash", "App", "cmd-shift-delete", O, ShowTrash),
    b!("settings", "Settings", "App", "cmd-,", None, OpenSettings),
    b!("quit", "Quit", "App", "cmd-q", None, Quit),
];

/// The keys a binding id currently uses ("" = unbound).
pub fn keys_for<'a>(id: &str, overrides: &'a HashMap<String, String>) -> Vec<String> {
    if let Some(k) = overrides.get(id) {
        return if k.is_empty() { Vec::new() } else { vec![k.clone()] };
    }
    let mut out: Vec<String> = BINDINGS.iter().filter(|b| b.id == id).map(|b| b.keys.to_string()).collect();
    out.dedup();
    out
}

/// (Re)install every binding, applying overrides. An override replaces all
/// default keys of that id, in each of its contexts.
pub fn install(cx: &mut App, overrides: &HashMap<String, String>) {
    cx.clear_key_bindings();
    let mut out = Vec::new();
    for b in BINDINGS {
        match overrides.get(b.id) {
            Some(k) if k.is_empty() => {}
            Some(k) => {
                // Bind once per distinct context.
                let first_in_ctx = BINDINGS.iter().position(|x| x.id == b.id && x.context == b.context)
                    == BINDINGS.iter().position(|x| std::ptr::eq(x, b));
                if first_in_ctx {
                    out.push((b.make)(k, b.context));
                }
            }
            None => out.push((b.make)(b.keys, b.context)),
        }
    }
    // Read-only selectable labels (About panel).
    out.push(KeyBinding::new("cmd-c", Copy, Some("Selectable")));
    out.push(KeyBinding::new("cmd-a", SelectAll, Some("Selectable")));
    cx.bind_keys(out);
    set_menus(cx);
}

/// Effective (keys, context) pairs of a binding id after overrides.
fn effective(id: &str, overrides: &HashMap<String, String>) -> Vec<(String, Option<&'static str>)> {
    BINDINGS
        .iter()
        .filter(|b| b.id == id)
        .filter_map(|b| match overrides.get(id) {
            Some(k) if k.is_empty() => None,
            Some(k) => Some((k.clone(), b.context)),
            None => Some((b.keys.to_string(), b.context)),
        })
        .collect()
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
pub fn conflicts(overrides: &HashMap<String, String>) -> std::collections::HashSet<&'static str> {
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

/// Unique rows for the settings list, in table order.
pub fn rows() -> Vec<&'static Binding> {
    let mut seen = std::collections::HashSet::new();
    BINDINGS.iter().filter(|b| seen.insert(b.id)).collect()
}

/// "cmd-shift-up" -> "⌘⇧↑"
pub fn pretty(keys: &str) -> String {
    keys.split_whitespace()
        .map(|stroke| {
            let parts: Vec<&str> = stroke.split('-').collect();
            // A trailing "-" key (e.g. "cmd--") splits into an empty part.
            let (mods, key) = if stroke.ends_with("--") {
                (&parts[..parts.len().saturating_sub(2)], "-")
            } else {
                (&parts[..parts.len() - 1], *parts.last().unwrap_or(&""))
            };
            let mut s = String::new();
            for m in ["ctrl", "alt", "shift", "cmd", "fn"] {
                if mods.contains(&m) {
                    s.push_str(match m {
                        "ctrl" => "⌃",
                        "alt" => "⌥",
                        "shift" => "⇧",
                        "cmd" => "⌘",
                        _ => "fn ",
                    });
                }
            }
            s.push_str(&match key {
                "enter" => "↩".to_string(),
                "backspace" => "⌫".to_string(),
                "delete" => "⌦".to_string(),
                "escape" => "Esc".to_string(),
                "tab" => "⇥".to_string(),
                "space" => "Space".to_string(),
                "up" => "↑".to_string(),
                "down" => "↓".to_string(),
                "left" => "←".to_string(),
                "right" => "→".to_string(),
                "home" => "Home".to_string(),
                "end" => "End".to_string(),
                k => k.to_uppercase(),
            });
            s
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "Flowie".into(),
            items: vec![
                MenuItem::action("About Flowie", OpenAbout),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit Flowie", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("Import…", Import),
                MenuItem::separator(),
                MenuItem::action("Export as Markdown…", ExportMarkdown),
                MenuItem::action("Export as OPML…", ExportOpml),
                MenuItem::action("Export as Plain Text…", ExportText),
                MenuItem::separator(),
                MenuItem::action("Trash", ShowTrash),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", Cut),
                MenuItem::action("Copy", Copy),
                MenuItem::action("Paste", Paste),
                MenuItem::action("Select All", SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find", Find),
                MenuItem::action("Find Next", NextMatch),
                MenuItem::action("Find Previous", PrevMatch),
                MenuItem::action("Save Search", SaveSearch),
            ],
        },
        Menu {
            name: "Format".into(),
            items: vec![
                MenuItem::action("Bold", Bold),
                MenuItem::action("Italic", Italic),
                MenuItem::action("Code", InlineCode),
                MenuItem::action("Math", WrapMath),
                MenuItem::action("Highlight", Highlight),
                MenuItem::action("Link", InsertLink),
                MenuItem::action("Link to Bullet…", LinkToBullet),
                MenuItem::separator(),
                MenuItem::action("Bullet", KindBullet),
                MenuItem::action("Heading 1", KindH1),
                MenuItem::action("Heading 2", KindH2),
                MenuItem::action("Heading 3", KindH3),
                MenuItem::action("Text", KindParagraph),
                MenuItem::action("To-do", KindTodo),
                MenuItem::action("Numbered", KindNumbered),
                MenuItem::action("Quote", KindQuote),
                MenuItem::action("Code Block", KindCode),
            ],
        },
        Menu {
            name: "Outline".into(),
            items: vec![
                MenuItem::action("Complete", ToggleComplete),
                MenuItem::action("Add Note", ToggleNote),
                MenuItem::separator(),
                MenuItem::action("Indent", Indent),
                MenuItem::action("Outdent", Outdent),
                MenuItem::action("Move Up", MoveUp),
                MenuItem::action("Move Down", MoveDown),
                MenuItem::action("Delete Bullet", DeleteNode),
                MenuItem::action("Move To…", MoveTo),
                MenuItem::action("Mirror…", MirrorBullet),
                MenuItem::separator(),
                MenuItem::action("Collapse", Collapse),
                MenuItem::action("Expand", Expand),
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
                MenuItem::action("Go Home", ZoomHome),
                MenuItem::action("Go To…", Jump),
                MenuItem::action("Star Page", ToggleStar),
                MenuItem::action("Today's Journal", Today),
                MenuItem::action("Calendar", ShowCalendar),
                MenuItem::separator(),
                MenuItem::action("Expand All", ExpandAll),
                MenuItem::action("Show 1 Level", Level1),
                MenuItem::action("Show 2 Levels", Level2),
                MenuItem::action("Show 3 Levels", Level3),
                MenuItem::action("Show 4 Levels", Level4),
                MenuItem::separator(),
                MenuItem::action("Show/Hide Completed", ToggleCompletedVisible),
            ],
        },
    ]);
}
