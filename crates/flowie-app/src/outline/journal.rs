//! Open dated pages in the journal. The journal itself (its deterministic ids,
//! finding and creating pages) is `flowie_core::journal`.

use flowie_core::{journal, search::today};
use gpui::{Context, Window};

use super::{Outline, Target, Today};

impl Outline {
    pub fn open_day(&mut self, date: &str, cx: &mut Context<Self>) {
        let day = journal::ensure_day(&mut self.ws.tree, date, true);
        self.commit_structure(None, cx);
        self.calendar = None;
        self.zoom_to(day, cx);
        if let Some(first) = self.rows.first().map(|r| r.id) {
            let len = self.ws.tree.node(first).text.len();
            self.start_edit(Target::Text(first), len, cx);
        }
    }

    pub fn today(&mut self, _: &Today, _: &mut Window, cx: &mut Context<Self>) {
        let d = today();
        self.open_day(&d, cx);
    }
}
