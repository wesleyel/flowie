//! Agent interface. The running app listens on a Unix socket (owner-only)
//! for newline-delimited JSON requests `{"id", "method", "params"}` and
//! answers `{"id", "result"}` or `{"id", "error"}`. Requests run on the UI
//! thread through the same code paths as keyboard edits, so changes show up
//! live, persist normally, and each request is a single undo step.
//! `flowie mcp` (see `mcp.rs`) bridges this socket to MCP over stdio.

pub mod mcp;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use flowie_core::{io, markup, search::today, BlockKind, NodeId, Query, ROOT};
use futures::StreamExt;
use gpui::{App, Context, Entity};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::anim::{self, Channel};
use crate::outline::Outline;
use crate::palette::node_label;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMode {
    Off,
    #[default]
    ReadOnly,
    ReadWrite,
}

/// Socket for the database at `db` (short path: Unix sockets allow ~100 bytes).
pub fn socket_path(db: &std::path::Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    db.hash(&mut h);
    let uid = unsafe { libc_getuid() };
    PathBuf::from(format!("/tmp/flowie-{uid}-{:08x}.sock", h.finish() as u32))
}

extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[derive(Debug, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

type Job = (Request, std::sync::mpsc::Sender<Result<String, String>>);

/// Start listening for agent requests on behalf of `view`.
pub fn start(db: &std::path::Path, view: Entity<Outline>, cx: &mut App) {
    let path = socket_path(db);
    // A stale socket from a crashed run: remove it if nobody answers.
    if path.exists() && UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("flowie: agent socket {} unavailable: {e}", path.display());
            return;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<Job>();
    std::thread::Builder::new()
        .name("flowie-agent".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve(stream, tx));
            }
        })
        .ok();
    let weak = view.downgrade();
    cx.spawn(async move |cx| {
        while let Some((req, reply)) = rx.next().await {
            let result = weak
                .update(cx, |o, cx| o.handle_agent(&req.method, &req.params, cx))
                .unwrap_or_else(|_| Err("Flowie is shutting down".into()));
            let _ = reply.send(result);
        }
    })
    .detach();
    let cleanup = path.clone();
    cx.on_app_quit(move |_| {
        let _ = std::fs::remove_file(&cleanup);
        async {}
    })
    .detach();
}

fn serve(stream: UnixStream, tx: futures::channel::mpsc::UnboundedSender<Job>) {
    let Ok(mut out) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(req) => {
                let id = req.id.clone();
                let (rtx, rrx) = std::sync::mpsc::channel();
                if tx.unbounded_send((req, rtx)).is_err() {
                    json!({"id": id, "error": "Flowie is not running"})
                } else {
                    match rrx.recv_timeout(Duration::from_secs(20)) {
                        Ok(Ok(text)) => json!({"id": id, "result": text}),
                        Ok(Err(e)) => json!({"id": id, "error": e}),
                        Err(_) => json!({"id": id, "error": "Flowie did not answer in time"}),
                    }
                }
            }
            Err(e) => json!({"id": null, "error": format!("bad request: {e}")}),
        };
        if writeln!(out, "{resp}").is_err() {
            break;
        }
    }
}

// ------------------------------------------------------------------ tools

fn s<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

fn n(p: &Value, k: &str) -> Option<u64> {
    p.get(k).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim_start_matches("id:").parse().ok())))
}

const WRITES: [&str; 6] = ["create", "update", "move", "delete", "journal", "complete"];

