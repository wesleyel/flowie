//! Flowie in the browser (M4): an outline in SQLite inside the page, the basic edits, and sync
//! with a self-hosted server through JavaScript's `fetch`. The engine is the same as on the
//! desktop and iOS; here it takes and gives the HTTP bodies the server speaks
//! (`ModifyRequest`, `ModifyResponse`, `ChangeBatch` as JSON), so `web/sync.js` only moves them.

use std::path::Path;

use flowie_core::images::ImageDir;
use flowie_core::{flatten_mapped, NodeId, ViewOptions, ROOT};
use flowie_doc::Workspace;
use flowie_sync::wire::{ModifyRequest, ModifyResponse};
use flowie_sync::{ChangeBatch, Engine, RecordStore};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// One row of a page (see `flowie-ffi`'s `RowView`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RowView {
    pub id: String,
    pub depth: u16,
    pub text: String,
    pub note: String,
    pub kind: String,
    pub completed: bool,
    pub collapsed: bool,
    pub has_children: bool,
}

fn error(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn parse(id: &str) -> Result<NodeId, JsError> {
    id.parse().map_err(|_| error(format!("no bullet {id}")))
}

/// An open outline.
#[wasm_bindgen]
pub struct Outline {
    ws: Workspace,
    engine: Engine,
}

#[wasm_bindgen]
impl Outline {
    /// Open (or create) the outline database `name` in SQLite's file system (in memory unless
    /// the page installed a persistent one), syncing under `remote` (the server's address).
    #[wasm_bindgen(constructor)]
    pub fn open(name: &str, remote: &str) -> Result<Outline, JsError> {
        let db = Path::new(name);
        let (mut ws, _) = Workspace::open(db, ImageDir::beside(db), |_| {}).map_err(error)?;
        ws.set_sync_remote(remote);
        Ok(Outline { ws, engine: Engine::new() })
    }

    /// The rows of the page at `root` (none: Home), as JSON (an array of `RowView`).
    pub fn page(&self, root: Option<String>) -> Result<String, JsError> {
        let tree = &self.ws.tree;
        let root = root.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        let (rows, _) = flatten_mapped(tree, root, &ViewOptions::default());
        let rows: Vec<RowView> = rows
            .into_iter()
            .filter_map(|row| {
                let node = tree.get(row.id)?;
                let source = tree.node(tree.source(row.id));
                Some(RowView {
                    id: row.id.to_string(),
                    depth: row.depth,
                    text: source.text.clone(),
                    note: source.note.clone(),
                    kind: format!("{:?}", node.kind).to_lowercase(),
                    completed: source.completed,
                    collapsed: node.collapsed,
                    has_children: tree.has_children(row.id),
                })
            })
            .collect();
        serde_json::to_string(&rows).map_err(error)
    }

    /// Add a bullet under `parent` (none: Home) at `index`; returns its id. An undo step.
    pub fn create(&mut self, parent: Option<String>, index: u32, text: &str) -> Result<String, JsError> {
        let parent = parent.as_deref().map(parse).transpose()?.unwrap_or(ROOT);
        let id = self.ws.tree.create_at(parent, index as usize, &text.replace('\n', " "));
        self.ws.commit_with_imports(None, None, None);
        Ok(id.to_string())
    }

    pub fn set_text(&mut self, id: &str, text: &str) -> Result<(), JsError> {
        let id = self.source(id)?;
        self.ws.tree.set_text(id, &text.replace('\n', " "));
        self.ws.commit_typing(id, None, None);
        Ok(())
    }

    pub fn set_completed(&mut self, id: &str, completed: bool) -> Result<(), JsError> {
        let id = self.source(id)?;
        self.ws.tree.set_completed(id, completed);
        self.ws.commit_with_imports(None, None, None);
        Ok(())
    }

    /// Move a bullet and what is under it to the Trash. An undo step.
    pub fn delete(&mut self, id: &str) -> Result<(), JsError> {
        let id = self.source(id)?;
        self.ws.tree.trash(id);
        self.ws.commit_with_imports(None, None, None);
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        self.ws.undo().is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.ws.redo().is_some()
    }

    /// The bullet being typed in: what comes in for it waits until editing ends (none).
    pub fn set_editing(&mut self, id: Option<String>) -> Result<(), JsError> {
        match id {
            Some(id) => {
                let id = self.source(&id)?;
                self.ws.begin_edit(id);
            }
            None => {
                self.ws.end_edit();
            }
        }
        Ok(())
    }

    /// The body of the next `POST /v1/zones/{zone}/modify`, or none when nothing waits.
    pub fn next_saves(&mut self, limit: u32) -> Option<String> {
        let saves = self.engine.next_saves(&self.ws, limit as usize);
        (!saves.is_empty()).then(|| serde_json::to_string(&ModifyRequest { saves }).expect("plain data"))
    }

    /// The server's answer to that request.
    pub fn on_saved(&mut self, response: &str) -> Result<(), JsError> {
        let response: ModifyResponse = serde_json::from_str(response).map_err(error)?;
        self.engine.on_saved(&mut self.ws, response.results);
        Ok(())
    }

    /// The saves that request carried failed (offline, an error answer): they go again later.
    pub fn on_save_failed(&mut self, request: &str, message: &str) -> Result<(), JsError> {
        let request: ModifyRequest = serde_json::from_str(request).map_err(error)?;
        let failed = request.saves.into_iter().map(|e| flowie_sync::SaveResult::Failed { id: e.id, error: message.to_string() }).collect();
        self.engine.on_saved(&mut self.ws, failed);
        Ok(())
    }

    /// Where the next `GET .../changes?since=` continues from.
    pub fn cursor(&self) -> Option<String> {
        self.ws.cursor()
    }

    /// A `ChangeBatch` from the server. Returns whether the outline changed.
    pub fn on_fetched(&mut self, batch: &str) -> Result<bool, JsError> {
        let batch: ChangeBatch = serde_json::from_str(batch).map_err(error)?;
        let revision = self.ws.tree.revision();
        self.engine.on_fetched(&mut self.ws, batch);
        Ok(self.ws.tree.revision() != revision)
    }

    /// Whether anything made here is waiting to be sent.
    pub fn has_unsent(&self) -> bool {
        self.ws.sync.has_unsent()
    }
}

impl Outline {
    fn source(&self, id: &str) -> Result<NodeId, JsError> {
        let id = parse(id)?;
        if !self.ws.tree.contains(id) {
            return Err(error(format!("no bullet {id}")));
        }
        Ok(self.ws.tree.source(id))
    }
}
