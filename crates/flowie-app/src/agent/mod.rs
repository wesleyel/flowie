//! Agent interface. The running app listens on a local IPC socket (Unix
//! domain socket on Unix, named pipe on Windows) for newline-delimited JSON
//! requests `{"id", "method", "params"}` and answers `{"id", "result"}` or
//! `{"id", "error"}`. Requests run on the UI thread through the same code
//! paths as keyboard edits, so changes show up live, persist normally, and
//! each request is a single undo step.
//! `ipc.rs` owns the socket, which also keeps Flowie to one instance per
//! database; `flowie mcp` (see `mcp.rs`) bridges it to MCP over stdio.

mod ipc;
pub mod mcp;
mod tools;

pub use ipc::{claim, Claim, Listener};

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use futures::StreamExt;
use gpui::{App, Context, WindowHandle};
use interprocess::local_socket::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::outline::Outline;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMode {
    Off,
    #[default]
    ReadOnly,
    ReadWrite,
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

/// The app's own requests, which work whatever the agent mode: a second
/// launch sends `app.activate` to bring this window forward.
const APP_ACTIVATE: &str = "app.activate";
/// `app.open {path}`: a second launch hands over a share package to open.
const APP_OPEN: &str = "app.open";

/// Hand over to the instance that already has the database open: bring its
/// window forward unless this launch was a background one.
pub fn hand_off(running: LocalSocketStream, background: bool, open: Option<&std::path::Path>) {
    let result = match open {
        Some(path) => ipc::call(&running, APP_OPEN, json!({ "path": path })),
        None if background => return,
        None => ipc::call(&running, APP_ACTIVATE, json!({})),
    };
    if let Err(e) = result {
        eprintln!("flowie: the running Flowie did not come forward: {e}");
    }
}

/// Answer requests on `listener` for the outline in `window`.
pub fn start(listener: Listener, window: WindowHandle<Outline>, cx: &mut App) {
    #[cfg(unix)]
    let path = listener.path.clone();
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<Job>();
    std::thread::Builder::new()
        .name("flowie-agent".into())
        .spawn(move || {
            // The listener (and on Unix its lock) lives as long as this thread.
            let listener = listener;
            for stream in listener.inner.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || serve(stream, tx));
            }
        })
        .ok();
    cx.spawn(async move |cx| {
        while let Some((req, reply)) = rx.next().await {
            let result = window
                .update(cx, |o, window, cx| {
                    if req.method == APP_ACTIVATE || req.method == APP_OPEN {
                        cx.activate(true);
                        window.activate_window();
                        // Only opens the import dialog: nothing changes until the user says so.
                        if let Some(path) =
                            req.params.get("path").and_then(Value::as_str).map(std::path::Path::new).filter(|p| crate::share::is_package(p))
                        {
                            o.open_package_file(path, cx);
                        }
                        Ok("ok".to_string())
                    } else {
                        o.handle_agent(&req.method, &req.params, cx)
                    }
                })
                .unwrap_or_else(|_| Err("Flowie is shutting down".into()));
            let _ = reply.send(result);
        }
    })
    .detach();
    // The process exits without unwinding the listener thread, so remove the
    // socket file here. The lock file stays: removing it would race a new launch.
    #[cfg(unix)]
    cx.on_app_quit(move |_| {
        let _ = std::fs::remove_file(&path);
        async {}
    })
    .detach();
}

fn serve(stream: LocalSocketStream, tx: futures::channel::mpsc::UnboundedSender<Job>) {
    let reader = BufReader::new(&stream);
    let mut out = &stream;
    for line in reader.lines() {
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
        if writeln!(out, "{resp}").is_err() || out.flush().is_err() {
            break;
        }
    }
}

const WRITES: [&str; 8] = ["create", "update", "move", "delete", "journal", "complete", "attach_image", "batch"];

impl Outline {
    pub fn handle_agent(&mut self, method: &str, p: &Value, cx: &mut Context<Self>) -> Result<String, String> {
        #[cfg(feature = "screenshots")]
        if let Some(m) = method.strip_prefix("shot.") {
            let result = self.handle_shot(m, p, cx);
            cx.notify();
            return result;
        }
        match self.config.agent {
            AgentMode::Off => return Err("Agent access is turned off in Flowie → Settings → Agent.".into()),
            AgentMode::ReadOnly if WRITES.contains(&method) => {
                return Err("Flowie is in read-only mode for agents. The user can allow edits in Flowie → Settings → Agent.".into())
            }
            _ => {}
        }
        let result = match method {
            "current" => Ok(self.agent_current()),
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
            "attach_image" => self.agent_attach_image(p, cx),
            "batch" => self.agent_batch(p, cx),
            _ => Err(format!("unknown method {method}")),
        };
        cx.notify();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_claim_finds_the_owner_and_talks_to_it() {
        let db = std::env::temp_dir().join(format!("flowie-ipc-test-{}.sqlite", std::process::id()));
        let Claim::Owner(listener) = claim(&db) else { panic!("first claim should own the socket") };
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Job>();
        let server = std::thread::spawn(move || {
            let stream = listener.inner.incoming().next().expect("incoming").expect("accept");
            serve(stream, tx);
            listener
        });
        let handler = std::thread::spawn(move || {
            let (req, reply) = futures::executor::block_on(rx.next()).expect("job");
            assert_eq!(req.method, APP_ACTIVATE);
            let _ = reply.send(Ok("ok".into()));
        });

        let Claim::Running(stream) = claim(&db) else { panic!("second claim should find the owner") };
        assert_eq!(ipc::call(&stream, APP_ACTIVATE, json!({})), Ok("ok".into()));
        drop(stream);

        handler.join().unwrap();
        let listener = server.join().unwrap();
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&listener.path);
            let _ = std::fs::remove_file(listener.path.with_extension("lock"));
        }
        drop(listener);
    }
}
