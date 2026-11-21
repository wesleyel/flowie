//! Every keyboard shortcut in one table, so the settings panel can list
//! and rebind them. User overrides (id -> keystrokes) come from settings.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{Action, App, KeyBinding, KeyBindingContextPredicate, Menu, MenuItem};

use crate::i18n::{self, t};
use crate::outline::*;

pub struct Binding {
    /// Stable id used for overrides. Entries sharing an id are rebound together.
    pub id: &'static str,
    /// Group id; the heading is `group.<id>` and the label `shortcut.<id>` in the locale files.
    pub group: &'static str,
    /// Default keystroke (`""` means unbound by default on this platform).
    pub keys: &'static str,
    context: Option<&'static str>,
    action: fn() -> Box<dyn Action>,
}

macro_rules! b {
    ($id:literal, $group:literal, $keys:literal, $ctx:expr, $action:expr) => {
        Binding { id: $id, group: $group, keys: $keys, context: $ctx, action: || Box::new($action) }
    };
}

const E: Option<&str> = Some("editing");
const S: Option<&str> = Some("selecting");
const O: Option<&str> = Some("Outline");

pub const GROUPS: [&str; 6] = ["editing", "formatting", "outline", "navigation", "search", "app"];

#[cfg(target_os = "macos")]
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
    // macOS text keys (Emacs style). GPUI drops the command macOS picks for
    // them (`doCommandBySelector:`), so they need bindings of their own.
    b!("backspace", "editing", "ctrl-h", E, Backspace),
    b!("delete", "editing", "ctrl-d", E, Delete),
    b!("kill-line", "editing", "ctrl-k", E, KillLine),
    b!("yank", "editing", "ctrl-y", E, Yank),
    b!("left", "editing", "left", E, Left),
    b!("right", "editing", "right", E, Right),
    b!("left", "editing", "ctrl-b", E, Left),
    b!("right", "editing", "ctrl-f", E, Right),
    b!("up", "editing", "up", O, Up),
    b!("down", "editing", "down", O, Down),
    b!("up", "editing", "ctrl-p", E, Up),
    b!("down", "editing", "ctrl-n", E, Down),
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
    b!("home", "editing", "ctrl-a", E, Home),
    b!("end", "editing", "ctrl-e", E, End),
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
    b!("color-cycle", "formatting", "cmd-ctrl-c", O, CycleColor),
    b!("color-red", "formatting", "cmd-ctrl-1", O, ColorRed),
    b!("color-orange", "formatting", "cmd-ctrl-2", O, ColorOrange),
    b!("color-yellow", "formatting", "cmd-ctrl-3", O, ColorYellow),
    b!("color-green", "formatting", "cmd-ctrl-4", O, ColorGreen),
    b!("color-blue", "formatting", "cmd-ctrl-5", O, ColorBlue),
    b!("color-purple", "formatting", "cmd-ctrl-6", O, ColorPurple),
    b!("color-gray", "formatting", "cmd-ctrl-7", O, ColorGray),
    b!("color-none", "formatting", "cmd-ctrl-0", O, ColorNone),
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

