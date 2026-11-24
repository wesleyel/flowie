//! Import and export outline documents through the file picker.

use std::path::{Path, PathBuf};

use flowie_core::{io, ROOT};
use gpui::{Context, PathPromptOptions, Window};

use super::{BlockSel, ExportMarkdown, ExportOpml, ExportText, Import, Outline};
use crate::app::i18n::t;
use crate::overlays::palette::node_label;

#[derive(Debug, PartialEq, Eq)]
enum ImportSelection<'a> {
    Documents,
    Package(&'a Path),
    MultipleWithPackage,
}

fn import_selection(paths: &[PathBuf]) -> ImportSelection<'_> {
    match paths.iter().find(|path| crate::share::is_package(path)) {
        Some(path) if paths.len() == 1 => ImportSelection::Package(path),
        Some(_) => ImportSelection::MultipleWithPackage,
        None => ImportSelection::Documents,
    }
}

impl Outline {
    pub fn import(&mut self, _: &Import, _: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("import.prompt").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            match import_selection(&paths) {
                ImportSelection::Documents => {}
                ImportSelection::Package(path) => {
                    this.update(cx, |this, cx| this.open_package_file(path, cx)).ok();
                    return;
                }
                ImportSelection::MultipleWithPackage => {
                    this.update(cx, |this, cx| {
                        this.toast(t!("toast.import_package_alone"));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            }
            let mut files = Vec::new();
            for p in paths {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    files.push((io::parse_any(&content), p.parent().map(|d| d.to_path_buf())));
                }
            }
            this.update(cx, |this, cx| {
                let documents = files
                    .into_iter()
                    .map(|(mut items, dir)| {
                        this.resolve_item_images(&mut items, dir.as_deref());
                        items
                    })
                    .collect();
                this.import_documents(documents, cx)
            })
            .ok();
        })
        .detach();
    }

    fn import_documents(&mut self, documents: Vec<Vec<io::Item>>, cx: &mut Context<Self>) {
        let count: usize = documents.iter().map(|items| io::Item::count(items)).sum();
        if count == 0 {
            self.toast(t!("toast.nothing_to_import"));
            cx.notify();
            return;
        }
        let (parent, index) = match self.editing_node().filter(|id| *id != self.zoom) {
            Some(id) => (self.tree.parent(id), self.tree.index_in_parent(id) + 1),
            None => (self.zoom, self.tree.children(self.zoom).len()),
        };
        let mut top = Vec::new();
        for items in documents {
            top.extend(self.tree.insert_items(parent, index + top.len(), &items));
        }
        self.edit = None;
        self.block = top.first().zip(top.last()).map(|(a, h)| BlockSel { anchor: *a, head: *h });
        self.toast(t!("toast.imported", count = count));
        self.commit_structure(None, cx);
    }

    /// What to export: the selected bullets, or everything on the current page.
    fn export_items(&self, opml: bool) -> (String, Vec<io::Item>) {
        let ids = if self.block.is_some() { self.targets() } else { self.tree.children(self.zoom).to_vec() };
        let title = if self.zoom == ROOT { "Flowie".to_string() } else { node_label(self, self.zoom) };
        (title, if opml { self.tree.to_opml_items(&ids) } else { self.tree.to_items(&ids) })
    }

    fn export_with(&mut self, ext: &str, render: fn(&str, &[io::Item]) -> String, cx: &mut Context<Self>) {
        let (title, items) = self.export_items(ext == "opml");
        let content = render(&title, &items);
        let safe: String = title.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c }).take(60).collect();
        let dir = dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some(&format!("{safe}.{ext}")));
        // Markdown refers to its images as `images/<name>` beside the file.
        let mut images = Vec::new();
        if ext == "md" {
            fn walk(items: &[io::Item], out: &mut Vec<String>) {
                for it in items {
                    if it.kind == flowie_core::BlockKind::Image && !it.image.is_empty() && !out.contains(&it.image) {
                        out.push(it.image.clone());
                    }
                    walk(&it.children, out);
                }
            }
            let mut names = Vec::new();
            walk(&items, &mut names);
            images = names.into_iter().filter_map(|n| Some((self.image_dir.as_ref()?.path(&n)?, n))).collect();
        }
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let copy_images = || -> std::io::Result<()> {
                if images.is_empty() {
                    return Ok(());
                }
                let dir = path.parent().unwrap_or(std::path::Path::new(".")).join("images");
                std::fs::create_dir_all(&dir)?;
                for (from, name) in &images {
                    std::fs::copy(from, dir.join(name))?;
                }
                Ok(())
            };
            let msg = match std::fs::write(&path, content).map(|()| copy_images()) {
                Ok(Ok(())) => t!("toast.exported", path = path.display()),
                Ok(Err(e)) => t!("image.export_images_failed", error = e),
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

    pub fn export_opml(&mut self, _: &ExportOpml, _: &mut Window, cx: &mut Context<Self>) {
        self.export_with("opml", io::to_opml, cx);
    }

    pub fn export_markdown(&mut self, _: &ExportMarkdown, _: &mut Window, cx: &mut Context<Self>) {
        self.export_with("md", |_, items| io::to_markdown(items), cx);
    }

    pub fn export_text(&mut self, _: &ExportText, _: &mut Window, cx: &mut Context<Self>) {
        self.export_with("txt", |_, items| io::to_plain_text(items), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_must_be_selected_alone_for_import() {
        let package = PathBuf::from("shared.FLOWIE");
        let document = PathBuf::from("notes.md");
        assert_eq!(import_selection(std::slice::from_ref(&package)), ImportSelection::Package(&package));
        assert_eq!(import_selection(&[document.clone(), package.clone()]), ImportSelection::MultipleWithPackage);
        assert_eq!(import_selection(&[package, PathBuf::from("other.flowie")]), ImportSelection::MultipleWithPackage);
        assert_eq!(import_selection(&[document, PathBuf::from("outline.opml")]), ImportSelection::Documents);
    }
}
