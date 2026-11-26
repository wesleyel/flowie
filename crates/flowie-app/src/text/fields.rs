//! Text editing in settings, dialogs, and temporary pages.

use gpui::{Context, Window};

use crate::outline::{Field, Outline, Target};

impl Outline {
    /// The text of a field outside the outline.
    pub fn field_text(&self, f: Field) -> &str {
        match f {
            Field::AuthorName => &self.config.author.name,
            Field::SyncServer => &self.config.sync.server,
            Field::SyncToken => &self.config.sync.token,
            Field::ShortcutSearch => self.settings.as_ref().map_or("", |s| s.shortcut_query.as_str()),
            Field::PackageTitle => self.package_export.as_ref().map_or("", |p| p.title.as_str()),
            Field::PackageDescription => self.package_export.as_ref().map_or("", |p| p.description.as_str()),
            Field::WhatsNew(id) => self.notes_page().and_then(|page| page.tree.get(id)).map_or("", |node| node.text.as_str()),
        }
    }

    pub fn set_field_text(&mut self, f: Field, text: String) {
        match f {
            Field::AuthorName => self.set_author_name(text),
            Field::SyncServer => {
                self.config.sync.server = text;
                self.save_config();
            }
            Field::SyncToken => {
                self.config.sync.token = text;
                self.save_config();
            }
            Field::ShortcutSearch => {
                if let Some(s) = self.settings.as_mut() {
                    s.shortcut_query = text;
                }
            }
            Field::PackageTitle => {
                if let Some(p) = self.package_export.as_mut() {
                    p.title = text;
                }
            }
            Field::PackageDescription => {
                if let Some(p) = self.package_export.as_mut() {
                    p.description = text;
                }
            }
            Field::WhatsNew(id) => {
                if let Some(page) = self.notes_page_mut().filter(|page| page.tree.contains(id)) {
                    page.tree.set_text(id, &text);
                    page.tree.take_journal();
                }
            }
        }
    }

    /// Stop editing a field in Settings or a dialog.
    pub fn end_field_edit(&mut self) -> bool {
        if self.edit.as_ref().is_some_and(|e| matches!(e.target, Target::Field(_))) {
            self.edit = None;
            return true;
        }
        false
    }

    /// Keys for the text field being edited in Settings or a dialog, where
    /// the outline's own bindings are off. True if the key was used.
    pub fn field_key(&mut self, ks: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        use crate::outline::*;
        if !self.edit.as_ref().is_some_and(|e| matches!(e.target, Target::Field(_))) {
            return false;
        }
        let m = &ks.modifiers;
        let mac = crate::app::platform::OS == crate::app::platform::Os::Mac;
        let word = if mac { m.alt } else { m.control };
        let primary = if mac { m.platform } else { m.control };
        match ks.key.as_str() {
            "escape" | "enter" | "tab" => {
                self.end_field_edit();
                self.apply_sync_config(cx);
            }
            "backspace" => self.backspace(&Backspace, window, cx),
            "delete" => self.delete(&Delete, window, cx),
            "left" if m.shift && word => self.select_word_left(&SelectWordLeft, window, cx),
            "right" if m.shift && word => self.select_word_right(&SelectWordRight, window, cx),
            "left" if m.shift => self.select_left(&SelectLeft, window, cx),
            "right" if m.shift => self.select_right(&SelectRight, window, cx),
            "left" if word => self.word_left(&WordLeft, window, cx),
            "right" if word => self.word_right(&WordRight, window, cx),
            "left" if primary => self.home(&Home, window, cx),
            "right" if primary => self.end(&End, window, cx),
            "left" => self.left(&Left, window, cx),
            "right" => self.right(&Right, window, cx),
            "home" | "up" => self.home(&Home, window, cx),
            "end" | "down" => self.end(&End, window, cx),
            "a" if primary => self.select_all(&SelectAll, window, cx),
            "c" if primary => self.copy(&Copy, window, cx),
            "x" if primary => self.cut(&Cut, window, cx),
            "v" if primary => self.paste(&Paste, window, cx),
            _ => return false,
        }
        true
    }
}
