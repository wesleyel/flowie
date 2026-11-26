//! The outline entity and its shared view state.

mod blocks;
mod clipboard;
mod display;
mod documents;
mod editing;
mod history;
mod journal;
mod navigation;
mod render;
mod rows;
mod selection;
mod structure;

use std::ops::Range;

use flowie_core::{DisplayRow, NodeId, Row, ROOT};
use flowie_doc::Workspace;
use gpui::{actions, px, Bounds, Context, FocusHandle, ListAlignment, ListState, Pixels, TextLayout, Window};

use crate::ui::{
    anim::{Anims, Tween},
    theme::Theme,
};

use navigation::{PageLocation, PageScroll, WheelScroll};
use structure::DropTarget;

pub(crate) use render::TITLE_SIZE;
pub(crate) use rows::{bullet_dot, bullet_row, indent_guides, row_metrics, FONT_SIZE, LINE_H};

actions!(
    flowie,
    [
        Newline,
        ToggleNote,
        Indent,
        Outdent,
        Backspace,
        Delete,
        DeleteWordLeft,
        KillLine,
        Yank,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectUp,
        SelectDown,
        Home,
        End,
        SelectHome,
        SelectEnd,
        SelectAll,
        ToggleComplete,
        Collapse,
        Expand,
        ZoomIn,
        ZoomOut,
        ZoomHome,
        PageBack,
        PageForward,
        MoveUp,
        MoveDown,
        DeleteNode,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        Bold,
        Italic,
        InlineCode,
        WrapMath,
        InsertLink,
        Find,
        Escape,
        ToggleCompletedVisible,
        CycleContentWidth,
        ScrollToTop,
        ScrollToBottom,
        Quit,
        Jump,
        MoveTo,
        LinkToBullet,
        MirrorBullet,
        ToggleStar,
        ExpandAll,
        Level1,
        Level2,
        Level3,
        Level4,
        Import,
        ExportOpml,
        ExportMarkdown,
        ExportText,
        ExportPackage,
        InsertImage,
        WhatsNew,
        ShowTrash,
        SaveSearch,
        Highlight,
        ColorRed,
        ColorOrange,
        ColorYellow,
        ColorGreen,
        ColorBlue,
        ColorPurple,
        ColorGray,
        ColorNone,
        CycleColor,
        KindBullet,
        KindH1,
        KindH2,
        KindH3,
        KindParagraph,
        KindTodo,
        KindNumbered,
        KindQuote,
        KindCode,
        Today,
        NextMatch,
        PrevMatch,
        ShowCalendar,
        OpenSettings,
        OpenAbout,
        SendFeedback,
        OpenTutorial,
        OpenPrivacyPolicy,
        OpenTerms,
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Text(NodeId),
    Note(NodeId),
    Search,
    Palette,
    /// A one-line text field in a dialog or in Settings.
    Field(Field),
}

/// The text fields outside the outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// Settings → General → Author name.
    AuthorName,
    /// Settings → Shortcuts → the search box.
    ShortcutSearch,
    /// The export dialog's title and description.
    PackageTitle,
    PackageDescription,
    /// A temporary What's New row, discarded when the page closes.
    WhatsNew(NodeId),
}

pub struct Edit {
    pub target: Target,
    pub sel: Range<usize>,
    pub reversed: bool,
    pub marked: Option<Range<usize>>,
    /// Layout of the editor's text from the last paint (caret mapping, IME).
    pub layout: Option<TextLayout>,
    pub bounds: Option<Bounds<Pixels>>,
    pub selecting_with_mouse: bool,
}

impl Edit {
    pub fn new(target: Target, offset: usize) -> Edit {
        Edit { target, sel: offset..offset, reversed: false, marked: None, layout: None, bounds: None, selecting_with_mouse: false }
    }

    pub fn caret(&self) -> usize {
        if self.reversed {
            self.sel.start
        } else {
            self.sel.end
        }
    }
}

