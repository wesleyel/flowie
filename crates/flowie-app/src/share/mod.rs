//! Share package dialogs and unread indicators. The format lives in `flowie_core::package`.

mod export;
mod import;
mod unread;

pub(crate) use export::PackageExport;
pub(crate) use import::{is_package, path_from_url, PackageImport};

use crate::app::i18n::t;

/// "12 KB", "1.4 MB".
fn size_label(bytes: usize) -> String {
    let kb = bytes.div_ceil(1024).max(1);
    if kb < 1024 {
        t!("package.size_kb", n = kb)
    } else {
        t!("package.size_mb", n = format!("{:.1}", bytes as f64 / (1024.0 * 1024.0)))
    }
}

impl crate::outline::Outline {
    /// Close the palette, menus and popovers a dialog would sit on.
    pub(super) fn shut_popups(&mut self) {
        self.palette = None;
        self.inline = None;
        self.calendar = None;
        self.menu = None;
        self.popover.close();
    }
}
