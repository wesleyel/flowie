//! Screenshot mode (cargo feature `screenshots`, used by `scripts/screenshots.py`).
//!
//! Extra `shot.*` methods on the agent socket that put the window into a given
//! state (clean outline, palette open with a query, `/` menu, bullet menu,
//! calendar, settings…) so App Store and website screenshots can be retaken
//! from a script. Never part of a release build.

use flowie_core::{BlockKind, ROOT};
use gpui::{point, px, Context};
use serde_json::Value;

use crate::menu::ContextMenu;
use crate::outline::{BlockSel, Outline, Target};
use crate::palette::Mode;
use crate::settings::Tab;

fn s<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

fn f(p: &Value, k: &str) -> Result<f32, String> {
    p.get(k).and_then(Value::as_f64).map(|v| v as f32).ok_or_else(|| format!("missing `{k}`"))
}

impl Outline {
    pub fn handle_shot(&mut self, method: &str, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        match method {
            // Delete everything (no Trash), stars, saved searches and the journal.
            "wipe" => {
                self.shot_close_ui();
                for kid in self.tree.children(ROOT).to_vec() {
                    self.tree.delete(kid);
                }
                self.starred.clear();
                self.saved_searches.clear();
                self.journal_id = None;
                self.hide_completed = false;
                self.set_meta("starred", "");
                self.set_meta("saved_searches", "");
                self.set_meta("journal", "");
                self.set_meta("hide_completed", "0");
                self.commit_structure(None, cx);
                self.history.seal();
                self.zoom_to(ROOT, cx);
                Ok("wiped".into())
            }
            // Close every popup and leave no caret or selection.
            "reset" => {
                self.shot_close_ui();
                self.query.clear();
                self.refresh(false);
                self.edit = None;
                self.block = None;
                self.anims.clear();
                Ok("reset".into())
            }
            "star" => {
                let id = self.agent_node(p, "id")?;
                if !self.starred.contains(&id) {
                    self.starred.push(id);
                }
                let v = self.starred.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(",");
                self.set_meta("starred", &v);
                Ok("starred".into())
            }
            "save_search" => {
                let q = s(p, "query").ok_or("missing `query`")?.to_string();
                if !self.saved_searches.contains(&q) {
                    self.saved_searches.push(q);
                }
                self.set_meta("saved_searches", &self.saved_searches.join("\n"));
                Ok("saved".into())
            }
            // A mirror of `source` as the last child of `parent`.
            "mirror" => {
                let (source, parent) = (self.agent_node(p, "source")?, self.agent_node(p, "parent")?);
                let index = self.tree.children(parent).len();
                let m = self.tree.create_mirror(source, parent, index).ok_or("can't mirror into itself")?;
                self.commit_structure(None, cx);
                self.history.seal();
                Ok(m.to_string())
            }
            "kind" => {
                let id = self.agent_node(p, "id")?;
                let kind = BlockKind::from_name(s(p, "kind").unwrap_or("")).ok_or("unknown kind")?;
                self.tree.set_kind(id, kind);
                self.commit_structure(None, cx);
                self.history.seal();
                Ok("kind".into())
            }
            "zoom" => {
                let id = if p.get("id").is_some() { self.agent_node(p, "id")? } else { ROOT };
                self.shot_close_ui();
                self.zoom_to(id, cx);
                self.edit = None;
                Ok("zoomed".into())
            }
            // Caret in a bullet's text (`offset` defaults to the end).
            "caret" => {
                let id = self.agent_node(p, "id")?;
                let len = self.tree.node(self.tree.source(id)).text.len();
                let off = p.get("offset").and_then(Value::as_u64).map_or(len, |o| o as usize);
                self.start_edit(Target::Text(id), off, cx);
                Ok("caret".into())
            }
            // Type at the caret one character at a time, as the keyboard does,
            // so `/`, `#`, `@` and `[[` open their menus.
            "type" => {
                let text = s(p, "text").ok_or("missing `text`")?;
                for ch in text.chars() {
                    let at = self.edit.as_ref().map_or(0, |e| e.caret());
                    self.replace_edit_text(at..at, &ch.to_string(), false, cx);
                }
                self.history.seal();
                Ok("typed".into())
            }
            "palette" => {
                self.shot_close_ui();
                self.open_palette(Mode::Jump, cx);
                let q = s(p, "query").unwrap_or("");
                if !q.is_empty() {
                    self.replace_edit_text(0..0, q, false, cx);
                }
                Ok("palette".into())
            }
            "search" => {
                self.shot_close_ui();
                self.search_for(s(p, "query").unwrap_or(""), cx);
                Ok("search".into())
            }
            "calendar" => {
                self.shot_close_ui();
                self.toggle_calendar(cx);
                Ok("calendar".into())
            }
            "settings" => {
                self.shot_close_ui();
                let tab = match s(p, "tab") {
                    Some("appearance") => Tab::Appearance,
                    Some("agent") => Tab::Agent,
                    Some("about") => Tab::About,
                    Some("general") => Tab::General,
                    _ => Tab::Shortcuts,
                };
                self.show_settings(tab, cx);
                Ok("settings".into())
            }
            // The right-click menu of a bullet, drawn at (x, y) in window points.
            "bullet_menu" => {
                let id = self.agent_node(p, "id")?;
                self.shot_close_ui();
                self.edit = None;
                self.block = Some(BlockSel { anchor: id, head: id });
                self.menu = Some(ContextMenu::Bullet { at: point(px(f(p, "x")?), px(f(p, "y")?)), ids: vec![id], restore: None });
                self.menu_scroll = gpui::ScrollHandle::new();
                Ok("menu".into())
            }
            // The export dialog for `id`; `description` fills that field.
            "package_export" => {
                let id = self.agent_node(p, "id")?;
                self.shot_close_ui();
                self.open_package_export(vec![id], cx);
                if let Some(d) = s(p, "description") {
                    self.set_field_text(crate::outline::Field::PackageDescription, d.to_string());
                }
                if p.get("include_completed").and_then(Value::as_bool) == Some(false) {
                    self.set_include_completed(false, cx);
                }
                Ok("package export".into())
            }
            // Open a package file (the import dialog), then `package_import` presses Import.
            "package_open" => {
                self.shot_close_ui();
                self.open_package_file(std::path::Path::new(s(p, "path").ok_or("missing `path`")?), cx);
                Ok(if self.package_import.is_some() {
                    "dialog".into()
                } else {
                    self.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
                })
            }
            "package_import" => {
                self.import_package_now(cx);
                Ok("imported".into())
            }
            // Write what the open export dialog's Export button would save.
            "package_write" => {
                let path = s(p, "path").ok_or("missing `path`")?;
                let (title, bytes) = self.package_file().ok_or("no export dialog")?;
                std::fs::write(path, bytes).map_err(|e| e.to_string())?;
                Ok(title)
            }
            _ => Err(format!("unknown method shot.{method}")),
        }
    }

