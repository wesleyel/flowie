//! Every keyboard shortcut in one table, so the settings panel can list
//! and rebind them. User overrides (id -> keystrokes) come from settings.
//!
//! Each entry carries its macOS keys and its Windows / Linux keys. Most use
//! the primary modifier (⌘ on macOS, Ctrl elsewhere); the rest follow each
//! platform's conventions. Defaults off macOS never combine Ctrl with Alt:
//! on Windows that chord is AltGr, which types `{ [ ] } @ €` on many layouts,
//! and Linux desktops reserve Ctrl+Alt for the window manager.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{Action, App, KeyBinding, KeyBindingContextPredicate, Menu, MenuItem};

use crate::app::i18n::{self, t};
use crate::app::platform::{self, Os};
use crate::outline::*;

pub struct Binding {
    /// Stable id used for overrides. Entries sharing an id are rebound together.
    pub id: &'static str,
    /// Group id; the heading is `group.<id>` and the label `shortcut.<id>` in the locale files.
    pub group: &'static str,
    /// Default keys on macOS and on Windows / Linux (`""`: unbound there).
    mac: &'static str,
    other: &'static str,
    context: Option<&'static str>,
    action: fn() -> Box<dyn Action>,
}

macro_rules! b {
    // ⌘ on macOS, Ctrl on Windows / Linux.
    ($id:literal, $group:literal, primary $keys:literal, $ctx:expr, $action:expr) => {
        b!(@ $id, $group, concat!("cmd-", $keys), concat!("ctrl-", $keys), $ctx, $action)
    };
    // Different keys per platform.
    ($id:literal, $group:literal, mac $mac:literal, other $other:literal, $ctx:expr, $action:expr) => {
        b!(@ $id, $group, $mac, $other, $ctx, $action)
    };
    // The same keys everywhere.
    ($id:literal, $group:literal, $keys:literal, $ctx:expr, $action:expr) => {
        b!(@ $id, $group, $keys, $keys, $ctx, $action)
    };
    (@ $id:literal, $group:literal, $mac:expr, $other:expr, $ctx:expr, $action:expr) => {
        Binding { id: $id, group: $group, mac: $mac, other: $other, context: $ctx, action: || Box::new($action) }
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
    b!("delete-word", "editing", mac "alt-backspace", other "ctrl-backspace", E, DeleteWordLeft),
    // macOS text keys (Emacs style). GPUI drops the command macOS picks for
    // them (`doCommandBySelector:`), so they need bindings of their own.
    // Elsewhere Ctrl+K is Link and Ctrl+Y is Redo; kill and yank stay
    // listed, unbound, so they can be given keys in Settings.
    b!("backspace", "editing", mac "ctrl-h", other "", E, Backspace),
    b!("delete", "editing", mac "ctrl-d", other "", E, Delete),
    b!("kill-line", "editing", mac "ctrl-k", other "", E, KillLine),
    b!("yank", "editing", mac "ctrl-y", other "", E, Yank),
    b!("left", "editing", "left", E, Left),
    b!("right", "editing", "right", E, Right),
    b!("left", "editing", mac "ctrl-b", other "", E, Left),
    b!("right", "editing", mac "ctrl-f", other "", E, Right),
    b!("up", "editing", "up", O, Up),
    b!("down", "editing", "down", O, Down),
    b!("up", "editing", mac "ctrl-p", other "", E, Up),
    b!("down", "editing", mac "ctrl-n", other "", E, Down),
    b!("word-left", "editing", mac "alt-left", other "ctrl-left", E, WordLeft),
    b!("word-right", "editing", mac "alt-right", other "ctrl-right", E, WordRight),
    b!("select-left", "editing", "shift-left", E, SelectLeft),
    b!("select-right", "editing", "shift-right", E, SelectRight),
    b!("select-word-left", "editing", mac "alt-shift-left", other "ctrl-shift-left", E, SelectWordLeft),
    b!("select-word-right", "editing", mac "alt-shift-right", other "ctrl-shift-right", E, SelectWordRight),
    b!("select-up", "editing", "shift-up", O, SelectUp),
    b!("select-down", "editing", "shift-down", O, SelectDown),
    b!("home", "editing", mac "cmd-left", other "", E, Home),
    b!("home", "editing", "home", E, Home),
    b!("end", "editing", mac "cmd-right", other "", E, End),
    b!("end", "editing", "end", E, End),
    b!("home", "editing", mac "ctrl-a", other "", E, Home),
    b!("end", "editing", mac "ctrl-e", other "", E, End),
    b!("select-home", "editing", mac "cmd-shift-left", other "shift-home", E, SelectHome),
    b!("select-end", "editing", mac "cmd-shift-right", other "shift-end", E, SelectEnd),
    b!("select-all", "editing", primary "a", O, SelectAll),
    b!("copy", "editing", primary "c", O, Copy),
    b!("cut", "editing", primary "x", O, Cut),
    b!("paste", "editing", primary "v", O, Paste),
    b!("undo", "editing", primary "z", O, Undo),
    b!("redo", "editing", primary "shift-z", O, Redo),
    b!("redo", "editing", mac "", other "ctrl-y", O, Redo),
    // Formatting
    b!("bold", "formatting", primary "b", E, Bold),
    b!("italic", "formatting", primary "i", E, Italic),
    b!("code", "formatting", primary "e", E, InlineCode),
    b!("math", "formatting", primary "m", E, WrapMath),
    b!("link", "formatting", primary "k", E, InsertLink),
    b!("highlight", "formatting", primary "shift-y", E, Highlight),
    b!("color-cycle", "formatting", mac "cmd-ctrl-c", other "alt-c", O, CycleColor),
    b!("color-red", "formatting", mac "cmd-ctrl-1", other "alt-1", O, ColorRed),
    b!("color-orange", "formatting", mac "cmd-ctrl-2", other "alt-2", O, ColorOrange),
    b!("color-yellow", "formatting", mac "cmd-ctrl-3", other "alt-3", O, ColorYellow),
    b!("color-green", "formatting", mac "cmd-ctrl-4", other "alt-4", O, ColorGreen),
    b!("color-blue", "formatting", mac "cmd-ctrl-5", other "alt-5", O, ColorBlue),
    b!("color-purple", "formatting", mac "cmd-ctrl-6", other "alt-6", O, ColorPurple),
    b!("color-gray", "formatting", mac "cmd-ctrl-7", other "alt-7", O, ColorGray),
    b!("color-none", "formatting", mac "cmd-ctrl-0", other "alt-0", O, ColorNone),
    b!("kind-bullet", "formatting", mac "cmd-alt-0", other "ctrl-shift-0", O, KindBullet),
    b!("kind-h1", "formatting", mac "cmd-alt-1", other "ctrl-shift-1", O, KindH1),
    b!("kind-h2", "formatting", mac "cmd-alt-2", other "ctrl-shift-2", O, KindH2),
    b!("kind-h3", "formatting", mac "cmd-alt-3", other "ctrl-shift-3", O, KindH3),
    b!("kind-text", "formatting", mac "cmd-alt-4", other "ctrl-shift-4", O, KindParagraph),
    b!("kind-todo", "formatting", mac "cmd-alt-5", other "ctrl-shift-5", O, KindTodo),
    b!("kind-numbered", "formatting", mac "cmd-alt-6", other "ctrl-shift-6", O, KindNumbered),
    b!("kind-quote", "formatting", mac "cmd-alt-7", other "ctrl-shift-7", O, KindQuote),
    b!("kind-code", "formatting", mac "cmd-alt-8", other "ctrl-shift-8", O, KindCode),
    // Outline
    b!("complete", "outline", primary "enter", O, ToggleComplete),
    b!("collapse", "outline", primary "up", O, Collapse),
    b!("expand", "outline", primary "down", O, Expand),
    b!("expand-all", "outline", "ctrl-0", O, ExpandAll),
    b!("level-1", "outline", "ctrl-1", O, Level1),
    b!("level-2", "outline", "ctrl-2", O, Level2),
    b!("level-3", "outline", "ctrl-3", O, Level3),
    b!("level-4", "outline", "ctrl-4", O, Level4),
    b!("move-up", "outline", primary "shift-up", O, MoveUp),
    b!("move-down", "outline", primary "shift-down", O, MoveDown),
    b!("move-to", "outline", primary "shift-m", O, MoveTo),
    b!("mirror", "outline", mac "cmd-alt-m", other "alt-m", O, MirrorBullet),
    b!("link-to", "outline", primary "shift-k", E, LinkToBullet),
    b!("delete-bullet", "outline", primary "shift-backspace", O, DeleteNode),
    b!("toggle-completed", "outline", primary "o", O, ToggleCompletedVisible),
    b!("content-width", "outline", mac "cmd-ctrl-w", other "alt-w", O, CycleContentWidth),
    // Navigation. Off macOS, Alt+←/→ is Back/Forward as in every browser, so
    // it doesn't also zoom while bullets are selected.
    b!("page-back", "navigation", "mouse-back", O, PageBack),
    b!("page-back", "navigation", mac "cmd-alt-left", other "alt-left", O, PageBack),
    b!("page-forward", "navigation", "mouse-forward", O, PageForward),
    b!("page-forward", "navigation", mac "cmd-alt-right", other "alt-right", O, PageForward),
    b!("zoom-in", "navigation", primary "]", O, ZoomIn),
    b!("zoom-in", "navigation", mac "alt-right", other "", S, ZoomIn),
    b!("zoom-out", "navigation", primary "[", O, ZoomOut),
    b!("zoom-out", "navigation", mac "alt-left", other "", S, ZoomOut),
    b!("home-page", "navigation", primary "shift-h", O, ZoomHome),
    b!("scroll-top", "navigation", mac "cmd-alt-up", other "ctrl-home", O, ScrollToTop),
    b!("scroll-bottom", "navigation", mac "cmd-alt-down", other "ctrl-end", O, ScrollToBottom),
    b!("jump", "navigation", primary "p", O, Jump),
    b!("star", "navigation", primary "shift-s", O, ToggleStar),
    b!("today", "navigation", primary "shift-d", O, Today),
    b!("calendar", "navigation", primary "shift-c", O, ShowCalendar),
    // Search
    b!("find", "search", primary "f", O, Find),
    b!("next-match", "search", primary "g", O, NextMatch),
    b!("next-match", "search", mac "", other "f3", O, NextMatch),
    b!("prev-match", "search", primary "shift-g", O, PrevMatch),
    b!("prev-match", "search", mac "", other "shift-f3", O, PrevMatch),
    b!("escape", "search", "escape", O, Escape),
    // App
    b!("import", "app", primary "shift-i", O, Import),
    b!("insert-image", "app", mac "cmd-alt-i", other "alt-shift-i", O, InsertImage),
    b!("export-md", "app", primary "shift-e", O, ExportMarkdown),
    b!("export-package", "app", mac "cmd-alt-e", other "alt-shift-e", O, ExportPackage),
    b!("trash", "app", primary "shift-delete", O, ShowTrash),
    b!("settings", "app", primary ",", None, OpenSettings),
    b!("quit", "app", primary "q", None, Quit),
];

impl Binding {
    /// Default keys on `os` (`""`: unbound there).
    fn keys_on(&self, os: Os) -> &'static str {
        match os {
            Os::Mac => self.mac,
            Os::Windows | Os::Linux => self.other,
        }
    }

    pub fn label(&self) -> &'static str {
        i18n::text(&format!("shortcut.{}", self.id))
    }

    pub fn english_label(&self) -> &'static str {
        i18n::english_text(&format!("shortcut.{}", self.id))
    }
}

