//! The self-hosted sync backend (docs/roadmap/per-node-sync.md §5.1): it stores records and
//! never reads them, like CloudKit does, so every client merges the same way and the records
//! can be encrypted end to end later without changing it.
//!
//! - Records live in zones (one outline each). A save is conditional on the version it was
//!   based on; a stale one gets the stored record back as a conflict.
//! - Pulling reads records saved after a cursor, oldest first, and can wait for new ones.
//! - Image files are stored by their sha256 name, checked on upload.
//! - Devices authenticate with tokens an administrator issues (`flowie-server token add`).
//!   Only their hashes are kept.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use flowie_sync::wire::{ModifyRequest, ModifyResponse};
use flowie_sync::{ChangeBatch, Envelope, SaveResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS records (
    zone     TEXT    NOT NULL,
    id       TEXT    NOT NULL,
    seq      INTEGER NOT NULL,   -- when it was last saved, across all zones; also its etag
    envelope TEXT    NOT NULL,   -- flowie_sync::Envelope as JSON
    PRIMARY KEY (zone, id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS records_seq ON records(zone, seq);
CREATE TABLE IF NOT EXISTS tokens (
    hash    TEXT PRIMARY KEY,    -- sha256 of the token, hex
    name    TEXT NOT NULL UNIQUE,
    created INTEGER NOT NULL
);
";

/// The longest a pull may wait for something new.
const MAX_WAIT: u64 = 60;
/// The largest image file accepted.
const MAX_BLOB: usize = 64 * 1024 * 1024;

/// Open (or create) the server's database.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

fn hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Issue a token for a device called `name`. Returns the token: shown once, never stored.
pub fn add_token(conn: &Connection, name: &str) -> rusqlite::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom(&mut bytes);
    let token = hex(&bytes);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    conn.execute("INSERT INTO tokens (hash, name, created) VALUES (?1, ?2, ?3)", params![hash(&token), name, now])?;
    Ok(token)
}

/// Revoke a device's token. False if there was none by that name.
pub fn revoke_token(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM tokens WHERE name = ?1", params![name])? > 0)
}

pub fn token_names(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM tokens ORDER BY name")?;
    let names = stmt.query_map([], |r| r.get(0))?.collect();
    names
}

/// Random bytes from the operating system (two v4 uuids' worth of random bits per 32 bytes).
fn getrandom(out: &mut [u8]) {
    for chunk in out.chunks_mut(16) {
        let random = uuid::Uuid::new_v4().into_bytes();
        chunk.copy_from_slice(&random[..chunk.len()]);
    }
}

/// What every request shares.
#[derive(Clone)]
pub struct AppState {
    db: Arc<Mutex<Connection>>,
    blobs: PathBuf,
    /// Woken on every save, for pulls that wait.
    saved: Arc<Notify>,
}

impl AppState {
    pub fn new(db: Connection, blobs: PathBuf) -> Self {
        AppState { db: Arc::new(Mutex::new(db)), blobs, saved: Arc::new(Notify::new()) }
    }
}

/// The HTTP API, and with `web`, that folder's files (the browser version) at the other paths.
pub fn router_with_web(state: AppState, web: Option<PathBuf>) -> Router {
    let router = router(state);
    match web {
        Some(dir) => router.fallback_service(tower_http::services::ServeDir::new(dir)),
        None => router,
    }
}

/// The HTTP API.
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/v1/zones/{zone}/modify", post(modify))
        .route("/v1/zones/{zone}/changes", get(changes))
        .route("/v1/blobs/{name}", put(put_blob).get(get_blob).head(head_blob))
        .layer(DefaultBodyLimit::max(MAX_BLOB))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    Router::new().route("/v1/health", get(|| async { "ok" })).merge(api).with_state(state)
}

/// Serve until the process is stopped.
pub async fn serve(listener: tokio::net::TcpListener, state: AppState) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

/// Serve the API and the browser version's files in `web`.
pub async fn serve_with_web(listener: tokio::net::TcpListener, state: AppState, web: Option<PathBuf>) -> std::io::Result<()> {
    axum::serve(listener, router_with_web(state, web)).await
}

/// Run `f` on the database off the async workers: SQLite blocks, and a slow write must not
/// hold up the long polls and uploads sharing those threads.
async fn with_db<T: Send + 'static>(
    state: &AppState,
    f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
) -> Result<T, String> {
    let db = state.db.clone();
    let done = tokio::task::spawn_blocking(move || f(&mut db.lock().unwrap_or_else(|e| e.into_inner()))).await;
    match done {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, message.into()).into_response()
}

