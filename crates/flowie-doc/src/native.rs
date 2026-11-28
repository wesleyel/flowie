//! Sync driven by a platform's own sync engine, CloudKit's CKSyncEngine on Apple platforms
//! (wiki: Per-Node-Sync §5.2): that engine decides when to send and fetch and calls in
//! here, and this answers with records as the platform's adapter carries them, an envelope
//! (JSON) and an opaque etag. The iOS app (through flowie-ffi) and the desktop (through
//! flowie-cloudkit) share it.
//!
//! Image files go the same way (wiki: Per-Node-Sync §3.5): a bullet's file is handed over as a
//! wire of its own (CloudKit: a `Blob` record with the file as a CKAsset) just before the
//! first save of the bullet, and a file that comes in is kept and noted in `blobs`, so the
//! row that shows it lays out at its size.

use std::collections::{BTreeSet, HashMap, HashSet};

use flowie_core::images;
use flowie_sync::{blob_name, ChangeBatch, Engine, Envelope, RecordStore, SaveResult};

use crate::Workspace;

/// A record as a native engine carries it: its name, the version it was last seen at (for
/// CloudKit, the record's archived system fields), and the envelope as JSON, without its etag.
/// An image file instead has `file` and no envelope; its name is [`blob_name`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Wire {
    pub id: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub envelope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<WireFile>,
}

/// An image file as a native engine carries it (CloudKit: a `Blob` record, its `file` a
/// CKAsset): where the file is on this device (to send: in the image folder; fetched: the
/// engine's download, readable while it hands it over), and what the outline knows about it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WireFile {
    pub path: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub mime: String,
}

impl Wire {
    pub fn from_envelope(mut envelope: Envelope) -> Wire {
        let etag = envelope.etag.take();
        Wire { id: envelope.id.clone(), etag, envelope: serde_json::to_string(&envelope).expect("an envelope is plain data"), file: None }
    }

    /// The envelope, with the etag put back; none for one that doesn't parse, or an image file.
    pub fn to_envelope(&self) -> Option<Envelope> {
        if self.file.is_some() {
            return None;
        }
        let mut envelope: Envelope = serde_json::from_str(&self.envelope).ok()?;
        envelope.etag = self.etag.clone();
        Some(envelope)
    }
}

/// How the native engine answered one save.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum NativeSaved {
    Saved {
        id: String,
        etag: String,
    },
    /// The backend has a newer version (CloudKit: serverRecordChanged): here it is.
    Conflict {
        server: Wire,
    },
    /// Not saved for another reason; it is sent again later.
    Failed {
        id: String,
        error: String,
    },
}

/// What [`NativeSync::on_fetched`] did with a batch of fetched records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fetched {
    /// The outline changed, or an image file came that a row may be waiting for.
    pub changed: bool,
    /// What became of each image file among them, by its record's name.
    pub files: Vec<(String, FileTaken)>,
}

/// What became of a fetched image file, for the platform's sync log: a file that isn't kept
/// is otherwise dropped without a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTaken {
    /// Kept: this device didn't have it.
    Kept,
    /// This device had it already.
    Had,
    /// The outline has no image folder (no database).
    NoImageFolder,
    /// The engine's download couldn't be read.
    Unreadable(String),
    /// Larger than `images::MAX_BYTES` (its size).
    TooLarge(u64),
    /// Not a PNG, JPEG, GIF or WebP image (its size).
    NotAnImage(u64),
    /// Its contents are another file than its record names (the name they have).
    OtherName(String),
    /// Couldn't be written to the image folder.
    NotStored(String),
}

/// FlowieCloudKit's log takes `kept` and `had it already` as kept, anything else as dropped.
impl std::fmt::Display for FileTaken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileTaken::Kept => write!(f, "kept"),
            FileTaken::Had => write!(f, "had it already"),
            FileTaken::NoImageFolder => write!(f, "dropped: no image folder"),
            FileTaken::Unreadable(error) => write!(f, "dropped: can't read the download: {error}"),
            FileTaken::TooLarge(size) => write!(f, "dropped: {size} bytes, larger than {} MB", images::MAX_BYTES >> 20),
            FileTaken::NotAnImage(size) => write!(f, "dropped: {size} bytes, not a PNG, JPEG, GIF or WebP image"),
            FileTaken::OtherName(name) => write!(f, "dropped: the file is {name}"),
            FileTaken::NotStored(error) => write!(f, "dropped: can't store it: {error}"),
        }
    }
}

