//! Implement the outline operations exposed to agents.

use flowie_core::short_id::Lookup;
use flowie_core::{io, journal, search::today, BlockKind, NodeId, ROOT};
use gpui::Context;
use serde_json::{json, Value};

use crate::outline::{Edit, Outline, Target};
use crate::overlays::palette::node_label;
use crate::ui::anim::{self, Channel};

fn s<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

fn n(p: &Value, k: &str) -> Option<u64> {
    p.get(k).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
}

impl Outline {
    /// Agents write links with the short ids they were shown:
    /// `[label](flowie:3f9a1c)` is stored with the full id, which stays valid
    /// however many nodes are added later.
    fn expand_links(&self, text: &str) -> Result<String, String> {
        const MARK: &str = "](flowie:";
        if !text.contains(MARK) {
            return Ok(text.to_string());
        }
        let mut short = self.short_ids.borrow_mut();
        short.update(&self.ws.tree);
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(i) = rest.find(MARK) {
            let start = i + MARK.len();
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            let len = rest.find(|c: char| !(c.is_ascii_hexdigit() || c == '-')).unwrap_or(rest.len());
            let (target, after) = rest.split_at(len);
            if target.parse::<NodeId>().is_ok() {
                out.push_str(target);
            } else {
                let id = short.resolve(target).map_err(|e| match e {
                    Lookup::Ambiguous(_) => format!("link target flowie:{target} is ambiguous; use more digits"),
                    _ => format!("link target flowie:{target} is not a bullet id"),
                })?;
                out.push_str(&id.to_string());
            }
            rest = after;
        }
        out.push_str(rest);
        Ok(out)
    }