#[cfg(not(target_os = "macos"))]
pub static BINDINGS: &[Binding] = &[
    // Editing
    b!("newline", "editing", "enter", E, Newline),
    b!("newline", "editing", "enter", S, Newline),
    b!("note", "editing", "shift-enter", E, ToggleNote),
    b!("indent", "editing", "tab", O, Indent),
    b!("outdent", "editing", "shift-tab", O, Outdent),
    b!("backspace", "editing", "backspace", O, Backspace),
    b!("delete", "editing", "delete", O, Delete),
    b!("delete-word", "editing", "ctrl-backspace", E, DeleteWordLeft),
    // Unbound by default on Windows/Linux (Ctrl+K is Link, Ctrl+Y is Redo);
    // still listed so users can bind them in Settings.
    b!("kill-line", "editing", "", E, KillLine),
    b!("yank", "editing", "", E, Yank),
    b!("left", "editing", "left", E, Left),
    b!("right", "editing", "right", E, Right),
    b!("up", "editing", "up", O, Up),
    b!("down", "editing", "down", O, Down),
    b!("word-left", "editing", "ctrl-left", E, WordLeft),
    b!("word-right", "editing", "ctrl-right", E, WordRight),
    b!("select-left", "editing", "shift-left", E, SelectLeft),
    b!("select-right", "editing", "shift-right", E, SelectRight),
    b!("select-word-left", "editing", "ctrl-shift-left", E, SelectWordLeft),
    b!("select-word-right", "editing", "ctrl-shift-right", E, SelectWordRight),
    b!("select-up", "editing", "shift-up", O, SelectUp),
    b!("select-down", "editing", "shift-down", O, SelectDown),
    b!("home", "editing", "home", E, Home),
    b!("end", "editing", "end", E, End),
    b!("select-home", "editing", "shift-home", E, SelectHome),
    b!("select-end", "editing", "shift-end", E, SelectEnd),
    b!("select-all", "editing", "ctrl-a", O, SelectAll),
    b!("copy", "editing", "ctrl-c", O, Copy),
    b!("cut", "editing", "ctrl-x", O, Cut),
    b!("paste", "editing", "ctrl-v", O, Paste),
    b!("undo", "editing", "ctrl-z", O, Undo),
    b!("redo", "editing", "ctrl-shift-z", O, Redo),
    b!("redo", "editing", "ctrl-y", O, Redo),
    // Formatting
    b!("bold", "formatting", "ctrl-b", E, Bold),
    b!("italic", "formatting", "ctrl-i", E, Italic),
    b!("code", "formatting", "ctrl-e", E, InlineCode),
    b!("math", "formatting", "ctrl-m", E, WrapMath),
    b!("link", "formatting", "ctrl-k", E, InsertLink),
    b!("highlight", "formatting", "ctrl-shift-y", E, Highlight),
    b!("color-cycle", "formatting", "alt-c", O, CycleColor),
    b!("color-red", "formatting", "alt-1", O, ColorRed),
    b!("color-orange", "formatting", "alt-2", O, ColorOrange),
    b!("color-yellow", "formatting", "alt-3", O, ColorYellow),
    b!("color-green", "formatting", "alt-4", O, ColorGreen),
    b!("color-blue", "formatting", "alt-5", O, ColorBlue),
    b!("color-purple", "formatting", "alt-6", O, ColorPurple),
    b!("color-gray", "formatting", "alt-7", O, ColorGray),
    b!("color-none", "formatting", "alt-0", O, ColorNone),
    b!("kind-bullet", "formatting", "ctrl-alt-0", O, KindBullet),
    b!("kind-h1", "formatting", "ctrl-alt-1", O, KindH1),
    b!("kind-h2", "formatting", "ctrl-alt-2", O, KindH2),
    b!("kind-h3", "formatting", "ctrl-alt-3", O, KindH3),
    b!("kind-text", "formatting", "ctrl-alt-4", O, KindParagraph),
    b!("kind-todo", "formatting", "ctrl-alt-5", O, KindTodo),
    b!("kind-numbered", "formatting", "ctrl-alt-6", O, KindNumbered),
    b!("kind-quote", "formatting", "ctrl-alt-7", O, KindQuote),
    b!("kind-code", "formatting", "ctrl-alt-8", O, KindCode),
    // Outline
    b!("complete", "outline", "ctrl-enter", O, ToggleComplete),
    b!("collapse", "outline", "ctrl-up", O, Collapse),
    b!("expand", "outline", "ctrl-down", O, Expand),
    b!("expand-all", "outline", "ctrl-0", O, ExpandAll),
    b!("level-1", "outline", "ctrl-1", O, Level1),
    b!("level-2", "outline", "ctrl-2", O, Level2),
    b!("level-3", "outline", "ctrl-3", O, Level3),
    b!("level-4", "outline", "ctrl-4", O, Level4),
    b!("move-up", "outline", "ctrl-shift-up", O, MoveUp),
    b!("move-down", "outline", "ctrl-shift-down", O, MoveDown),
    b!("move-to", "outline", "ctrl-shift-m", O, MoveTo),
    b!("mirror", "outline", "ctrl-alt-m", O, MirrorBullet),
    b!("link-to", "outline", "ctrl-shift-k", E, LinkToBullet),
    b!("delete-bullet", "outline", "ctrl-shift-backspace", O, DeleteNode),
    b!("toggle-completed", "outline", "ctrl-o", O, ToggleCompletedVisible),
    // Navigation
    b!("page-back", "navigation", "mouse-back", O, PageBack),
    b!("page-back", "navigation", "ctrl-alt-left", O, PageBack),
    b!("page-forward", "navigation", "mouse-forward", O, PageForward),
    b!("page-forward", "navigation", "ctrl-alt-right", O, PageForward),
    b!("zoom-in", "navigation", "ctrl-]", O, ZoomIn),
    b!("zoom-in", "navigation", "alt-right", S, ZoomIn),
    b!("zoom-out", "navigation", "ctrl-[", O, ZoomOut),
    b!("zoom-out", "navigation", "alt-left", S, ZoomOut),
    b!("home-page", "navigation", "ctrl-shift-h", O, ZoomHome),
    b!("jump", "navigation", "ctrl-p", O, Jump),
    b!("star", "navigation", "ctrl-shift-s", O, ToggleStar),
    b!("today", "navigation", "ctrl-shift-d", O, Today),
    b!("calendar", "navigation", "ctrl-shift-c", O, ShowCalendar),
    // Search
    b!("find", "search", "ctrl-f", O, Find),
    b!("next-match", "search", "ctrl-g", O, NextMatch),
    b!("next-match", "search", "f3", O, NextMatch),
    b!("prev-match", "search", "ctrl-shift-g", O, PrevMatch),
    b!("prev-match", "search", "shift-f3", O, PrevMatch),
    b!("escape", "search", "escape", O, Escape),
    // App
    b!("import", "app", "ctrl-shift-i", O, Import),
    b!("export-md", "app", "ctrl-shift-e", O, ExportMarkdown),
    b!("trash", "app", "ctrl-shift-delete", O, ShowTrash),
    b!("settings", "app", "ctrl-,", None, OpenSettings),
    b!("quit", "app", "ctrl-q", None, Quit),
];

