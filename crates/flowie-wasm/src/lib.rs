//! Flowie in the browser (M4): an outline in SQLite inside the page, with the same facade the
//! iOS app uses (`flowie_doc::api`, wiki: Web-Client §3.1), and sync with a self-hosted server
//! through JavaScript's `fetch`. The engine is the same as on the desktop and iOS; here it takes
//! and gives the HTTP bodies the server speaks (`ModifyRequest`, `ModifyResponse`, `ChangeBatch`
//! as JSON), so `web/` only moves them.
//!
//! Rich results are JSON strings. Text offsets are UTF-16 code units, as JavaScript strings
//! count them. Edits that `flowie-ffi` answers with an `EditOutcome` answer with it here too, as
//! JSON; the first web client's calls (`create`, `indent`, `undo`, `set_text`…) keep their
//! simpler answers, and [`Outline::last_outcome`] gives the `EditOutcome` of any edit.

use std::path::Path;

use flowie_doc::api::{self, DropPlace, EditOutcome, RowView, TextFormat};
use flowie_sync::wire::{ModifyRequest, ModifyResponse};
use flowie_sync::{ChangeBatch, Engine, RecordStore};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Keep databases in the origin private file system from now on, so the outline is still there
/// after the page reloads. Call once, before opening an outline, from a dedicated worker (OPFS
/// gives synchronous file access only there). Without it, SQLite keeps databases in memory.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn install_opfs() -> Result<(), JsError> {
    use sqlite_wasm_vfs::sahpool::{install, OpfsSAHPoolCfg};
    install::<sqlite_wasm_rs::WasmOsCallback>(&OpfsSAHPoolCfg::default(), true).await.map(drop).map_err(error)
}

/// One row of a page, as [`Outline::page`] and [`Outline::rows`] give it: `flowie_doc::api`'s
/// `RowView`, with the image as the browser shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WebRow {
    pub id: String,
    pub depth: u16,
    pub text: String,
    pub note: String,
    pub kind: String,
    pub completed: bool,
    pub collapsed: bool,
    pub has_children: bool,
    pub starred: bool,
    /// The image file an image row shows (`<sha256>.<ext>`, fetched from the server's
    /// `/v1/blobs/`), or empty.
    pub image: String,
    /// The image's size in pixels; 0 when not known (or not an image).
    pub width: u32,
    pub height: u32,
}

/// A starred bullet, as [`Outline::starred`] lists it: `flowie_doc::api`'s `BulletView`, with
/// `text` as stored (markup included) and `plain` without markup.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StarredView {
    pub id: String,
    pub text: String,
    pub plain: String,
    pub completed: bool,
    pub starred: bool,
    pub has_children: bool,
    pub path: Vec<api::Crumb>,
}

fn error(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("plain data")
}

/// The markup runs of a bullet's text, as JSON (`[{start, end, kind: {type, …}, bold, italic,
/// strike, highlight, color}]`, UTF-16 offsets): `flowie_doc::api::markup`.
#[wasm_bindgen]
pub fn markup(text: &str) -> String {
    json(&api::markup(text))
}

/// Format the selection `start..end` (UTF-16) of `text` with `format` (`bold`, `italic`,
/// `strikethrough`, `code`, `highlight` or `link`), as the desktop does; JSON
/// `{text, start, end}`: `flowie_doc::api::toggle_format`.
#[wasm_bindgen]
pub fn toggle_format(text: &str, start: u32, end: u32, format: &str) -> Result<String, JsError> {
    let format: TextFormat = serde_json::from_value(serde_json::Value::String(format.into())).map_err(error)?;
    Ok(json(&api::toggle_format(text, start, end, format)))
}

/// An open outline.
#[wasm_bindgen]
pub struct Outline {
    outline: api::Outline,
    engine: Engine,
    /// What the last edit changed.
    last: EditOutcome,
}

#[wasm_bindgen]
impl Outline {
    /// Open (or create) the outline database `name` in SQLite's file system (in memory unless
    /// the page installed a persistent one), syncing under `remote` (the server's address).
    #[wasm_bindgen(constructor)]
    pub fn open(name: &str, remote: &str) -> Result<Outline, JsError> {
        let mut outline = api::Outline::open(Path::new(name)).map_err(error)?;
        outline.ws.set_sync_remote(remote);
        Ok(Outline { outline, engine: Engine::new(), last: EditOutcome::default() })
    }