impl Outline {
    pub fn handle_agent(&mut self, method: &str, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        match self.config.agent {
            AgentMode::Off => return Err("Agent access is turned off in Flowie → Settings → Agent.".into()),
            AgentMode::ReadOnly if WRITES.contains(&method) => {
                return Err("Flowie is in read-only mode for agents. The user can allow edits in Flowie → Settings → Agent.".into())
            }
            _ => {}
        }
        let result = match method {
            "search" => self.agent_search(p),
            "read" => self.agent_read(p),
            "starred" => Ok(self.agent_starred()),
            "open" => self.agent_open(p, cx),
            "create" => self.agent_create(p, cx),
            "update" => self.agent_update(p, cx),
            "complete" => self.agent_complete(p, cx),
            "move" => self.agent_move(p, cx),
            "delete" => self.agent_delete(p, cx),
            "journal" => self.agent_journal(p, cx),
            _ => Err(format!("unknown method {method}")),
        };
        cx.notify();
        result
    }

    /// Resolve "home", "today", "id:12" or 12 to a real node id.
    fn agent_node(&mut self, p: &Value, key: &str) -> Result<NodeId, String> {
        let v = p.get(key).ok_or_else(|| format!("missing `{key}`"))?;
        let id = match v.as_str() {
            Some("home") | Some("root") => ROOT,
            Some("today") => self.ensure_day(&today(), false),
            Some(s) if s.len() == 10 && markup::date_len(s) == Some(10) => self.ensure_day(s, false),
            _ => n(p, key).ok_or_else(|| format!("`{key}` must be a bullet id, \"home\" or \"today\""))?,
        };
        let id = self.tree.resolve(id);
        if id != ROOT && !self.tree.contains(id) {
            return Err(format!("no bullet with id {id}"));
        }
        Ok(id)
    }

    fn path_of(&self, id: NodeId) -> String {
        let mut parts = vec!["Home".to_string()];
        for a in self.tree.ancestors(id) {
            parts.push(node_label(self, a).chars().take(40).collect());
        }
        parts.join(" › ")
    }

    fn line_for(&self, id: NodeId) -> String {
        let node = self.tree.node(id);
        let src = self.tree.node(self.tree.source(id));
        let text = &src.text;
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
        let mut tags = vec![format!("id:{id}")];
        if node.mirror != ROOT {
            tags.push(format!("mirror of id:{}", node.mirror));
        }
        if !matches!(node.kind, BlockKind::Bullet | BlockKind::H1 | BlockKind::H2 | BlockKind::H3 | BlockKind::Todo | BlockKind::Quote | BlockKind::Code) {
            tags.push(node.kind.name().to_string());
        }
        format!("{body}  [{}]", tags.join(", "))
    }

    fn write_tree(&self, id: NodeId, depth: usize, max: usize, out: &mut String) {
        let pad = "  ".repeat(depth);
        out.push_str(&format!("{pad}- {}\n", self.line_for(id)));
        let src = self.tree.source(id);
        let note = &self.tree.node(src).note;
        for l in note.lines() {
            out.push_str(&format!("{pad}  > {l}\n"));
        }
        let kids = self.tree.children(src);
        if kids.is_empty() {
            return;
        }
        if depth + 1 >= max {
            out.push_str(&format!("{pad}  - … {} more (read id:{id} to expand)\n", self.tree.descendants(src).len()));
            return;
        }
        for &k in kids {
            self.write_tree(k, depth + 1, max, out);
        }
    }

    fn agent_read(&mut self, p: &Value) -> Result<String, String> {
        let id = if p.get("id").is_some() { self.agent_node(p, "id")? } else { ROOT };
        let depth = n(p, "depth").unwrap_or(3).clamp(1, 20) as usize;
        let mut out = String::new();
        if id == ROOT {
            out.push_str("# Home  [id:home]\n");
            for &k in self.tree.children(ROOT) {
                self.write_tree(k, 0, depth, &mut out);
            }
        } else {
            out.push_str(&format!("Path: {}\n", self.path_of(id)));
            let src = self.tree.source(id);
            out.push_str(&format!("# {}\n", self.line_for(id)));
            for l in self.tree.node(src).note.lines() {
                out.push_str(&format!("> {l}\n"));
            }
            for &k in self.tree.children(src) {
                self.write_tree(k, 0, depth, &mut out);
            }
            let links = self.tree.backlinks(src);
            if !links.is_empty() {
                out.push_str(&format!("\nLinked from: {}\n", links.iter().map(|l| format!("id:{l}")).collect::<Vec<_>>().join(", ")));
            }
        }
        if self.tree.children(self.tree.source(id)).is_empty() && id == ROOT {
            out.push_str("(empty)\n");
        }
        Ok(out)
    }