/// Primary modifier + key formatted for prose hints (e.g. `⌘P` on macOS, `Ctrl+P` elsewhere).
pub fn primary_hint(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// Alt/Option modifier name for prose hints.
pub fn alt_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌥"
    } else {
        "Alt"
    }
}

/// Backspace key name for prose hints.
pub fn backspace_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌫"
    } else {
        "Backspace"
    }
}

/// The keys a binding id currently uses (empty = unbound).
pub fn keys_for(id: &str, overrides: &HashMap<String, Vec<String>>) -> Vec<String> {
    if let Some(k) = overrides.get(id) {
        return k.clone();
    }
    let mut out: Vec<String> = BINDINGS.iter().filter(|b| b.id == id && !b.keys.is_empty()).map(|b| b.keys.to_string()).collect();
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
            let first_in_ctx =
                BINDINGS.iter().position(|x| x.id == b.id && x.context == b.context) == BINDINGS.iter().position(|x| std::ptr::eq(x, b));
            if first_in_ctx {
                for key in keys.iter().filter(|k| !k.is_empty() && !k.starts_with("mouse-")) {
                    out.extend(bind(key, b, cx));
                }
            }
        } else if !b.keys.is_empty() && !b.keys.starts_with("mouse-") {
            out.extend(bind(b.keys, b, cx));
        }
    }
    // Read-only selectable labels (About and Agent panels).
    let sel_ctx = Some(Rc::new(KeyBindingContextPredicate::parse("Selectable").expect("valid context")));
    out.extend(KeyBinding::load("secondary-c", Box::new(Copy), sel_ctx.clone(), false, None, cx).ok());
    out.extend(KeyBinding::load("secondary-a", Box::new(SelectAll), sel_ctx, false, None, cx).ok());
    cx.bind_keys(out);
    set_menus(cx);
}