/// Block selection of whole rows (Esc from the editor, Shift-↑/↓).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSel {
    pub anchor: NodeId,
    pub head: NodeId,
}

pub struct Outline {
    pub focus: FocusHandle,
    /// The outline's data: tree, undo history, database writer, imports and images.
    pub ws: Workspace,
    pub zoom: NodeId,
    back_pages: Vec<PageLocation>,
    forward_pages: Vec<PageLocation>,
    /// Scroll position of each page opened this session, so reopening a node returns to it.
    page_scrolls: std::collections::HashMap<NodeId, PageScroll>,
    /// The window's size and place, saved on quit (see `window_place`).
    window_place: Option<crate::app::window_place::Place>,
    pub hide_completed: bool,
    pub query: String,
    pub rows: Vec<Row>,
    pub display: Vec<DisplayRow>,
    pub list: ListState,
    pub edit: Option<Edit>,
    pub block: Option<BlockSel>,
    pub anims: Anims,
    pub zoom_tween: Option<Tween>,
    /// How far the page was scrolled last frame, and the timer that snaps a
    /// half-collapsed title open or shut once scrolling stops.
    title_scroll: f32,
    /// `title_collapse()` as of this frame, for the page header to read.
    title_collapsed: f32,
    title_settle: Option<gpui::Task<()>>,
    /// The search box opening from its icon or closing back to it: whether
    /// it is (going) open, and how open it was when that last changed.
    search_open: bool,
    search_tween: Option<(Tween, f32)>,
    pub theme: Theme,
    pub drop: Option<DropTarget>,
    pub highlight_terms: Vec<String>,
    pub palette: Option<crate::overlays::palette::Palette>,
    pub inline: Option<crate::overlays::palette::Inline>,
    pub inline_entries_len: usize,
    pub block_saved: Option<BlockSel>,
    /// The `saved_searches` pref, one query per line.
    pub saved_searches: Vec<String>,
    pub(crate) trash: Option<crate::pages::trash::TrashPage>,
    pub(crate) volatile_page: Option<crate::pages::whats_new::VolatilePage>,
    pub db_path: Option<std::path::PathBuf>,
    pub calendar: Option<crate::overlays::calendar::Calendar>,
    /// This device, for share packages and (later) the change history: a UUID kept in the database's meta.
    pub device_id: String,
    /// A block selection is being extended by dragging the mouse.
    pub mouse_block: bool,
    /// Snapshots of deleted nodes, so their rows can animate out.
    pub ghosts: std::collections::HashMap<NodeId, flowie_core::Node>,
    /// Scroll position of the open popup list (palette or inline menu).
    pub popup_scroll: gpui::ScrollHandle,
    /// Short ids shown to agents, rebuilt when the tree changes.
    pub short_ids: std::cell::RefCell<flowie_core::ShortIds>,
    pub popover_scroll: gpui::ScrollHandle,
    pub menu_scroll: gpui::ScrollHandle,
    pub settings: Option<crate::settings::SettingsUi>,
    pub settings_return: Option<(Target, usize)>,
    pub config: crate::settings::Config,
    /// Mouse-wheel scrolling still to animate (`config.smooth_scroll`).
    wheel: WheelScroll,
    pub config_path: std::path::PathBuf,
    /// Selectable read-only labels, keyed by name.
    pub labels: std::collections::HashMap<&'static str, gpui::Entity<crate::text::selectable::SelectableText>>,
    pub toast: Option<(String, Tween)>,
    /// Row / display positions by id, rebuilt whenever `rows` / `display`
    /// change, so per-row lookups during render are O(1).
    row_pos: std::collections::HashMap<NodeId, usize>,
    display_pos: std::collections::HashMap<NodeId, usize>,
    /// The one floating popover (references, hidden path levels).
    pub popover: crate::overlays::popover::PopoverState,
    /// Text removed by ⌃K, put back by ⌃Y (macOS keeps this apart from the clipboard).
    pub kill_buffer: String,
    /// The open right-click menu (text or bullet).
    pub menu: Option<crate::overlays::context_menu::ContextMenu>,
    /// The open menu of the in-window menu bar (see `app_menu`).
    pub app_menu: Option<usize>,
    pub app_menu_scroll: gpui::ScrollHandle,
    /// The open export dialog.
    pub package_export: Option<crate::share::PackageExport>,
    /// The open import dialog.
    pub package_import: Option<crate::share::PackageImport>,
    /// The image shown full-window (Space on an image bullet).
    pub image_preview: Option<NodeId>,
    /// Frames left in which to keep revealing the edited row: a row Enter or
    /// undo just created has no height until it's measured, and grows in.
    pub follow_edit: u8,
    /// What's New to show once the window is (after a background launch).
    pub whats_new_pending: Option<Option<String>>,
    /// Set when the background database writer failed and is retrying.
    pub save_error: Option<String>,
    /// Set when the database couldn't be opened and the window shows an empty
    /// outline that isn't saved: the message to show.
    pub load_error: Option<String>,
    close_prompt_open: bool,
}

