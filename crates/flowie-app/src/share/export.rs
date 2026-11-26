//! Build and export a share package from the chosen bullets.

use flowie_core::package::{ExportMeta, ExportOptions, Package, Person, EXTENSION};
use flowie_core::{NodeId, ROOT};
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, SharedString, Window};

use super::size_label;
use crate::app::i18n::t;
use crate::outline::{ExportPackage, Field, Outline, Target};
use crate::overlays::palette::node_label;
use crate::ui::button::{button, Variant};

/// The open export dialog.
pub struct PackageExport {
    pub roots: Vec<NodeId>,
    pub title: String,
    pub description: String,
    pub include_completed: bool,
    /// Bullets and bytes with the current options.
    nodes: usize,
    bytes: usize,
    /// Some bullet below the roots is completed (the switch shows only then).
    has_completed: bool,
    /// Where the caret was, put back on close.
    return_to: Option<(Target, usize)>,
}

/// A file name for `title`: no path separators or characters Windows refuses.
fn file_name(title: &str) -> String {
    let safe: String = title.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '-' } else { c }).take(60).collect();
    let safe = safe.trim().trim_matches('.');
    format!("{}.{EXTENSION}", if safe.is_empty() { "Flowie" } else { safe })
}

impl Outline {
    /// File → Export as Package…: the selected bullets, else the one being
    /// edited, else the page.
    pub fn export_package(&mut self, _: &ExportPackage, _: &mut Window, cx: &mut Context<Self>) {
        let mut roots = self.targets();
        if roots.is_empty() {
            roots = if self.zoom == ROOT { self.ws.tree.children(ROOT).to_vec() } else { vec![self.zoom] };
        }
        self.open_package_export(roots, cx);
    }

    pub fn open_package_export(&mut self, roots: Vec<NodeId>, cx: &mut Context<Self>) {
        let roots: Vec<NodeId> = roots.into_iter().filter(|id| self.ws.tree.contains(*id)).map(|id| self.ws.tree.resolve(id)).collect();
        let Some(&first) = roots.first() else {
            self.toast(t!("toast.nothing_to_export"));
            return cx.notify();
        };
        self.palette = None;
        self.inline = None;
        self.calendar = None;
        self.menu = None;
        let title = match roots.len() {
            1 => node_label(self, first),
            n if self.zoom != ROOT && n == self.ws.tree.children(self.zoom).len() => node_label(self, self.zoom),
            _ => node_label(self, first),
        };
        let return_to = self.edit.take().map(|e| (e.target, e.caret()));
        let all = self.build_package(&roots, &title, "", true);
        let open = self.build_package(&roots, &title, "", false);
        let mut export = PackageExport {
            roots,
            title,
            description: String::new(),
            include_completed: true,
            nodes: 0,
            bytes: 0,
            has_completed: all.outline.nodes.len() != open.outline.nodes.len(),
            return_to,
        };
        export.nodes = all.outline.nodes.len();
        export.bytes = all.to_bytes().len();
        self.package_export = Some(export);
        // The title selected, ready to be typed over.
        let len = self.field_text(Field::PackageTitle).len();
        self.start_edit(Target::Field(Field::PackageTitle), len, cx);
        if let Some(e) = self.edit.as_mut() {
            e.sel = 0..len;
        }
        cx.notify();
    }

    fn build_package(&self, roots: &[NodeId], title: &str, description: &str, include_completed: bool) -> Package {
        let meta = ExportMeta {
            title: title.trim().to_string(),
            description: description.trim().to_string(),
            author: Person { id: self.config.author.id.clone(), name: self.author_name(), device: self.device_id.clone() },
            app: format!("Flowie {}", crate::pages::whats_new::running_version()),
        };
        Package::export(&self.ws.tree, roots, ExportOptions { include_completed }, meta, self.ws.image_dir.as_ref())
    }

    pub fn set_include_completed(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(p) = self.package_export.as_ref() else { return };
        let package = self.build_package(&p.roots, &p.title, &p.description, on);
        let (nodes, bytes) = (package.outline.nodes.len(), package.to_bytes().len());
        if let Some(p) = self.package_export.as_mut() {
            p.include_completed = on;
            p.nodes = nodes;
            p.bytes = bytes;
        }
        cx.notify();
    }

