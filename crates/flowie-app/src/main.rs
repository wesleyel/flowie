mod anim;
mod calendar;
mod commands;
mod editor;
mod inline;
mod palette;
mod outline;
mod rich;
mod theme;
mod view;

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Instant;

use flowie_core::{store, Tree, ROOT};
use gpui::{
    px, size, App, AppContext, Application, AssetSource, Bounds, KeyBinding, Menu, MenuItem, SharedString,
    TitlebarOptions, WindowBounds, WindowOptions,
};

use outline::*;

struct Assets;

const CHEVRON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M3 1.5 L7 5 L3 8.5" fill="none" stroke="black" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "icons/chevron.svg" => Some(Cow::Borrowed(CHEVRON.as_bytes())),
            _ => None,
        })
    }

    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec!["icons/chevron.svg".into()])
    }
}

fn data_path() -> PathBuf {
    if let Ok(p) = std::env::var("FLOWIE_DB") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Flowie/flowie.sqlite")
    } else {
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".local/share"))
            .join("flowie/flowie.sqlite")
    }
}

/// `--stress N`: build an in-memory outline of N nodes (not saved) to check speed.
fn stress_tree(n: usize) -> Tree {
    let mut t = Tree::new();
    let words = ["alpha", "beta", "gamma", "delta", "研究", "笔记", "#todo", "@sam", "$x^2$", "**bold**"];
    let mut parents = vec![ROOT];
    for i in 0..n {
        let depth_parent = parents[(i * 7) % parents.len()];
        let text = format!("Item {i} {} {} {}", words[i % 10], words[(i / 10) % 10], words[(i / 100) % 10]);
        let idx = t.children(depth_parent).len();
        let id = t.create_at(depth_parent, idx, &text);
        if i % 5 == 0 && parents.len() < 2000 {
            parents.push(id);
        }
    }
    t.take_journal();
    t.take_dirty();
    t
}

fn bind_keys(cx: &mut App) {
    let e = Some("editing");
    let s = Some("selecting");
    let o = Some("Outline");
    cx.bind_keys([
        // Text editing
        KeyBinding::new("enter", Newline, e),
        KeyBinding::new("shift-enter", ToggleNote, e),
        KeyBinding::new("tab", Indent, o),
        KeyBinding::new("shift-tab", Outdent, o),
        KeyBinding::new("backspace", Backspace, o),
        KeyBinding::new("delete", Delete, o),
        KeyBinding::new("alt-backspace", DeleteWordLeft, e),
        KeyBinding::new("left", Left, e),
        KeyBinding::new("right", Right, e),
        KeyBinding::new("up", Up, o),
        KeyBinding::new("down", Down, o),
        KeyBinding::new("alt-left", WordLeft, e),
        KeyBinding::new("alt-right", WordRight, e),
        KeyBinding::new("shift-left", SelectLeft, e),
        KeyBinding::new("shift-right", SelectRight, e),
        KeyBinding::new("alt-shift-left", SelectWordLeft, e),
        KeyBinding::new("alt-shift-right", SelectWordRight, e),
        KeyBinding::new("shift-up", SelectUp, o),
        KeyBinding::new("shift-down", SelectDown, o),
        KeyBinding::new("cmd-left", Home, e),
        KeyBinding::new("cmd-right", End, e),
        KeyBinding::new("home", Home, e),
        KeyBinding::new("end", End, e),
        KeyBinding::new("cmd-shift-left", SelectHome, e),
        KeyBinding::new("cmd-shift-right", SelectEnd, e),
        KeyBinding::new("cmd-a", SelectAll, o),
        KeyBinding::new("cmd-c", Copy, o),
        KeyBinding::new("cmd-x", Cut, o),
        KeyBinding::new("cmd-v", Paste, o),
        KeyBinding::new("cmd-b", Bold, e),
        KeyBinding::new("cmd-i", Italic, e),
        KeyBinding::new("cmd-e", InlineCode, e),
        KeyBinding::new("cmd-m", WrapMath, e),
        KeyBinding::new("cmd-k", InsertLink, e),
        // Outline
        KeyBinding::new("enter", Newline, s),
        KeyBinding::new("cmd-enter", ToggleComplete, o),
        KeyBinding::new("cmd-up", Collapse, o),
        KeyBinding::new("cmd-down", Expand, o),
        KeyBinding::new("cmd-]", ZoomIn, o),
        KeyBinding::new("cmd-[", ZoomOut, o),
        KeyBinding::new("alt-right", ZoomIn, s),
        KeyBinding::new("alt-left", ZoomOut, s),
        KeyBinding::new("cmd-shift-h", ZoomHome, o),
        KeyBinding::new("cmd-shift-up", MoveUp, o),
        KeyBinding::new("cmd-shift-down", MoveDown, o),
        KeyBinding::new("cmd-shift-backspace", DeleteNode, o),
        KeyBinding::new("cmd-z", Undo, o),
        KeyBinding::new("cmd-shift-z", Redo, o),
        KeyBinding::new("cmd-f", Find, o),
        KeyBinding::new("escape", Escape, o),
        KeyBinding::new("cmd-o", ToggleCompletedVisible, o),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-p", Jump, o),
        KeyBinding::new("cmd-shift-m", MoveTo, o),
        KeyBinding::new("cmd-shift-k", LinkToBullet, e),
        KeyBinding::new("cmd-alt-m", MirrorBullet, o),
        KeyBinding::new("cmd-shift-s", ToggleStar, o),
        KeyBinding::new("ctrl-0", ExpandAll, o),
        KeyBinding::new("ctrl-1", Level1, o),
        KeyBinding::new("ctrl-2", Level2, o),
        KeyBinding::new("ctrl-3", Level3, o),
        KeyBinding::new("ctrl-4", Level4, o),
        KeyBinding::new("cmd-shift-i", Import, o),
        KeyBinding::new("cmd-shift-e", ExportMarkdown, o),
        KeyBinding::new("cmd-shift-delete", ShowTrash, o),
        KeyBinding::new("cmd-shift-y", Highlight, e),
        KeyBinding::new("cmd-alt-0", KindBullet, o),
        KeyBinding::new("cmd-alt-1", KindH1, o),
        KeyBinding::new("cmd-alt-2", KindH2, o),
        KeyBinding::new("cmd-alt-3", KindH3, o),
        KeyBinding::new("cmd-alt-4", KindParagraph, o),
        KeyBinding::new("cmd-alt-5", KindTodo, o),
        KeyBinding::new("cmd-alt-6", KindNumbered, o),
        KeyBinding::new("cmd-alt-7", KindQuote, o),
        KeyBinding::new("cmd-alt-8", KindCode, o),
        KeyBinding::new("cmd-shift-d", Today, o),
        KeyBinding::new("cmd-shift-c", ShowCalendar, o),
        KeyBinding::new("cmd-g", NextMatch, o),
        KeyBinding::new("cmd-shift-g", PrevMatch, o),
    ]);
}

fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu { name: "Flowie".into(), items: vec![MenuItem::action("Quit Flowie", Quit)] },
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

fn main() {
    let t0 = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    let stress = args.iter().position(|a| a == "--stress").and_then(|i| args.get(i + 1)).and_then(|n| n.parse::<usize>().ok());

    let (tree, writer, db_path, meta) = match stress {
        Some(n) => (stress_tree(n), None, None, Vec::new()),
        None => {
            let path = data_path();
            let loaded = store::open(&path).and_then(|c| store::load(&c));
            match loaded {
                Ok((nodes, meta)) => (Tree::from_nodes(nodes), Some(store::Writer::spawn(path.clone())), Some(path), meta),
                Err(e) => {
                    eprintln!("flowie: could not load {}: {e}", path.display());
                    (Tree::new(), None, None, Vec::new())
                }
            }
        }
    };
    let n = tree.len();
    if stress.is_some() {
        // Rough timings for the hot paths on a big outline.
        let t = Instant::now();
        let rows = flowie_core::flatten(&tree, ROOT, &Default::default());
        let flat = t.elapsed();
        let t = Instant::now();
        let opts = flowie_core::ViewOptions { query: Some(flowie_core::Query::parse("gamma 笔记")), ..Default::default() };
        let hits = flowie_core::flatten(&tree, ROOT, &opts);
        let search = t.elapsed();
        eprintln!(
            "flowie: flatten {} rows in {:.2} ms; search -> {} rows in {:.2} ms",
            rows.len(),
            flat.as_secs_f64() * 1e3,
            hits.len(),
            search.as_secs_f64() * 1e3
        );
    }
    let loaded_at = t0.elapsed();

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        flowie_math::load_fonts(cx);
        bind_keys(cx);
        set_menus(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(920.), px(760.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some("Flowie".into()), ..Default::default() }),
                    window_min_size: Some(size(px(420.), px(300.))),
                    app_id: Some("dev.flowie.app".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Outline::new(tree, writer, db_path, meta, window, cx)),
            )
            .expect("open window");
        let view = window.update(cx, |_, _, cx| cx.entity()).unwrap();
        window
            .update(cx, |o, window, _| {
                window.focus(&o.focus);
            })
            .ok();
        cx.activate(true);

        // Flush pending writes before quitting.
        let v = view.clone();
        cx.on_app_quit(move |cx| {
            v.update(cx, |o, _| o.flush());
            async {}
        })
        .detach();
        cx.on_window_closed(|cx| cx.quit()).detach();

        eprintln!("flowie: {n} nodes loaded in {:.1} ms, window at {:.1} ms", loaded_at.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3);
    });
}