/// The sync engine for one outline while a native engine drives it.
#[derive(Debug, Default)]
pub struct NativeSync {
    engine: Engine,
    blobs: Blobs,
}

/// Image files on their way to the backend, by the names it keeps them under.
#[derive(Debug, Default)]
struct Blobs {
    /// On the backend: saved from here, fetched, or found there already.
    there: HashSet<String>,
    /// Sent and not answered yet, with the file's name here.
    in_flight: HashMap<String, String>,
    /// Files (names here) whose save failed: sent again on their own.
    retry: BTreeSet<String>,
}

impl NativeSync {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ids with something to send, at most `limit`: what the native engine should list as
    /// pending saves. The synced preferences are the record `flowie_sync::PREFS_ID`.
    pub fn unsent_ids(ws: &Workspace, limit: usize) -> Vec<String> {
        let mut ids: Vec<String> = ws.unsent(limit).into_iter().map(|id| id.to_string()).collect();
        if ws.prefs_unsent() && ids.len() < limit {
            ids.push(flowie_sync::PREFS_ID.to_string());
        }
        ids
    }

    /// [`unsent_ids`](Self::unsent_ids), and the image files whose save failed, to be sent
    /// again: everything the native engine should list as pending saves.
    pub fn to_send(&self, ws: &Workspace, limit: usize) -> Vec<String> {
        let mut ids = Self::unsent_ids(ws, limit);
        let room = limit.saturating_sub(ids.len());
        ids.extend(self.blobs.retry.iter().map(|name| blob_name(name)).take(room));
        ids
    }

    /// The next records to save, at most `limit`, each from the version last seen. Each stays
    /// in flight until [`on_saved`](Self::on_saved) answers it. A bullet's image file comes
    /// just before the bullet the first time (per-node-sync.md §3.5), unless it is known to be
    /// on the backend; a file missing here is left out, and the bullet still goes.
    pub fn next_saves(&mut self, ws: &mut Workspace, limit: usize) -> Vec<Wire> {
        let mut out: Vec<Wire> = Vec::new();
        for name in std::mem::take(&mut self.blobs.retry) {
            if out.len() < limit {
                out.extend(self.blob_to_send(ws, &name));
            } else {
                self.blobs.retry.insert(name);
            }
        }
        let mut left = Vec::new();
        for envelope in self.engine.next_saves(ws, limit.saturating_sub(out.len())) {
            let file = envelope.image().filter(|_| left.is_empty()).and_then(|name| self.blob_to_send(ws, &name));
            if !left.is_empty() || (!out.is_empty() && out.len() + 1 + usize::from(file.is_some()) > limit) {
                if let Some(file) = file {
                    self.blobs.in_flight.remove(&file.id);
                }
                left.push(SaveResult::Failed { id: envelope.id.clone(), error: "not in this batch".into() });
                continue;
            }
            out.extend(file);
            out.push(Wire::from_envelope(envelope));
        }
        // What didn't fit after the image files comes again next time.
        self.engine.on_saved(ws, left);
        out
    }

    /// The wire for the image file `name` here, unless it is on the backend or on its way.
    fn blob_to_send(&mut self, ws: &Workspace, name: &str) -> Option<Wire> {
        let remote = blob_name(name);
        if self.blobs.there.contains(&remote) || self.blobs.in_flight.contains_key(&remote) {
            return None;
        }
        let path = ws.image_dir.as_ref()?.path(name).filter(|path| path.is_file())?;
        let (width, height, mime) = match ws.blobs.get(name) {
            Some(info) => (info.width, info.height, info.mime.clone()),
            None => (0, 0, images::mime_of(name).to_string()),
        };
        self.blobs.in_flight.insert(remote.clone(), name.to_string());
        let file = WireFile { path: path.to_string_lossy().into_owned(), width, height, mime };
        Some(Wire { id: remote, etag: None, envelope: String::new(), file: Some(file) })
    }