    pub fn close_package_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.package_export.take() else { return };
        window.focus(&self.focus, cx);
        self.edit = None;
        if let Some((target, offset)) = p.return_to.filter(|(t, _)| !matches!(t, Target::Field(_))) {
            self.start_edit(target, offset, cx);
        }
        cx.notify();
    }

    /// The open dialog's title (its default if left empty) and the file it exports.
    pub fn package_file(&self) -> Option<(String, Vec<u8>)> {
        let p = self.package_export.as_ref()?;
        let title = if p.title.trim().is_empty() { node_label(self, p.roots[0]) } else { p.title.trim().to_string() };
        let bytes = self.build_package(&p.roots, &title, &p.description, p.include_completed).to_bytes();
        Some((title, bytes))
    }

    /// Export: ask where to save, then write the file.
    fn finish_package_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((title, bytes)) = self.package_file() else { return };
        self.close_package_export(window, cx);
        let dir = dirs::desktop_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some(&file_name(&title)));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let msg = match std::fs::write(&path, bytes) {
                Ok(()) => t!("toast.exported", path = path.display()),
                Err(e) => t!("toast.export_failed", error = e),
            };
            this.update(cx, |this, cx| {
                this.toast(msg);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Keys while the dialog is open (the outline's bindings are off).
    pub fn package_export_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let plain = !ks.modifiers.modified();
        let editing = self.edit.as_ref().and_then(|e| match e.target {
            Target::Field(f) => Some(f),
            _ => None,
        });
        match ks.key.as_str() {
            "escape" if plain => self.close_package_export(window, cx),
            "enter" if plain => self.finish_package_export(window, cx),
            "tab" if !ks.modifiers.platform && !ks.modifiers.control && !ks.modifiers.alt => {
                let next = match editing {
                    Some(Field::PackageTitle) => Field::PackageDescription,
                    _ => Field::PackageTitle,
                };
                let len = self.field_text(next).len();
                self.start_edit(Target::Field(next), len, cx);
                if let Some(e) = self.edit.as_mut() {
                    e.sel = 0..len;
                }
            }
            _ => {
                if !self.field_key(ks, window, cx) {
                    return;
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// A one-line text field bound to `field`.
    fn dialog_field(&mut self, field: Field, placeholder: SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let editing = self.edit.as_ref().is_some_and(|e| e.target == Target::Field(field));
        let text = self.field_text(field).to_string();
        let inner = if editing {
            let (font, _) = self.base_style(window, false);
            let run =
                gpui::TextRun { len: text.len(), font, color: theme.text, background_color: None, underline: None, strikethrough: None };
            let ed = self.editor_element(text, vec![run], theme.caret, theme.selection, cx);
            self.editor_mouse(cx).child(ed).into_any_element()
        } else if text.is_empty() {
            div().text_color(theme.faint).child(placeholder).into_any_element()
        } else {
            div().text_color(theme.text).overflow_hidden().whitespace_nowrap().text_ellipsis().child(text).into_any_element()
        };
        crate::ui::controls::search_field(theme, SharedString::from(format!("{field:?}")), editing)
            .w_full()
            .child(div().flex_1().min_w_0().child(inner))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if this.edit.as_ref().map(|e| e.target) != Some(Target::Field(field)) {
                        let len = this.field_text(field).len();
                        this.start_edit(Target::Field(field), len, cx);
                    }
                }),
            )
            .into_any_element()
    }

    pub fn render_package_export(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.package_export.as_ref()?;
        let theme = self.theme;
        let (nodes, bytes, include_completed, has_completed) = (p.nodes, p.bytes, p.include_completed, p.has_completed);
        let label = |text: &'static str| div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(theme.muted).child(text);
        let title_field = self.dialog_field(Field::PackageTitle, t!("package.export.title_placeholder").into(), window, cx);
        let description_field =
            self.dialog_field(Field::PackageDescription, t!("package.export.description_placeholder").into(), window, cx);
        let completed = has_completed.then(|| {
            div()
                .id("package-include-completed")
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .child(crate::ui::controls::checkbox(theme, include_completed))
                .child(t!("package.export.include_completed"))
                .on_click(cx.listener(move |this, _, _, cx| this.set_include_completed(!include_completed, cx)))
        });
        // The default name is the OS's full name, often a real name: say it's in the file.
        let author = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .text_size(px(12.))
            .text_color(theme.muted)
            .child(t!("package.export.author", name = self.author_name()))
            .child(button(theme, "package-change-author", Variant::Link).child(t!("package.export.change_author")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.show_settings(crate::settings::Tab::General, cx);
                },
            )));
        let buttons = div()
            .flex()
            .justify_end()
            .gap_2()
            .child(
                button(theme, "package-cancel", Variant::Secondary)
                    .child(t!("package.export.cancel"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_package_export(window, cx))),
            )
            .child(
                button(theme, "package-export", Variant::Primary)
                    .child(t!("package.export.export"))
                    .on_click(cx.listener(|this, _, window, cx| this.finish_package_export(window, cx))),
            );
        let card = crate::ui::surface::sheet(theme, "package-export-card")
            .w(px(440.))
            .max_w(gpui::relative(0.9))
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.end_field_edit() {
                        cx.notify();
                    }
                }),
            )
            .child(crate::ui::text::page_title(
                theme,
                t!("package.export.heading"),
                Some(t!("package.export.summary", count = nodes, size = size_label(bytes)).into()),
            ))
            .child(div().flex().flex_col().gap_1().child(label(t!("package.export.title"))).child(title_field))
            .child(div().flex().flex_col().gap_1().child(label(t!("package.export.description"))).child(description_field))
            .children(completed)
            .child(div().h(px(1.)).bg(theme.divider))
            .child(author)
            .child(buttons);
        Some(
            crate::ui::surface::scrim(theme)
                .items_center()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_package_export(window, cx)))
                .child(card)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("Reading: notes/2026"), "Reading- notes-2026.flowie");
        assert_eq!(file_name("  ..  "), "Flowie.flowie");
        assert_eq!(file_name(&"x".repeat(100)), format!("{}.flowie", "x".repeat(60)));
    }
}