pub const INDENT: f32 = 24.0;
pub const PAD_X: f32 = 36.0;
/// List items before the first row: the page header (title, note and
/// no-match hint), which scrolls with the rows. Row `ix` is list item `ix + HEAD`.
pub const HEAD: usize = 1;

impl Outline {
    pub fn new(
        ws: Workspace,
        db_path: Option<std::path::PathBuf>,
        meta: Vec<(String, String)>,
        config: crate::settings::Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let db_path_for_config = db_path.clone();
        let mut zoom = ROOT;
        let mut hide_completed = false;
        let saved_searches =
            ws.pref("saved_searches").unwrap_or_default().split('\n').filter(|x| !x.is_empty()).map(String::from).collect();
        let mut page_scrolls = std::collections::HashMap::new();
        for (k, v) in &meta {
            match k.as_str() {
                "zoom" => zoom = v.parse().unwrap_or(ROOT),
                "hide_completed" => hide_completed = v == "1",
                "scroll" => page_scrolls.extend(PageScroll::decode(v)),
                _ => {}
            }
        }
        if !ws.tree.contains(zoom) {
            zoom = ROOT;
        }
        let mut this = Outline {
            focus: cx.focus_handle(),
            ws,
            zoom,
            back_pages: Vec::new(),
            forward_pages: Vec::new(),
            page_scrolls,
            window_place: None,
            hide_completed,
            query: String::new(),
            rows: Vec::new(),
            display: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            wheel: WheelScroll::default(),
            edit: None,
            block: None,
            anims: Anims::default(),
            zoom_tween: None,
            title_scroll: 0.,
            title_collapsed: 0.,
            title_settle: None,
            search_open: false,
            search_tween: None,
            theme: Theme::build(window.appearance(), &config),
            drop: None,
            highlight_terms: Vec::new(),
            palette: None,
            inline: None,
            inline_entries_len: 0,
            block_saved: None,
            saved_searches,
            trash: None,
            volatile_page: None,
            db_path,
            calendar: None,
            device_id: meta.iter().find(|(k, _)| k == "device").map(|(_, v)| v.clone()).unwrap_or_default(),
            toast: None,
            row_pos: Default::default(),
            display_pos: Default::default(),
            popover: Default::default(),
            menu: None,
            app_menu: None,
            app_menu_scroll: gpui::ScrollHandle::new(),
            package_export: None,
            package_import: None,
            image_preview: None,
            follow_edit: 0,
            whats_new_pending: None,
            save_error: None,
            load_error: None,
            close_prompt_open: false,
            kill_buffer: String::new(),
            mouse_block: false,
            ghosts: Default::default(),
            popup_scroll: gpui::ScrollHandle::new(),
            short_ids: Default::default(),
            popover_scroll: gpui::ScrollHandle::new(),
            menu_scroll: gpui::ScrollHandle::new(),
            settings: None,
            settings_return: None,
            config_path: crate::settings::Config::path(db_path_for_config.as_ref()),
            labels: Default::default(),
            config,
        };
        if this.device_id.is_empty() {
            this.device_id = crate::settings::Author::new_id();
            this.set_meta("device", &this.device_id.clone());
        }
        this.ensure_author();
        // A new database starts with the tutorial. Without one (it couldn't be opened, and the
        // window's banner says so) the outline stays empty: nothing in it would be saved.
        if this.ws.writer.is_some() && this.ws.tree.children(ROOT).is_empty() {
            this.seed_welcome();
        }
        this.refresh(false);
        let top = this.restore_scroll();
        if let Some(first) = top.or_else(|| this.rows.first().map(|r| r.id)) {
            this.edit = Some(Edit::new(Target::Text(first), this.ws.tree.node(first).text.len()));
        }
        let saved_place = meta.iter().find(|(k, _)| k == "window").and_then(|(_, v)| crate::app::window_place::Place::parse(v));
        this.window_place = crate::app::window_place::Place::observe(saved_place.as_ref(), window, cx);
        cx.observe_window_bounds(window, |this, window, cx| {
            this.window_place =
                crate::app::window_place::Place::observe(this.window_place.as_ref(), window, cx).or(this.window_place.take());
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.whats_new_window_shown(cx);
            }
        })
        .detach();
        cx.observe_window_appearance(window, |this, window, cx| {
            this.apply_theme(window);
            cx.notify();
        })
        .detach();
        this
    }

    fn seed_welcome(&mut self) {
        self.insert_tutorial(0);
        self.ws.tree.take_journal();
        self.ws.persist();
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Tween::new(std::time::Duration::from_millis(2600))));
    }

    pub fn set_meta(&self, key: &str, value: &str) {
        self.ws.set_meta(key, value);
    }

    /// Save `saved_searches` (a pref, so it syncs).
    pub fn save_searches(&mut self) {
        let value = self.saved_searches.join("\n");
        self.ws.set_pref("saved_searches", &value);
    }

    /// Settings or a dialog is open: the outline's own shortcuts are off
    /// (except under a palette opened from the dialog, which needs them).
    pub fn modal_open(&self) -> bool {
        self.settings.is_some()
            || self.image_preview.is_some()
            || ((self.package_export.is_some() || self.package_import.is_some()) && self.palette.is_none())
    }
}

