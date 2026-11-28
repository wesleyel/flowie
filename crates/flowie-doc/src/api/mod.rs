//! The outline as the apps outside the desktop use it: an open outline, the rows of a page, the
//! edits, search, stars, the journal and the Trash (wiki: Web-Client §3.1, iOS-Cross-Platform
//! §3.3). `flowie-ffi` (UniFFI, for iOS) and `flowie-wasm` (wasm-bindgen, for the browser) are
//! thin bindings over this, so both edit the outline the same way; each adds its own sync on
//! top of [`Outline::ws`]. Ids are 32 hex digits, as on the wire; text offsets are UTF-16 code
//! units (what UIKit and JavaScript count).

use std::path::Path;

use flowie_core::images::ImageDir;
use flowie_core::search::Query;
use flowie_core::{flatten_mapped, journal, markup, Caret, NodeId, Row, Tree, ViewOptions, ROOT};

use crate::{find, utf16, Outcome, Workspace};

mod rows;
#[cfg(test)]
mod tests;
mod text;
mod types;

pub use rows::pack_rows;
pub use text::{markup, toggle_format};
pub use types::*;

/// An open outline.
pub struct Outline {
    /// The document layer underneath, for a binding's sync (and tests).
    pub ws: Workspace,
    /// Completed rows are left off pages (this device's setting, as on the desktop).
    hide_completed: bool,
}

fn parse(id: &str) -> Result<NodeId, Error> {
    id.parse().map_err(|_| Error::NoSuchBullet { id: id.to_string() })
}

/// A page's own bullet as given (none: Home), parsed but not checked.
fn root_of(root: Option<&str>) -> Result<NodeId, Error> {
    Ok(root.map(parse).transpose()?.unwrap_or(ROOT))
}

/// A date from outside, checked: journal ids are derived from it.
fn day(date: &str) -> Result<&str, Error> {
    if journal::is_date(date) {
        Ok(date)
    } else {
        Err(Error::NotADate { date: date.to_string() })
    }
}

/// What `outcome` changed, as the app is told. `hiding`: completed rows are hidden and the edit
/// may have completed or reopened some, so rows changing count as rows coming and going.
fn edit_outcome(tree: &Tree, outcome: Outcome, hiding: bool) -> EditOutcome {
    let focus = outcome.focus.filter(|caret| tree.contains(caret.node)).map(|caret| {
        let offset = utf16::to_utf16(&tree.node(tree.source(caret.node)).text, caret.offset) as u32;
        FocusView { id: caret.node.to_string(), start: offset, end: offset }
    });
    EditOutcome {
        changed_ids: outcome.changed_ids.iter().map(ToString::to_string).collect(),
        structure_changed: outcome.structure_changed || (hiding && !outcome.changed_ids.is_empty()),
        focus,
    }
}

impl Outline {
    /// Open (or create) the outline database at `db`, its images beside it. On an error the
    /// database file is left as it was (`store::open`).
    pub fn open(db: &Path) -> Result<Outline, Error> {
        let (ws, meta) = Workspace::open(db, ImageDir::beside(db), |_| {})?;
        let hide_completed = meta.iter().any(|(k, v)| k == "hide_completed" && v == "1");
        Ok(Outline { ws, hide_completed })
    }

    /// An outline kept only in memory (previews and tests).
    pub fn in_memory() -> Outline {
        Outline { ws: Workspace::new(Tree::new()), hide_completed: false }
    }

    fn view(&self, query: Option<Query>) -> ViewOptions {
        ViewOptions { hide_completed: self.hide_completed, query }
    }

    fn page_rows(&mut self, root: NodeId, query: Option<Query>) -> Vec<Row> {
        let (rows, virtuals) = flatten_mapped(&self.ws.tree, root, &self.view(query));
        self.ws.tree.add_virtuals(virtuals);
        rows
    }

    /// The page's own bullet (none: Home), which must be in the outline.
    fn page_root(&self, root: Option<&str>) -> Result<NodeId, Error> {
        let root = root_of(root)?;
        if root != ROOT && !self.ws.tree.contains(root) {
            return Err(Error::NoSuchBullet { id: root.to_string() });
        }
        Ok(root)
    }

