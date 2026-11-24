//! `flowie mcp`: an MCP server over stdio (newline-delimited JSON-RPC 2.0)
//! that forwards tool calls to the running Flowie app's agent socket,
//! starting Flowie in the background if needed.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use interprocess::local_socket::prelude::*;
use serde_json::{json, Value};

const PROTOCOL: &str = "2025-06-18";

fn tools() -> Value {
    let id = json!({"type": "string", "description": "Bullet id as shown by read/search (any unique prefix of 4+ hex digits works), or \"home\" / \"today\" / a YYYY-MM-DD journal day"});
    json!([
        {
            "name": "search",
            "description": "Search all bullets. Words must all match; also supports \"phrases\", -exclude, #tag, @person, is:done, is:open, is:todo, is:heading, has:note, changed:7d, date:2026-11, before:YYYY-MM-DD, after:today. Returns each hit with its id and path.",
            "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "default": 50}}, "required": ["query"]},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "read",
            "description": "Read an outline as Markdown with [id:…] after every bullet (a short id; any unique prefix of 4+ hex digits works as input). Without `id`, reads Home. Deeper levels are summarized; read that id to expand.",
            "inputSchema": {"type": "object", "properties": {"id": id, "depth": {"type": "integer", "default": 3, "minimum": 1, "maximum": 20}}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "starred",
            "description": "List the user's starred pages and saved searches.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "create",
            "description": "Insert bullets written as a Markdown list (nesting by 2-space indent; supports '- [ ] todo', '- # heading', '- > quote', notes as indented non-list lines, and Flowie markup like **bold**, #tags, 2026-11-20 dates, $math$). Place them under `parent` (default Home) at `index` (default end), or right after the bullet `after`. One undo step in Flowie.",
            "inputSchema": {"type": "object", "properties": {"markdown": {"type": "string"}, "parent": id, "index": {"type": "integer"}, "after": id}, "required": ["markdown"]}
        },
        {
            "name": "update",
            "description": "Change a bullet's text, note, completed state, kind (bullet, paragraph, h1, h2, h3, todo, numbered, quote, code) or collapsed state. Only given fields change.",
            "inputSchema": {"type": "object", "properties": {"id": id, "text": {"type": "string"}, "note": {"type": "string"}, "completed": {"type": "boolean"}, "kind": {"type": "string"}, "collapsed": {"type": "boolean"}}, "required": ["id"]}
        },
        {
            "name": "complete",
            "description": "Mark a bullet completed (or reopen it with completed=false).",
            "inputSchema": {"type": "object", "properties": {"id": id, "completed": {"type": "boolean", "default": true}}, "required": ["id"]}
        },
        {
            "name": "move",
            "description": "Move a bullet (with its children) under `parent` at `index` (default end), or right after `after`.",
            "inputSchema": {"type": "object", "properties": {"id": id, "parent": id, "index": {"type": "integer"}, "after": id}, "required": ["id"]}
        },
        {
            "name": "delete",
            "description": "Delete bullets and their children. They go to Flowie's Trash and can be restored for 30 days.",
            "inputSchema": {"type": "object", "properties": {"ids": {"type": "array", "items": id}}, "required": ["ids"]},
            "annotations": {"destructiveHint": true}
        },
        {
            "name": "attach_image",
            "description": "Add an image bullet: PNG, JPEG, GIF or WebP bytes as base64 in `data`, with an optional caption, under `parent` (default Home) at `index` (default end), or right after `after`.",
            "inputSchema": {"type": "object", "properties": {"data": {"type": "string", "description": "base64 image bytes"}, "caption": {"type": "string"}, "parent": id, "index": {"type": "integer"}, "after": id}, "required": ["data"]}
        },
        {
            "name": "journal",
            "description": "Append bullets (Markdown list) to the journal page of `date` (default today), creating the page if needed.",
            "inputSchema": {"type": "object", "properties": {"markdown": {"type": "string"}, "date": {"type": "string", "description": "YYYY-MM-DD"}}, "required": ["markdown"]}
        },
        {
            "name": "open",
            "description": "Show a bullet (zoomed in) in the Flowie window, e.g. after making changes the user should look at.",
            "inputSchema": {"type": "object", "properties": {"id": id}, "required": ["id"]}
        }
    ])
}

/// How an agent starts Flowie's MCP server: this executable with `mcp`,
/// plus `FLOWIE_DB` when the app runs on a database other than the default.
pub struct Setup {
    exe: String,
    db: Option<String>,
}

impl Setup {
    pub fn new(db: Option<&Path>) -> Self {
        // Screenshots show where the app is installed, not the build directory.
        let exe = if cfg!(feature = "screenshots") {
            "/Applications/Flowie.app/Contents/MacOS/Flowie".to_string()
        } else {
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "flowie".into())
        };
        let db = db.filter(|p| *p != crate::data_path()).map(|p| p.display().to_string());
        Setup { exe, db }
    }

    /// `claude mcp add …`, quoted for the shell the user pastes it into.
    pub fn claude_command(&self) -> String {
        let quote = crate::platform::shell_quote;
        let env = self.db.as_ref().map(|db| format!("-e {} ", quote(&format!("FLOWIE_DB={db}")))).unwrap_or_default();
        format!("claude mcp add flowie {env}-- {} mcp", quote(&self.exe))
    }

    /// The `mcpServers` entry other MCP clients read.
    pub fn client_config(&self) -> String {
        let mut server = json!({"command": self.exe, "args": ["mcp"]});
        if let Some(db) = &self.db {
            server["env"] = json!({"FLOWIE_DB": db});
        }
        serde_json::to_string_pretty(&json!({"mcpServers": {"flowie": server}})).unwrap_or_default()
    }
}

