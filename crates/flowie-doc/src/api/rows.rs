//! Reading a page: its rows (one at a time or packed), how deep it goes, where a bullet's row
//! is on it, and the pages the system's search indexes.

use flowie_core::{journal, markup, BlockKind, NodeId, Tree, ROOT};

use super::{BulletView, Crumb, ImageView, IndexedPage, RowView};
use crate::Workspace;

/// How much of a page's children's text goes into the index (characters).
const INDEXED_CONTENT: usize = 600;

/// Bullet `id`'s text as plain text (a mirror's source's).
pub(super) fn plain(tree: &Tree, id: NodeId) -> String {
    markup::plain_text(&tree.node(tree.source(id)).text)
}

pub(super) fn crumbs(tree: &Tree, id: NodeId) -> Vec<Crumb> {
    tree.ancestors(id).into_iter().map(|a| Crumb { id: a.to_string(), text: plain(tree, a) }).collect()
}

pub(super) fn bullet_view(tree: &Tree, id: NodeId) -> BulletView {
    let source = tree.node(tree.source(id));
    BulletView {
        id: id.to_string(),
        text: markup::plain_text(&source.text),
        completed: source.completed,
        starred: tree.is_starred(id),
        has_children: tree.has_children(source.id),
        path: crumbs(tree, id),
    }
}

pub(super) fn kind_name(kind: BlockKind) -> String {
    format!("{kind:?}").to_lowercase()
}

pub(super) fn row_view(ws: &Workspace, id: NodeId, depth: u16) -> Option<RowView> {
    let tree = &ws.tree;
    let node = tree.get(id)?;
    let source = tree.node(tree.source(id));
    // As the desktop's `row_image`: the row's own kind, its source's file.
    let image = (node.kind == BlockKind::Image && !source.image.is_empty()).then(|| {
        let blob = ws.blobs.get(&source.image);
        ImageView {
            path: ws.image_dir.as_ref().and_then(|dir| dir.path(&source.image)).map(|p| p.to_string_lossy().into_owned()),
            width: blob.map_or(0, |b| b.width),
            height: blob.map_or(0, |b| b.height),
        }
    });
    Some(RowView {
        id: id.to_string(),
        depth,
        text: source.text.clone(),
        note: source.note.clone(),
        kind: kind_name(node.kind),
        completed: source.completed,
        collapsed: node.collapsed,
        has_children: tree.has_children(source.id),
        image,
    })
}

/// Rows in [`Outline::page_packed`](super::Outline::page_packed)'s format.
pub fn pack_rows(rows: &[RowView]) -> Vec<u8> {
    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    let text: usize = rows.iter().map(|r| r.id.len() + r.text.len() + r.note.len() + r.kind.len()).sum();
    let mut out = Vec::with_capacity(4 + rows.len() * 19 + text);
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        let image = row.image.as_ref();
        let flags = u8::from(row.completed)
            | u8::from(row.collapsed) << 1
            | u8::from(row.has_children) << 2
            | u8::from(image.is_some()) << 3
            | u8::from(image.is_some_and(|i| i.path.is_some())) << 4;
        out.extend_from_slice(&row.depth.to_le_bytes());
        out.push(flags);
        for s in [&row.id, &row.text, &row.note, &row.kind] {
            string(&mut out, s);
        }
        if let Some(image) = image {
            out.extend_from_slice(&image.width.to_le_bytes());
            out.extend_from_slice(&image.height.to_le_bytes());
            if let Some(path) = &image.path {
                string(&mut out, path);
            }
        }
    }
    out
}

