#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod app;
#[cfg(feature = "screenshots")]
mod dev;
mod image;
mod outline;
mod overlays;
mod pages;
mod settings;
mod share;
mod text;
mod ui;

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Instant;

use flowie_core::{store, Tree, ROOT};
use gpui::{px, size, App, AppContext, AssetSource, Bounds, QuitMode, SharedString, TitlebarOptions, WindowBounds, WindowOptions};

use app::{i18n, keymap, platform, window_place};
#[cfg(feature = "screenshots")]
use dev::shots;
use outline::{Outline, Quit};
use ui::icons;

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(icons::ICONS.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(icons::ICONS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

/// The outline database: `FLOWIE_DB`, or `flowie.sqlite` in the platform's data folder.
pub fn data_path() -> PathBuf {
    match std::env::var_os("FLOWIE_DB") {
        Some(p) => PathBuf::from(p),
        None => platform::data_dir().join("flowie.sqlite"),
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
    // A share package to open, passed by a file association (Windows, Linux) or by hand.
    let open_path = args.iter().skip(1).map(PathBuf::from).find(|p| share::is_package(p)).map(|p| std::path::absolute(&p).unwrap_or(p));
    let stress = args.iter().position(|a| a == "--stress").and_then(|i| args.get(i + 1)).and_then(|n| n.parse::<usize>().ok());

    // One instance per database: a second launch brings the first one forward.
    let listener = match stress {
        Some(_) => None,
        None => match agent::claim(&data_path()) {
            agent::Claim::Owner(listener) => Some(listener),
            agent::Claim::Running(running) => {
                agent::hand_off(running, background, open_path.as_deref());
                return;
            }
            agent::Claim::Unresponsive => {
                eprintln!("flowie: another Flowie has {} open but isn't responding", data_path().display());
                std::process::exit(1);
            }
            agent::Claim::Unavailable(e) => {
                eprintln!("flowie: agent socket unavailable, running without it: {e}");
                None
            }
        },
    };

    let (save_err_tx, mut save_err_rx) = futures::channel::mpsc::unbounded::<Option<String>>();
    let (tree, writer, db_path, meta, imports, blobs) = match stress {
        Some(n) => (stress_tree(n), None, None, Vec::new(), Vec::new(), Vec::new()),
        None => {
            let path = data_path();
            let loaded =
                store::open(&path).and_then(|conn| Ok((store::load(&conn)?, store::load_imports(&conn)?, store::load_blobs(&conn)?, conn)));
            match loaded {
                Ok(((nodes, meta), imports, blobs, conn)) => {
                    Outline::collect_images(&path);
                    let err_tx = save_err_tx.clone();
                    let writer = store::Writer::spawn_with_error_hook(conn, move |status| {
                        let _ = err_tx.unbounded_send(status);
                    });
                    (Tree::from_nodes(nodes), Some(writer), Some(path), meta, imports, blobs)
                }
                Err(e) => {
                    eprintln!("flowie: could not load {}: {e}", path.display());
                    (Tree::new(), None, None, Vec::new(), Vec::new(), Vec::new())
                }
            }
        }
    };
    drop(save_err_tx);
    let n = tree.len();
    let config = settings::Config::load(&settings::Config::path(db_path.as_ref()));
    i18n::set(&config.language);
    // Without a loaded database there is nothing to serve.
    let listener = listener.filter(|_| db_path.is_some());
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

    // Files macOS asks to open (a double-click on a package), before or after launch.
    let (open_tx, mut open_rx) = futures::channel::mpsc::unbounded::<PathBuf>();
    if let Some(p) = open_path {
        let _ = open_tx.unbounded_send(p);
    }
    let app = gpui::application().with_assets(Assets);
    let url_tx = open_tx.clone();
    app.on_open_urls(move |urls| {
        for p in urls.iter().filter_map(|u| share::path_from_url(u)).filter(|p| share::is_package(p)) {
            let _ = url_tx.unbounded_send(p);
        }
    });
    app.run(move |cx: &mut App| {
        flowie_math::load_fonts(cx);
        keymap::install(cx, &keys);
        cx.set_quit_mode(QuitMode::LastWindowClosed);

        // Screenshot builds: a fixed window size (points) for 2880×1800 shots on a Retina display.
        #[cfg(feature = "screenshots")]
        shots::apply_appearance();
        #[cfg(feature = "screenshots")]
        let window_size = std::env::var("FLOWIE_WINDOW_SIZE")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(w, h)| Some(size(px(w.parse().ok()?), px(h.parse().ok()?)))))
            .unwrap_or(size(px(1440.), px(900.)));
        #[cfg(not(feature = "screenshots"))]
        let window_size = size(px(920.), px(760.));
        let min_size = size(px(420.), px(300.));
        // Reopen where the window was left; screenshot builds always use their fixed size.
        let saved = if cfg!(feature = "screenshots") {
            None
        } else {
            meta.iter().find(|(k, _)| k == "window").and_then(|(_, v)| window_place::Place::parse(v)?.open_at(min_size, cx))
        };
        let (display_id, window_bounds) =
            saved.map_or_else(|| (None, WindowBounds::Windowed(Bounds::centered(None, window_size, cx))), |(d, b)| (Some(d), b));
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds),
                    display_id,
                    titlebar: Some(TitlebarOptions { title: Some("Flowie".into()), ..Default::default() }),
                    window_min_size: Some(min_size),
                    app_id: Some("flowie.spokey.cn".into()),
                    // Started for an agent: appear without taking focus.
                    focus: !background,
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Outline::new(tree, writer, db_path, meta, imports, blobs, config, window, cx)),
            )
            .expect("open window");
        window
            .update(cx, |o, window, cx| {
                window.focus(&o.focus, cx);
                o.whats_new_on_launch(background, cx);
                let outline = cx.weak_entity();
                window.on_window_should_close(cx, move |window, cx| {
                    outline.update(cx, |outline, cx| outline.request_close(window, cx)).unwrap_or(true)
                });
            })
            .ok();
        cx.on_action(move |_: &Quit, cx| {
            cx.defer(move |cx| {
                let _ = window.update(cx, |outline, window, cx| {
                    if outline.request_close(window, cx) {
                        window.remove_window();
                    }
                });
            });
        });
        if !background {
            cx.activate(true);
        }
        if let Some(listener) = listener {
            agent::start(listener, window, cx);
        }
        cx.spawn(async move |cx| {
            use futures::StreamExt;
            while let Some(status) = save_err_rx.next().await {
                window
                    .update(cx, |o, _, cx| {
                        o.save_error = status;
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            use futures::StreamExt;
            while let Some(path) = open_rx.next().await {
                window
                    .update(cx, |o, window, cx| {
                        cx.activate(true);
                        window.activate_window();
                        o.open_package_file(&path, cx);
                    })
                    .ok();
            }
        })
        .detach();
        drop(open_tx);

        cx.on_app_quit(move |cx| {
            let _ = window.update(cx, |outline, _, _| {
                if let Err(error) = outline.flush() {
                    eprintln!("flowie: could not save before application shutdown: {error}");
                }
            });
            async {}
        })
        .detach();

        eprintln!(
            "flowie: {n} nodes loaded in {:.1} ms, window at {:.1} ms",
            loaded_at.as_secs_f64() * 1e3,
            t0.elapsed().as_secs_f64() * 1e3
        );
    });
}