/// One key binding; `None` (logged) when user-entered keys don't parse.
fn bind(keys: &str, b: &Binding, cx: &App) -> Option<KeyBinding> {
    let ctx = b.context.map(|c| Rc::new(KeyBindingContextPredicate::parse(c).expect("valid context")));
    KeyBinding::load(keys, (b.action)(), ctx, false, None, cx)
        .map_err(|e| eprintln!("flowie: skipping shortcut {keys:?} for {}: {e}", b.id))
        .ok()
}

/// Effective (keys, context) pairs of a binding id after overrides.
fn effective(id: &str, overrides: &HashMap<String, Vec<String>>) -> Vec<(String, Option<&'static str>)> {
    BINDINGS
        .iter()
        .filter(|b| b.id == id)
        .flat_map(|b| match overrides.get(id) {
            Some(keys) => keys.iter().filter(|k| !k.is_empty()).map(|k| (k.clone(), b.context)).collect::<Vec<_>>(),
            None if !b.keys.is_empty() => vec![(b.keys.to_string(), b.context)],
            None => Vec::new(),
        })
        .collect()
}

pub fn mouse_action(key: &str, overrides: &HashMap<String, Vec<String>>, editing: bool, selecting: bool) -> Option<Box<dyn gpui::Action>> {
    BINDINGS
        .iter()
        .rev()
        .find(|b| {
            let active = match b.context {
                Some("editing") => editing,
                Some("selecting") => selecting,
                _ => true,
            };
            active && keys_for(b.id, overrides).iter().any(|k| k == key)
        })
        .map(|b| (b.action)())
}

/// Whether two binding contexts can be active at the same time
/// ("editing" and "selecting" never are).
fn overlap(a: Option<&str>, b: Option<&str>) -> bool {
    !matches!((a, b), (Some("editing"), Some("selecting")) | (Some("selecting"), Some("editing")))
}

/// Ids whose shortcut collides with another action's in an overlapping context.
pub fn conflicts(overrides: &HashMap<String, Vec<String>>) -> std::collections::HashSet<&'static str> {
    let all: Vec<(&'static str, String, Option<&'static str>)> =
        rows().into_iter().flat_map(|r| effective(r.id, overrides).into_iter().map(move |(k, c)| (r.id, k, c))).collect();
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

// ------------------------------------------------------------------ menus

/// One entry in an application menu, shared between macOS's native menu bar
/// (`cx.set_menus`) and the in-window menu bar on Windows / Linux.
pub enum AppMenuItem {
    Action {
        label: &'static str,
        #[cfg_attr(target_os = "macos", allow(dead_code))]
        shortcut_id: Option<&'static str>,
        action: fn() -> Box<dyn Action>,
    },
    Separator,
}

pub struct AppMenu {
    pub name: &'static str,
    pub items: Vec<AppMenuItem>,
}

fn item<A: Action + Default>(label: &'static str, shortcut_id: Option<&'static str>) -> AppMenuItem {
    AppMenuItem::Action { label, shortcut_id, action: || Box::new(A::default()) }
}

