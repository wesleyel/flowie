//! iCloud sync for the desktop app on macOS (wiki: Per-Node-Sync §5.2): CKSyncEngine, run by
//! FlowieCloudKit (the Swift package the iOS app uses too), drives flowie-doc's
//! [`NativeSync`] over the open [`Workspace`]. CloudKit calls in on its own threads; the
//! questions it asks wait in a queue until the thread that owns the workspace calls
//! [`CloudDriver::pump`], which answers them. That thread is woken through the `wake`
//! function given at start, as with the self-hosted [`flowie_doc::SyncDriver`].
//!
//! Only builds signed with Flowie's iCloud entitlements (the Mac App Store build) can sync;
//! anywhere else [`CloudDriver::start`] says why it can't, and nothing touches CloudKit.

use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use flowie_doc::{NativeSaved, NativeSync, Wire, Workspace};
use serde::Deserialize;

/// How often to fetch from iCloud even without a push: GPUI's app delegate doesn't hand
/// silent pushes on, so the Mac also looks every so often.
const FETCH_EVERY: Duration = Duration::from_secs(60);

/// The name iCloud sync keeps its place under in the outline (`Workspace::use_remote`).
pub const REMOTE: &str = "icloud";

/// Why iCloud can't be used, or that it can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Available,
    /// No iCloud account is signed in on this Mac.
    NoAccount,
    /// Parental controls or a device profile don't allow it.
    Restricted,
    TemporarilyUnavailable,
    /// This build isn't signed for Flowie's iCloud container (built from source, Homebrew,
    /// GitHub releases): only the Mac App Store build is.
    NotEntitled,
    /// CKSyncEngine needs macOS 14.
    Unsupported,
    Failed(String),
}

/// Where iCloud sync stands, for the app to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudPhase {
    /// Asking iCloud whether an account is signed in.
    Checking,
    /// iCloud can't be used now.
    Unavailable(Access),
    Syncing,
    Synced,
    /// The last fetch or send failed; CKSyncEngine tries again.
    Offline(String),
}

/// Whether this build can sync through iCloud at all: it was built with the Swift bridge and
/// is signed with the entitlements for Flowie's container.
pub fn entitled() -> bool {
    #[cfg(cloudkit_swift)]
    {
        // SAFETY: no arguments; reads this process's own signature.
        unsafe { ffi::flowie_ck_entitled() }
    }
    #[cfg(not(cloudkit_swift))]
    {
        false
    }
}

/// A message from the Swift side, with the token to answer it by (0: news, no answer).
struct Message {
    json: String,
    reply: usize,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Incoming {
    Unsent,
    NextSaves { limit: usize },
    Saved { outcomes: Vec<NativeSaved> },
    Fetched { records: Vec<Wire> },
    ResendAll,
    Access { access: String, error: Option<String> },
    FetchedAll,
    Sent,
    Failed { error: String },
}

/// iCloud sync running for an open workspace.
pub struct CloudDriver {
    #[cfg_attr(not(cloudkit_swift), allow(dead_code))]
    handle: usize,
    inbox: Receiver<Message>,
    native: NativeSync,
    access: Option<Access>,
    error: Option<String>,
    holding: bool,
    caught_up: bool,
    received: usize,
    /// The ids last handed to CKSyncEngine as pending.
    queued: Vec<String>,
    /// When the last fetch was asked for.
    fetched_at: Instant,
}

impl CloudDriver {
    /// Start syncing `ws` through iCloud, with CKSyncEngine's state in the file at `state`.
    /// `fresh`: the outline didn't sync with iCloud last (`Workspace::use_remote(REMOTE)` said
    /// so), so that state is dropped. `hold`: send nothing until [`release_saves`]. `wake` is
    /// called from CloudKit's threads when there is something to [`pump`](Self::pump).
    ///
    /// [`release_saves`]: Self::release_saves
    pub fn start(state: &Path, fresh: bool, hold: bool, wake: impl Fn() + Send + Sync + 'static) -> Result<CloudDriver, Access> {
        if !entitled() {
            return Err(Access::NotEntitled);
        }
        let (tx, inbox) = channel();
        let handle = bridge::start(state, fresh, hold, tx, Box::new(wake));
        Ok(CloudDriver {
            handle,
            inbox,
            native: NativeSync::new(),
            access: None,
            error: None,
            holding: hold,
            caught_up: false,
            received: 0,
            queued: Vec::new(),
            fetched_at: Instant::now(),
        })
    }