    /// The native engine answered saves.
    pub fn on_saved(&mut self, ws: &mut Workspace, outcomes: Vec<NativeSaved>) {
        let mut results = Vec::new();
        for outcome in outcomes {
            let id = match &outcome {
                NativeSaved::Saved { id, .. } | NativeSaved::Failed { id, .. } => id,
                NativeSaved::Conflict { server } => &server.id,
            };
            if let Some(name) = self.blobs.in_flight.remove(id) {
                match outcome {
                    // A file is named after its contents: one there already is the same file.
                    NativeSaved::Saved { id, .. } | NativeSaved::Conflict { server: Wire { id, .. } } => {
                        self.blobs.there.insert(id);
                    }
                    NativeSaved::Failed { .. } => {
                        self.blobs.retry.insert(name);
                    }
                }
                continue;
            }
            results.extend(match outcome {
                NativeSaved::Saved { id, etag } => Some(SaveResult::Saved { id, etag }),
                NativeSaved::Conflict { server } => server.to_envelope().map(|server| SaveResult::Conflict { server }),
                NativeSaved::Failed { id, error } => Some(SaveResult::Failed { id, error }),
            });
        }
        self.engine.on_saved(ws, results);
    }

    /// Records the native engine fetched. It keeps its own place in the backend's changes, so
    /// `cursor` only marks that this backend has been fetched from (see
    /// [`Workspace::use_remote`]). Image files among them are kept first, so a bullet that
    /// comes with its file shows it at once. Says whether the outline changed, or an image
    /// file came that a row may be waiting for, and what became of each image file.
    pub fn on_fetched(&mut self, ws: &mut Workspace, records: &[Wire], cursor: &str) -> Fetched {
        let revision = ws.tree.revision();
        let mut images = false;
        let mut files = Vec::new();
        for wire in records {
            if let Some(file) = &wire.file {
                let taken = self.take_file(ws, &wire.id, file);
                images |= taken == FileTaken::Kept;
                files.push((wire.id.clone(), taken));
            }
        }
        let records = records.iter().filter_map(Wire::to_envelope).collect();
        self.engine.on_fetched(ws, ChangeBatch { records, cursor: cursor.to_string(), more: false });
        Fetched { changed: images || ws.tree.revision() != revision, files }
    }

    /// Keep a fetched image file, if it is one and matches its name.
    fn take_file(&mut self, ws: &mut Workspace, remote: &str, file: &WireFile) -> FileTaken {
        let Some(dir) = ws.image_dir.clone() else { return FileTaken::NoImageFolder };
        let bytes = match std::fs::metadata(&file.path) {
            Ok(meta) if meta.len() > images::MAX_BYTES as u64 => return FileTaken::TooLarge(meta.len()),
            Ok(_) => std::fs::read(&file.path),
            Err(error) => Err(error),
        };
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(error) => return FileTaken::Unreadable(error.to_string()),
        };
        // Only an image that matches its name is kept.
        let Ok(info) = images::info(&bytes) else { return FileTaken::NotAnImage(bytes.len() as u64) };
        if blob_name(&info.name) != remote {
            return FileTaken::OtherName(info.name);
        }
        let had = dir.has(&info.name) && ws.blobs.contains_key(&info.name);
        if let Err(error) = dir.put(&bytes) {
            return FileTaken::NotStored(error.to_string());
        }
        self.blobs.there.insert(remote.to_string());
        ws.record_blob(info);
        if had {
            FileTaken::Had
        } else {
            FileTaken::Kept
        }
    }

    /// The image files rows here show but this device doesn't have, by their names on the
    /// backend, at most `limit`: to ask the backend for by name. A file normally comes with
    /// the fetch that brings its row, but one the native engine handed over without its
    /// download (or that an older version ignored) never comes again by itself.
    pub fn missing_files(ws: &Workspace, limit: usize) -> Vec<String> {
        let Some(dir) = &ws.image_dir else { return Vec::new() };
        let names: BTreeSet<&str> = ws
            .tree
            .nodes()
            .map(|node| node.image.as_str())
            .filter(|name| images::is_name(name) && !(ws.blobs.contains_key(*name) && dir.has(name)))
            .collect();
        names.into_iter().take(limit).map(blob_name).collect()
    }

    /// The backend lost everything (its zone was deleted, another account): image files are
    /// sent again with their bullets, which [`Workspace::resend_all`] queues.
    pub fn resend_all(&mut self, ws: &mut Workspace) {
        self.blobs = Blobs::default();
        ws.resend_all();
    }
}