async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let token = request.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    let Some(token) = token else { return error(StatusCode::UNAUTHORIZED, "a device token is needed") };
    let hashed = hash(token);
    let known =
        with_db(&state, move |db| db.query_row("SELECT 1 FROM tokens WHERE hash = ?1", params![hashed], |_| Ok(())).optional()).await;
    match known {
        Ok(Some(())) => next.run(request).await,
        Ok(None) => error(StatusCode::UNAUTHORIZED, "unknown device token"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

fn valid_zone(zone: &str) -> bool {
    (1..=64).contains(&zone.len()) && zone.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

async fn modify(State(state): State<AppState>, UrlPath(zone): UrlPath<String>, Json(request): Json<ModifyRequest>) -> Response {
    if !valid_zone(&zone) {
        return error(StatusCode::BAD_REQUEST, "bad zone name");
    }
    let results = match with_db(&state, move |db| save(db, &zone, request.saves)).await {
        Ok(results) => results,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    state.saved.notify_waiters();
    Json(ModifyResponse { results }).into_response()
}

/// Save records in one transaction, each only if it starts from the stored version.
fn save(db: &mut Connection, zone: &str, saves: Vec<Envelope>) -> rusqlite::Result<Vec<SaveResult>> {
    let tx = db.transaction()?;
    let mut seq: i64 = tx.query_row("SELECT ifnull(MAX(seq), 0) FROM records", [], |r| r.get(0))?;
    let mut results = Vec::with_capacity(saves.len());
    for mut save in saves {
        let stored: Option<(i64, String)> = tx
            .query_row("SELECT seq, envelope FROM records WHERE zone = ?1 AND id = ?2", params![zone, save.id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if let Some((current, envelope)) = &stored {
            if save.etag.as_deref() != Some(current.to_string().as_str()) {
                if let Ok(server) = serde_json::from_str(envelope) {
                    results.push(SaveResult::Conflict { server });
                    continue;
                }
            }
        }
        seq += 1;
        save.etag = Some(seq.to_string());
        let json = serde_json::to_string(&save).expect("an envelope is plain data");
        tx.execute(
            "INSERT INTO records (zone, id, seq, envelope) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(zone, id) DO UPDATE SET seq = ?3, envelope = ?4",
            params![zone, save.id, seq, json],
        )?;
        results.push(SaveResult::Saved { id: save.id, etag: seq.to_string() });
    }
    tx.commit()?;
    Ok(results)
}

#[derive(Deserialize)]
struct ChangesQuery {
    since: Option<i64>,
    limit: Option<usize>,
    wait: Option<u64>,
}

async fn changes(State(state): State<AppState>, UrlPath(zone): UrlPath<String>, Query(q): Query<ChangesQuery>) -> Response {
    if !valid_zone(&zone) {
        return error(StatusCode::BAD_REQUEST, "bad zone name");
    }
    let since = q.since.unwrap_or(0);
    let limit = q.limit.unwrap_or(500).clamp(1, 5000);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(q.wait.unwrap_or(0).min(MAX_WAIT));
    loop {
        // Listen before looking, so a save between the two isn't missed.
        let saved = state.saved.notified();
        tokio::pin!(saved);
        saved.as_mut().enable();
        let zone = zone.clone();
        match with_db(&state, move |db| read_changes(db, &zone, since, limit)).await {
            Ok(batch) if !batch.records.is_empty() || tokio::time::Instant::now() >= deadline => return Json(batch).into_response(),
            Ok(_) => {
                let _ = tokio::time::timeout_at(deadline, saved).await;
            }
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
        }
    }
}

fn read_changes(db: &Connection, zone: &str, since: i64, limit: usize) -> rusqlite::Result<ChangeBatch> {
    let mut stmt = db.prepare("SELECT seq, envelope FROM records WHERE zone = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3")?;
    let rows: Vec<(i64, String)> =
        stmt.query_map(params![zone, since, limit as i64 + 1], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let more = rows.len() > limit;
    let rows = &rows[..rows.len().min(limit)];
    let cursor = rows.last().map_or(since, |(seq, _)| *seq).to_string();
    let records = rows.iter().filter_map(|(_, json)| serde_json::from_str(json).ok()).collect();
    Ok(ChangeBatch { records, cursor, more })
}

/// An image name: `<sha256 hex>.<ext>`.
fn blob_path(state: &AppState, name: &str) -> Option<(PathBuf, String)> {
    let (digest, ext) = name.split_once('.')?;
    let ok = digest.len() == 64
        && digest.bytes().all(|b| b.is_ascii_hexdigit())
        && (1..=8).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric());
    ok.then(|| (state.blobs.join(name), digest.to_ascii_lowercase()))
}

async fn put_blob(State(state): State<AppState>, UrlPath(name): UrlPath<String>, body: Bytes) -> Response {
    let Some((path, digest)) = blob_path(&state, &name) else { return error(StatusCode::BAD_REQUEST, "bad image name") };
    if hex(&Sha256::digest(&body)) != digest {
        return error(StatusCode::BAD_REQUEST, "the file doesn't match its name");
    }
    if path.exists() {
        return StatusCode::NO_CONTENT.into_response();
    }
    // A name of its own, so uploads of the same file at once don't write over each other.
    let tmp = state.blobs.join(format!("{name}.{}.upload", uuid::Uuid::new_v4().simple()));
    let written =
        tokio::fs::create_dir_all(&state.blobs).await.and(tokio::fs::write(&tmp, &body).await).and(tokio::fs::rename(&tmp, &path).await);
    if written.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    match written {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn get_blob(State(state): State<AppState>, UrlPath(name): UrlPath<String>) -> Response {
    let Some((path, _)) = blob_path(&state, &name) else { return error(StatusCode::BAD_REQUEST, "bad image name") };
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn head_blob(State(state): State<AppState>, UrlPath(name): UrlPath<String>) -> Response {
    let Some((path, _)) = blob_path(&state, &name) else { return error(StatusCode::BAD_REQUEST, "bad image name") };
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        StatusCode::OK.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}
