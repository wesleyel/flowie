//! `flowie-server`: Flowie's self-hosted sync backend.
//!
//! ```text
//! flowie-server serve [--db PATH] [--listen ADDR] [--blobs DIR]
//! flowie-server token add NAME        issue a device token (shown once)
//! flowie-server token list
//! flowie-server token revoke NAME
//! ```
//!
//! It speaks plain HTTP: put it behind a reverse proxy that terminates TLS.

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  flowie-server serve [--db PATH] [--listen ADDR] [--blobs DIR]
  flowie-server token add NAME | list | revoke NAME   [--db PATH]

defaults: --db flowie-server.sqlite  --listen 127.0.0.1:8787  --blobs blobs";

struct Options {
    db: PathBuf,
    listen: String,
    blobs: PathBuf,
    rest: Vec<String>,
}

fn parse(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options =
        Options { db: "flowie-server.sqlite".into(), listen: "127.0.0.1:8787".into(), blobs: "blobs".into(), rest: Vec::new() };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--db" => options.db = value("--db")?.into(),
            "--listen" => options.listen = value("--listen")?,
            "--blobs" => options.blobs = value("--blobs")?.into(),
            "-h" | "--help" => return Err(String::new()),
            _ => options.rest.push(arg),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    let options = match parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("flowie-server: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: Options) -> Result<(), String> {
    let conn = flowie_server::open(&options.db).map_err(|e| format!("can't open {}: {e}", options.db.display()))?;
    let rest: Vec<&str> = options.rest.iter().map(String::as_str).collect();
    match rest.as_slice() {
        ["serve"] => {
            let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            runtime.block_on(async {
                let listener =
                    tokio::net::TcpListener::bind(&options.listen).await.map_err(|e| format!("can't listen on {}: {e}", options.listen))?;
                if flowie_server::token_names(&conn).map_err(|e| e.to_string())?.is_empty() {
                    eprintln!("flowie-server: no device tokens yet; issue one with `flowie-server token add NAME`");
                }
                eprintln!("flowie-server: listening on {}", options.listen);
                let state = flowie_server::AppState::new(conn, options.blobs);
                flowie_server::serve(listener, state).await.map_err(|e| e.to_string())
            })
        }
        ["token", "add", name] => {
            let token = flowie_server::add_token(&conn, name).map_err(|e| format!("can't add {name}: {e}"))?;
            println!("{token}");
            eprintln!("Give this token to the device {name:?}: it is shown only now.");
            Ok(())
        }
        ["token", "list"] => {
            for name in flowie_server::token_names(&conn).map_err(|e| e.to_string())? {
                println!("{name}");
            }
            Ok(())
        }
        ["token", "revoke", name] => match flowie_server::revoke_token(&conn, name).map_err(|e| e.to_string())? {
            true => Ok(()),
            false => Err(format!("no device called {name:?}")),
        },
        _ => Err(format!("unknown command\n{USAGE}")),
    }
}
