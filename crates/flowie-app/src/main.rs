mod agent;
mod anim;
mod calendar;
mod commands;
mod editor;
mod format_menu;
mod i18n;
mod icons;
mod inline;
mod keymap;
mod menu;
mod outline;
mod palette;
mod popover;
mod rich;
mod selectable;
mod settings;
mod theme;
mod tutorial;
mod view;

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Instant;

use flowie_core::{store, Tree, ROOT};
use gpui::{px, size, App, AppContext, AssetSource, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions};

use outline::*;

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(icons::ICONS.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(icons::ICONS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

/// Inside the App Sandbox (Mac App Store build): the container's `Data`
/// directory. Built from the real home directory and the container id, so
/// `flowie mcp` started by an agent from a terminal (whose `HOME` is the
/// user's real home) finds the same database and socket as the app.
#[cfg(target_os = "macos")]
pub fn sandbox_container() -> Option<PathBuf> {
    let id = std::env::var("APP_SANDBOX_CONTAINER_ID").ok().filter(|s| !s.is_empty())?;
    // SAFETY: getpwuid returns a pointer to static storage or null; pw_dir is a C string.
    let real_home = unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned()
    };
    Some(PathBuf::from(real_home).join("Library/Containers").join(id).join("Data"))
}

#[cfg(not(target_os = "macos"))]
pub fn sandbox_container() -> Option<PathBuf> {
    None
}

pub fn data_path() -> PathBuf {
    if let Ok(p) = std::env::var("FLOWIE_DB") {
        return PathBuf::from(p);
    }
    let home = sandbox_container().or_else(|| std::env::var("HOME").ok().map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Flowie/flowie.sqlite")
    } else {
        std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".local/share")).join("flowie/flowie.sqlite")
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

fn main() {
    let t0 = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("mcp") {
        agent::mcp::run(&data_path());
        return;
    }
    let background = args.iter().any(|a| a == "--background");
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
    let config = settings::Config::load(&settings::Config::path(db_path.as_ref()));
    i18n::set(&config.language);
    let agent_db = db_path.clone();
    let keys = config.keys.clone();
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

    gpui::application().with_assets(Assets).run(move |cx: &mut App| {
        flowie_math::load_fonts(cx);
        keymap::install(cx, &keys);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(920.), px(760.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some("Flowie".into()), ..Default::default() }),
                    window_min_size: Some(size(px(420.), px(300.))),
                    app_id: Some("flowie.spokey.cn".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Outline::new(tree, writer, db_path, meta, config, window, cx)),
            )
            .expect("open window");
        let view = window.update(cx, |_, _, cx| cx.entity()).unwrap();
        window
            .update(cx, |o, window, cx| {
                window.focus(&o.focus, cx);
            })
            .ok();
        if !background {
            cx.activate(true);
        }
        if let Some(db) = &agent_db {
            agent::start(db, view.clone(), cx);
        }

        // Flush pending writes before quitting.
        let v = view.clone();
        cx.on_app_quit(move |cx| {
            v.update(cx, |o, _| o.flush());
            async {}
        })
        .detach();
        cx.on_window_closed(|cx, _| cx.quit()).detach();

        eprintln!(
            "flowie: {n} nodes loaded in {:.1} ms, window at {:.1} ms",
            loaded_at.as_secs_f64() * 1e3,
            t0.elapsed().as_secs_f64() * 1e3
        );
    });
}
