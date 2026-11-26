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
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use flowie_core::{images::ImageDir, Tree, ROOT};
use flowie_doc::Workspace;
use gpui::{
    px, size, App, AppContext, AssetSource, Bounds, Entity, Pixels, QuitMode, SharedString, Size, TitlebarOptions, WindowBounds,
    WindowHandle, WindowOptions,
};

use app::i18n::t;
use app::{i18n, keymap, platform, tray};
#[cfg(feature = "screenshots")]
use dev::shots;
use outline::{CloseWindow, Outline, Quit};
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

/// Owns the single `Outline` entity and opens/reopens its GPUI window on demand
/// (on Windows and Linux the window can be closed to the system tray while the
/// app and its MCP server keep running).
#[derive(Clone)]
struct WindowHost {
    outline: Entity<Outline>,
    window: Rc<RefCell<Option<WindowHandle<Outline>>>>,
    default_size: Size<Pixels>,
    min_size: Size<Pixels>,
}

impl WindowHost {
    fn open(&self, focus: bool, cx: &mut App) -> Option<WindowHandle<Outline>> {
        if let Some(existing) = *self.window.borrow() {
            if existing
                .update(cx, |_, window, cx| {
                    if focus {
                        cx.activate(true);
                        window.activate_window();
                    }
                })
                .is_ok()
            {
                return Some(existing);
            }
            *self.window.borrow_mut() = None;
        }
        let saved = if cfg!(feature = "screenshots") {
            None
        } else {
            self.outline.read(cx).window_place().and_then(|place| place.open_at(self.min_size, cx))
        };
        let (display_id, window_bounds) =
            saved.map_or_else(|| (None, WindowBounds::Windowed(Bounds::centered(None, self.default_size, cx))), |(d, b)| (Some(d), b));
        let outline = self.outline.clone();
        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds),
                    display_id,
                    titlebar: Some(TitlebarOptions { title: Some("Flowie".into()), ..Default::default() }),
                    window_min_size: Some(self.min_size),
                    app_id: Some("flowie.spokey.cn".into()),
                    // Linux: the window manager's title bar where it offers one; GNOME on
                    // Wayland doesn't, and Flowie draws its own (app::window_frame).
                    window_decorations: Some(gpui::WindowDecorations::Server),
                    // Started for an agent: appear without taking focus.
                    focus,
                    ..Default::default()
                },
                move |_, _| outline,
            )
            .ok()?;
        *self.window.borrow_mut() = Some(handle);
        let window_slot = Rc::clone(&self.window);
        handle
            .update(cx, |o, window, cx| {
                o.attach_window(window, cx);
                let weak = cx.weak_entity();
                window.on_window_should_close(cx, move |window, cx| {
                    let close = weak.update(cx, |o, cx| o.handle_window_close(window, cx)).unwrap_or(true);
                    if close {
                        *window_slot.borrow_mut() = None;
                    }
                    close
                });
            })
            .ok();
        if focus {
            cx.activate(true);
        }
        Some(handle)
    }

    fn close_window(&self, cx: &mut App) {
        let Some(handle) = *self.window.borrow() else { return };
        let window_slot = Rc::clone(&self.window);
        let _ = handle.update(cx, |outline, window, cx| {
            if outline.handle_window_close(window, cx) {
                *window_slot.borrow_mut() = None;
                window.remove_window();
            }
        });
    }

    fn quit(&self, cx: &mut App) {
        if let Some(handle) = *self.window.borrow() {
            let window_slot = Rc::clone(&self.window);
            if handle
                .update(cx, |outline, window, cx| {
                    if outline.request_quit(window, cx) {
                        *window_slot.borrow_mut() = None;
                        if platform::OS == platform::Os::Mac {
                            window.remove_window();
                        } else {
                            cx.quit();
                        }
                    }
                })
                .is_ok()
            {
                return;
            }
            *self.window.borrow_mut() = None;
        }
        // Hidden in the tray: flush first; if saving fails, reopen the window to show the warning.
        if self.outline.update(cx, |outline, _| outline.flush()).is_ok() {
            cx.quit();
        } else if let Some(handle) = self.open(true, cx) {
            let _ = handle.update(cx, |outline, window, cx| {
                if outline.request_quit(window, cx) {
                    cx.quit();
                }
            });
        }
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
    // Without the database, the window shows an empty outline that isn't saved, and the
    // database file is neither upgraded nor replaced (see `store::open`).
    let mut load_error = None;
    let (ws, db_path, meta) = match stress {
        Some(n) => (Workspace::new(stress_tree(n)), None, Vec::new()),
        None => {
            let path = data_path();
            let err_tx = save_err_tx.clone();
            let opened = Workspace::open(&path, ImageDir::beside(&path), move |status| {
                let _ = err_tx.unbounded_send(status);
            });
            match opened {
                Ok((ws, meta)) => (ws, Some(path), meta),
                Err(e) => {
                    eprintln!("flowie: could not load {}: {e}", path.display());
                    load_error = Some(e);
                    (Workspace::new(Tree::new()), None, Vec::new())
                }
            }
        }
    };
    drop(save_err_tx);
    let n = ws.tree.len();
    let config = settings::Config::load(&settings::Config::path(db_path.as_ref()));
    i18n::set(&config.language);
    let load_error =
        load_error.map(|e| if e.is_too_new() { t!("outline.db_too_new").to_string() } else { t!("outline.load_error", error = e) });
    // Without a loaded database there is nothing to serve.
    let listener = listener.filter(|_| db_path.is_some());
    let keys = config.keys.clone();
    if stress.is_some() {
        // Rough timings for the hot paths on a big outline.
        let t = Instant::now();
        let rows = flowie_core::flatten(&ws.tree, ROOT, &Default::default());
        let flat = t.elapsed();
        let t = Instant::now();
        let opts = flowie_core::ViewOptions { query: Some(flowie_core::Query::parse("gamma 笔记")), ..Default::default() };
        let hits = flowie_core::flatten(&ws.tree, ROOT, &opts);
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
    let app = platform::choose_display(gpui::application()).with_assets(Assets);
    let url_tx = open_tx.clone();
    app.on_open_urls(move |urls| {
        for p in urls.iter().filter_map(|u| share::path_from_url(u)).filter(|p| share::is_package(p)) {
            let _ = url_tx.unbounded_send(p);
        }
    });
    app.run(move |cx: &mut App| {
        flowie_math::load_fonts(cx);
        keymap::install(cx, &keys);
        cx.set_quit_mode(if platform::OS == platform::Os::Mac { QuitMode::LastWindowClosed } else { QuitMode::Explicit });

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

        let (tray_tx, mut tray_rx) = futures::channel::mpsc::unbounded::<tray::TrayEvent>();
        let tray_handle = tray::Tray::start(tray::TrayLabels::current(), tray_tx).map(Rc::new);

        let outline = cx.new(|cx| Outline::new(ws, db_path, meta, config, cx));
        outline.update(cx, |o, cx| {
            o.load_error = load_error;
            o.tray = tray_handle;
            o.whats_new_on_launch(background, cx);
        });

        let host = WindowHost { outline: outline.clone(), window: Rc::new(RefCell::new(None)), default_size: window_size, min_size };
        host.open(!background, cx).expect("open window");

        {
            let host = host.clone();
            cx.on_action(move |_: &CloseWindow, cx| {
                let host = host.clone();
                cx.defer(move |cx| host.close_window(cx));
            });
        }
        {
            let host = host.clone();
            cx.on_action(move |_: &Quit, cx| {
                let host = host.clone();
                cx.defer(move |cx| host.quit(cx));
            });
        }
        if let Some(listener) = listener {
            let show_host = host.clone();
            agent::start(
                listener,
                outline.clone(),
                Rc::new(move |cx| {
                    let _ = show_host.open(true, cx);
                }),
                cx,
            );
        }
        {
            let host = host.clone();
            cx.spawn(async move |cx| {
                use futures::StreamExt;
                while let Some(event) = tray_rx.next().await {
                    cx.update(|cx| match event {
                        tray::TrayEvent::Show => {
                            let _ = host.open(true, cx);
                        }
                        tray::TrayEvent::Quit => host.quit(cx),
                    });
                }
            })
            .detach();
        }
        {
            let outline = outline.clone();
            cx.spawn(async move |cx| {
                use futures::StreamExt;
                while let Some(status) = save_err_rx.next().await {
                    outline.update(cx, |o, cx| {
                        o.save_error = status;
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        {
            let host = host.clone();
            cx.spawn(async move |cx| {
                use futures::StreamExt;
                while let Some(path) = open_rx.next().await {
                    cx.update(|cx| {
                        let _ = host.open(true, cx);
                        host.outline.update(cx, |o, cx| o.open_package_file(&path, cx));
                    });
                }
            })
            .detach();
        }
        drop(open_tx);

        cx.on_app_quit(move |cx| {
            outline.update(cx, |outline, _| {
                // Drop the tray handle immediately so the icon disappears on shutdown.
                outline.tray = None;
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