/// The keys a binding id currently uses (empty = unbound).
pub fn keys_for(id: &str, overrides: &HashMap<String, Vec<String>>) -> Vec<String> {
    keys_on(platform::OS, id, overrides)
}

fn keys_on(os: Os, id: &str, overrides: &HashMap<String, Vec<String>>) -> Vec<String> {
    if let Some(k) = overrides.get(id) {
        return k.clone();
    }
    let mut out: Vec<String> =
        BINDINGS.iter().filter(|b| b.id == id).map(|b| b.keys_on(os)).filter(|k| !k.is_empty()).map(str::to_string).collect();
    out.dedup();
    out
}

/// A binding's first key as prose ("⌘P", "Ctrl+P") for hints and toasts;
/// its label when it has no key.
pub fn hint(id: &str, overrides: &HashMap<String, Vec<String>>) -> String {
    match keys_for(id, overrides).first() {
        Some(k) => platform::key_text(k),
        None => BINDINGS.iter().find(|b| b.id == id).map(|b| b.label().to_string()).unwrap_or_default(),
    }
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
        } else {
            let key = b.keys_on(platform::OS);
            if !key.is_empty() && !key.starts_with("mouse-") {
                out.extend(bind(key, b, cx));
            }
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

/// Effective (keys, context) pairs of a binding id on `os` after overrides.
fn effective(os: Os, id: &str, overrides: &HashMap<String, Vec<String>>) -> Vec<(String, Option<&'static str>)> {
    BINDINGS
        .iter()
        .filter(|b| b.id == id)
        .flat_map(|b| match overrides.get(id) {
            Some(keys) => keys.iter().filter(|k| !k.is_empty()).map(|k| (k.clone(), b.context)).collect::<Vec<_>>(),
            None if !b.keys_on(os).is_empty() => vec![(b.keys_on(os).to_string(), b.context)],
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
    conflicts_on(platform::OS, overrides)
}

fn conflicts_on(os: Os, overrides: &HashMap<String, Vec<String>>) -> std::collections::HashSet<&'static str> {
    let all: Vec<(&'static str, String, Option<&'static str>)> =
        rows().into_iter().flat_map(|r| effective(os, r.id, overrides).into_iter().map(move |(k, c)| (r.id, k, c))).collect();
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
    Action { label: &'static str, shortcut_id: Option<&'static str>, action: fn() -> Box<dyn Action> },
    Separator,
}

pub struct AppMenu {
    pub name: &'static str,
    pub items: Vec<AppMenuItem>,
}

fn item<A: Action + Default>(label: &'static str, shortcut_id: Option<&'static str>) -> AppMenuItem {
    AppMenuItem::Action { label, shortcut_id, action: || Box::new(A::default()) }
}

/// The application menus for this platform, in the current locale.
pub fn app_menus() -> Vec<AppMenu> {
    menus_for(platform::OS)
}

/// macOS keeps About, Settings and Quit in the app-name menu; elsewhere
/// they go to the File and Help menus.
fn menus_for(os: Os) -> Vec<AppMenu> {
    let app_menu = os == Os::Mac;
    let mut file_items = vec![
        item::<Import>(t!("menu.file.import"), Some("import")),
        item::<InsertImage>(t!("menu.file.insert_image"), Some("insert-image")),
        AppMenuItem::Separator,
        item::<ExportPackage>(t!("menu.file.export_package"), Some("export-package")),
        AppMenuItem::Separator,
        item::<ExportMarkdown>(t!("menu.file.export_markdown"), Some("export-md")),
        item::<ExportOpml>(t!("menu.file.export_opml"), None),
        item::<ExportText>(t!("menu.file.export_text"), None),
        AppMenuItem::Separator,
        item::<ShowTrash>(t!("menu.file.trash"), Some("trash")),
    ];
    if !app_menu {
        file_items.push(AppMenuItem::Separator);
        file_items.push(item::<OpenSettings>(t!("menu.app.settings"), Some("settings")));
        file_items.push(AppMenuItem::Separator);
        file_items.push(item::<Quit>(t!("menu.app.quit"), Some("quit")));
    }

    let mut help_items = vec![
        item::<WhatsNew>(t!("menu.help.whats_new"), None),
        item::<OpenTutorial>(t!("menu.help.tutorial"), None),
        item::<SendFeedback>(t!("menu.help.feedback"), None),
        AppMenuItem::Separator,
        item::<OpenPrivacyPolicy>(t!("menu.help.privacy"), None),
        item::<OpenTerms>(t!("menu.help.terms"), None),
    ];
    if !app_menu {
        help_items.push(AppMenuItem::Separator);
        help_items.push(item::<OpenAbout>(t!("menu.app.about"), None));
    }

    let mut menus = Vec::new();
    if app_menu {
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
                item::<ScrollToTop>(t!("menu.outline.scroll_top"), Some("scroll-top")),
                item::<ScrollToBottom>(t!("menu.outline.scroll_bottom"), Some("scroll-bottom")),
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
                item::<CycleContentWidth>(t!("menu.outline.content_width"), Some("content-width")),
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
    platform::install_window_menu();
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

    const ALL_OS: [Os; 3] = [Os::Mac, Os::Windows, Os::Linux];

    #[test]
    fn default_bindings_have_no_conflicts_and_parse_on_every_platform() {
        for os in ALL_OS {
            let conflicts = conflicts_on(os, &HashMap::new());
            assert!(conflicts.is_empty(), "{os:?}: default bindings conflict: {conflicts:?}");
            for b in BINDINGS {
                let keys = b.keys_on(os);
                if keys.starts_with("mouse-") {
                    continue;
                }
                for stroke in keys.split_whitespace() {
                    gpui::Keystroke::parse(stroke).unwrap_or_else(|e| panic!("{os:?}: invalid keystroke {stroke:?} for {}: {e}", b.id));
                }
            }
        }
    }

    #[test]
    fn every_shortcut_has_a_default_somewhere() {
        for r in rows() {
            assert!(ALL_OS.iter().any(|&os| !keys_on(os, r.id, &HashMap::new()).is_empty()), "{} is unbound everywhere", r.id);
        }
    }

    #[test]
    fn defaults_off_macos_avoid_cmd_and_altgr() {
        for b in BINDINGS {
            for stroke in b.other.split_whitespace() {
                let (mods, _) = platform::split_stroke(stroke);
                assert!(!mods.contains(&"cmd"), "{}: {stroke:?} uses cmd off macOS", b.id);
                // Ctrl+Alt is AltGr on Windows and the window manager's on Linux.
                assert!(!(mods.contains(&"ctrl") && mods.contains(&"alt")), "{}: {stroke:?} is a Ctrl+Alt chord", b.id);
            }
        }
    }

    #[test]
    fn app_menus_reference_known_shortcuts_and_keep_settings_and_quit() {
        for os in ALL_OS {
            let menus = menus_for(os);
            let ids: Vec<&str> = menus
                .iter()
                .flat_map(|m| &m.items)
                .filter_map(|it| match it {
                    AppMenuItem::Action { shortcut_id, .. } => *shortcut_id,
                    AppMenuItem::Separator => None,
                })
                .collect();
            for id in &ids {
                assert!(BINDINGS.iter().any(|b| b.id == *id), "{os:?}: menu references unknown shortcut id {id:?}");
            }
            for id in ["settings", "quit"] {
                assert_eq!(ids.iter().filter(|i| **i == id).count(), 1, "{os:?}: {id} should appear once");
            }
        }
    }
}