/// Build the application menus for the current platform and locale.
pub fn app_menus() -> Vec<AppMenu> {
    let mut file_items = vec![
        item::<Import>(t!("menu.file.import"), Some("import")),
        AppMenuItem::Separator,
        item::<ExportMarkdown>(t!("menu.file.export_markdown"), Some("export-md")),
        item::<ExportOpml>(t!("menu.file.export_opml"), None),
        item::<ExportText>(t!("menu.file.export_text"), None),
        AppMenuItem::Separator,
        item::<ShowTrash>(t!("menu.file.trash"), Some("trash")),
    ];
    if !cfg!(target_os = "macos") {
        file_items.push(AppMenuItem::Separator);
        file_items.push(item::<OpenSettings>(t!("menu.app.settings"), Some("settings")));
        file_items.push(AppMenuItem::Separator);
        file_items.push(item::<Quit>(t!("menu.app.quit"), Some("quit")));
    }

    let mut help_items = vec![
        item::<OpenTutorial>(t!("menu.help.tutorial"), None),
        item::<SendFeedback>(t!("menu.help.feedback"), None),
        AppMenuItem::Separator,
        item::<OpenPrivacyPolicy>(t!("menu.help.privacy"), None),
        item::<OpenTerms>(t!("menu.help.terms"), None),
    ];
    if !cfg!(target_os = "macos") {
        help_items.push(AppMenuItem::Separator);
        help_items.push(item::<OpenAbout>(t!("menu.app.about"), None));
    }

    let mut menus = Vec::new();
    if cfg!(target_os = "macos") {
        menus.push(AppMenu {
            name: "Flowie",
            items: vec![
                item::<OpenAbout>(t!("menu.app.about"), None),
                AppMenuItem::Separator,
                item::<OpenSettings>(t!("menu.app.settings"), Some("settings")),
                AppMenuItem::Separator,
                item::<Quit>(t!("menu.app.quit"), Some("quit")),
            ],
        });
    }
    menus.extend([
        AppMenu { name: t!("menu.file.title"), items: file_items },
        AppMenu {
            name: t!("menu.edit.title"),
            items: vec![
                item::<Undo>(t!("menu.edit.undo"), Some("undo")),
                item::<Redo>(t!("menu.edit.redo"), Some("redo")),
                AppMenuItem::Separator,
                item::<Cut>(t!("menu.edit.cut"), Some("cut")),
                item::<Copy>(t!("menu.edit.copy"), Some("copy")),
                item::<Paste>(t!("menu.edit.paste"), Some("paste")),
                item::<SelectAll>(t!("menu.edit.select_all"), Some("select-all")),
                AppMenuItem::Separator,
                item::<Find>(t!("menu.edit.find"), Some("find")),
                item::<NextMatch>(t!("menu.edit.find_next"), Some("next-match")),
                item::<PrevMatch>(t!("menu.edit.find_previous"), Some("prev-match")),
                item::<SaveSearch>(t!("menu.edit.save_search"), None),
            ],
        },
        AppMenu {
            name: t!("menu.format.title"),
            items: vec![
                item::<Bold>(t!("menu.format.bold"), Some("bold")),
                item::<Italic>(t!("menu.format.italic"), Some("italic")),
                item::<InlineCode>(t!("menu.format.code"), Some("code")),
                item::<WrapMath>(t!("menu.format.math"), Some("math")),
                item::<Highlight>(t!("menu.format.highlight"), Some("highlight")),
                item::<InsertLink>(t!("menu.format.link"), Some("link")),
                item::<LinkToBullet>(t!("menu.format.link_to_bullet"), Some("link-to")),
                AppMenuItem::Separator,
                item::<CycleColor>(t!("menu.format.color_cycle"), Some("color-cycle")),
                item::<ColorRed>(t!("menu.format.color_red"), Some("color-red")),
                item::<ColorOrange>(t!("menu.format.color_orange"), Some("color-orange")),
                item::<ColorYellow>(t!("menu.format.color_yellow"), Some("color-yellow")),
                item::<ColorGreen>(t!("menu.format.color_green"), Some("color-green")),
                item::<ColorBlue>(t!("menu.format.color_blue"), Some("color-blue")),
                item::<ColorPurple>(t!("menu.format.color_purple"), Some("color-purple")),
                item::<ColorGray>(t!("menu.format.color_gray"), Some("color-gray")),
                item::<ColorNone>(t!("menu.format.color_none"), Some("color-none")),
                AppMenuItem::Separator,
                item::<KindBullet>(t!("menu.format.bullet"), Some("kind-bullet")),
                item::<KindH1>(t!("menu.format.h1"), Some("kind-h1")),
                item::<KindH2>(t!("menu.format.h2"), Some("kind-h2")),
                item::<KindH3>(t!("menu.format.h3"), Some("kind-h3")),
                item::<KindParagraph>(t!("menu.format.text"), Some("kind-text")),
                item::<KindTodo>(t!("menu.format.todo"), Some("kind-todo")),
                item::<KindNumbered>(t!("menu.format.numbered"), Some("kind-numbered")),
                item::<KindQuote>(t!("menu.format.quote"), Some("kind-quote")),
                item::<KindCode>(t!("menu.format.code_block"), Some("kind-code")),
            ],
        },
        AppMenu {
            name: t!("menu.outline.title"),
            items: vec![
                item::<ToggleComplete>(t!("menu.outline.complete"), Some("complete")),
                item::<ToggleNote>(t!("menu.outline.note"), Some("note")),
                AppMenuItem::Separator,
                item::<Indent>(t!("menu.outline.indent"), Some("indent")),
                item::<Outdent>(t!("menu.outline.outdent"), Some("outdent")),
                item::<MoveUp>(t!("menu.outline.move_up"), Some("move-up")),
                item::<MoveDown>(t!("menu.outline.move_down"), Some("move-down")),
                item::<DeleteNode>(t!("menu.outline.delete"), Some("delete-bullet")),
                item::<MoveTo>(t!("menu.outline.move_to"), Some("move-to")),
                item::<MirrorBullet>(t!("menu.outline.mirror"), Some("mirror")),
                AppMenuItem::Separator,
                item::<Collapse>(t!("menu.outline.collapse"), Some("collapse")),
                item::<Expand>(t!("menu.outline.expand"), Some("expand")),
                item::<ZoomIn>(t!("menu.outline.zoom_in"), Some("zoom-in")),
                item::<ZoomOut>(t!("menu.outline.zoom_out"), Some("zoom-out")),
                item::<ZoomHome>(t!("menu.outline.home"), Some("home-page")),
                item::<Jump>(t!("menu.outline.jump"), Some("jump")),
                item::<ToggleStar>(t!("menu.outline.star"), Some("star")),
                item::<Today>(t!("menu.outline.today"), Some("today")),
                item::<ShowCalendar>(t!("menu.outline.calendar"), Some("calendar")),
                AppMenuItem::Separator,
                item::<ExpandAll>(t!("menu.outline.expand_all"), Some("expand-all")),
                item::<Level1>(t!("menu.outline.level_1"), Some("level-1")),
                item::<Level2>(t!("menu.outline.level_2"), Some("level-2")),
                item::<Level3>(t!("menu.outline.level_3"), Some("level-3")),
                item::<Level4>(t!("menu.outline.level_4"), Some("level-4")),
                AppMenuItem::Separator,
                item::<ToggleCompletedVisible>(t!("menu.outline.toggle_completed"), Some("toggle-completed")),
            ],
        },
        AppMenu { name: t!("menu.help.title"), items: help_items },
    ]);
    menus
}