    /// The rows of the page at `root` (none: Home), as JSON (an array of `WebRow`); with a
    /// search `query` (the desktop's syntax: words, "phrases", #tags, is:todo…), the rows that
    /// match and the bullets above them.
    pub fn page(&mut self, root: Option<String>, query: Option<String>) -> Result<String, JsError> {
        let rows = self.outline.page_matching(root.as_deref(), query.as_deref()).map_err(error)?;
        Ok(json(&self.web_rows(rows)))
    }

    /// The page as one byte buffer (`flowie_doc::api::Outline::page_packed`'s format).
    pub fn page_packed(&mut self, root: Option<String>) -> Result<Vec<u8>, JsError> {
        self.outline.page_packed(root.as_deref()).map_err(error)
    }

    /// How many levels the page at `root` has when unfolded, capped at 3.
    pub fn page_levels(&self, root: Option<String>) -> Result<u8, JsError> {
        self.outline.page_levels(root.as_deref()).map_err(error)
    }

    /// The rows with these ids on the page at `root`, as [`page`](Self::page) has them (JSON),
    /// in the order asked; ids not on the page are left out.
    pub fn rows(&mut self, ids: Vec<String>, root: Option<String>) -> Result<String, JsError> {
        let rows = self.outline.rows(&ids, root.as_deref()).map_err(error)?;
        Ok(json(&self.web_rows(rows)))
    }

    /// What the last edit changed, as JSON (`{changed_ids, structure_changed, focus: {id, start,
    /// end} | null}`): for the calls that don't return it.
    pub fn last_outcome(&self) -> String {
        json(&self.last)
    }

    /// Add a bullet under `parent` (none: Home) at `index`; returns its id. An undo step.
    pub fn create(&mut self, parent: Option<String>, index: u32, text: &str) -> Result<String, JsError> {
        let out = self.edit(|o| o.create(parent.as_deref(), index, text))?;
        Ok(out.focus.map(|f| f.id).unwrap_or_default())
    }

    /// Add a bullet right after `sibling`; returns its id. An undo step.
    pub fn create_after(&mut self, sibling: &str, text: &str) -> Result<String, JsError> {
        let out = self.edit(|o| o.create_after(sibling, text))?;
        Ok(out.focus.map(|f| f.id).unwrap_or_default())
    }

    /// Make a bullet the last child of the one above it. False if there is none.
    pub fn indent(&mut self, id: &str) -> Result<bool, JsError> {
        Ok(self.edit(|o| o.indent(id))?.structure_changed)
    }

    /// Move a bullet out to follow its parent, but not out of the page at `root` (none: Home).
    /// False if it can't go further.
    pub fn outdent(&mut self, id: &str, root: Option<String>) -> Result<bool, JsError> {
        Ok(self.edit(|o| o.outdent(id, root.as_deref()))?.structure_changed)
    }

    /// Fold or unfold a bullet's children (this device only).
    pub fn set_collapsed(&mut self, id: &str, collapsed: bool) -> Result<(), JsError> {
        self.edit(|o| o.set_collapsed(id, collapsed)).map(drop)
    }

    /// The starred pages, in order, as JSON (an array of `StarredView`).
    pub fn starred(&self) -> Result<String, JsError> {
        let tree = &self.outline.ws.tree;
        let starred: Vec<StarredView> = self
            .outline
            .starred()
            .into_iter()
            .map(|b| {
                let text = b.id.parse().map(|id| tree.node(tree.source(id)).text.clone()).unwrap_or_default();
                StarredView {
                    id: b.id,
                    text,
                    plain: b.text,
                    completed: b.completed,
                    starred: b.starred,
                    has_children: b.has_children,
                    path: b.path,
                }
            })
            .collect();
        Ok(json(&starred))
    }

    /// Star a bullet (at the end of the starred list) or unstar it. Not an undo step.
    pub fn set_starred(&mut self, id: &str, starred: bool) -> Result<(), JsError> {
        self.outline.set_starred(id, starred).map_err(error)
    }

    /// Today's journal page, made if it isn't there yet; returns its id.
    pub fn today(&mut self) -> String {
        let (id, outcome) = self.outline.ws.tracked(|ws| {
            let id = flowie_core::journal::ensure_day(&mut ws.tree, &flowie_core::search::today(), true);
            ws.commit_with_imports(None, None, None);
            id
        });
        self.last = EditOutcome {
            changed_ids: outcome.changed_ids.iter().map(ToString::to_string).collect(),
            structure_changed: outcome.structure_changed,
            focus: None,
        };
        id.to_string()
    }