    /// `flowie:` links as agents see them: with short ids (`expand_links` undoes this).
    fn shorten_links(&self, text: &str) -> String {
        const MARK: &str = "](flowie:";
        if !text.contains(MARK) {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(i) = rest.find(MARK) {
            let start = i + MARK.len();
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            let len = rest.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(rest.len());
            let (target, after) = rest.split_at(len);
            match target.parse::<NodeId>() {
                Ok(id) => out.push_str(&self.sid(id)),
                Err(()) => out.push_str(target),
            }
            rest = after;
        }
        out.push_str(rest);
        out
    }

    /// The id shown to agents: the shortest unambiguous prefix, like git.
    fn sid(&self, id: NodeId) -> String {
        let mut short = self.short_ids.borrow_mut();
        short.update(&self.ws.tree);
        short.short(id)
    }

    /// Resolve "home", "current", "today", a YYYY-MM-DD day, or a bullet id (full, or a
    /// unique prefix of at least 4 hex digits, optionally after "id:") to a real node id.
    /// A missing journal day is created (uncommitted): for methods that write.
    pub(crate) fn agent_node(&mut self, p: &Value, key: &str) -> Result<NodeId, String> {
        self.agent_node_with(p, key, true)
    }

    /// Like `agent_node`, but a missing journal day is an error instead of
    /// being created: for methods that only read.
    fn agent_existing_node(&mut self, p: &Value, key: &str) -> Result<NodeId, String> {
        self.agent_node_with(p, key, false)
    }

    fn agent_node_with(&mut self, p: &Value, key: &str, create_day: bool) -> Result<NodeId, String> {
        let v = p.get(key).ok_or_else(|| format!("missing `{key}`"))?;
        let bad = || format!("`{key}` must be a bullet id (from read/search), \"home\", \"current\", \"today\" or a YYYY-MM-DD day");
        let day = |this: &mut Self, date: &str| {
            if create_day {
                Ok(journal::ensure_day(&mut this.ws.tree, date, false))
            } else {
                journal::find_day(&this.ws.tree, date).ok_or_else(|| format!("there is no journal page for {date}"))
            }
        };
        let id = match v.as_str().map(|s| s.trim().trim_start_matches("id:")) {
            Some("home") | Some("root") => ROOT,
            Some("current") => self.zoom,
            Some("today") => day(self, &today())?,
            Some(s) if journal::is_date(s) => day(self, s)?,
            Some(s) => match s.parse::<NodeId>() {
                Ok(id) => id,
                Err(()) => {
                    let mut short = self.short_ids.borrow_mut();
                    short.update(&self.ws.tree);
                    match short.resolve(s) {
                        Ok(id) => id,
                        Err(Lookup::Invalid) => return Err(bad()),
                        Err(Lookup::NotFound) => return Err(format!("no bullet with id {s}")),
                        Err(Lookup::Ambiguous(ids)) => {
                            let list: Vec<String> = ids.iter().map(|id| short.short(*id)).collect();
                            return Err(format!("id {s} is ambiguous; it could be {}", list.join(", ")));
                        }
                    }
                }
            },
            None => return Err(bad()),
        };
        let id = self.ws.tree.resolve(id);
        if id != ROOT && !self.ws.tree.contains(id) {
            return Err(format!("no bullet with id {}", self.sid(id)));
        }
        Ok(id)
    }

    fn path_of(&self, id: NodeId) -> String {
        let mut parts = vec!["Home".to_string()];
        for a in self.ws.tree.ancestors(id) {
            parts.push(node_label(self, a).chars().take(40).collect());
        }
        parts.join(" › ")
    }

    fn line_for(&self, id: NodeId) -> String {
        let node = self.ws.tree.node(id);
        let src = self.ws.tree.node(self.ws.tree.source(id));
        let text = &self.shorten_links(&src.text);
        let body = match node.kind {
            BlockKind::H1 => format!("# {text}"),
            BlockKind::H2 => format!("## {text}"),
            BlockKind::H3 => format!("### {text}"),
            BlockKind::Todo => format!("[{}] {text}", if src.completed { "x" } else { " " }),
            BlockKind::Quote => format!("> {text}"),
            BlockKind::Code => format!("`{text}`"),
            _ if src.completed => format!("~~{text}~~"),
            _ => text.clone(),
        };
        let mut tags = vec![format!("id:{}", self.sid(id))];
        if node.mirror != ROOT {
            tags.push(format!("mirror of id:{}", self.sid(node.mirror)));
        }
        if !matches!(
            node.kind,
            BlockKind::Bullet | BlockKind::H1 | BlockKind::H2 | BlockKind::H3 | BlockKind::Todo | BlockKind::Quote | BlockKind::Code
        ) {
            tags.push(node.kind.name().to_string());
        }
        format!("{body}  [{}]", tags.join(", "))
    }

    fn write_tree(&self, id: NodeId, depth: usize, max: usize, out: &mut String) {
        let pad = "  ".repeat(depth);
        out.push_str(&format!("{pad}- {}\n", self.line_for(id)));
        let src = self.ws.tree.source(id);
        let note = &self.ws.tree.node(src).note;
        for l in self.shorten_links(note).lines() {
            out.push_str(&format!("{pad}  > {l}\n"));
        }
        let kids = self.ws.tree.children(src);
        if kids.is_empty() {
            return;
        }
        if depth + 1 >= max {
            out.push_str(&format!("{pad}  - … {} more (read id:{} to expand)\n", self.ws.tree.descendants(src).len(), self.sid(id)));
            return;
        }
        for &k in kids {
            self.write_tree(k, depth + 1, max, out);
        }
    }

    pub(super) fn agent_read(&mut self, p: &Value) -> Result<String, String> {
        if let Some(ids) = p.get("ids") {
            let ids = Self::id_list(&json!({ "ids": ids }))?;
            let mut parts = Vec::new();
            for id in ids {
                let mut one = p.clone();
                one.as_object_mut().map(|o| o.remove("ids"));
                one["id"] = id;
                parts.push(self.agent_read(&one)?);
            }
            return Ok(parts.join("\n---\n\n"));
        }
        let id = if p.get("id").is_some() { self.agent_existing_node(p, "id")? } else { ROOT };
        let depth = n(p, "depth").unwrap_or(3).clamp(1, 20) as usize;
        let mut out = String::new();
        if id == ROOT {
            out.push_str("# Home  [id:home]\n");
            for &k in self.ws.tree.children(ROOT) {
                self.write_tree(k, 0, depth, &mut out);
            }
        } else {
            out.push_str(&format!("Path: {}\n", self.path_of(id)));
            let src = self.ws.tree.source(id);
            out.push_str(&format!("# {}\n", self.line_for(id)));
            for l in self.shorten_links(&self.ws.tree.node(src).note).lines() {
                out.push_str(&format!("> {l}\n"));
            }
            for &k in self.ws.tree.children(src) {
                self.write_tree(k, 0, depth, &mut out);
            }
            let links = self.ws.tree.backlinks(src);
            if !links.is_empty() {
                out.push_str(&format!(
                    "\nLinked from: {}\n",
                    links.iter().map(|l| format!("id:{}", self.sid(*l))).collect::<Vec<_>>().join(", ")
                ));
            }
        }
        if self.ws.tree.children(self.ws.tree.source(id)).is_empty() && id == ROOT {
            out.push_str("(empty)\n");
        }
        Ok(out)
    }

    pub(super) fn agent_search(&mut self, p: &Value) -> Result<String, String> {
        let q = s(p, "query").unwrap_or("").to_string();
        let limit = n(p, "limit").unwrap_or(50).clamp(1, 500) as usize;
        let query = self.parse_query(&q);
        if query.is_empty() {
            return Err("empty query".into());
        }
        let hits = flowie_doc::find::search(&self.ws.tree, &query);
        let total = hits.len();
        let mut out = format!(
            "{total} match{} for {q:?}{}\n",
            if total == 1 { "" } else { "es" },
            if total > limit { format!(" (showing {limit})") } else { String::new() }
        );
        for id in hits.into_iter().take(limit) {
            out.push_str(&format!("- {}  — in {}\n", self.line_for(id), self.path_of(id)));
        }
        Ok(out)
    }

    pub(super) fn agent_starred(&self) -> String {
        let mut out = String::from("Starred pages:\n");
        let live: Vec<NodeId> = self.ws.tree.starred().collect();
        if live.is_empty() {
            out.push_str("(none)\n");
        }
        for id in live {
            out.push_str(&format!("- {}  — in {}\n", self.line_for(id), self.path_of(id)));
        }
        if !self.saved_searches.is_empty() {
            out.push_str("Saved searches:\n");
            for q in &self.saved_searches {
                out.push_str(&format!("- {q}\n"));
            }
        }
        out
    }

    pub(super) fn agent_current(&self) -> String {
        let mut out = String::new();
        if self.zoom == ROOT {
            out.push_str("Page: Home  [id:home]\n");
        } else {
            out.push_str(&format!("Path: {}\n", self.path_of(self.zoom)));
            out.push_str(&format!("Page: {}\n", self.line_for(self.zoom)));
        }
        if !self.query.is_empty() {
            out.push_str(&format!("Search: {}\n", self.query));
        }
        if self.trash.is_some() {
            out.push_str("View: Trash\n");
        } else if self.volatile_page.is_some() {
            out.push_str("View: What's New\n");
        }
        let edit_target = self.edit.as_ref().map(|e| e.target).or_else(|| self.settings_return.map(|(t, _)| t));
        match edit_target {
            Some(Target::Text(id)) => {
                let real = self.ws.tree.resolve(id);
                if real != ROOT && self.ws.tree.contains(real) {
                    out.push_str(&format!("Focused bullet: {}  — in {}\n", self.line_for(real), self.path_of(real)));
                }
            }
            Some(Target::Note(id)) => {
                let real = self.ws.tree.resolve(id);
                if real != ROOT && self.ws.tree.contains(real) {
                    out.push_str(&format!("Focused bullet (note): {}  — in {}\n", self.line_for(real), self.path_of(real)));
                }
            }
            _ => {
                if let Some(ids) = self.block_roots() {
                    let live: Vec<NodeId> =
                        ids.into_iter().map(|id| self.ws.tree.resolve(id)).filter(|&id| id != ROOT && self.ws.tree.contains(id)).collect();
                    if !live.is_empty() {
                        out.push_str("Selected bullets:\n");
                        for id in live {
                            out.push_str(&format!("- {}  — in {}\n", self.line_for(id), self.path_of(id)));
                        }
                    }
                }
            }
        }
        out
    }

    pub(super) fn agent_open(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let id = self.agent_existing_node(p, "id")?;
        self.zoom_to(id, cx);
        if self.settings.is_some() {
            self.edit = None;
        }
        Ok(format!("Showing {} in Flowie", if id == ROOT { "Home".into() } else { format!("id:{}", self.sid(id)) }))
    }

    /// Finish an agent edit: one undo step, persist, optionally focus the
    /// changed bullet, and flash the touched rows.
    fn agent_commit(&mut self, touched: &[NodeId], focus_fallback: Option<NodeId>, cx: &mut Context<Self>) {
        let before = self.caret();
        // Don't leave the caret or zoom on a node that no longer exists.
        if let Some(id) = self.editing_node() {
            if !self.ws.tree.contains(self.ws.tree.resolve(id)) {
                self.edit = None;
            }
        }
        let zoom_gone = self.zoom != ROOT && !self.ws.tree.contains(self.zoom);
        if zoom_gone {
            self.zoom = ROOT;
        }
        self.commit_structure(before, cx);
        self.ws.history.seal();
        if zoom_gone {
            self.zoom_to(ROOT, cx);
            if self.settings.is_some() {
                self.edit = None;
            }
        }
        if self.config.agent_follow_edits {
            if let Some(target) = touched.first().copied().filter(|&id| self.ws.tree.contains(id)).or(focus_fallback) {
                self.agent_focus_target(target, cx);
            }
        }
        for &id in touched {
            if self.ws.tree.contains(id) {
                self.anims.start(id, Channel::Flash, anim::FLASH);
            }
        }
    }

    /// Bring a bullet changed by an agent into view: close auxiliary pages,
    /// jump to its parent page if it's outside the current zoom, expand
    /// collapsed ancestors, and scroll to it.
    fn agent_focus_target(&mut self, target: NodeId, cx: &mut Context<Self>) {
        if target != ROOT && !self.ws.tree.contains(target) {
            return;
        }
        if self.is_auxiliary_page_open() {
            self.close_auxiliary_page(false, cx);
        }
        if target == ROOT {
            if self.zoom != ROOT {
                self.zoom_to(ROOT, cx);
                if self.settings.is_some() {
                    self.edit = None;
                }
            }
            return;
        }
        let mut jumped = false;
        let in_view = target == self.zoom || self.ws.tree.is_ancestor(self.zoom, target);
        if !in_view {
            let dest = self.ws.tree.parent(target);
            self.zoom_to(dest, cx);
            if self.settings.is_some() {
                self.edit = None;
            }
            jumped = true;
        }
        let mut need_refresh = false;
        let mut expanded = false;
        for a in self.ws.tree.ancestors(target) {
            if self.ws.tree.node(a).collapsed && self.ws.tree.is_ancestor(self.zoom, a) {
                self.ws.tree.set_collapsed(a, false);
                expanded = true;
            }
        }
        if expanded {
            self.ws.tree.take_journal();
            self.ws.persist();
            need_refresh = true;
        }
        if target != self.zoom && !self.query.is_empty() && self.display_index(target).is_none() {
            self.query.clear();
            need_refresh = true;
        }
        if need_refresh {
            self.refresh(false);
        }
        if target != self.zoom {
            if jumped && self.settings.is_none() && self.row_index(target).is_some() {
                let len = self.ws.tree.node(self.ws.tree.source(target)).text.len();
                self.edit = Some(Edit::new(Target::Text(target), len));
            }
            self.reveal(target);
        }
    }

    /// A new image bullet from base64 data, under `parent` (default Home) or after `after`.
    pub(super) fn agent_attach_image(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let data = s(p, "data").ok_or("missing `data` (base64 image bytes)")?;
        let data = data.split_once("base64,").map_or(data, |(_, d)| d);
        let bytes = crate::image::decode_base64(data).ok_or("`data` is not base64")?;
        let name = self.store_image(&bytes)?;
        self.agent_write(cx, |this| {
            let (parent, index) = this.agent_place(p, true)?;
            let caption = this.expand_links(s(p, "caption").unwrap_or(""))?;
            let id = this.ws.tree.create_at(parent, index, &caption.replace('\n', " "));
            let mut node = this.ws.tree.node(id).clone();
            node.kind = BlockKind::Image;
            node.image = name;
            this.ws.tree.put(node);
            if parent != ROOT && this.ws.tree.node(parent).collapsed {
                this.ws.tree.set_collapsed(parent, false);
            }
            let info = this.ws.blobs.get(&this.ws.tree.node(id).image);
            let msg = format!("Added image [id:{}] ({}×{})", this.sid(id), info.map_or(0, |b| b.width), info.map_or(0, |b| b.height));
            Ok(Applied { msg, touched: vec![id], created: vec![id], ..Applied::default() })
        })
    }

    /// Where new or moved bullets go: right after `after`, or under `parent`
    /// (default Home when `parent_optional`) at `index` (default the end).
    fn agent_place(&mut self, p: &Value, parent_optional: bool) -> Result<(NodeId, usize), String> {
        if p.get("after").is_some() {
            let after = self.agent_node(p, "after")?;
            if after == ROOT {
                return Err("can't place bullets after Home; use `parent`".into());
            }
            return Ok((self.ws.tree.parent(after), self.ws.tree.index_in_parent(after) + 1));
        }
        let parent = if p.get("parent").is_some() || !parent_optional { self.agent_node(p, "parent")? } else { ROOT };
        let parent = self.ws.tree.source(parent);
        let len = self.ws.tree.children(parent).len();
        Ok((parent, n(p, "index").map(|i| (i as usize).min(len)).unwrap_or(len)))
    }

    /// Run one agent edit all-or-nothing: if `apply` fails, every change it
    /// made is rolled back; otherwise it becomes one undo step.
    fn agent_write(&mut self, cx: &mut Context<Self>, apply: impl FnOnce(&mut Self) -> Result<Applied, String>) -> Result<String, String> {
        let mark = self.ws.tree.journal_mark();
        match apply(self) {
            Err(e) => {
                self.ws.tree.rollback_to(mark);
                Err(e)
            }
            Ok(done) => {
                if self.ws.tree.journal_mark() == mark {
                    return Ok(done.msg);
                }
                self.agent_commit(&done.touched, done.fallback, cx);
                for id in done.completed {
                    if self.ws.tree.contains(id) {
                        self.anims.start(id, Channel::Complete, anim::FLAG);
                    }
                }
                Ok(done.msg)
            }
        }
    }

    pub(super) fn agent_create(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        self.agent_write(cx, |this| this.apply_create(p))
    }

    fn apply_create(&mut self, p: &Value) -> Result<Applied, String> {
        let items = self.agent_items(p)?;
        let (parent, index) = self.agent_place(p, true)?;
        // Drop a lone empty placeholder bullet (e.g. a fresh journal day).
        let kids = self.ws.tree.children(parent).to_vec();
        let placeholder = (kids.len() == 1 && journal::is_blank_placeholder(&self.ws.tree, kids[0])).then(|| kids[0]);
        let top = self.ws.tree.insert_items(parent, index, &items);
        if let Some(ph) = placeholder {
            if self.editing_node() != Some(ph) {
                self.ws.tree.delete(ph);
            }
        }
        if parent != ROOT && self.ws.tree.node(parent).collapsed {
            self.ws.tree.set_collapsed(parent, false);
        }
        Ok(self.created(parent, top))
    }

    /// The bullets in a create or journal call's `markdown`.
    fn agent_items(&mut self, p: &Value) -> Result<Vec<io::Item>, String> {
        let markdown = self.expand_links(s(p, "markdown").ok_or("missing `markdown`")?)?;
        let mut items = io::parse_any(&markdown);
        // The sandboxed `flowie mcp` can't read the agent's files: images come through `attach_image`.
        self.resolve_item_images(&mut items, None);
        if items.is_empty() {
            return Err("`markdown` contained no bullets".into());
        }
        Ok(items)
    }

    /// The result of creating the bullets `top` (and their subtrees) under `parent`.
    fn created(&mut self, parent: NodeId, top: Vec<NodeId>) -> Applied {
        let mut all = Vec::new();
        for &t in &top {
            all.push(t);
            all.extend(self.ws.tree.descendants(t));
        }
        let mut msg = format!(
            "Created {} bullet{} under {}:\n",
            all.len(),
            if all.len() == 1 { "" } else { "s" },
            if parent == ROOT { "Home".to_string() } else { format!("id:{}", self.sid(parent)) }
        );
        for &t in &top {
            self.write_tree(t, 0, 20, &mut msg);
        }
        Applied { msg: msg.trim_end().to_string(), touched: all, created: top, ..Applied::default() }
    }

    pub(super) fn agent_update(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        self.agent_write(cx, |this| this.apply_update(p))
    }

    fn apply_update(&mut self, p: &Value) -> Result<Applied, String> {
        let id = self.agent_node(p, "id")?;
        if id == ROOT {
            return Err("Home can't be edited".into());
        }
        let kind = match s(p, "kind") {
            Some(k) => Some(
                BlockKind::from_name(k)
                    .ok_or_else(|| format!("unknown kind {k:?}; use bullet, paragraph, h1, h2, h3, todo, numbered, quote or code"))?,
            ),
            None => None,
        };
        if kind == Some(BlockKind::Image) && self.ws.tree.node(self.ws.tree.source(id)).image.is_empty() {
            return Err("only a bullet with an image can be kind image; add images with attach_image".into());
        }
        let text = s(p, "text").map(|t| self.expand_links(&t.replace('\n', " "))).transpose()?;
        let note = s(p, "note").map(|n| self.expand_links(n)).transpose()?;
        let mark = self.ws.tree.journal_mark();
        let src = self.ws.tree.source(id);
        if let Some(t) = text {
            self.ws.tree.set_text(src, &t);
        }
        if let Some(note) = note {
            self.ws.tree.set_note(src, &note);
        }
        if let Some(done) = p.get("completed").and_then(Value::as_bool) {
            self.ws.tree.set_completed(src, done);
        }
        if let Some(kind) = kind {
            self.ws.tree.set_kind(id, kind);
        }
        if let Some(c) = p.get("collapsed").and_then(Value::as_bool) {
            self.ws.tree.set_collapsed(id, c);
        }
        if self.ws.tree.journal_mark() == mark {
            return Ok(Applied { msg: format!("Nothing changed: {}", self.line_for(id)), ..Applied::default() });
        }
        Ok(Applied { msg: format!("Updated: {}", self.line_for(id)), touched: vec![id], ..Applied::default() })
    }

    pub(super) fn agent_complete(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        self.agent_write(cx, |this| this.apply_complete(p))
    }

    fn apply_complete(&mut self, p: &Value) -> Result<Applied, String> {
        let id = self.agent_node(p, "id")?;
        if id == ROOT {
            return Err("Home can't be completed".into());
        }
        let done = p.get("completed").and_then(Value::as_bool).unwrap_or(true);
        self.ws.tree.set_completed(self.ws.tree.source(id), done);
        let msg = format!("{}: {}", if done { "Completed" } else { "Reopened" }, self.line_for(id));
        Ok(Applied { msg, touched: vec![id], completed: vec![id], ..Applied::default() })
    }

    /// `ids` (or `id`) as a list of raw values.
    fn id_list(p: &Value) -> Result<Vec<Value>, String> {
        match p.get("ids").or_else(|| p.get("id")) {
            Some(Value::Array(a)) if !a.is_empty() => Ok(a.clone()),
            Some(Value::Array(_)) => Err("`ids` is empty".into()),
            Some(v) => Ok(vec![v.clone()]),
            None => Err("missing `ids`".into()),
        }
    }

    pub(super) fn agent_move(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        self.agent_write(cx, |this| this.apply_move(p))
    }

    /// Move `id` or every bullet in `ids`, in that order, to one place.
    fn apply_move(&mut self, p: &Value) -> Result<Applied, String> {
        let ids = Self::id_list(p)?.into_iter().map(|v| self.agent_node(&json!({ "id": v }), "id")).collect::<Result<Vec<_>, _>>()?;
        if ids.contains(&ROOT) {
            return Err("Home can't be moved".into());
        }
        let (parent, index) = self.agent_place(p, false)?;
        let mut prev: Option<NodeId> = None;
        for &id in &ids {
            // Each next bullet goes right after the one moved before it.
            let (to, at) = match prev {
                None => (parent, index),
                Some(prev) => (self.ws.tree.parent(prev), self.ws.tree.index_in_parent(prev) + 1),
            };
            if !self.ws.tree.move_to(id, to, at) {
                return Err(format!("can't move id:{} into itself", self.sid(id)));
            }
            prev = Some(id);
        }
        if parent != ROOT {
            self.ws.tree.set_collapsed(parent, false);
        }
        let msg = match ids.as_slice() {
            [id] => format!("Moved {} to {}", self.line_for(*id), self.path_of(*id)),
            _ => format!("Moved {} bullets to {}", ids.len(), self.path_of(ids[0])),
        };
        Ok(Applied { msg, touched: ids, ..Applied::default() })
    }

    pub(super) fn agent_delete(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        self.agent_write(cx, |this| this.apply_delete(p))
    }

    fn apply_delete(&mut self, p: &Value) -> Result<Applied, String> {
        // Resolve every id before deleting anything.
        let ids =
            Self::id_list(p)?.into_iter().map(|v| self.agent_existing_node(&json!({ "id": v }), "id")).collect::<Result<Vec<_>, _>>()?;
        let mut fallback_chain = Vec::new();
        let mut done = Vec::new();
        for id in ids {
            // Skip Home, and bullets already gone with an earlier id's subtree.
            if id == ROOT || !self.ws.tree.contains(id) {
                continue;
            }
            if fallback_chain.is_empty() {
                fallback_chain = self.ws.tree.ancestors(id);
                fallback_chain.reverse();
                fallback_chain.push(ROOT);
            }
            done.push(self.line_for(id));
            self.ws.tree.trash(id);
        }
        let fallback = fallback_chain.into_iter().find(|&a| a == ROOT || self.ws.tree.contains(a));
        let msg = format!("Moved {} bullet(s) to Trash (restorable from File → Trash):\n- {}", done.len(), done.join("\n- "));
        Ok(Applied { msg, fallback, ..Applied::default() })
    }

    pub(super) fn agent_journal(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        self.agent_write(cx, |this| this.apply_journal(p))
    }

    fn apply_journal(&mut self, p: &Value) -> Result<Applied, String> {
        let date = s(p, "date").map(String::from).unwrap_or_else(today);
        if !journal::is_date(&date) {
            return Err("`date` must be YYYY-MM-DD".into());
        }
        let items = self.agent_items(p)?;
        let editing = self.editing_node();
        let top = journal::journal_append(&mut self.ws.tree, &date, &items, editing);
        Ok(self.created(journal::day_id(&date), top))
    }

    /// Several writes as one all-or-nothing call and one undo step. A
    /// `create`/`journal` op with a `key` lets later ops name its first new
    /// bullet as `"$key"` in `id`, `ids`, `parent` or `after`.
    pub(super) fn agent_batch(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let ops = p.get("ops").and_then(Value::as_array).ok_or("missing `ops` (an array of operations)")?;
        if ops.is_empty() {
            return Err("`ops` is empty".into());
        }
        if ops.len() > MAX_BATCH {
            return Err(format!("too many ops ({}); send at most {MAX_BATCH} per batch", ops.len()));
        }
        self.agent_write(cx, |this| {
            let mut keys: Vec<(String, NodeId)> = Vec::new();
            let mut all = Applied::default();
            let mut lines = Vec::new();
            for (i, op) in ops.iter().enumerate() {
                let name = op.get("op").and_then(Value::as_str).unwrap_or("");
                let fail =
                    |e: String| format!("op {} ({}) failed: {e}. Nothing was changed.", i + 1, if name.is_empty() { "?" } else { name });
                let args = substitute_keys(op, &keys).map_err(fail)?;
                let done = match name {
                    "create" => this.apply_create(&args),
                    "journal" => this.apply_journal(&args),
                    "update" => this.apply_update(&args),
                    "complete" => this.apply_complete(&args),
                    "move" => this.apply_move(&args),
                    "delete" => this.apply_delete(&args),
                    "" => Err("missing `op`".into()),
                    other => Err(format!("unknown op {other:?}; use create, journal, update, complete, move or delete")),
                }
                .map_err(fail)?;
                if let Some(key) = s(op, "key") {
                    let first = *done.created.first().ok_or_else(|| fail("`key` only applies to create and journal".into()))?;
                    keys.retain(|(k, _)| k != key);
                    keys.push((key.to_string(), first));
                }
                lines.push(format!("{}. {}", i + 1, done.msg));
                all.touched.extend(done.touched);
                all.completed.extend(done.completed);
                all.fallback = all.fallback.or(done.fallback);
            }
            all.touched.retain(|id| this.ws.tree.contains(*id));
            let mut msg =
                format!("Applied {} op{} (one undo step):\n{}", ops.len(), if ops.len() == 1 { "" } else { "s" }, lines.join("\n"));
            if !keys.is_empty() {
                let list: Vec<String> = keys
                    .iter()
                    .filter(|(_, id)| this.ws.tree.contains(*id))
                    .map(|(k, id)| format!("${k} = id:{}", this.sid(*id)))
                    .collect();
                msg.push_str(&format!("\nKeys: {}", list.join(", ")));
            }
            all.msg = msg;
            Ok(all)
        })
    }
}

/// Most ops one `batch` call takes.
const MAX_BATCH: usize = 500;

/// What one write did, for `agent_write` to finish.
#[derive(Default)]
struct Applied {
    /// The reply to the agent.
    msg: String,
    /// Bullets to flash and, with follow-edits on, to focus.
    touched: Vec<NodeId>,
    /// Top-level bullets created (what a batch `key` names).
    created: Vec<NodeId>,
    /// Bullets that were completed or reopened (they get the check animation).
    completed: Vec<NodeId>,

    /// Where to go if the bullet in focus was deleted.
    fallback: Option<NodeId>,
}

/// An op with `"$key"` references in its id fields replaced by the bullets
/// created earlier in the same batch.
fn substitute_keys(op: &Value, keys: &[(String, NodeId)]) -> Result<Value, String> {
    let resolve = |v: &Value| -> Result<Value, String> {
        match v.as_str().and_then(|s| s.strip_prefix('$')) {
            Some(key) => keys
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, id)| json!(id.to_string()))
                .ok_or_else(|| format!("no earlier op in this batch has key {key:?}")),
            None => Ok(v.clone()),
        }
    };
    let mut out = op.clone();
    for field in ["id", "parent", "after"] {
        if let Some(v) = op.get(field) {
            out[field] = resolve(v)?;
        }
    }
    if let Some(Value::Array(ids)) = op.get("ids") {
        out["ids"] = Value::Array(ids.iter().map(resolve).collect::<Result<_, _>>()?);
    }
    Ok(out)
}