pub fn set_menus(cx: &mut App) {
    let menus: Vec<Menu> = app_menus()
        .into_iter()
        .map(|m| Menu {
            disabled: false,
            name: m.name.into(),
            items: m
                .items
                .into_iter()
                .map(|it| match it {
                    AppMenuItem::Action { label, action, .. } => {
                        MenuItem::Action { name: label.into(), action: action(), os_action: None, checked: false, disabled: false }
                    }
                    AppMenuItem::Separator => MenuItem::separator(),
                })
                .collect(),
        })
        .collect();
    cx.set_menus(menus);
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

    #[test]
    fn default_bindings_have_no_conflicts_and_parse() {
        assert!(conflicts(&HashMap::new()).is_empty(), "default bindings conflict: {:?}", conflicts(&HashMap::new()));
        for b in BINDINGS {
            if b.keys.is_empty() || b.keys.starts_with("mouse-") {
                continue;
            }
            for stroke in b.keys.split_whitespace() {
                gpui::Keystroke::parse(stroke).unwrap_or_else(|e| panic!("invalid keystroke {stroke:?} for {}: {e}", b.id));
            }
        }
    }

    #[test]
    fn app_menu_shortcuts_exist() {
        for m in app_menus() {
            for it in m.items {
                if let AppMenuItem::Action { shortcut_id: Some(id), .. } = it {
                    assert!(BINDINGS.iter().any(|b| b.id == id), "menu references unknown shortcut id {id:?}");
                }
            }
        }
    }
}
