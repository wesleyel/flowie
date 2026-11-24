//! Image bullets in the app: storing files (`flowie_core::images`), adding
//! them (paste, drop, Insert Image…, agents), showing them in a row and in
//! a full-window preview.

use std::path::{Path, PathBuf};

use flowie_core::images::{self, BlobInfo};
use flowie_core::{BlockKind, NodeId, ROOT};
use gpui::{div, img, prelude::*, px, AnyElement, Context, MouseButton, ObjectFit, PathPromptOptions, StyledImage, Window};

use crate::i18n::t;
use crate::outline::{BlockSel, Edit, InsertImage, Outline, Target};

/// Widest and tallest an image shows in a row, in points.
const MAX_W: f32 = 560.;
const MAX_H: f32 = 360.;

/// Files whose extension says image.
pub fn is_image_path(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "gif", "webp"].contains(&e.to_ascii_lowercase().as_str()))
}

impl Outline {
    /// Store image bytes; the name a bullet refers to them by.
    pub fn store_image(&mut self, bytes: &[u8]) -> Result<String, String> {
        let dir = self.image_dir.as_ref().ok_or_else(|| t!("image.no_folder").to_string())?;
        let info = dir.put(bytes).map_err(|e| match e {
            images::ImageError::Unsupported => t!("image.unsupported").to_string(),
            images::ImageError::TooLarge => t!("image.too_large").to_string(),
            images::ImageError::Io(e) => e,
        })?;
        if !self.blobs.contains_key(&info.name) {
            if let Some(w) = &self.writer {
                w.blob(info.clone());
            }
            self.blobs.insert(info.name.clone(), info.clone());
        }
        Ok(info.name)
    }

    /// Where a new image goes: into the empty bullet being edited, else
    /// after the edited or last selected bullet, else at the end of the page.
    fn image_slot(&self) -> (NodeId, usize, Option<NodeId>) {
        if let Some(id) = self.editing_node().filter(|id| *id != self.zoom && !id.is_virtual()) {
            let n = self.tree.node(id);
            if n.text.is_empty() && n.kind != BlockKind::Image && !self.tree.has_children(id) && n.mirror == ROOT {
                return (self.tree.parent(id), self.tree.index_in_parent(id), Some(id));
            }
            return (self.tree.parent(id), self.tree.index_in_parent(id) + 1, None);
        }
        if let Some(&last) = self.block_ids().last() {
            return (self.tree.parent(last), self.tree.index_in_parent(last) + 1, None);
        }
        (self.zoom, self.tree.children(self.zoom).len(), None)
    }

    /// Add images as bullets at `slot` (or `image_slot`), one undo step.
    /// Returns the new bullets; problems are toasted.
    pub fn insert_images(&mut self, files: Vec<Vec<u8>>, slot: Option<(NodeId, usize)>, cx: &mut Context<Self>) -> Vec<NodeId> {
        let before = self.edit.as_ref().and_then(|e| match e.target {
            Target::Text(id) | Target::Note(id) => Some(flowie_core::Caret { node: id, offset: e.caret() }),
            _ => None,
        });
        let (parent, mut index, mut reuse) = match slot {
            Some((p, i)) => (p, i, None),
            None => self.image_slot(),
        };
        let mut made = Vec::new();
        let mut error = None;
        for bytes in files {
            let name = match self.store_image(&bytes) {
                Ok(n) => n,
                Err(e) => {
                    error = Some(e);
                    continue;
                }
            };
            let id = match reuse.take() {
                Some(id) => id,
                None => {
                    let id = self.tree.create_at(parent, index, "");
                    index += 1;
                    id
                }
            };
            let mut node = self.tree.node(id).clone();
            node.kind = BlockKind::Image;
            node.image = name;
            self.tree.put(node);
            made.push(id);
        }
        if let Some(e) = error {
            self.toast(e);
        }
        if let Some(&last) = made.last() {
            self.tree.set_collapsed(parent, false);
            // The caret in the caption, ready to type one.
            self.block = None;
            self.edit = Some(Edit::new(Target::Text(last), 0));
            self.commit_structure(before, cx);
        }
        cx.notify();
        made
    }

    /// Image files by path (drop, Insert Image…).
    pub fn insert_image_files(&mut self, paths: &[PathBuf], slot: Option<(NodeId, usize)>, cx: &mut Context<Self>) {
        let mut files = Vec::new();
        for p in paths {
            match std::fs::metadata(p) {
                Ok(m) if m.len() as usize > images::MAX_BYTES => self.toast(t!("image.too_large")),
                Ok(_) => match std::fs::read(p) {
                    Ok(b) => files.push(b),
                    Err(e) => self.toast(t!("package.import.unreadable", path = p.display(), error = e)),
                },
                Err(e) => self.toast(t!("package.import.unreadable", path = p.display(), error = e)),
            }
        }
        if !files.is_empty() {
            self.insert_images(files, slot, cx);
        }
    }