    fn agent_search(&mut self, p: &Value) -> Result<String, String> {
        let q = s(p, "query").unwrap_or("").to_string();
        let limit = n(p, "limit").unwrap_or(50).clamp(1, 500) as usize;
        let query = Query::parse(&q);
        if query.is_empty() {
            return Err("empty query".into());
        }
        let mut hits: Vec<NodeId> = self.tree.nodes().filter(|n| n.mirror == ROOT && query.matches(n)).map(|n| n.id).collect();
        hits.sort_by_key(|id| std::cmp::Reverse(self.tree.node(*id).modified));
        let total = hits.len();
        let mut out = format!("{total} match{} for {q:?}{}\n", if total == 1 { "" } else { "es" }, if total > limit { format!(" (showing {limit})") } else { String::new() });
        for id in hits.into_iter().take(limit) {
            out.push_str(&format!("- {}  — in {}\n", self.line_for(id), self.path_of(id)));
        }
        Ok(out)
    }

    fn agent_starred(&self) -> String {
        let mut out = String::from("Starred pages:\n");
        let live: Vec<NodeId> = self.starred.iter().copied().filter(|id| self.tree.contains(*id)).collect();
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

    fn agent_open(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let id = self.agent_node(p, "id")?;
        self.zoom_to(id, cx);
        Ok(format!("Showing {} in Flowie", if id == ROOT { "Home".into() } else { format!("id:{id}") }))
    }

    /// Finish an agent edit: one undo step, persist, and flash the touched rows.
    fn agent_commit(&mut self, touched: &[NodeId], cx: &mut Context<Self>) {
        for &id in touched {
            self.anims.start(id, Channel::Flash, anim::FLASH);
        }
        // Don't leave the caret on a node that no longer exists.
        if let Some(id) = self.editing_node() {
            if !self.tree.contains(id) {
                self.edit = None;
            }
        }
        self.commit_structure(None, cx);
        self.history.seal();
    }

    fn agent_create(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let markdown = s(p, "markdown").ok_or("missing `markdown`")?;
        let items = io::parse_any(markdown);
        if items.is_empty() {
            return Err("`markdown` contained no bullets".into());
        }
        let (parent, index) = if p.get("after").is_some() {
            let after = self.agent_node(p, "after")?;
            (self.tree.parent(after), self.tree.index_in_parent(after) + 1)
        } else {
            let parent = if p.get("parent").is_some() { self.agent_node(p, "parent")? } else { ROOT };
            let parent = self.tree.source(parent);
            let len = self.tree.children(parent).len();
            let index = n(p, "index").map(|i| (i as usize).min(len)).unwrap_or(len);
            (parent, index)
        };
        // Drop a lone empty placeholder bullet (e.g. a fresh journal day).
        let kids = self.tree.children(parent).to_vec();
        let placeholder = (kids.len() == 1 && self.tree.node(kids[0]).text.is_empty() && !self.tree.has_children(kids[0])).then(|| kids[0]);
        let top = self.tree.insert_items(parent, index, &items);
        if let Some(ph) = placeholder {
            if self.editing_node() != Some(ph) {
                self.tree.delete(ph);
            }
        }
        if parent != ROOT && self.tree.node(parent).collapsed {
            self.tree.set_collapsed(parent, false);
        }
        let mut all = Vec::new();
        for &t in &top {
            all.push(t);
            all.extend(self.tree.descendants(t));
        }
        self.agent_commit(&all, cx);
        let mut out = format!("Created {} bullet{} under {}:\n", all.len(), if all.len() == 1 { "" } else { "s" }, if parent == ROOT { "Home".to_string() } else { format!("id:{parent}") });
        for &t in &top {
            self.write_tree(t, 0, 20, &mut out);
        }
        Ok(out)
    }

    fn agent_update(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let id = self.agent_node(p, "id")?;
        if id == ROOT {
            return Err("Home can't be edited".into());
        }
        // Validate everything before changing anything.
        let kind = match s(p, "kind") {
            Some(k) => Some(BlockKind::from_name(k).ok_or_else(|| format!("unknown kind {k:?}; use bullet, paragraph, h1, h2, h3, todo, numbered, quote or code"))?),
            None => None,
        };
        let src = self.tree.source(id);
        if let Some(t) = s(p, "text") {
            self.tree.set_text(src, &t.replace('\n', " "));
        }
        if let Some(note) = s(p, "note") {
            self.tree.set_note(src, note);
        }
        if let Some(done) = p.get("completed").and_then(Value::as_bool) {
            self.tree.set_completed(src, done);
        }
        if let Some(kind) = kind {
            self.tree.set_kind(id, kind);
        }
        if let Some(c) = p.get("collapsed").and_then(Value::as_bool) {
            self.tree.set_collapsed(id, c);
        }
        if self.tree.journal_is_empty() {
            return Ok(format!("Nothing changed: {}", self.line_for(id)));
        }
        self.agent_commit(&[id], cx);
        Ok(format!("Updated: {}", self.line_for(id)))
    }

    fn agent_complete(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let id = self.agent_node(p, "id")?;
        let done = p.get("completed").and_then(Value::as_bool).unwrap_or(true);
        self.tree.set_completed(self.tree.source(id), done);
        self.anims.start(id, Channel::Complete, anim::FLAG);
        self.agent_commit(&[id], cx);
        Ok(format!("{}: {}", if done { "Completed" } else { "Reopened" }, self.line_for(id)))
    }

    fn agent_move(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let id = self.agent_node(p, "id")?;
        let (parent, index) = if p.get("after").is_some() {
            let after = self.agent_node(p, "after")?;
            (self.tree.parent(after), self.tree.index_in_parent(after) + 1)
        } else {
            let parent = self.agent_node(p, "parent")?;
            let parent = self.tree.source(parent);
            let len = self.tree.children(parent).len();
            (parent, n(p, "index").map(|i| (i as usize).min(len)).unwrap_or(len))
        };
        if !self.tree.move_to(id, parent, index) {
            return Err("can't move a bullet into itself".into());
        }
        self.tree.set_collapsed(parent, false);
        self.agent_commit(&[id], cx);
        Ok(format!("Moved {} to {}", self.line_for(id), self.path_of(id)))
    }

    fn agent_delete(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let ids: Vec<Value> = match p.get("ids").or_else(|| p.get("id")) {
            Some(Value::Array(a)) => a.clone(),
            Some(v) => vec![v.clone()],
            None => return Err("missing `ids`".into()),
        };
        let mut done = Vec::new();
        for v in ids {
            let id = self.agent_node(&json!({ "id": v }), "id")?;
            if id == ROOT || !self.tree.contains(id) {
                continue;
            }
            let label = self.line_for(id);
            self.send_to_trash(id);
            self.tree.delete(id);
            done.push(label);
        }
        self.agent_commit(&[], cx);
        Ok(format!("Moved {} bullet(s) to Trash (restorable from File → Trash):\n- {}", done.len(), done.join("\n- ")))
    }

    fn agent_journal(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        let date = s(p, "date").map(String::from).unwrap_or_else(today);
        if markup::date_len(&date) != Some(10) {
            return Err("`date` must be YYYY-MM-DD".into());
        }
        let day = self.ensure_day(&date, false);
        let mut params = p.clone();
        params["parent"] = json!(day);
        params.as_object_mut().map(|o| o.remove("after"));
        self.agent_create(&params, cx)
    }
}