    /// The rows of the page at `root` (none: Home), expanded bullets' children included, and
    /// completed ones left out when [`hide_completed`](Self::hide_completed).
    /// Rows inside a mirror have virtual ids, which edits then take (as on the desktop: they
    /// change the mirrored bullets).
    pub fn page(&mut self, root: Option<&str>) -> Result<Vec<RowView>, Error> {
        self.page_matching(root, None)
    }

    /// [`page`](Self::page), with a search `query` (the desktop's syntax: words, "phrases",
    /// #tags, is:todo…): the rows that match and the bullets above them. A blank query is none.
    pub fn page_matching(&mut self, root: Option<&str>, query: Option<&str>) -> Result<Vec<RowView>, Error> {
        let root = self.page_root(root)?;
        let query = query.filter(|q| !q.trim().is_empty()).map(Query::parse);
        Ok(self.page_rows(root, query).into_iter().filter_map(|row| rows::row_view(&self.ws, row.id, row.depth)).collect())
    }

    /// [`page`](Self::page) as one byte buffer, which Swift reads into rows far faster than
    /// UniFFI reads the list field by field: 20 000 rows took 45–75 ms that way on iOS, which
    /// every structural edit paid (wiki: TODO-ios ios:24). Little-endian: the row count (u32),
    /// then per row its depth (u16), flags (u8: 1 completed, 2 collapsed, 4 has children, 8
    /// image, 16 image file), id, text, note and kind as strings (u32 byte length, UTF-8), and
    /// for an image its width and height (u32) and file path (a string, with flag 16).
    pub fn page_packed(&mut self, root: Option<&str>) -> Result<Vec<u8>, Error> {
        Ok(pack_rows(&self.page(root)?))
    }

    /// How many levels of bullets the page at `root` (none: Home) has when unfolded, capped at `3`
    /// (`0`: empty, `1`: flat rows, `2`: two levels, `3`: three or more), leaving out completed
    /// ones when [`hide_completed`](Self::hide_completed).
    pub fn page_levels(&self, root: Option<&str>) -> Result<u8, Error> {
        let root = self.page_root(root)?;
        Ok(rows::page_levels(&self.ws.tree, root, self.hide_completed))
    }

    /// The rows with these ids on the page at `root` (none: Home), as [`page`](Self::page)
    /// has them, in the order asked; ids not on the page are left out. For the rows an edit
    /// changed (`EditOutcome::changed_ids`), without reading the whole page.
    pub fn rows(&mut self, ids: &[String], root: Option<&str>) -> Result<Vec<RowView>, Error> {
        let root = self.page_root(root)?;
        // Rows inside a mirror (virtual ids) are found on the page.
        let mut page: Option<Vec<Row>> = None;
        let mut out = Vec::new();
        for id in ids {
            let Ok(id) = id.parse::<NodeId>() else { continue };
            let depth = match rows::depth_on_page(&self.ws.tree, root, id, self.hide_completed) {
                Some(depth) => Some(depth),
                None => {
                    if page.is_none() {
                        page = Some(self.page_rows(root, None));
                    }
                    page.as_ref().and_then(|rows| rows.iter().find(|row| row.id == id)).map(|row| row.depth)
                }
            };
            if let Some(row) = depth.and_then(|depth| rows::row_view(&self.ws, id, depth)) {
                out.push(row);
            }
        }
        Ok(out)
    }

    /// Add a bullet with `text` under `parent` (none: Home) at `index`; the focus is on it, at
    /// the end of its text. An undo step.
    pub fn create(&mut self, parent: Option<&str>, index: u32, text: &str) -> Result<EditOutcome, Error> {
        let parent = self.page_root(parent)?;
        let text = text.replace('\n', " ");
        let (id, mut outcome) = self.ws.tracked(|ws| {
            let id = ws.tree.create_at(parent, index as usize, &text);
            ws.commit_with_imports(None, None, None);
            id
        });
        outcome.focus = Some(Caret { node: id, offset: text.len() });
        Ok(edit_outcome(&self.ws.tree, outcome, false))
    }

    /// Add a bullet with `text` right after `sibling` (a row inside a mirror: after the bullet
    /// it shows); the focus is on it, at the end of its text. An undo step.
    pub fn create_after(&mut self, sibling: &str, text: &str) -> Result<EditOutcome, Error> {
        let sibling = self.ws.tree.resolve(parse(sibling)?);
        if !self.ws.tree.contains(sibling) {
            return Err(Error::NoSuchBullet { id: sibling.to_string() });
        }
        let text = text.replace('\n', " ");
        let (id, mut outcome) = self.ws.tracked(|ws| {
            let id = ws.tree.create_after(sibling, &text);
            ws.commit_with_imports(None, None, None);
            id
        });
        outcome.focus = Some(Caret { node: id, offset: text.len() });
        Ok(edit_outcome(&self.ws.tree, outcome, false))
    }