    /// File → Insert Image…
    pub fn insert_image(&mut self, _: &InsertImage, _: &mut Window, cx: &mut Context<Self>) {
        self.pick_images(None, cx);
    }

    /// Ask for image files, then add them at `slot` (or where the caret is).
    pub fn pick_images(&mut self, slot: Option<(NodeId, usize)>, cx: &mut Context<Self>) {
        let slot = slot.or_else(|| {
            let (p, i, reuse) = self.image_slot();
            reuse.is_none().then_some((p, i))
        });
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("image.prompt").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            this.update(cx, |this, cx| this.insert_image_files(&paths, slot, cx)).ok();
        })
        .detach();
    }

    /// Images on the clipboard (copied pixels, or image files copied in the
    /// file manager); true if there were any.
    pub fn paste_images(&mut self, item: &gpui::ClipboardItem, cx: &mut Context<Self>) -> bool {
        let mut files = Vec::new();
        let mut paths = Vec::new();
        for e in item.entries() {
            match e {
                gpui::ClipboardEntry::Image(i) => files.push(i.bytes.clone()),
                gpui::ClipboardEntry::ExternalPaths(p) => paths.extend(p.paths().iter().filter(|p| is_image_path(p)).cloned()),
                gpui::ClipboardEntry::String(_) => {}
            }
        }
        if files.is_empty() && paths.is_empty() {
            return false;
        }
        if !files.is_empty() {
            self.insert_images(files, None, cx);
        } else {
            self.insert_image_files(&paths, None, cx);
        }
        true
    }

    /// The image a row shows, with its file's metadata.
    pub fn row_image(&self, id: NodeId) -> Option<(PathBuf, Option<&BlobInfo>)> {
        let n = self.tree.node(self.tree.source(id));
        if n.image.is_empty() || self.tree.node(id).kind != BlockKind::Image {
            return None;
        }
        Some((self.image_dir.as_ref()?.path(&n.image)?, self.blobs.get(&n.image)))
    }

    /// The size an image shows at in a row (points): its pixel size at 2×,
    /// scaled down to fit.
    pub fn image_box(info: Option<&BlobInfo>) -> (f32, f32) {
        let (w, h) = info.map_or((MAX_W, MAX_H * 0.6), |b| (b.width as f32 / 2., b.height as f32 / 2.));
        let scale = (MAX_W / w).min(MAX_H / h).min(1.0);
        ((w * scale).max(16.), (h * scale).max(16.))
    }

    /// The picture part of an image row.
    pub fn render_row_image(&mut self, id: NodeId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (path, info) = self.row_image(id)?;
        let (w, h) = Self::image_box(info);
        let theme = self.theme;
        let missing = t!("image.missing");
        Some(
            div()
                .id(("image", id.as_usize()))
                .mt(px(2.))
                .mb(px(4.))
                .w(px(w))
                .h(px(h))
                .max_w_full()
                .rounded(px(6.))
                .overflow_hidden()
                .cursor_pointer()
                .child(
                    img(path)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(move || {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(theme.code_bg)
                                .text_color(theme.muted)
                                .text_size(px(12.))
                                .child(missing)
                                .into_any_element()
                        })
                        .with_loading(move || div().size_full().bg(theme.code_bg).into_any_element()),
                )
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                    if ev.click_count() >= 2 {
                        this.open_image_preview(id, cx);
                    } else {
                        // Select the bullet; Space or a double-click opens the preview.
                        this.edit = None;
                        this.block = Some(BlockSel { anchor: id, head: id });
                        cx.notify();
                    }
                }))
                .into_any_element(),
        )
    }

    pub fn open_image_preview(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let src = self.tree.source(id);
        if self.row_image(id).is_some() {
            self.image_preview = Some(src);
            cx.notify();
        }
    }

    /// Space with one image bullet selected opens its preview. True if it did.
    pub fn preview_selected_image(&mut self, cx: &mut Context<Self>) -> bool {
        let ids = self.block_ids();
        match ids.as_slice() {
            [id] if self.row_image(*id).is_some() => {
                self.open_image_preview(*id, cx);
                true
            }
            _ => false,
        }
    }

    pub fn image_preview_key(&mut self, ev: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        if matches!(ev.keystroke.key.as_str(), "escape" | "space" | "enter") {
            self.image_preview = None;
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub fn render_image_preview(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.image_preview?;
        let Some((path, _)) = self.tree.contains(id).then(|| self.row_image(id)).flatten() else {
            self.image_preview = None;
            return None;
        };
        let caption = flowie_core::markup::plain_text(&self.tree.node(id).text);
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .p(px(32.))
                .bg(gpui::black().opacity(0.82))
                .occlude()
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.image_preview = None;
                        cx.notify();
                    }),
                )
                .child(div().flex_1().min_h_0().w_full().child(img(path).size_full().object_fit(ObjectFit::ScaleDown)))
                .children((!caption.is_empty()).then(|| div().text_color(gpui::white()).text_size(px(14.)).child(caption)))
                .child(div().text_size(px(11.)).text_color(gpui::white().opacity(0.5)).child(t!("image.preview_hint")))
                .into_any_element(),
        )
    }

    /// ⌘C with one image bullet selected copies the picture (other apps paste it).
    pub fn copy_selected_image(&self, cx: &mut Context<Self>) -> bool {
        let ids = self.block_ids();
        let [id] = ids.as_slice() else { return false };
        let name = self.tree.node(self.tree.source(*id)).image.clone();
        let (Some(dir), true) = (self.image_dir.as_ref(), self.tree.node(*id).kind == BlockKind::Image) else { return false };
        let Ok(bytes) = dir.read(&name) else { return false };
        let format = match images::mime_of(&name) {
            "image/jpeg" => gpui::ImageFormat::Jpeg,
            "image/gif" => gpui::ImageFormat::Gif,
            "image/webp" => gpui::ImageFormat::Webp,
            _ => gpui::ImageFormat::Png,
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&gpui::Image::from_bytes(format, bytes)));
        true
    }

    /// Image items from Markdown name a path (`![caption](path)`): store
    /// those files (paths relative to `base`, the Markdown file's folder) and
    /// refer to them by name. One that can't be read stays as its text.
    pub fn resolve_item_images(&mut self, items: &mut [flowie_core::Item], base: Option<&Path>) {
        for it in items.iter_mut() {
            if it.kind == BlockKind::Image && it.image.is_empty() {
                it.kind = BlockKind::Bullet;
            } else if it.kind == BlockKind::Image && !self.image_dir.as_ref().is_some_and(|d| d.has(&it.image)) {
                let stored = base.and_then(|b| {
                    let path = b.join(it.image.trim_start_matches("./"));
                    let bytes = std::fs::read(&path).ok()?;
                    self.store_image(&bytes).ok()
                });
                match stored {
                    Some(name) => it.image = name,
                    None => {
                        it.text = format!("![{}]({})", it.text, it.image);
                        it.kind = BlockKind::Bullet;
                        it.image.clear();
                    }
                }
            }
            self.resolve_item_images(&mut it.children, base);
        }
    }

    /// Delete image files no bullet (or trashed bullet) uses any more. Runs
    /// once at launch, off the UI thread, before this session adds any.
    pub fn collect_images(db: &Path) {
        let db = db.to_path_buf();
        std::thread::Builder::new()
            .name("flowie-images".into())
            .spawn(move || {
                let Ok(conn) = flowie_core::store::open(&db) else { return };
                let Ok(used) = flowie_core::store::image_names_in_use(&conn) else { return };
                let removed = images::ImageDir::beside(&db).collect(&used, std::time::Duration::from_secs(3600));
                if !removed.is_empty() {
                    let _ = flowie_core::store::drop_blobs(&conn, &removed);
                    eprintln!("flowie: removed {} unused image files", removed.len());
                }
            })
            .ok();
    }
}

/// Standard base64 (with or without padding and line breaks), for agents
/// sending image data.
pub fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes() {
        assert_eq!(decode_base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("aGVs\nbG8"), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("a*b"), None);
    }

    #[test]
    fn image_boxes_fit_the_column() {
        let info = |w, h| BlobInfo { name: String::new(), mime: String::new(), width: w, height: h, size: 0 };
        assert_eq!(Outline::image_box(Some(&info(400, 200))), (200., 100.));
        assert_eq!(Outline::image_box(Some(&info(4000, 1000))), (560., 140.));
        assert_eq!(Outline::image_box(Some(&info(1000, 4000))), (90., 360.));
        assert!(is_image_path(Path::new("a/B.JPG")) && !is_image_path(Path::new("a.svg")));
    }
}
