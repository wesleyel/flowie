//! Block kinds, completion, folding, and colors.

use flowie_core::{markup, BlockKind, NodeId};
use gpui::{Context, Window};

use super::*;
use crate::ui::anim::{self, Channel};

impl Outline {
    fn show_levels(&mut self, levels: usize, cx: &mut Context<Self>) {
        let before = None;
        self.tree.expand_to_level(self.zoom, levels);
        if let Some(id) = self.editing_node() {
            if !self.tree.ancestors(id).iter().all(|a| !self.tree.node(*a).collapsed || !self.tree.is_ancestor(self.zoom, *a)) {
                self.edit = None;
            }
        }
        self.commit_structure(before, cx);
    }

    pub fn expand_all(&mut self, _: &ExpandAll, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(usize::MAX, cx);
    }
    pub fn level1(&mut self, _: &Level1, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(1, cx);
    }
    pub fn level2(&mut self, _: &Level2, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(2, cx);
    }
    pub fn level3(&mut self, _: &Level3, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(3, cx);
    }
    pub fn level4(&mut self, _: &Level4, _: &mut Window, cx: &mut Context<Self>) {
        self.show_levels(4, cx);
    }

    pub fn collapse(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        self.set_fold(true, cx);
    }

    pub fn expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        self.set_fold(false, cx);
    }

    fn set_fold(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        let ids = self.block_roots().or_else(|| self.editing_node().map(|id| vec![id])).unwrap_or_default();
        let before = self.caret();
        let mut changed = false;
        for id in ids {
            if id != self.zoom && self.tree.has_children(self.tree.source(id)) && self.tree.node(id).collapsed != collapsed {
                self.tree.set_collapsed(id, collapsed);
                self.anims.start(id, Channel::Fold, anim::ROW_IN);
                changed = true;
            }
        }
        if changed {
            self.commit_structure(before, cx);
        }
    }

    pub fn toggle_fold(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if !self.tree.has_children(self.tree.source(id)) {
            return;
        }
        let before = self.caret();
        let collapsed = !self.tree.node(id).collapsed;
        self.tree.set_collapsed(id, collapsed);
        self.anims.start(id, Channel::Fold, anim::ROW_IN);
        // Don't leave the caret inside a hidden row.
        if collapsed {
            if let Some(eid) = self.editing_node() {
                if self.tree.is_ancestor(id, eid) {
                    self.edit = Some(Edit::new(Target::Text(id), self.tree.node(id).text.len()));
                }
            }
        }
        self.commit_structure(before, cx);
    }

    pub fn set_kind_of_targets(&mut self, kind: BlockKind, cx: &mut Context<Self>) {
        let mut ids = self.targets();
        if ids.is_empty() {
            if let Some(id) = self.editing_node() {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return;
        }
        // Toggle back to a bullet when every target already has this kind.
        let kind = if ids.iter().all(|id| self.tree.node(*id).kind == kind) { BlockKind::Bullet } else { kind };
        let before = None;
        for id in ids {
            self.tree.set_kind(id, kind);
        }
        self.commit_structure(before, cx);
    }

    pub fn kind_bullet(&mut self, _: &KindBullet, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Bullet, cx);
    }
    pub fn kind_h1(&mut self, _: &KindH1, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::H1, cx);
    }
    pub fn kind_h2(&mut self, _: &KindH2, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::H2, cx);
    }
    pub fn kind_h3(&mut self, _: &KindH3, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::H3, cx);
    }
    pub fn kind_paragraph(&mut self, _: &KindParagraph, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Paragraph, cx);
    }
    pub fn kind_todo(&mut self, _: &KindTodo, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Todo, cx);
    }
    pub fn kind_numbered(&mut self, _: &KindNumbered, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Numbered, cx);
    }
    pub fn kind_quote(&mut self, _: &KindQuote, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Quote, cx);
    }
    pub fn kind_code(&mut self, _: &KindCode, _: &mut Window, cx: &mut Context<Self>) {
        self.set_kind_of_targets(BlockKind::Code, cx);
    }

    pub fn highlight(&mut self, _: &Highlight, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(crate::overlays::palette::Action::Wrap("==", "=="), window, cx);
    }

    fn recolor_targets(&mut self, pick: impl Fn(&[Option<u8>]) -> Option<u8>, cx: &mut Context<Self>) {
        if let Some(e) = self.edit.as_ref().filter(|e| matches!(e.target, Target::Text(_) | Target::Note(_))) {
            let text = self.edit_text().to_string();
            let sel = e.sel.clone();
            let range = if sel.is_empty() { 0..text.len() } else { sel.clone() };
            let colors = markup::colors_in(&text, range.clone());
            if colors.is_empty() {
                return;
            }
            let r = markup::recolor(&text, range, pick(&colors));
            if r.text == text {
                return;
            }
            let target = e.target;
            self.replace_edit_text(0..text.len(), &r.text, false, cx);
            self.inline = None;
            // The editor shows `{color}` markers; for a whole bullet, select
            // the row instead so the color shows (and the next press cycles it).
            if let Target::Text(id) = target {
                if sel.is_empty() && id != self.zoom {
                    self.history.seal();
                    self.edit = None;
                    self.block = Some(BlockSel { anchor: id, head: id });
                    cx.notify();
                    return;
                }
            }
            if let Some(e) = self.edit.as_mut() {
                e.sel = if sel.is_empty() {
                    let c = r.map(sel.start);
                    c..c
                } else {
                    r.range.clone()
                };
            }
            cx.notify();
            return;
        }
        let ids: Vec<NodeId> = self.targets().into_iter().map(|id| self.tree.source(id)).collect();
        let colors: Vec<Option<u8>> = ids
            .iter()
            .flat_map(|id| {
                let t = &self.tree.node(*id).text;
                markup::colors_in(t, 0..t.len())
            })
            .collect();
        if colors.is_empty() {
            return;
        }
        let color = pick(&colors);
        for id in ids {
            let text = self.tree.node(id).text.clone();
            let r = markup::recolor(&text, 0..text.len(), color);
            self.tree.set_text(id, &r.text);
        }
        self.commit_structure(None, cx);
    }

    /// Set one color; again on text that already has it removes it.
    fn set_color(&mut self, ix: u8, cx: &mut Context<Self>) {
        self.recolor_targets(|cs| if cs.iter().all(|c| *c == Some(ix)) { None } else { Some(ix) }, cx);
    }

    pub fn color_red(&mut self, _: &ColorRed, _: &mut Window, cx: &mut Context<Self>) {
        self.set_color(0, cx);
    }
    pub fn color_orange(&mut self, _: &ColorOrange, _: &mut Window, cx: &mut Context<Self>) {
        self.set_color(1, cx);
    }
    pub fn color_yellow(&mut self, _: &ColorYellow, _: &mut Window, cx: &mut Context<Self>) {
        self.set_color(2, cx);
    }
    pub fn color_green(&mut self, _: &ColorGreen, _: &mut Window, cx: &mut Context<Self>) {
        self.set_color(3, cx);
    }
    pub fn color_blue(&mut self, _: &ColorBlue, _: &mut Window, cx: &mut Context<Self>) {
        self.set_color(4, cx);
    }
    pub fn color_purple(&mut self, _: &ColorPurple, _: &mut Window, cx: &mut Context<Self>) {
        self.set_color(5, cx);
    }
    pub fn color_gray(&mut self, _: &ColorGray, _: &mut Window, cx: &mut Context<Self>) {
        self.set_color(6, cx);
    }
    pub fn color_none(&mut self, _: &ColorNone, _: &mut Window, cx: &mut Context<Self>) {
        self.recolor_targets(|_| None, cx);
    }
    /// Next color in order, starting from the first piece of text's color.
    pub fn cycle_color(&mut self, _: &CycleColor, _: &mut Window, cx: &mut Context<Self>) {
        self.recolor_targets(|cs| markup::next_color(cs[0]), cx);
    }

    /// Click on a to-do checkbox.
    pub fn toggle_checkbox(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let src = self.tree.source(id);
        let done = !self.tree.node(src).completed;
        self.tree.set_completed(src, done);
        self.anims.start(id, Channel::Complete, anim::FLAG);
        self.commit_structure(None, cx);
    }

    pub fn toggle_complete(&mut self, _: &ToggleComplete, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.caret();
        let ids = self.block_roots().or_else(|| self.editing_node().map(|id| vec![id])).unwrap_or_default();
        if ids.is_empty() {
            return;
        }
        let done = !ids.iter().all(|id| self.tree.node(*id).completed);
        for id in &ids {
            self.tree.set_completed(*id, done);
            self.anims.start(*id, Channel::Complete, anim::FLAG);
        }
        // When completing the edited row with completed rows hidden, move on to the next row.
        if done && self.hide_completed {
            if let Some(Target::Text(id)) = self.edit_target() {
                let next = self.neighbor(id, 1).or_else(|| self.neighbor(id, -1));
                self.edit = next.map(|n| Edit::new(Target::Text(n), 0));
            }
            self.block = None;
        }
        self.commit_structure(before, cx);
    }

    pub fn toggle_completed_visible(&mut self, _: &ToggleCompletedVisible, _: &mut Window, cx: &mut Context<Self>) {
        self.hide_completed = !self.hide_completed;
        self.set_meta("hide_completed", if self.hide_completed { "1" } else { "0" });
        if let Some(id) = self.editing_node() {
            if self.hide_completed && self.tree.node(id).completed {
                self.edit = None;
            }
        }
        self.refresh(true);
        cx.notify();
    }
}