    /// Replace a bullet's text. Typing in one bullet merges into one undo step.
    pub fn set_text(&mut self, id: &str, text: &str) -> Result<EditOutcome, Error> {
        let text = text.replace('\n', " ");
        self.edit_text(id, |ws, id| {
            ws.tree.set_text(id, &text);
            ws.commit_typing(id, None, None);
            None
        })
    }

    /// Save the notes sheet as one undo step, including when opened just after typing in the body.
    pub fn set_note(&mut self, id: &str, note: &str) -> Result<EditOutcome, Error> {
        self.edit_text(id, |ws, id| {
            ws.tree.set_note(id, note);
            ws.commit_with_imports(None, None, None);
            None
        })
    }

    pub fn set_completed(&mut self, id: &str, completed: bool) -> Result<EditOutcome, Error> {
        self.edit(id, |ws, id| {
            ws.tree.set_completed(id, completed);
            ws.commit_with_imports(None, None, None);
            None
        })
    }

    /// Make a bullet the last child of the one above it (nothing changes if there is none). An
    /// undo step.
    pub fn indent(&mut self, id: &str) -> Result<EditOutcome, Error> {
        self.edit_row(id, |ws, id| {
            if ws.tree.indent(id) {
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Move a bullet out to follow its parent, but not out of the page at `root` (none: Home;
    /// nothing changes if it can't go further). An undo step.
    pub fn outdent(&mut self, id: &str, root: Option<&str>) -> Result<EditOutcome, Error> {
        let root = root_of(root)?;
        self.edit_row(id, |ws, id| {
            if ws.tree.outdent(id, root) {
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Move a bullet above the one before it; the first of its parent becomes the last child
    /// of the bullet before its parent, but not out of the page at `root` (none: Home). As the
    /// desktop's Move Up (`Tree::move_up`); nothing changes if it can't go further. An undo
    /// step.
    pub fn move_up(&mut self, id: &str, root: Option<&str>) -> Result<EditOutcome, Error> {
        let root = root_of(root)?;
        self.edit_row(id, |ws, id| {
            if ws.tree.move_up(id, root) {
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Move a bullet below the one after it; the last of its parent becomes the first child
    /// of the bullet after its parent, but not out of the page at `root` (none: Home). As the
    /// desktop's Move Down (`Tree::move_down`). An undo step.
    pub fn move_down(&mut self, id: &str, root: Option<&str>) -> Result<EditOutcome, Error> {
        let root = root_of(root)?;
        self.edit_row(id, |ws, id| {
            if ws.tree.move_down(id, root) {
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Return in a row's text with `start..end` selected (UTF-16 offsets; a caret when equal),
    /// on the page at `root` (none: Home), as on the desktop (`crate::keys::enter`): the text
    /// after the caret moves to a new bullet below, where the focus goes. An undo step.
    pub fn enter(&mut self, id: &str, start: u32, end: u32, root: Option<&str>) -> Result<EditOutcome, Error> {
        let root = root_of(root)?;
        self.edit_row(id, |ws, id| {
            let text = &ws.tree.node(ws.tree.source(id)).text;
            let sel = utf16::from_utf16(text, start as usize)..utf16::from_utf16(text, end.max(start) as usize);
            Some(ws.enter(id, sel, root))
        })
    }

    /// Backspace with the caret at the start of a row's text, on the page at `root` (none:
    /// Home), as on the desktop (`crate::keys::backspace_at_start`): the row joins the one
    /// above, or goes if it is empty, and the focus goes where they meet. Nothing changes on
    /// the first row. An undo step.
    pub fn backspace_at_start(&mut self, id: &str, root: Option<&str>) -> Result<EditOutcome, Error> {
        let root = root_of(root)?;
        let view = self.view(None);
        self.edit_row(id, |ws, id| {
            let (rows, _) = flatten_mapped(&ws.tree, root, &view);
            let prev = rows.iter().position(|r| r.id == id).and_then(|i| i.checked_sub(1)).map(|i| rows[i].id);
            ws.backspace_at_start(id, prev, root)
        })
    }

    /// Move a bullet and what is under it to the Trash. An undo step.
    pub fn delete(&mut self, id: &str) -> Result<EditOutcome, Error> {
        self.edit(id, |ws, id| {
            ws.tree.trash(id);
            ws.commit_with_imports(None, None, None);
            None
        })
    }

    /// Undo the last step (nothing changes if there is none); the focus goes where the caret
    /// was before it, when that is known.
    pub fn undo(&mut self) -> EditOutcome {
        self.tracked(false, |ws| ws.undo().flatten())
    }

    /// Redo the last undone step; the focus goes where the caret was after it.
    pub fn redo(&mut self) -> EditOutcome {
        self.tracked(false, |ws| ws.redo().flatten())
    }

    /// Whether there is a step to undo (the system's Undo shows enabled then).
    pub fn can_undo(&self) -> bool {
        self.ws.history.can_undo()
    }

    /// Whether there is an undone step to redo.
    pub fn can_redo(&self) -> bool {
        self.ws.history.can_redo()
    }

    /// Fold or unfold a bullet with children, as the desktop's fold toggle: its children leave
    /// or join the page. Nothing changes for a bullet without children. An undo step; folding
    /// doesn't sync (each device keeps its own).
    pub fn set_collapsed(&mut self, id: &str, collapsed: bool) -> Result<EditOutcome, Error> {
        self.edit_row(id, |ws, id| {
            if ws.tree.has_children(ws.tree.source(id)) && ws.tree.node(id).collapsed != collapsed {
                ws.tree.set_collapsed(id, collapsed);
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Show `levels` levels of the page at `root` (none: Home), as the desktop's Show N Levels
    /// and Expand All: bullets with children above that depth unfold, the ones at it fold.
    /// `1` folds every top-level row (Collapse All); `u32::MAX` unfolds everything. One undo
    /// step; folding doesn't sync (each device keeps its own).
    pub fn expand_to_level(&mut self, root: Option<&str>, levels: u32) -> Result<EditOutcome, Error> {
        let root = self.page_root(root)?;
        let levels = if levels == u32::MAX { usize::MAX } else { levels as usize };
        Ok(self.tracked(false, |ws| {
            let root = ws.tree.source(root);
            ws.tree.expand_to_level(root, levels);
            ws.commit_with_imports(None, None, None);
            None
        }))
    }

    /// Move a bullet, with what is under it, to the end of `parent` (none: Home), unfolding
    /// `parent` so it shows there: the desktop's Move To. Nothing changes when `parent` is the
    /// bullet itself or under it. An undo step.
    pub fn move_to(&mut self, id: &str, parent: Option<&str>) -> Result<EditOutcome, Error> {
        let parent = parent.map(|p| self.existing(p)).transpose()?.unwrap_or(ROOT);
        self.edit_row(id, |ws, id| {
            let n = ws.tree.children(parent).len();
            if ws.tree.move_to(id, parent, n) {
                ws.tree.set_collapsed(parent, false);
                ws.commit_with_imports(None, None, None);
            }
            None
        })
    }

    /// Where bullet `dragged` lands on the page at `root` (none: Home) when dropped in the gap
    /// below row `above` (at `above_depth`; none: the top of the page), over a row at
    /// `below_depth` (none: the end of the page), with the finger `depth` levels in: as on the
    /// desktop, no deeper than a child of the row above and no shallower than the row below.
    /// None when it would go under itself. `dragged` may come from another page; none for
    /// something new (text dropped from another app).
    pub fn drop_place(
        &self,
        root: Option<&str>,
        dragged: Option<&str>,
        above: Option<&str>,
        above_depth: u16,
        below_depth: Option<u16>,
        depth: u16,
    ) -> Result<Option<DropPlace>, Error> {
        let dragged = dragged.map(|d| self.existing(d)).transpose()?;
        let above = above.map(|a| self.existing(a)).transpose()?;
        let root = self.page_root(root)?;
        let place = crate::drop::place(&self.ws.tree, root, above.map(|a| (a, above_depth)), below_depth, depth, dragged);
        Ok(place.map(|p| DropPlace { parent: (p.parent != ROOT).then(|| p.parent.to_string()), index: p.index as u32, depth: p.depth }))
    }

    /// Move bullet `id`, with what is under it, to `place` (from [`Outline::drop_place`]),
    /// unfolding its new parent; a folded bullet stays folded. Nothing changes when it would go
    /// under itself. An undo step.
    pub fn drop_bullet(&mut self, id: &str, place: &DropPlace) -> Result<EditOutcome, Error> {
        let parent = place.parent.as_deref().map(|p| self.existing(p)).transpose()?.unwrap_or(ROOT);
        let to = crate::drop::Place { parent, index: place.index as usize, depth: place.depth };
        self.edit_row(id, |ws, id| {
            ws.drop_bullet(id, to);
            None
        })
    }

    /// Whether pages leave out completed rows: this device's setting, the desktop's Hide
    /// Completed.
    pub fn hide_completed(&self) -> bool {
        self.hide_completed
    }

    /// Show or hide completed rows on pages from now on (saved; read every page again).
    pub fn set_hide_completed(&mut self, hide: bool) {
        self.hide_completed = hide;
        self.ws.set_meta("hide_completed", if hide { "1" } else { "0" });
    }

    /// A bullet as lists name it, with the path to it.
    pub fn bullet(&self, id: &str) -> Result<BulletView, Error> {
        let node = self.existing(id)?;
        Ok(rows::bullet_view(&self.ws.tree, node))
    }

    /// What points at a bullet's page, as the desktop's footer lists it (`Tree::backlinks`, then
    /// `Tree::mirrors_of`), each in outline order: the bullets linking to it first, then its
    /// mirrors. A mirror's page points at its source's.
    pub fn backlinks(&self, id: &str) -> Result<Vec<BacklinkView>, Error> {
        let node = self.existing(id)?;
        let tree = &self.ws.tree;
        let src = tree.source(node);
        let view = |id: NodeId, mirror: bool| {
            let source = tree.node(tree.source(id));
            let parent = tree.parent(id);
            BacklinkView {
                id: id.to_string(),
                target: if !mirror {
                    Some(id.to_string())
                } else if parent == ROOT {
                    None
                } else {
                    Some(parent.to_string())
                },
                text: source.text.clone(),
                completed: source.completed,
                mirror,
                path: rows::crumbs(tree, id),
            }
        };
        let links = tree.backlinks(src).into_iter().map(|id| view(id, false));
        let mirrors = tree.mirrors_of(src).into_iter().map(|id| view(id, true));
        Ok(links.chain(mirrors).collect())
    }

    /// The pages for the system's search (Spotlight): bullets with children, starred bullets
    /// and the journal's days, not every bullet; each with a digest of what is indexed, so the
    /// app sends only what changed.
    pub fn index_pages(&self) -> Vec<IndexedPage> {
        rows::index_pages(&self.ws.tree)
    }

    /// The starred bullets, in the starred list's order.
    pub fn starred(&self) -> Vec<BulletView> {
        let tree = &self.ws.tree;
        tree.starred().map(|id| rows::bullet_view(tree, id)).collect()
    }

    /// The saved searches (the `saved_searches` pref, which syncs), in their order: the iPad
    /// sidebar lists them, as the desktop's palette does.
    pub fn saved_searches(&self) -> Vec<String> {
        self.ws.pref("saved_searches").unwrap_or_default().split('\n').filter(|q| !q.trim().is_empty()).map(String::from).collect()
    }

    /// Star a bullet (at the end of the starred list) or unstar it. Saved, and synced, but not
    /// an undo step, as on the desktop.
    pub fn set_starred(&mut self, id: &str, starred: bool) -> Result<(), Error> {
        let node = self.existing(id)?;
        self.ws.set_starred(node, starred);
        Ok(())
    }

    /// The bullets `query` matches, with the desktop's filter syntax (`flowie_core::search`:
    /// words, `"phrases"`, `#tags`, `-exclude`, `is:done`, `has:note`, `date:`, `changed:7d`,
    /// …), the most recently changed first; at most `limit`. Nothing for an empty query.
    pub fn search(&self, query: &str, limit: u32) -> Vec<BulletView> {
        let query = Query::parse(query);
        if query.is_empty() {
            return Vec::new();
        }
        let tree = &self.ws.tree;
        find::search(tree, &query).into_iter().take(limit as usize).map(|id| rows::bullet_view(tree, id)).collect()
    }

    /// Bullets to pick by name, as the desktop's palette finds them (`find::quick_find`): every
    /// word of `query` in the text, starred ones first; `exclude` and what is under them left
    /// out (for Move To, the bullets being moved). At most `limit`.
    pub fn quick_find(&self, query: &str, exclude: &[String], limit: u32) -> Vec<BulletView> {
        let exclude: Vec<NodeId> = exclude.iter().filter_map(|id| id.parse().ok()).collect();
        let tree = &self.ws.tree;
        find::quick_find(tree, query, limit as usize, &exclude).into_iter().map(|id| rows::bullet_view(tree, id)).collect()
    }

    /// A Markdown link to a bullet, as the desktop's Copy Link makes it: `[text](flowie:ID)`,
    /// `untitled` standing for an empty text.
    pub fn link(&self, id: &str, untitled: &str) -> Result<String, Error> {
        let node = self.existing(id)?;
        Ok(find::link(&self.ws.tree, node, untitled))
    }

    /// What is in the Trash, newest first, each entry a deleted bullet with what is under it.
    /// Entries past the retention period are deleted for good first, as at launch.
    pub fn trashed(&mut self) -> Vec<TrashView> {
        self.ws.purge_expired();
        let tree = &self.ws.tree;
        tree.trashed()
            .into_iter()
            .map(|root| {
                let node = tree.node(root);
                let shown = (node.mirror != ROOT).then(|| tree.record(node.mirror)).flatten().unwrap_or(node);
                TrashView {
                    id: root.to_string(),
                    text: markup::plain_text(&shown.text).chars().take(80).collect(),
                    count: tree.trashed_subtree(root).len() as u32,
                    deleted_at: node.deleted_at.unwrap_or_default(),
                }
            })
            .collect()
    }

    /// Bring a Trash entry back where it was (to the end of Home if that is gone too); the
    /// focus goes to the end of its text. An undo step.
    pub fn restore(&mut self, id: &str) -> Result<EditOutcome, Error> {
        let node = parse(id)?;
        let outcome = self.tracked(false, |ws| {
            if !ws.tree.untrash(node) {
                return None;
            }
            ws.commit_with_imports(None, None, None);
            Some(Caret { node, offset: ws.tree.node(node).text.len() })
        });
        if outcome.focus.is_none() {
            return Err(Error::NoSuchBullet { id: id.to_string() });
        }
        Ok(outcome)
    }

    /// The journal page for `date` (YYYY-MM-DD), if there is one. Its id is the same on every
    /// device (`flowie_core::journal`).
    pub fn journal_day(&self, date: &str) -> Result<Option<String>, Error> {
        let date = day(date)?;
        Ok(journal::find_day(&self.ws.tree, date).map(|id| id.to_string()))
    }

    /// The date (YYYY-MM-DD) bullet `id` is the journal page of, if it is one: a page under the
    /// journal titled with a date. The app opens it as that day, not as a plain bullet page.
    pub fn journal_date(&self, id: &str) -> Option<String> {
        let node = parse(id).ok()?;
        rows::journal_date(&self.ws.tree, node)
    }

    /// Open the journal page for `date` (YYYY-MM-DD) to write in, as the desktop does: a
    /// missing page is made among the journal's pages with a blank first line (ids the same on
    /// every device, so two devices make one page), and the focus goes to the end of its first
    /// row. An undo step when it made something.
    pub fn open_day(&mut self, date: &str) -> Result<EditOutcome, Error> {
        let date = day(date)?.to_string();
        let view = self.view(None);
        Ok(self.tracked(false, |ws| {
            let page = journal::ensure_day(&mut ws.tree, &date, true);
            if !ws.tree.journal_is_empty() {
                ws.commit_with_imports(None, None, None);
            }
            let (rows, _) = flatten_mapped(&ws.tree, page, &view);
            let first = rows.first()?.id;
            Some(Caret { node: first, offset: ws.tree.node(ws.tree.source(first)).text.len() })
        }))
    }

    /// "Add to Today's Journal": `text` at the end of the journal page for `date`
    /// (YYYY-MM-DD), one bullet a line, taken literally (`crate::capture::append`). The page is
    /// made as the desktop makes it, with the same id on every device; its lone blank line is
    /// replaced unless it is being typed in. One undo step; the focus is the end of the last
    /// bullet added, none (and nothing changed) when `text` is blank.
    pub fn journal_append(&mut self, date: &str, text: &str) -> Result<EditOutcome, Error> {
        let date = day(date)?;
        Ok(self.tracked(false, |ws| ws.journal_append(date, text)))
    }

    /// A share from a share extension (its inbox file `key`): `text`, one bullet a line, with
    /// `url` as a link on the first (`crate::capture::share`), at the end of the journal page
    /// for `date` (YYYY-MM-DD), made as [`journal_append`](Self::journal_append) makes it. A
    /// share already in the outline (in the Trash too) adds nothing: importing an inbox file
    /// again never adds it twice. One undo step; the focus is the end of the last bullet added,
    /// none when nothing changed.
    pub fn journal_share(&mut self, date: &str, key: &str, text: &str, url: &str) -> Result<EditOutcome, Error> {
        let date = day(date)?;
        Ok(self.tracked(false, |ws| ws.journal_share(date, key, text, url)))
    }

    /// Open the journal page for `date` (YYYY-MM-DD) to capture into: the focus goes to its
    /// last row when that is empty, otherwise to a new empty row at the end
    /// (`crate::capture::capture_row`). An undo step when it made something.
    pub fn journal_capture(&mut self, date: &str) -> Result<EditOutcome, Error> {
        let date = day(date)?;
        Ok(self.tracked(false, |ws| Some(ws.journal_capture(date))))
    }

    /// The bullet whose text is being typed in (a mirror row's source is found from it): what
    /// another device changes in it waits until editing ends. None ends editing.
    pub fn set_editing(&mut self, id: Option<&str>) -> Result<(), Error> {
        match id {
            Some(id) => {
                let id = parse(id)?;
                let source = self.ws.tree.source(id);
                self.ws.begin_edit(source);
            }
            None => {
                self.ws.end_edit();
            }
        }
        Ok(())
    }

    /// Wait until everything so far is saved.
    pub fn flush(&mut self) -> Result<(), Error> {
        self.ws.flush().map_err(|message| Error::Storage { message })
    }

    /// Whether the outline has nothing in it yet: no bullets at the top level (Trash aside).
    /// A new device shows that it is fetching the outline while it is.
    pub fn is_empty(&self) -> bool {
        self.ws.tree.children(ROOT).is_empty()
    }

    /// Run an edit that returns where the caret goes, and say what it changed. `text_only`: the
    /// edit changes only text, never whether a row is completed.
    fn tracked(&mut self, text_only: bool, f: impl FnOnce(&mut Workspace) -> Option<Caret>) -> EditOutcome {
        let hiding = self.hide_completed && !text_only;
        let (focus, outcome) = self.ws.tracked(f);
        edit_outcome(&self.ws.tree, Outcome { focus, ..outcome }, hiding)
    }

    /// Change a bullet's content: a mirror's row changes its source.
    fn edit(&mut self, id: &str, f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>) -> Result<EditOutcome, Error> {
        self.edit_row(id, |ws, id| {
            let id = ws.tree.source(id);
            f(ws, id)
        })
    }

    /// Change a bullet's text or note only (a mirror's row changes its source).
    fn edit_text(&mut self, id: &str, f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>) -> Result<EditOutcome, Error> {
        self.edit_with(id, true, |ws, id| {
            let id = ws.tree.source(id);
            f(ws, id)
        })
    }

    /// Change a bullet's place: the row itself moves, a mirror included.
    fn edit_row(&mut self, id: &str, f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>) -> Result<EditOutcome, Error> {
        self.edit_with(id, false, f)
    }

    fn edit_with(
        &mut self,
        id: &str,
        text_only: bool,
        f: impl FnOnce(&mut Workspace, NodeId) -> Option<Caret>,
    ) -> Result<EditOutcome, Error> {
        let id = parse(id)?;
        if !self.ws.tree.contains(id) {
            return Err(Error::NoSuchBullet { id: id.to_string() });
        }
        Ok(self.tracked(text_only, |ws| f(ws, id)))
    }

    /// A bullet the outline has (not one in the Trash), parsed.
    fn existing(&self, id: &str) -> Result<NodeId, Error> {
        let node = parse(id)?;
        if self.ws.tree.contains(node) {
            Ok(node)
        } else {
            Err(Error::NoSuchBullet { id: id.to_string() })
        }
    }
}