#[cfg(test)]
mod tests {
    use flowie_core::store::{LOCAL_META, PREFS};

    /// Every key the app saves goes to the right table: `meta` for this device's
    /// state, `prefs` for what syncs. A new key must be added to one list on purpose.
    #[test]
    fn meta_and_pref_keys_are_classified() {
        fn keys(source: &str, call: &str) -> Vec<String> {
            source.match_indices(call).filter_map(|(at, _)| source[at + call.len()..].split('"').next().map(String::from)).collect()
        }
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push((path.display().to_string(), std::fs::read_to_string(&path).unwrap()));
                }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let (mut metas, mut prefs) = (0, 0);
        for (path, source) in &files {
            for key in keys(source, "set_meta(\"") {
                assert!(LOCAL_META.contains(&key.as_str()), "{path}: meta key {key:?} is not in store::LOCAL_META");
                metas += 1;
            }
            for key in keys(source, "set_pref(\"") {
                assert!(PREFS.contains(&key.as_str()), "{path}: pref key {key:?} is not in store::PREFS");
                prefs += 1;
            }
        }
        assert!(metas > 0 && prefs > 0, "found no set_meta/set_pref calls to check");
        assert!(LOCAL_META.contains(&crate::app::tutorial::TUTORIAL_META));
    }
}