/// Connect to the app, starting it in the background if it isn't running.
fn connect(db: &Path) -> Result<LocalSocketStream, String> {
    let name = super::ipc::socket_name(db)?;
    if let Ok(s) = LocalSocketStream::connect(name.clone()) {
        return Ok(s);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--background").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map_err(|e| format!("could not start Flowie: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(s) = LocalSocketStream::connect(name.clone()) {
            return Ok(s);
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err("Flowie did not start in time".into())
}

struct Link {
    db: PathBuf,
    conn: Option<BufReader<LocalSocketStream>>,
    next: u64,
}

impl Link {
    fn call(&mut self, method: &str, params: Value) -> Result<String, String> {
        for attempt in 0..2 {
            if self.conn.is_none() {
                let s = connect(&self.db)?;
                self.conn = Some(BufReader::new(s));
            }
            self.next += 1;
            let req = json!({"id": self.next, "method": method, "params": params});
            let r = self.conn.as_mut().unwrap();
            let mut w = r.get_ref();
            let mut line = String::new();
            let ok = writeln!(w, "{req}").is_ok() && w.flush().is_ok() && r.read_line(&mut line).map(|n| n > 0).unwrap_or(false);
            if !ok {
                // The app may have restarted; reconnect once.
                self.conn = None;
                if attempt == 0 {
                    continue;
                }
                return Err("lost connection to Flowie".into());
            }
            let resp: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            return match (resp.get("result"), resp.get("error")) {
                (Some(Value::String(t)), _) => Ok(t.clone()),
                (_, Some(Value::String(e))) => Err(e.clone()),
                _ => Err("malformed reply from Flowie".into()),
            };
        }
        Err("lost connection to Flowie".into())
    }
}

pub fn run(db: &Path) {
    let mut link = Link { db: db.to_path_buf(), conn: None, next: 0 };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = writeln!(stdout, "{}", json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}));
            continue;
        };
        let Some(id) = msg.get("id").cloned() else { continue }; // notification
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => {
                let version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL).to_string();
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "flowie", "title": "Flowie outliner", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "Flowie is the user's outliner. Use `read` (no id = Home) and `search` to find bullets; every bullet is shown with a short id like [id:3f9a1c], which any tool takes as `id`; link to a bullet with [label](flowie:3f9a1c). Write with `create` (Markdown lists), `update`, `move`, `complete`, `delete` (to Trash) and `journal`. Each call is one undo step for the user. Writing needs the user to allow it in Flowie → Settings → Agent."
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                // `app.*` requests are the app's own (a second launch), not tools.
                let called = if name.starts_with("app.") { Err(format!("unknown tool {name}")) } else { link.call(&name, args) };
                match called {
                    Ok(text) => Ok(json!({"content": [{"type": "text", "text": text}], "isError": false})),
                    Err(e) => Ok(json!({"content": [{"type": "text", "text": e}], "isError": true})),
                }
            }
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        let resp = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        };
        if writeln!(stdout, "{resp}").is_err() {
            break;
        }
        let _ = stdout.flush();
    }
}