    /// Answer what CloudKit asked and take what it brought; then tell it what changed here.
    /// Call it when woken and after saving edits. Returns whether the outline changed (or an
    /// image file came that a row shows).
    pub fn pump(&mut self, ws: &mut Workspace) -> bool {
        let revision = ws.tree.revision();
        let mut images = false;
        while let Ok(message) = self.inbox.try_recv() {
            let answer = match serde_json::from_str::<Incoming>(&message.json) {
                Ok(Incoming::Fetched { records }) => {
                    self.received += records.iter().filter(|r| r.file.is_none()).count();
                    images |= self.native.on_fetched(ws, &records, REMOTE);
                    "null".to_string()
                }
                Ok(incoming) => self.take(ws, incoming),
                Err(_) => "null".to_string(),
            };
            if message.reply != 0 {
                bridge::reply(message.reply, &answer);
            }
        }
        if !self.holding {
            let ids = self.native.to_send(ws, 10_000);
            if ids != self.queued {
                let fresh: Vec<&String> = ids.iter().filter(|id| !self.queued.contains(id)).collect();
                if !fresh.is_empty() {
                    bridge::queue(self.handle, &serde_json::to_string(&fresh).expect("ids"));
                }
                self.queued = ids;
            }
        }
        if self.fetched_at.elapsed() >= FETCH_EVERY {
            self.fetched_at = Instant::now();
            bridge::fetch(self.handle);
        }
        images || ws.tree.revision() != revision
    }

    fn take(&mut self, ws: &mut Workspace, incoming: Incoming) -> String {
        match incoming {
            Incoming::Unsent => serde_json::to_string(&self.native.to_send(ws, 10_000)).expect("ids"),
            Incoming::NextSaves { limit } if !self.holding => serde_json::to_string(&self.native.next_saves(ws, limit)).expect("wires"),
            Incoming::NextSaves { .. } => "[]".into(),
            Incoming::Saved { outcomes } => {
                self.native.on_saved(ws, outcomes);
                // What CKSyncEngine has pending is told again from the outbox.
                self.queued.clear();
                "null".into()
            }
            // Taken in `pump`, which notes whether an image file came.
            Incoming::Fetched { .. } => "null".into(),
            Incoming::ResendAll => {
                self.native.resend_all(ws);
                self.queued.clear();
                "null".into()
            }
            Incoming::Access { access, error } => {
                self.access = Some(match access.as_str() {
                    "available" => Access::Available,
                    "no_account" => Access::NoAccount,
                    "restricted" => Access::Restricted,
                    "temporarily_unavailable" => Access::TemporarilyUnavailable,
                    "not_entitled" => Access::NotEntitled,
                    "unsupported" => Access::Unsupported,
                    _ => Access::Failed(error.unwrap_or_default()),
                });
                "null".into()
            }
            Incoming::FetchedAll => {
                self.caught_up = true;
                self.error = None;
                "null".into()
            }
            Incoming::Sent => {
                self.error = None;
                "null".into()
            }
            Incoming::Failed { error } => {
                self.error = Some(error);
                "null".into()
            }
        }
    }

    /// Send from now on: a first sync has looked at what iCloud has.
    pub fn release_saves(&mut self) {
        self.holding = false;
        self.queued.clear();
        bridge::release(self.handle);
    }

    /// Fetch what changed in iCloud now.
    pub fn fetch_now(&self) {
        bridge::fetch(self.handle);
    }

    /// A fetch has finished since the driver started: this device has what iCloud had.
    pub fn caught_up(&self) -> bool {
        self.caught_up
    }

    /// Records that came from iCloud since the driver started.
    pub fn received(&self) -> usize {
        self.received
    }

    pub fn phase(&self, ws: &Workspace) -> CloudPhase {
        match (&self.access, &self.error) {
            (None, _) => CloudPhase::Checking,
            (Some(access), _) if *access != Access::Available => CloudPhase::Unavailable(access.clone()),
            (_, Some(error)) => CloudPhase::Offline(error.clone()),
            _ if self.holding || !self.caught_up || ws.sync.has_unsent() => CloudPhase::Syncing,
            _ => CloudPhase::Synced,
        }
    }
}

impl Drop for CloudDriver {
    fn drop(&mut self) {
        bridge::stop(self.handle);
        // Nothing waits for an answer that won't come.
        while let Ok(message) = self.inbox.try_recv() {
            if message.reply != 0 {
                bridge::reply(message.reply, "null");
            }
        }
    }
}

#[cfg(cloudkit_swift)]
mod ffi {
    use std::ffi::{c_char, c_void};

    pub type Callback = extern "C" fn(*mut c_void, *const c_char, *mut c_void);

    extern "C" {
        pub fn flowie_ck_entitled() -> bool;
        pub fn flowie_ck_start(state: *const c_char, fresh: bool, hold: bool, context: *mut c_void, callback: Callback) -> *mut c_void;
        pub fn flowie_ck_queue(handle: *mut c_void, json: *const c_char);
        pub fn flowie_ck_release(handle: *mut c_void);
        pub fn flowie_ck_fetch(handle: *mut c_void);
        pub fn flowie_ck_stop(handle: *mut c_void);
        pub fn flowie_ck_reply(token: *mut c_void, json: *const c_char);
    }
}

/// The calls into Swift, as plain values (handles and tokens are addresses).
#[cfg(cloudkit_swift)]
mod bridge {
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::sync::mpsc::Sender;

    use super::{ffi, Message};

    /// Where the Swift side's messages go. Leaked at start: Swift may still call in for a
    /// moment after the driver stopped (a task finishing), and then finds the channel closed.
    struct Inbox {
        tx: Sender<Message>,
        wake: Box<dyn Fn() + Send + Sync>,
    }

