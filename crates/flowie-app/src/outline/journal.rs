//! Create and open dated pages in the journal.

use flowie_core::{markup, search::today, NodeId, ROOT};
use gpui::{Context, Window};

use super::{Outline, Target, Today};

impl Outline {
    /// The "Journal" bullet (created on first use).
    pub fn journal_root(&mut self) -> NodeId {
        if let Some(id) = self.journal_id.filter(|id| self.tree.contains(*id)) {
            return id;
        }
        let id = self.find_journal().unwrap_or_else(|| {
            let n = self.tree.children(ROOT).len();
            self.tree.create_at(ROOT, n, "Journal")
        });
        self.journal_id = Some(id);
        self.set_meta("journal", &id.to_string());
        id
    }

    /// The "Journal" bullet, if there is one.
    fn find_journal(&self) -> Option<NodeId> {
        self.journal_id
            .filter(|id| self.tree.contains(*id))
            .or_else(|| self.tree.children(ROOT).iter().copied().find(|id| markup::plain_text(&self.tree.node(*id).text) == "Journal"))
    }

    /// The journal page for `date` (YYYY-MM-DD), if it exists.
    pub fn find_day(&self, date: &str) -> Option<NodeId> {
        let journal = self.find_journal()?;
        self.tree.children(journal).iter().copied().find(|id| self.tree.node(*id).text.starts_with(date))
    }

    /// The journal page for `date` (YYYY-MM-DD), created if needed (not committed).
    pub fn ensure_day(&mut self, date: &str, with_blank: bool) -> NodeId {
        let journal = self.journal_root();
        let existing = self.tree.children(journal).iter().copied().find(|id| self.tree.node(*id).text.starts_with(date));
        match existing {
            Some(d) => d,
            None => {
                // Keep days sorted newest first.
                let index = self
                    .tree
                    .children(journal)
                    .iter()
                    .position(|id| self.tree.node(*id).text.as_str() < date)
                    .unwrap_or(self.tree.children(journal).len());
                let d = self.tree.create_at(journal, index, date);
                if with_blank {
                    self.tree.create_at(d, 0, "");
                }
                d
            }
        }
    }

    pub fn open_day(&mut self, date: &str, cx: &mut Context<Self>) {
        let day = self.ensure_day(date, true);
        self.commit_structure(None, cx);
        self.calendar = None;
        self.zoom_to(day, cx);
        if let Some(first) = self.rows.first().map(|r| r.id) {
            let len = self.tree.node(first).text.len();
            self.start_edit(Target::Text(first), len, cx);
        }
    }

    pub fn today(&mut self, _: &Today, _: &mut Window, cx: &mut Context<Self>) {
        let d = today();
        self.open_day(&d, cx);
    }
}