    /// The pages from Home's child down to `id`, as JSON (`[{id, text}]`, text as stored), for
    /// breadcrumbs.
    pub fn path(&self, id: &str) -> Result<String, JsError> {
        let tree = &self.outline.ws.tree;
        let mut id = tree.resolve(id.parse().map_err(|_| error(format!("no bullet {id}")))?);
        if !tree.contains(id) {
            return Err(error(format!("no bullet {id}")));
        }
        let mut path = Vec::new();
        while id != flowie_core::ROOT {
            path.push(serde_json::json!({ "id": id.to_string(), "text": tree.node(tree.source(id)).text }));
            id = tree.parent(id);
        }
        path.reverse();
        Ok(json(&path))
    }

    /// Replace a bullet's text. Typing in one bullet merges into one undo step.
    pub fn set_text(&mut self, id: &str, text: &str) -> Result<(), JsError> {
        self.edit(|o| o.set_text(id, text)).map(drop)
    }

    pub fn set_completed(&mut self, id: &str, completed: bool) -> Result<(), JsError> {
        self.edit(|o| o.set_completed(id, completed)).map(drop)
    }

    /// Move a bullet and what is under it to the Trash. An undo step.
    pub fn delete(&mut self, id: &str) -> Result<(), JsError> {
        self.edit(|o| o.delete(id)).map(drop)
    }

    /// Undo the last step; false if there was none.
    pub fn undo(&mut self) -> bool {
        let had = self.outline.can_undo();
        self.last = self.outline.undo();
        had
    }

    /// Redo the last undone step; false if there was none.
    pub fn redo(&mut self) -> bool {
        let had = self.outline.can_redo();
        self.last = self.outline.redo();
        had
    }

    pub fn can_undo(&self) -> bool {
        self.outline.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.outline.can_redo()
    }

    /// Save the notes sheet as one undo step. JSON `EditOutcome`.
    pub fn set_note(&mut self, id: &str, note: &str) -> Result<String, JsError> {
        self.edit_json(|o| o.set_note(id, note))
    }

    /// The desktop's Move Up, not out of the page at `root`. JSON `EditOutcome`.
    pub fn move_up(&mut self, id: &str, root: Option<String>) -> Result<String, JsError> {
        self.edit_json(|o| o.move_up(id, root.as_deref()))
    }

    /// The desktop's Move Down, not out of the page at `root`. JSON `EditOutcome`.
    pub fn move_down(&mut self, id: &str, root: Option<String>) -> Result<String, JsError> {
        self.edit_json(|o| o.move_down(id, root.as_deref()))
    }

    /// Return with `start..end` (UTF-16) selected in a row's text on the page at `root`: the
    /// text after the caret moves to a new bullet below, where the focus goes. JSON
    /// `EditOutcome`.
    pub fn enter(&mut self, id: &str, start: u32, end: u32, root: Option<String>) -> Result<String, JsError> {
        self.edit_json(|o| o.enter(id, start, end, root.as_deref()))
    }

    /// Backspace at the start of a row's text on the page at `root`: it joins the row above,
    /// or goes if empty. JSON `EditOutcome`.
    pub fn backspace_at_start(&mut self, id: &str, root: Option<String>) -> Result<String, JsError> {
        self.edit_json(|o| o.backspace_at_start(id, root.as_deref()))
    }

    /// Show `levels` levels of the page at `root` (1: Collapse All; 4294967295: Expand All).
    /// JSON `EditOutcome`.
    pub fn expand_to_level(&mut self, root: Option<String>, levels: u32) -> Result<String, JsError> {
        self.edit_json(|o| o.expand_to_level(root.as_deref(), levels))
    }

    /// The desktop's Move To: to the end of `parent` (none: Home). JSON `EditOutcome`.
    pub fn move_to(&mut self, id: &str, parent: Option<String>) -> Result<String, JsError> {
        self.edit_json(|o| o.move_to(id, parent.as_deref()))
    }