    extern "C" fn on_message(context: *mut c_void, json: *const c_char, reply: *mut c_void) {
        // SAFETY: `context` is the leaked `Inbox` from `start`; `json` is a C string Swift
        // keeps alive for this call.
        let inbox = unsafe { &*(context as *const Inbox) };
        let json = if json.is_null() { String::new() } else { unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned() };
        let sent = inbox.tx.send(Message { json, reply: reply as usize }).is_ok();
        if sent {
            (inbox.wake)();
        } else if !reply.is_null() {
            self::reply(reply as usize, "null");
        }
    }

    pub fn start(state: &std::path::Path, fresh: bool, hold: bool, tx: Sender<Message>, wake: Box<dyn Fn() + Send + Sync>) -> usize {
        let inbox = Box::into_raw(Box::new(Inbox { tx, wake }));
        let path = CString::new(state.to_string_lossy().as_bytes()).unwrap_or_default();
        // SAFETY: the path outlives the call; the inbox is never freed.
        unsafe { ffi::flowie_ck_start(path.as_ptr(), fresh, hold, inbox as *mut c_void, on_message) as usize }
    }

    pub fn queue(handle: usize, json: &str) {
        if handle != 0 {
            let json = CString::new(json).unwrap_or_default();
            // SAFETY: a live handle from `start`; the string outlives the call.
            unsafe { ffi::flowie_ck_queue(handle as *mut c_void, json.as_ptr()) }
        }
    }

    pub fn release(handle: usize) {
        if handle != 0 {
            // SAFETY: a live handle from `start`.
            unsafe { ffi::flowie_ck_release(handle as *mut c_void) }
        }
    }

    pub fn fetch(handle: usize) {
        if handle != 0 {
            // SAFETY: a live handle from `start`.
            unsafe { ffi::flowie_ck_fetch(handle as *mut c_void) }
        }
    }

    pub fn stop(handle: usize) {
        if handle != 0 {
            // SAFETY: a live handle from `start`, given up here.
            unsafe { ffi::flowie_ck_stop(handle as *mut c_void) }
        }
    }

    pub fn reply(token: usize, json: &str) {
        let json = CString::new(json).unwrap_or_default();
        // SAFETY: each token is answered once (here, or by `on_message` when nobody listens).
        unsafe { ffi::flowie_ck_reply(token as *mut c_void, json.as_ptr()) }
    }
}

/// Without the Swift bridge nothing starts (`entitled` is false), so these are never reached.
#[cfg(not(cloudkit_swift))]
mod bridge {
    use std::sync::mpsc::Sender;

    use super::Message;

    pub fn start(_: &std::path::Path, _: bool, _: bool, _: Sender<Message>, _: Box<dyn Fn() + Send + Sync>) -> usize {
        0
    }
    pub fn queue(_: usize, _: &str) {}
    pub fn release(_: usize) {}
    pub fn fetch(_: usize) {}
    pub fn stop(_: usize) {}
    pub fn reply(_: usize, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_without_the_entitlements_never_starts() {
        // Test binaries are never signed for iCloud: CloudKit must not be touched.
        assert!(!entitled());
        let state = std::env::temp_dir().join("flowie-cloudkit-test-state.json");
        assert_eq!(CloudDriver::start(&state, true, false, || {}).err(), Some(Access::NotEntitled));
    }

    #[test]
    fn messages_from_swift_parse() {
        let saved: Incoming =
            serde_json::from_str(r#"{"type":"saved","outcomes":[{"result":"saved","id":"a","etag":"e"},{"result":"conflict","server":{"id":"b","envelope":"{}"}},{"result":"failed","id":"c","error":"x"}]}"#).unwrap();
        let Incoming::Saved { outcomes } = saved else { panic!("saved") };
        assert_eq!(outcomes.len(), 3);
        assert!(matches!(&outcomes[1], NativeSaved::Conflict { server } if server.etag.is_none()));
        assert!(matches!(serde_json::from_str(r#"{"type":"access","access":"no_account"}"#), Ok(Incoming::Access { .. })));
        assert!(matches!(serde_json::from_str(r#"{"type":"next_saves","limit":400}"#), Ok(Incoming::NextSaves { limit: 400 })));
        assert!(matches!(serde_json::from_str(r#"{"type":"fetched_all"}"#), Ok(Incoming::FetchedAll)));
        // A `Blob` record: its file where CKSyncEngine downloaded it, and no envelope.
        let fetched: Incoming = serde_json::from_str(
            r#"{"type":"fetched","records":[{"id":"n","etag":"e","envelope":"{}"},{"id":"ab.png","file":{"path":"/tmp/asset","width":3,"height":2,"mime":"image/png"}}]}"#,
        )
        .unwrap();
        let Incoming::Fetched { records } = fetched else { panic!("fetched") };
        assert!(records[0].file.is_none());
        assert_eq!(records[1].file.as_ref().map(|f| (f.path.as_str(), f.width, f.height)), Some(("/tmp/asset", 3, 2)));
        assert_eq!(records[1].envelope, "");
    }
}
