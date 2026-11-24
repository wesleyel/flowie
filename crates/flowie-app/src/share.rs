//! Share packages (`.flowie`) in the app: the export and import dialogs,
//! and the unread dot on imported roots. The format itself is
//! `flowie_core::package`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use flowie_core::images::{BlobInfo, ImageDir};
use flowie_core::package::{ExportMeta, ExportOptions, ImportOptions, Package, PackageError, Person, UpdatePlan, EXTENSION};
use flowie_core::store::{ImportRecord, ImportState};
use flowie_core::{NodeId, ROOT};
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, KeyDownEvent, MouseButton, SharedString, Window};

use crate::i18n::t;
use crate::outline::{ExportPackage, Field, Outline, Target};
use crate::palette::node_label;
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

/// "12 KB", "1.4 MB".
fn size_label(bytes: usize) -> String {
    let kb = bytes.div_ceil(1024).max(1);
    if kb < 1024 {
        t!("package.size_kb", n = kb)
    } else {
        t!("package.size_mb", n = format!("{:.1}", bytes as f64 / (1024.0 * 1024.0)))
    }
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
            roots = if self.zoom == ROOT { self.tree.children(ROOT).to_vec() } else { vec![self.zoom] };
        }
        self.open_package_export(roots, cx);
    }

    pub fn open_package_export(&mut self, roots: Vec<NodeId>, cx: &mut Context<Self>) {
        let roots: Vec<NodeId> = roots.into_iter().filter(|id| self.tree.contains(*id)).map(|id| self.tree.resolve(id)).collect();
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
            n if self.zoom != ROOT && n == self.tree.children(self.zoom).len() => node_label(self, self.zoom),
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
            app: format!("Flowie {}", crate::whats_new::running_version()),
        };
        Package::export(&self.tree, roots, ExportOptions { include_completed }, meta, self.image_dir.as_ref())
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

/// Largest package file Flowie opens.
const MAX_FILE: u64 = 512 << 20;

/// Whether `path` names a share package (by its extension).
pub fn is_package(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// A path from a `file://` URL (what macOS hands over on a double-click).
pub fn path_from_url(url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(String::from_utf8(out).ok()?.into())
}

/// The open import dialog.
pub struct PackageImport {
    comparison: ImportComparison,
    scroll: gpui::ScrollHandle,
    package: Package,
    /// Where the roots go (at the end).
    pub parent: NodeId,
    keep_completion: bool,
    has_completed: bool,
    bytes: usize,
    /// The first levels, as (depth, plain text).
    preview: Vec<(usize, String)>,
    /// Bullets the preview leaves out.
    more: usize,
    return_to: Option<(Target, usize)>,
}

struct ImportComparison {
    revision: u64,
    before: ImportState,
    already: bool,
    update: Option<(UpdatePlan, HashSet<NodeId>)>,
}

fn write_package_images(package: &Package, dir: Option<&ImageDir>) -> Result<Vec<BlobInfo>, String> {
    package.images.values().map(|bytes| crate::image::write_image(dir, bytes)).collect()
}

#[cfg(feature = "screenshots")]
impl PackageImport {
    /// The update plan and the conflicts taken from the package (`shot.package_update`).
    pub fn update_mut(&mut self) -> Option<&mut (UpdatePlan, HashSet<NodeId>)> {
        self.comparison.update.as_mut()
    }
}

/// Conflicts the update dialog lists (the rest keep the user's version).
const CONFLICTS_SHOWN: usize = 6;

/// Lines of the import preview.
const PREVIEW_LINES: usize = 8;

fn copied_imports<'a>(package: &Package, tree: &flowie_core::Tree, records: &'a HashMap<NodeId, ImportRecord>) -> Vec<&'a ImportRecord> {
    let roots: HashSet<_> = package.outline.roots.iter().copied().collect();
    let mut batches = BTreeMap::<_, Vec<_>>::new();
    for record in records.values().filter(|record| record.package_id == package.manifest.package_id) {
        if let Some(&(source, _)) = record.source_ids.iter().find(|&&(source, local)| local == record.root && source != local) {
            if roots.contains(&source) {
                batches.entry(record.imported_at).or_default().push((source, record));
            }
        }
    }
    let complete = |batch: &[(NodeId, &'a ImportRecord)]| {
        batch.len() == roots.len()
            && batch.iter().all(|&(_, record)| tree.contains(record.root))
            && batch.iter().map(|&(source, _)| source).collect::<HashSet<_>>() == roots
    };
    let batch = batches.values().rev().find(|batch| complete(batch)).cloned().unwrap_or_else(|| {
        let batch: Vec<_> = batches.into_values().flatten().collect();
        if complete(&batch) {
            batch
        } else {
            Vec::new()
        }
    });
    let by_source: HashMap<_, _> = batch.into_iter().collect();
    package.outline.roots.iter().filter_map(|source| by_source.get(source).copied()).collect()
}