    /// Where `dragged` lands when dropped below row `above` (at `above_depth`), over a row at
    /// `below_depth`, `depth` levels in: JSON `{parent, index, depth}`, or `null` when it would
    /// go under itself.
    pub fn drop_place(
        &self,
        root: Option<String>,
        dragged: Option<String>,
        above: Option<String>,
        above_depth: u16,
        below_depth: Option<u16>,
        depth: u16,
    ) -> Result<String, JsError> {
        let place = self
            .outline
            .drop_place(root.as_deref(), dragged.as_deref(), above.as_deref(), above_depth, below_depth, depth)
            .map_err(error)?;
        Ok(json(&place))
    }

    /// Move bullet `id` to `place` (JSON, from [`drop_place`](Self::drop_place)). JSON
    /// `EditOutcome`.
    pub fn drop_bullet(&mut self, id: &str, place: &str) -> Result<String, JsError> {
        let place: DropPlace = serde_json::from_str(place).map_err(error)?;
        self.edit_json(|o| o.drop_bullet(id, &place))
    }

    /// Whether pages leave out completed rows (this device's setting).
    pub fn hide_completed(&self) -> bool {
        self.outline.hide_completed()
    }

    /// Show or hide completed rows from now on (saved; read every page again).
    pub fn set_hide_completed(&mut self, hide: bool) {
        self.outline.set_hide_completed(hide);
    }

    /// A bullet as lists name it, JSON `{id, text, completed, starred, has_children, path:
    /// [{id, text}]}` (texts plain).
    pub fn bullet(&self, id: &str) -> Result<String, JsError> {
        Ok(json(&self.outline.bullet(id).map_err(error)?))
    }

    /// What points at a bullet's page: JSON `[{id, target, text, completed, mirror, path}]`,
    /// links first, then mirrors.
    pub fn backlinks(&self, id: &str) -> Result<String, JsError> {
        Ok(json(&self.outline.backlinks(id).map_err(error)?))
    }

    /// The pages to index for search: JSON `[{id, title, path, content, starred, day,
    /// completed, digest}]` (`digest` a decimal string).
    pub fn index_pages(&self) -> String {
        json(&self.outline.index_pages())
    }

    /// The saved searches, JSON `[string]`.
    pub fn saved_searches(&self) -> String {
        json(&self.outline.saved_searches())
    }

    /// The bullets `query` matches (the desktop's filter syntax), most recently changed first,
    /// at most `limit`: JSON, as [`bullet`](Self::bullet) gives each.
    pub fn search(&self, query: &str, limit: u32) -> String {
        json(&self.outline.search(query, limit))
    }

    /// Bullets to pick by name, starred first, `exclude` and what is under them left out, at
    /// most `limit`: JSON, as [`bullet`](Self::bullet) gives each.
    pub fn quick_find(&self, query: &str, exclude: Vec<String>, limit: u32) -> String {
        json(&self.outline.quick_find(query, &exclude, limit))
    }

    /// A Markdown link to a bullet (`[text](flowie:ID)`), `untitled` for an empty text.
    pub fn link(&self, id: &str, untitled: &str) -> Result<String, JsError> {
        self.outline.link(id, untitled).map_err(error)
    }

    /// What is in the Trash, newest first: JSON `[{id, text, count, deleted_at}]`.
    pub fn trashed(&mut self) -> String {
        json(&self.outline.trashed())
    }

    /// Bring a Trash entry back. JSON `EditOutcome`.
    pub fn restore(&mut self, id: &str) -> Result<String, JsError> {
        self.edit_json(|o| o.restore(id))
    }

    /// The journal page for `date` (YYYY-MM-DD), if there is one.
    pub fn journal_day(&self, date: &str) -> Result<Option<String>, JsError> {
        self.outline.journal_day(date).map_err(error)
    }

    /// The date (YYYY-MM-DD) bullet `id` is the journal page of, if it is one.
    pub fn journal_date(&self, id: &str) -> Option<String> {
        self.outline.journal_date(id)
    }

    /// Open the journal page for `date` to write in, made if missing; the focus goes to the end
    /// of its first row. JSON `EditOutcome`.
    pub fn open_day(&mut self, date: &str) -> Result<String, JsError> {
        self.edit_json(|o| o.open_day(date))
    }

    /// `text` at the end of the journal page for `date`, one bullet a line. JSON `EditOutcome`.
    pub fn journal_append(&mut self, date: &str, text: &str) -> Result<String, JsError> {
        self.edit_json(|o| o.journal_append(date, text))
    }

