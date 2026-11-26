//! Track and display unread roots from imported packages.

use std::collections::HashSet;

use flowie_core::{store::ImportRecord, NodeId};

use crate::app::i18n::t;
use crate::outline::Outline;

impl Outline {
    /// The import record of `id` while it shows the unread dot.
    pub fn unread(&self, id: NodeId) -> Option<&ImportRecord> {
        self.ws.imports.get(&id).filter(|r| r.seen_at.is_none() && self.ws.tree.contains(id))
    }

    /// Imported roots not seen yet (`is:new`).
    pub fn unread_roots(&self) -> HashSet<NodeId> {
        self.ws.imports.values().filter(|r| r.seen_at.is_none()).map(|r| r.root).collect()
    }

    pub fn mark_seen(&mut self, id: NodeId) {
        let Some(r) = self.ws.imports.get_mut(&id).filter(|r| r.seen_at.is_none()) else { return };
        let now = flowie_core::tree::now_millis();
        r.seen_at = Some(now);
        if let Some(w) = &self.ws.writer {
            w.seen(id, now);
        }
    }

    /// Zooming into or editing inside an imported root counts as seeing it.
    pub fn see_around(&mut self, id: NodeId) {
        if !self.ws.imports.values().any(|r| r.seen_at.is_none()) {
            return;
        }
        let id = self.ws.tree.resolve(id);
        let mut path = self.ws.tree.ancestors(id);
        path.push(id);
        for n in path {
            self.mark_seen(n);
        }
    }

    /// "From Ann · imported 2026-11-24 10:00" for the dot's popover.
    pub fn unread_label(&self, id: NodeId) -> Option<String> {
        let r = self.ws.imports.get(&id)?;
        let name = if r.author_name.is_empty() { "?" } else { r.author_name.as_str() };
        Some(t!("package.unread", name = name, date = crate::overlays::context_menu::date_time(r.imported_at)))
    }
}