fn package_import_state(package: &Package, id_map: &HashMap<NodeId, NodeId>, imported_at: i64) -> ImportState {
    let root_of = package.root_of();
    let mut source_ids = HashMap::<_, Vec<_>>::new();
    let bases = package
        .mapped_base_hashes(id_map)
        .map(|(source, hash)| {
            let root = id_map[&root_of[&source]];
            let local = id_map[&source];
            source_ids.entry(root).or_default().push((source, local));
            (local, root, hash)
        })
        .collect();
    let roots: Vec<_> = package.outline.roots.iter().map(|source| id_map[source]).collect();
    let manifest = &package.manifest;
    let records = roots
        .iter()
        .map(|&root| ImportRecord {
            root,
            package_id: manifest.package_id.clone(),
            title: manifest.title.clone(),
            author_id: manifest.exported_by.id.clone(),
            author_name: manifest.exported_by.name.clone(),
            exported_at: manifest.exported_at_millis(),
            imported_at,
            seen_at: None,
            source_ids: source_ids.remove(&root).unwrap_or_default(),
        })
        .collect();
    ImportState { roots, records, bases, ..Default::default() }
}

fn compare_package(
    package: &Package,
    tree: &flowie_core::Tree,
    imports: &HashMap<NodeId, ImportRecord>,
    db_path: Option<&Path>,
) -> Result<ImportComparison, String> {
    let connection = db_path.map(flowie_core::store::open).transpose().map_err(|error| error.to_string())?;
    let load_bases = |roots: &[NodeId]| {
        connection.as_ref().map(|connection| flowie_core::store::load_bases(connection, roots)).transpose().map(Option::unwrap_or_default)
    };
    let source_roots = &package.outline.roots;
    let source_bases = load_bases(source_roots).map_err(|error| error.to_string())?;
    let can_update_original = !source_bases.is_empty() && source_roots.iter().all(|&root| tree.contains(root));
    let copies = if can_update_original { Vec::new() } else { copied_imports(package, tree, imports) };
    let mut already =
        !package.present_in(tree).is_empty() || imports.values().any(|record| record.package_id == package.manifest.package_id);
    if let Some(connection) = connection.as_ref().filter(|_| !already) {
        for &root in source_roots {
            if flowie_core::store::in_trash(connection, root).map_err(|error| error.to_string())? {
                already = true;
                break;
            }
        }
    }
    let (roots, bases) = if copies.is_empty() {
        (source_roots.clone(), source_bases)
    } else {
        let roots: Vec<_> = copies.iter().map(|record| record.root).collect();
        let bases = load_bases(&roots).map_err(|error| error.to_string())?;
        (roots, bases)
    };
    let update = if (can_update_original || !copies.is_empty()) && !bases.is_empty() {
        let id_map = copies.iter().flat_map(|record| record.source_ids.iter().copied()).collect();
        let hashes: HashMap<_, _> = bases.iter().map(|(node, _, hash)| (*node, hash.clone())).collect();
        let known = hashes.keys().copied().collect();
        Some((package.plan_update_mapped(tree, &hashes, &known, &id_map), HashSet::new()))
    } else {
        None
    };
    let records = roots.iter().filter_map(|id| imports.get(id).cloned()).collect();
    let before = ImportState { records, roots, bases, ..Default::default() };
    Ok(ImportComparison { revision: tree.revision(), before, already, update })
}