/// How many levels of bullets the page at `root` can show (`0`, `1`, `2`, or `3` for three or
/// more), regardless of what is currently collapsed, leaving out completed bullets when
/// `hide_completed`. Used by the fold button: "Expand All" is offered only past two levels.
pub(super) fn page_levels(tree: &Tree, root: NodeId, hide_completed: bool) -> u8 {
    let visible = |id: NodeId| !(hide_completed && tree.node(tree.source(id)).completed);
    let root_src = tree.source(root);
    let mut levels = 0u8;
    for &c0 in tree.children(root_src) {
        if !visible(c0) {
            continue;
        }
        levels = levels.max(1);
        let s0 = tree.source(c0);
        let m0 = (s0 != c0).then_some(s0);
        for &c1 in tree.children(s0) {
            if !visible(c1) {
                continue;
            }
            levels = levels.max(2);
            let s1 = tree.source(c1);
            if s1 != c1 && m0 == Some(s1) {
                continue;
            }
            if tree.children(s1).iter().any(|&c2| visible(c2)) {
                return 3;
            }
        }
    }
    levels
}

/// The depth of bullet `id`'s row on the page at `root`, found by walking up from it: none if
/// it isn't under `root`, a collapsed bullet hides it, or (with `hide_completed`) it or a bullet
/// above it is completed. Rows inside mirrors (virtual ids) are not found this way.
pub(super) fn depth_on_page(tree: &Tree, root: NodeId, id: NodeId, hide_completed: bool) -> Option<u16> {
    let done = |id: NodeId| hide_completed && tree.node(tree.source(id)).completed;
    if id.is_virtual() || id == root || !tree.contains(id) || done(id) {
        return None;
    }
    let mut depth = 0;
    let mut cur = tree.parent(id);
    while cur != root {
        if cur == ROOT || tree.node(cur).collapsed || done(cur) {
            return None;
        }
        depth += 1;
        cur = tree.parent(cur);
    }
    Some(depth)
}

/// A stable hash (FNV-1a), the same in every build and on every run, unlike std's.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn add(&mut self, bytes: &[u8]) {
        for b in bytes.iter().chain([&0xff]) {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// Every page to index (`IndexedPage`), in outline order. Mirrors are left out (their source
/// is indexed), and so are bullets without text, except the journal's days.
pub(super) fn index_pages(tree: &Tree) -> Vec<IndexedPage> {
    let journal = journal::find_journal(tree);
    let mut out = Vec::new();
    // Depth-first, with the path (plain texts) to the bullet being looked at.
    let mut path: Vec<String> = Vec::new();
    let mut stack: Vec<(NodeId, usize)> = tree.children(ROOT).iter().rev().map(|&c| (c, 0)).collect();
    while let Some((id, depth)) = stack.pop() {
        path.truncate(depth);
        let node = tree.node(id);
        if node.mirror != ROOT {
            continue;
        }
        let title = markup::plain_text(&node.text);
        let children = tree.children(id);
        let starred = tree.is_starred(id);
        let day = (journal == Some(tree.parent(id)) && journal::is_date(&node.text)).then(|| node.text.clone());
        if (!children.is_empty() || starred || day.is_some()) && (!title.trim().is_empty() || day.is_some()) {
            let mut content = String::new();
            for &child in children {
                if content.chars().count() >= INDEXED_CONTENT {
                    break;
                }
                let text = plain(tree, child);
                if !text.trim().is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&text);
                }
            }
            content = content.chars().take(INDEXED_CONTENT).collect();
            let mut hash = Fnv::new();
            for part in path.iter().chain([&title, &content]) {
                hash.add(part.as_bytes());
            }
            hash.add(&[u8::from(starred), u8::from(node.completed)]);
            out.push(IndexedPage {
                id: id.to_string(),
                title: title.clone(),
                path: path.clone(),
                content,
                starred,
                day,
                completed: node.completed,
                digest: hash.0,
            });
        }
        path.push(title);
        stack.extend(children.iter().rev().map(|&c| (c, depth + 1)));
    }
    out
}

/// The date of a journal page: a bullet under the journal titled with a date.
pub(super) fn journal_date(tree: &Tree, id: NodeId) -> Option<String> {
    let journal = journal::find_journal(tree)?;
    if !tree.contains(id) || tree.parent(id) != journal {
        return None;
    }
    let text = &tree.node(id).text;
    journal::is_date(text).then(|| text.clone())
}
