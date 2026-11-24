//! The local socket that belongs to the Flowie instance holding a database:
//! a Unix domain socket (owner-only, in `/tmp` or the sandbox's tmp) or a
//! named pipe on Windows. Claiming it is also how Flowie stays a single
//! instance per database: a second launch finds the first one here and
//! hands over instead of opening the same database twice.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use interprocess::local_socket::{prelude::*, ListenerOptions, Name};
use serde_json::{json, Value};

/// How long a second launch waits for the instance that holds the database
/// to answer (it may still be starting).
const OWNER_WAIT: Duration = Duration::from_secs(10);

fn hash_u32(v: impl std::hash::Hash) -> u32 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish() as u32
}

/// Socket file for the database at `db` (short path: Unix sockets allow ~100 bytes).
#[cfg(unix)]
pub fn socket_path(db: &Path) -> std::path::PathBuf {
    // Screenshot builds: the script picks the socket.
    #[cfg(feature = "screenshots")]
    if let Ok(p) = std::env::var("FLOWIE_SHOT_SOCKET") {
        return std::path::PathBuf::from(p);
    }
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let name = format!("flowie-{uid}-{:08x}.sock", hash_u32(db));
    // The sandbox can't use /tmp; its container has its own tmp.
    match crate::app::platform::sandbox_container() {
        Some(data) => data.join("tmp").join(name),
        None => std::path::PathBuf::from("/tmp").join(name),
    }
}

/// The socket's name for the database at `db`. Named pipes live in one
/// namespace for the whole machine, so on Windows the name carries the
/// signed-in account (domain and user) as well as the database.
pub fn socket_name(db: &Path) -> Result<Name<'static>, String> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        socket_path(db).to_fs_name::<GenericFilePath>().map(|n| n.into_owned()).map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        #[cfg(feature = "screenshots")]
        if let Ok(p) = std::env::var("FLOWIE_SHOT_SOCKET") {
            return p.to_ns_name::<GenericNamespaced>().map(|n| n.into_owned()).map_err(|e| e.to_string());
        }
        let var = |k| std::env::var(k).unwrap_or_default();
        let account = format!("{}\\{}", var("USERDOMAIN"), var("USERNAME"));
        let name = format!("flowie-{:08x}-{:08x}.sock", hash_u32(account), hash_u32(db));
        name.to_ns_name::<GenericNamespaced>().map(|n| n.into_owned()).map_err(|e| e.to_string())
    }
}

/// The socket this process listens on. On Unix it also holds the database's
/// lock file, which keeps other launches out until this process exits.
pub struct Listener {
    pub(super) inner: interprocess::local_socket::Listener,
    #[cfg(unix)]
    pub(super) path: std::path::PathBuf,
    #[cfg(unix)]
    _lock: std::fs::File,
}

/// What a launch finds at the database's socket.
pub enum Claim {
    /// Nobody else has the database open; this process now owns the socket.
    Owner(Listener),
    /// Another Flowie has the database open and answers on this connection.
    Running(LocalSocketStream),
    /// Another Flowie has the database open but doesn't answer.
    Unresponsive,
    /// The socket can't be set up here (e.g. an unwritable temp folder):
    /// run on your own, without the agent interface.
    Unavailable(String),
}

/// Claim the socket for the database at `db`, or find who holds it.
pub fn claim(db: &Path) -> Claim {
    let name = match socket_name(db) {
        Ok(n) => n,
        Err(e) => return Claim::Unavailable(e),
    };
    claim_name(db, name)
}

/// Unix: an exclusive `flock` on `<socket>.lock` decides ownership without
/// races; the owner then replaces whatever socket file a crashed run left.
#[cfg(unix)]
fn claim_name(db: &Path, name: Name<'static>) -> Claim {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let path = socket_path(db);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        // /tmp is shared: never follow a link someone else planted there.
        .custom_flags(libc::O_NOFOLLOW)
        .open(path.with_extension("lock"));
    let lock = match lock {
        Ok(f) => f,
        Err(e) => return Claim::Unavailable(format!("lock file: {e}")),
    };
    // SAFETY: the descriptor is open for the duration of the call.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return wait_for_owner(name);
    }
    // A Flowie from before the lock file may still be serving the socket.
    if let Ok(s) = LocalSocketStream::connect(name.clone()) {
        return Claim::Running(s);
    }
    match ListenerOptions::new().name(name).try_overwrite(true).create_sync() {
        Ok(inner) => {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            Claim::Owner(Listener { inner, path, _lock: lock })
        }
        Err(e) => Claim::Unavailable(e.to_string()),
    }
}

/// Windows: the first pipe instance is created with
/// `FILE_FLAG_FIRST_PIPE_INSTANCE`, so creating it fails with access denied
/// while another process owns the name.
#[cfg(windows)]
fn claim_name(_db: &Path, name: Name<'static>) -> Claim {
    match ListenerOptions::new().name(name.clone()).create_sync() {
        Ok(inner) => Claim::Owner(Listener { inner }),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::AddrInUse) => wait_for_owner(name),
        Err(e) => Claim::Unavailable(e.to_string()),
    }
}

/// The database is taken: connect to its owner, which may still be binding
/// or may have every pipe instance busy for a moment.
fn wait_for_owner(name: Name<'static>) -> Claim {
    let deadline = Instant::now() + OWNER_WAIT;
    loop {
        if let Ok(s) = LocalSocketStream::connect(name.clone()) {
            return Claim::Running(s);
        }
        if Instant::now() >= deadline {
            return Claim::Unresponsive;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Send one request on `stream` and wait for its answer.
pub fn call(stream: &LocalSocketStream, method: &str, params: Value) -> Result<String, String> {
    let mut w = stream;
    let req = json!({"id": 0, "method": method, "params": params});
    writeln!(w, "{req}").and_then(|_| w.flush()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let resp: Value = serde_json::from_str(&line).map_err(|e| format!("bad answer: {e}"))?;
    match (resp.get("result"), resp.get("error")) {
        (Some(r), _) => Ok(r.as_str().unwrap_or_default().to_string()),
        (_, Some(e)) => Err(e.as_str().unwrap_or("error").to_string()),
        _ => Err("bad answer".into()),
    }
}