    /// A share (`key` names it, so it is never added twice): `text` with `url` as a link on the
    /// first line, at the end of the journal page for `date`. JSON `EditOutcome`.
    pub fn journal_share(&mut self, date: &str, key: &str, text: &str, url: &str) -> Result<String, JsError> {
        self.edit_json(|o| o.journal_share(date, key, text, url))
    }

    /// Open the journal page for `date` to capture into: its last row if empty, else a new one.
    /// JSON `EditOutcome`.
    pub fn journal_capture(&mut self, date: &str) -> Result<String, JsError> {
        self.edit_json(|o| o.journal_capture(date))
    }

    /// Whether the outline has no bullets at the top level yet.
    pub fn is_empty(&self) -> bool {
        self.outline.is_empty()
    }

    /// The bullet being typed in: what comes in for it waits until editing ends (none).
    pub fn set_editing(&mut self, id: Option<String>) -> Result<(), JsError> {
        self.outline.set_editing(id.as_deref()).map_err(error)
    }

    /// The body of the next `POST /v1/zones/{zone}/modify`, or none when nothing waits.
    pub fn next_saves(&mut self, limit: u32) -> Option<String> {
        let saves = self.engine.next_saves(&self.outline.ws, limit as usize);
        (!saves.is_empty()).then(|| json(&ModifyRequest { saves }))
    }

    /// The server's answer to that request.
    pub fn on_saved(&mut self, response: &str) -> Result<(), JsError> {
        let response: ModifyResponse = serde_json::from_str(response).map_err(error)?;
        self.engine.on_saved(&mut self.outline.ws, response.results);
        Ok(())
    }

    /// The saves that request carried failed (offline, an error answer): they go again later.
    pub fn on_save_failed(&mut self, request: &str, message: &str) -> Result<(), JsError> {
        let request: ModifyRequest = serde_json::from_str(request).map_err(error)?;
        let failed = request.saves.into_iter().map(|e| flowie_sync::SaveResult::Failed { id: e.id, error: message.to_string() }).collect();
        self.engine.on_saved(&mut self.outline.ws, failed);
        Ok(())
    }

    /// Where the next `GET .../changes?since=` continues from.
    pub fn cursor(&self) -> Option<String> {
        self.outline.ws.cursor()
    }

    /// A `ChangeBatch` from the server. Returns whether the outline changed.
    pub fn on_fetched(&mut self, batch: &str) -> Result<bool, JsError> {
        let batch: ChangeBatch = serde_json::from_str(batch).map_err(error)?;
        let revision = self.outline.ws.tree.revision();
        self.engine.on_fetched(&mut self.outline.ws, batch);
        Ok(self.outline.ws.tree.revision() != revision)
    }

    /// Whether anything made here is waiting to be sent.
    pub fn has_unsent(&self) -> bool {
        self.outline.ws.sync.has_unsent()
    }
}

impl Outline {
    /// Run an edit, keeping what it changed for [`last_outcome`](Self::last_outcome).
    fn edit(&mut self, f: impl FnOnce(&mut api::Outline) -> Result<EditOutcome, api::Error>) -> Result<EditOutcome, JsError> {
        let out = f(&mut self.outline).map_err(error)?;
        self.last = out.clone();
        Ok(out)
    }

    /// [`edit`](Self::edit), its outcome as JSON.
    fn edit_json(&mut self, f: impl FnOnce(&mut api::Outline) -> Result<EditOutcome, api::Error>) -> Result<String, JsError> {
        self.edit(f).map(|out| json(&out))
    }

    fn web_rows(&self, rows: Vec<RowView>) -> Vec<WebRow> {
        let tree = &self.outline.ws.tree;
        rows.into_iter()
            .filter_map(|row| {
                let id = row.id.parse().ok()?;
                let source = tree.node(tree.source(id));
                let image = row.image.as_ref();
                Some(WebRow {
                    starred: tree.is_starred(source.id),
                    image: if image.is_some() { source.image.clone() } else { String::new() },
                    width: image.map_or(0, |i| i.width),
                    height: image.map_or(0, |i| i.height),
                    id: row.id,
                    depth: row.depth,
                    text: row.text,
                    note: row.note,
                    kind: row.kind,
                    completed: row.completed,
                    collapsed: row.collapsed,
                    has_children: row.has_children,
                })
            })
            .collect()
    }
}