    fn shot_close_ui(&mut self) {
        self.close_menu();
        self.popover.close();
        self.palette = None;
        self.inline = None;
        self.calendar = None;
        self.settings = None;
        self.settings_return = None;
        self.package_export = None;
        self.package_import = None;
        self.trash = None;
        self.toast = None;
        // A fresh scroll position for the next popup (palette, `/` menu, `[[` menu).
        self.popup_scroll = gpui::ScrollHandle::new();
        if matches!(self.edit.as_ref().map(|e| e.target), Some(Target::Palette | Target::Search)) {
            self.edit = None;
        }
    }
}

/// `FLOWIE_SHOT_APPEARANCE=dark|light`: the whole app's appearance (title bar
/// included) for this process, whatever the Mac is set to. Call before the
/// window opens.
#[cfg(target_os = "macos")]
pub fn apply_appearance() {
    use std::ffi::{c_char, c_void, CStr};
    let name: &CStr = match std::env::var("FLOWIE_SHOT_APPEARANCE").as_deref() {
        Ok("dark") => c"NSAppearanceNameDarkAqua",
        Ok("light") => c"NSAppearanceNameAqua",
        _ => return,
    };
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }
    type Send0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type Send1 = unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void) -> *mut c_void;
    // SAFETY: plain AppKit calls on the main thread with the signatures cast to match.
    unsafe {
        let send0: Send0 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let send1: Send1 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = send0(objc_getClass(c"NSApplication".as_ptr()), sel_registerName(c"sharedApplication".as_ptr()));
        let string = send1(objc_getClass(c"NSString".as_ptr()), sel_registerName(c"stringWithUTF8String:".as_ptr()), name.as_ptr().cast());
        let appearance = send1(objc_getClass(c"NSAppearance".as_ptr()), sel_registerName(c"appearanceNamed:".as_ptr()), string);
        send1(app, sel_registerName(c"setAppearance:".as_ptr()), appearance);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_appearance() {}