impl Outline {
    /// Open a package file: the import dialog, or a message saying why not.
    pub fn open_package_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        let read = std::fs::metadata(path).and_then(|m| {
            if m.len() > MAX_FILE {
                Err(std::io::Error::other("too large"))
            } else {
                std::fs::read(path)
            }
        });
        let bytes = match read {
            Ok(b) => b,
            Err(e) if e.to_string() == "too large" => {
                self.toast(t!("package.import.too_large"));
                return cx.notify();
            }
            Err(e) => {
                self.toast(t!("package.import.unreadable", path = path.display(), error = e));
                return cx.notify();
            }
        };
        match Package::read(&bytes) {
            Ok(package) => self.open_package_import(package, bytes.len(), cx),
            Err(e) => {
                eprintln!("flowie: {}: {e}", path.display());
                self.toast(match e {
                    PackageError::NotAPackage => t!("package.import.not_a_package").to_string(),
                    PackageError::Newer(_) => t!("package.import.newer").to_string(),
                    PackageError::TooLarge => t!("package.import.too_large").to_string(),
                    PackageError::Damaged(_) => t!("package.import.damaged").to_string(),
                });
                cx.notify();
            }
        }
    }

    fn compare_package(&self, package: &Package) -> Result<ImportComparison, String> {
        if let Some(writer) = &self.writer {
            writer.flush();
        }
        compare_package(package, &self.tree, &self.imports, self.db_path.as_deref())
    }

    pub fn open_package_import(&mut self, package: Package, bytes: usize, cx: &mut Context<Self>) {
        self.shut_popups();
        self.package_export = None;
        let comparison = match self.compare_package(&package) {
            Ok(comparison) => comparison,
            Err(error) => {
                self.toast(error);
                return cx.notify();
            }
        };
        let preview_items = package.preview_items(PREVIEW_LINES, 1);
        let has_completed = package.outline.nodes.iter().any(|n| n.completed);
        // Breadth-limited preview: the roots and their children.
        let mut preview = Vec::new();
        fn walk(items: &[flowie_core::Item], depth: usize, out: &mut Vec<(usize, String)>) {
            for it in items {
                if out.len() >= PREVIEW_LINES {
                    return;
                }
                out.push((depth, flowie_core::markup::plain_text(&it.text)));
                if depth < 1 {
                    walk(&it.children, depth + 1, out);
                }
            }
        }
        walk(&preview_items, 0, &mut preview);
        let more = package.outline.nodes.len().saturating_sub(preview.len());
        let return_to = self.edit.take().map(|e| (e.target, e.caret()));
        self.package_import = Some(PackageImport {
            comparison,
            scroll: gpui::ScrollHandle::new(),
            package,
            parent: ROOT,
            keep_completion: true,
            has_completed,
            bytes,
            preview,
            more,
            return_to,
        });
        cx.notify();
    }

    /// Close the palette, menus and popovers a dialog would sit on.
    fn shut_popups(&mut self) {
        self.palette = None;
        self.inline = None;
        self.calendar = None;
        self.menu = None;
        self.popover.close();
    }

    /// The "Put in" picker chose `target`.
    pub fn import_put_in(&mut self, target: NodeId, cx: &mut Context<Self>) {
        if let Some(p) = self.package_import.as_mut() {
            p.parent = if self.tree.contains(target) { self.tree.source(target) } else { ROOT };
        }
        self.edit = None;
        cx.notify();
    }

    pub fn close_package_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.package_import.take() else { return };
        window.focus(&self.focus, cx);
        self.edit = None;
        if let Some((target, offset)) =
            p.return_to.filter(|(t, _)| matches!(t, Target::Text(id) | Target::Note(id) if self.tree.contains(*id)))
        {
            self.start_edit(target, offset, cx);
        }
        cx.notify();
    }

    /// The dialog's main button: Update when it can, else Import (a Copy).
    fn finish_package_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let update = self.package_import.as_ref().and_then(|p| p.comparison.update.as_ref()).is_some_and(|(plan, _)| !plan.is_empty());
        if update {
            self.update_package_now(cx);
        } else {
            self.import_package_now(cx);
        }
    }

    fn take_confirmed_import(&mut self, cx: &mut Context<Self>) -> Option<PackageImport> {
        let dialog = self.package_import.as_ref()?;
        if dialog.comparison.revision != self.tree.revision() {
            match self.compare_package(&dialog.package) {
                Ok(comparison) => {
                    self.package_import.as_mut()?.comparison = comparison;
                    self.toast(t!("package.import.outline_changed"));
                }
                Err(error) => self.toast(error),
            }
            cx.notify();
            return None;
        }
        match write_package_images(&dialog.package, self.image_dir.as_ref()) {
            Ok(blobs) => {
                for blob in blobs {
                    self.record_image(blob);
                }
                self.package_import.take()
            }
            Err(error) => {
                self.toast(error);
                cx.notify();
                None
            }
        }
    }

    /// Import what the open dialog shows, as it is set (a copy if it was imported before).
    pub fn import_package_now(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.take_confirmed_import(cx) else { return };
        let parent = if self.tree.contains(p.parent) { p.parent } else { ROOT };
        let index = self.tree.children(parent).len();
        let opts = ImportOptions { fresh_ids: p.comparison.already, keep_completion: p.keep_completion };
        let (roots, id_map) = p.package.insert_into_mapped(&mut self.tree, parent, index, opts);
        self.tree.set_collapsed(parent, false);
        self.edit = None;
        self.block = None;
        let m = &p.package.manifest;
        let now = flowie_core::tree::now_millis();
        let before = if p.comparison.already { ImportState { roots: roots.clone(), ..Default::default() } } else { p.comparison.before };
        let after = package_import_state(&p.package, &id_map, now);
        self.commit_with_imports(None, Some((before, after)), cx);
        // Show them in place, not zoomed into: zooming in marks a root seen.
        if self.zoom != parent && !self.tree.is_ancestor(self.zoom, parent) {
            self.zoom_to(parent, cx);
        }
        self.refresh(false);
        for &root in &roots {
            self.anims.start(root, crate::anim::Channel::Flash, crate::anim::FLASH);
        }
        if let Some(ix) = roots.first().and_then(|r| self.display_index(*r)) {
            self.list.scroll_to_reveal_item(ix);
        }
        self.toast(t!("toast.package_imported", count = p.package.outline.nodes.len(), title = m.title));
        cx.notify();
    }

    pub(crate) fn set_import_state(&mut self, state: ImportState) {
        for root in &state.roots {
            self.imports.remove(root);
        }
        self.imports.extend(state.records.iter().map(|record| (record.root, record.clone())));
    }

    /// Bring an imported package up to date: one undo step.
    pub fn update_package_now(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.take_confirmed_import(cx) else { return };
        let Some((plan, take)) = p.comparison.update.as_ref() else { return };
        let mut touched = p.package.apply_update(&mut self.tree, plan, take);
        let mut trash = Vec::new();
        for &id in &plan.removed {
            if self.tree.contains(id) {
                trash.push((id, node_label(self, id), self.tree.subtree_nodes(id)));
                self.tree.delete(id);
            }
        }
        self.edit = None;
        self.block = None;
        let m = &p.package.manifest;
        let after = package_import_state(&p.package, plan.id_map(), flowie_core::tree::now_millis());
        self.commit_with_imports_and_trash(None, Some((p.comparison.before, after)), trash, cx);
        self.refresh(false);
        touched.retain(|id| self.tree.contains(*id));
        for &id in &touched {
            self.anims.start(id, crate::anim::Channel::Flash, crate::anim::FLASH);
        }
        let taken = plan.conflicts.iter().filter(|c| take.contains(c)).count();
        self.toast(t!(
            "package.update.done",
            title = m.title,
            changed = plan.changed.len() + taken,
            added = plan.added.len(),
            removed = plan.removed.len()
        ));
        cx.notify();
    }

    pub fn package_import_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        if ks.modifiers.modified() {
            return;
        }
        match ks.key.as_str() {
            "escape" => self.close_package_import(window, cx),
            "enter" => self.finish_package_import(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    pub fn render_package_import(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.package_import.as_ref()?;
        let theme = self.theme;
        let m = &p.package.manifest;
        let title = if m.title.trim().is_empty() { t!("common.untitled").to_string() } else { m.title.clone() };
        let from = t!(
            "package.import.from",
            name = if m.exported_by.name.is_empty() { "?" } else { &m.exported_by.name },
            date = crate::menu::date_time(m.exported_at_millis())
        );
        let summary = t!("package.export.summary", count = p.package.outline.nodes.len(), size = size_label(p.bytes));
        let parent_label = if self.tree.contains(p.parent) { node_label(self, p.parent) } else { t!("common.home").to_string() };
        let (keep, has_completed, already) = (p.keep_completion, p.has_completed, p.comparison.already);
        let update = p.comparison.update.as_ref();
        let lines = p.preview.iter().map(|(depth, text)| {
            div()
                .flex()
                .items_start()
                .gap_2()
                .pl(px(*depth as f32 * 16.))
                .child(div().flex_none().mt(px(9.)).size(px(4.)).rounded_full().bg(theme.faint))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(text.clone()))
        });
        let more = (p.more > 0).then(|| div().pl(px(12.)).text_color(theme.muted).child(t!("package.import.more", count = p.more)));
        let preview = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p_3()
            .rounded(px(crate::theme::R_MD))
            .bg(theme.input_bg)
            .border_1()
            .border_color(theme.border)
            .children(lines)
            .children(more);
        let description = (!m.description.trim().is_empty()).then(|| div().text_color(theme.text).child(m.description.clone()));
        let put_in = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(theme.muted).child(t!("package.import.put_in")))
            .child(div().font_weight(FontWeight::MEDIUM).child(parent_label))
            .child(
                button(theme, "package-put-in", Variant::Link)
                    .child(t!("package.import.change"))
                    .on_click(cx.listener(|this, _, _, cx| this.open_palette(crate::palette::Mode::ImportTo, cx))),
            );
        let completed = has_completed.then(|| {
            div()
                .id("package-keep-completion")
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .child(crate::ui::controls::checkbox(theme, keep))
                .child(t!("package.import.keep_completion"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(p) = this.package_import.as_mut() {
                        p.keep_completion = !keep;
                    }
                    cx.notify();
                }))
        });
        let can_update = update.is_some_and(|(plan, _)| !plan.is_empty());
        let note = already.then(|| {
            let text = match update {
                Some((plan, _)) if plan.is_empty() => t!("package.update.nothing").to_string(),
                Some((plan, _)) => t!(
                    "package.update.summary",
                    changed = plan.changed.len(),
                    added = plan.added.len(),
                    removed = plan.removed.len(),
                    conflicts = plan.conflicts.len()
                ),
                None => t!("package.import.already").to_string(),
            };
            div().text_size(px(12.)).text_color(theme.muted).child(text)
        });
        let conflicts = update.filter(|(plan, _)| !plan.conflicts.is_empty()).map(|(plan, take)| {
            let rows = plan.conflicts.iter().take(CONFLICTS_SHOWN).filter_map(|&id| {
                let mine = flowie_core::markup::plain_text(&self.tree.get(id)?.text);
                let theirs = flowie_core::markup::plain_text(&p.package.update_conflict(plan, id)?.text);
                let theirs_on = take.contains(&id);
                let choice = |label: &'static str, on: bool, pick_theirs: bool| {
                    crate::ui::segmented::segment(theme, SharedString::from(format!("conflict-{id}-{pick_theirs}")), on)
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some((_, take)) = this.package_import.as_mut().and_then(|p| p.comparison.update.as_mut()) {
                                if pick_theirs {
                                    take.insert(id);
                                } else {
                                    take.remove(&id);
                                }
                            }
                            cx.notify();
                        }))
                };
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .py_1()
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(div().w(px(56.)).flex_none().text_color(theme.muted).child(t!("package.update.mine")))
                                .child(div().min_w_0().child(mine)),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(div().w(px(56.)).flex_none().text_color(theme.muted).child(t!("package.update.theirs")))
                                .child(div().min_w_0().child(theirs)),
                        )
                        .child(
                            crate::ui::segmented::track(theme)
                                .child(choice(t!("package.update.keep_mine"), !theirs_on, false))
                                .child(choice(t!("package.update.take_theirs"), theirs_on, true)),
                        ),
                )
            });
            let more = plan.conflicts.len().saturating_sub(CONFLICTS_SHOWN);
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .rounded(px(crate::theme::R_MD))
                .border_1()
                .border_color(theme.border)
                .text_size(px(12.))
                .child(div().font_weight(FontWeight::MEDIUM).text_color(theme.text).child(t!("package.update.conflicts_heading")))
                .children(rows)
                .children((more > 0).then(|| div().text_color(theme.muted).child(t!("package.update.more_conflicts", count = more))))
        });
        let buttons = div()
            .flex()
            .justify_end()
            .gap_2()
            .child(
                button(theme, "package-import-cancel", Variant::Secondary)
                    .child(t!("package.export.cancel"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_package_import(window, cx))),
            )
            .when(can_update, |d| {
                d.child(button(theme, "package-import-copy", Variant::Secondary).child(t!("package.import.import_copy")).on_click(
                    cx.listener(|this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        this.import_package_now(cx);
                    }),
                ))
            })
            .child(
                button(theme, "package-import", Variant::Primary)
                    .child(if can_update {
                        t!("package.update.update")
                    } else if already {
                        t!("package.import.import_copy")
                    } else {
                        t!("package.import.import")
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.finish_package_import(window, cx))),
            );
        let scroll = p.scroll.clone();
        let content = div()
            .id("package-import-content")
            .track_scroll(&scroll)
            .overflow_y_scroll()
            .max_h((window.viewport_size().height - px(110.)).max(px(80.)))
            .pr_2()
            .flex()
            .flex_col()
            .gap_3()
            .child(crate::ui::text::overline(theme, t!("package.import.heading")))
            .child(crate::ui::text::page_title(theme, title, Some(format!("{from} · {summary}").into())))
            .children(description)
            .child(preview)
            .child(put_in)
            .children(completed)
            .children(note)
            .children(conflicts);
        let card = crate::ui::surface::sheet(theme, "package-import-card")
            .w(px(460.))
            .max_w(gpui::relative(0.9))
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(crate::ui::scrollbar::area("package-import-scroll", &scroll, theme, content))
            .child(buttons.flex_none());
        Some(
            crate::ui::surface::scrim(theme)
                .items_center()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_package_import(window, cx)))
                .child(card)
                .into_any_element(),
        )
    }

    // ------------------------------------------------------ unread dot

    /// The import record of `id` while it shows the unread dot.
    pub fn unread(&self, id: NodeId) -> Option<&ImportRecord> {
        self.imports.get(&id).filter(|r| r.seen_at.is_none() && self.tree.contains(id))
    }

    /// Imported roots not seen yet (`is:new`).
    pub fn unread_roots(&self) -> HashSet<NodeId> {
        self.imports.values().filter(|r| r.seen_at.is_none()).map(|r| r.root).collect()
    }

    pub fn mark_seen(&mut self, id: NodeId) {
        let Some(r) = self.imports.get_mut(&id).filter(|r| r.seen_at.is_none()) else { return };
        let now = flowie_core::tree::now_millis();
        r.seen_at = Some(now);
        if let Some(w) = &self.writer {
            w.seen(id, now);
        }
    }

    /// Zooming into or editing inside an imported root counts as seeing it.
    pub fn see_around(&mut self, id: NodeId) {
        if !self.imports.values().any(|r| r.seen_at.is_none()) {
            return;
        }
        let id = self.tree.resolve(id);
        let mut path = self.tree.ancestors(id);
        path.push(id);
        for n in path {
            self.mark_seen(n);
        }
    }

    /// "From Ann · imported 2026-11-24 10:00" for the dot's popover.
    pub fn unread_label(&self, id: NodeId) -> Option<String> {
        let r = self.imports.get(&id)?;
        let name = if r.author_name.is_empty() { "?" } else { r.author_name.as_str() };
        Some(t!("package.unread", name = name, date = crate::menu::date_time(r.imported_at)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_uses_a_tracked_copy_when_the_original_has_no_baseline() {
        let work = std::env::temp_dir().join(format!("flowie-package-baseline-{}", NodeId::random()));
        let db = work.join("flowie.sqlite");
        let mut connection = flowie_core::store::open(&db).unwrap();
        let mut sender = flowie_core::Tree::new();
        let root = sender.create_at(ROOT, 0, "root");
        let child = sender.create_at(root, 0, "before");
        let previous = Package::export(&sender, &[root], ExportOptions::default(), ExportMeta::default(), None);
        let mut receiver = flowie_core::Tree::from_nodes(sender.nodes().cloned());
        let (copies, id_map) =
            previous.insert_into_mapped(&mut receiver, ROOT, 1, ImportOptions { fresh_ids: true, keep_completion: true });
        let state = package_import_state(&previous, &id_map, 1);
        flowie_core::store::write_import_batch(&mut connection, &[], &[], &state).unwrap();
        let mut imports: HashMap<_, _> = state.records.into_iter().map(|record| (record.root, record)).collect();
        sender.set_text(child, "after");
        let updated = Package::export(&sender, &[root], ExportOptions::default(), ExportMeta::default(), None);

        let comparison = compare_package(&updated, &receiver, &imports, Some(&db)).unwrap();
        assert_eq!(comparison.before.roots, copies);
        let (plan, take) = comparison.update.unwrap();
        assert_eq!(plan.changed, [id_map[&child]]);
        updated.apply_update(&mut receiver, &plan, &take);
        assert_eq!(receiver.node(id_map[&child]).text, "after");
        assert_eq!(receiver.node(child).text, "before");

        let original = package_import_state(&previous, &HashMap::from([(root, root), (child, child)]), 2);
        flowie_core::store::write_import_batch(&mut connection, &[], &[], &original).unwrap();
        imports.extend(original.records.into_iter().map(|record| (record.root, record)));
        let comparison = compare_package(&updated, &receiver, &imports, Some(&db)).unwrap();
        assert_eq!(comparison.before.roots, [root]);
        assert_eq!(comparison.update.unwrap().0.changed, [child]);
        drop(connection);
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn copied_imports_use_one_complete_batch_and_keep_all_mappings() {
        let mut sender = flowie_core::Tree::new();
        let first = sender.create_at(ROOT, 0, "first");
        let second = sender.create_at(ROOT, 1, "second");
        let child = sender.create_at(first, 0, "child");
        let package = Package::export(&sender, &[first, second], ExportOptions::default(), ExportMeta::default(), None);
        let mut receiver = flowie_core::Tree::new();
        let opts = ImportOptions { fresh_ids: true, keep_completion: true };
        let mut records = HashMap::new();
        let insert = |receiver: &mut flowie_core::Tree, records: &mut HashMap<_, _>, imported_at| {
            let (roots, id_map) = package.insert_into_mapped(receiver, ROOT, 0, opts);
            let state = package_import_state(&package, &id_map, imported_at);
            assert_eq!(state.roots, roots);
            assert!(state.bases.iter().any(|(node, root, _)| *node == id_map[&child] && *root == roots[0]));
            records.extend(state.records.into_iter().map(|record| (record.root, record)));
            (roots, id_map)
        };
        let (older, _) = insert(&mut receiver, &mut records, 10);
        let (newer, newer_map) = insert(&mut receiver, &mut records, 20);
        let (incomplete, _) = insert(&mut receiver, &mut records, 30);
        receiver.delete(incomplete[1]);

        let batch = copied_imports(&package, &receiver, &records);
        assert_eq!(batch.iter().map(|record| record.root).collect::<Vec<_>>(), newer);
        assert!(batch[0].source_ids.contains(&(child, newer_map[&child])));
        receiver.delete(newer[0]);
        let batch = copied_imports(&package, &receiver, &records);
        assert_eq!(batch.iter().map(|record| record.root).collect::<Vec<_>>(), older);
        receiver.delete(older[0]);
        assert!(copied_imports(&package, &receiver, &records).is_empty());
    }

    #[test]
    fn package_image_write_failure_can_be_retried() {
        let work = std::env::temp_dir().join(format!("flowie-package-write-{}", NodeId::random()));
        let sender = ImageDir::new(work.join("sender"));
        let receiver = ImageDir::new(work.join("receiver"));
        let mut tree = flowie_core::Tree::new();
        let root = tree.create_at(ROOT, 0, "album");
        for bytes in [b"GIF89a\x05\x00\x07\x00", b"GIF89a\x06\x00\x07\x00"] {
            let image = sender.put(bytes).unwrap();
            let id = tree.create_at(root, tree.children(root).len(), "caption");
            let mut node = tree.node(id).clone();
            node.kind = flowie_core::BlockKind::Image;
            node.image = image.name;
            tree.put(node);
        }
        let package = Package::export(&tree, &[root], ExportOptions::default(), ExportMeta::default(), Some(&sender));
        let names: Vec<_> = package.images.keys().collect();
        let blocked = receiver.path(names[1]).unwrap();
        std::fs::create_dir_all(&blocked).unwrap();
        assert!(write_package_images(&package, Some(&receiver)).is_err());
        assert!(receiver.has(names[0]));
        assert!(!receiver.has(names[1]));

        std::fs::remove_dir(&blocked).unwrap();
        let stored = write_package_images(&package, Some(&receiver)).unwrap();
        assert_eq!(stored.iter().map(|blob| &blob.name).collect::<Vec<_>>(), names);
        assert!(names.iter().all(|name| receiver.has(name)));
        std::fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("Reading: notes/2026"), "Reading- notes-2026.flowie");
        assert_eq!(file_name("  ..  "), "Flowie.flowie");
        assert_eq!(file_name(&"x".repeat(100)), format!("{}.flowie", "x".repeat(60)));
    }

    #[test]
    fn file_urls_become_paths() {
        assert_eq!(path_from_url("file:///Users/a/My%20Notes.flowie"), Some("/Users/a/My Notes.flowie".into()));
        assert_eq!(path_from_url("file:///tmp/%E7%AC%94%E8%AE%B0.flowie"), Some("/tmp/笔记.flowie".into()));
        assert_eq!(path_from_url("https://x.org/a.flowie"), None);
        assert!(is_package(Path::new("/a/b.FLOWIE")));
        assert!(!is_package(Path::new("/a/b.md")));
    }
}
