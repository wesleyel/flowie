//! Opening a database: the format check, upgrades through `MIGRATIONS`, the
//! backup taken before them, and setting aside a release candidate's database.
//! How to add a migration is in the `store` module docs.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Transaction, TransactionBehavior};

/// One upgrade step. `MIGRATIONS[i]` takes a database from format
/// `BASELINE + i` to `BASELINE + i + 1`. It runs inside the upgrade's
/// transaction, which also sets `user_version`, so it must not commit and
/// must not touch `user_version` itself.
pub type Migration = fn(&Transaction<'_>) -> rusqlite::Result<()>;

/// Upgrade backups kept beside the database; older ones are deleted after an
/// upgrade succeeds.
const BACKUPS_KEPT: usize = 3;

/// The formats a build knows: `schema` creates a database at `current()`,
/// and `migrations` lead there from `baseline`.
pub(super) struct Format<'a> {
    pub schema: &'a str,
    pub baseline: i64,
    pub migrations: &'a [Migration],
}

impl Format<'_> {
    pub fn current(&self) -> i64 {
        self.baseline + self.migrations.len() as i64
    }
}

/// Why a database couldn't be opened. In every case but `Sqlite` the
/// database file was left as it was.
#[derive(Debug)]
pub enum OpenError {
    /// A newer Flowie wrote the database: its format is `found`, and this
    /// build reads up to `current`. Nothing was changed.
    TooNew {
        found: i64,
        current: i64,
    },
    /// The copy taken before upgrading couldn't be saved, so the upgrade
    /// didn't start.
    Backup {
        path: PathBuf,
        error: rusqlite::Error,
    },
    /// The upgrade from `from` to `to` failed and was rolled back; the
    /// database is still at the format it had, and `backup` holds a copy.
    Migration {
        from: i64,
        to: i64,
        backup: PathBuf,
        error: rusqlite::Error,
    },
    /// A release candidate's database couldn't be moved aside.
    SetAside(std::io::Error),
    Sqlite(rusqlite::Error),
}

impl OpenError {
    /// The database was written by a newer Flowie.
    pub fn is_too_new(&self) -> bool {
        matches!(self, OpenError::TooNew { .. })
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::TooNew { found, current } => {
                write!(f, "the database has format {found}, from a newer Flowie (this one reads up to {current})")
            }
            OpenError::Backup { path, error } => write!(f, "could not save the backup {} before upgrading: {error}", path.display()),
            OpenError::Migration { from, to, backup, error } => {
                write!(
                    f,
                    "upgrading the database from format {from} to {to} failed, nothing was changed (backup: {}): {error}",
                    backup.display()
                )
            }
            OpenError::SetAside(error) => write!(f, "could not move the old database aside: {error}"),
            OpenError::Sqlite(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OpenError::TooNew { .. } => None,
            OpenError::Backup { error, .. } | OpenError::Migration { error, .. } | OpenError::Sqlite(error) => Some(error),
            OpenError::SetAside(error) => Some(error),
        }
    }
}

impl From<rusqlite::Error> for OpenError {
    fn from(error: rusqlite::Error) -> Self {
        OpenError::Sqlite(error)
    }
}

/// So that code returning `rusqlite::Result` can `?` an open.
impl From<OpenError> for rusqlite::Error {
    fn from(error: OpenError) -> Self {
        match error {
            OpenError::Sqlite(error) => error,
            other => rusqlite::Error::ToSqlConversionFailure(Box::new(other)),
        }
    }
}

pub(super) fn open(path: &Path, format: &Format<'_>) -> Result<Connection, OpenError> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let current = format.current();
    let mut conn = connect(path)?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let has_nodes: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'nodes'", [], |r| r.get(0))?;
    if version > current {
        return Err(OpenError::TooNew { found: version, current });
    }
    if has_nodes > 0 && version >= format.baseline {
        if version < current {
            upgrade(&mut conn, path, format, version)?;
        }
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        return Ok(conn);
    }
    if has_nodes > 0 {
        // A release candidate's database, older than 1.0: no migrations lead from
        // it. Fold the WAL into the file so the three files move as one.
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        drop(conn);
        let aside = set_aside(path).map_err(OpenError::SetAside)?;
        eprintln!("flowie: {} has an older format; moved it to {}", path.display(), aside.display());
        conn = connect(path)?;
    }
    conn.execute_batch(format.schema)?;
    conn.pragma_update(None, "user_version", current)?;
    Ok(conn)
}

fn connect(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

/// Back the database up, then run the migrations from `from` in one
/// transaction that ends by setting `user_version`.
fn upgrade(conn: &mut Connection, path: &Path, format: &Format<'_>, from: i64) -> Result<(), OpenError> {
    let current = format.current();
    let backup = backup(conn, path, from)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (step, migration) in (from..).zip(&format.migrations[(from - format.baseline) as usize..]) {
        // On error the transaction is dropped, which rolls it back.
        migration(&tx).map_err(|error| OpenError::Migration { from: step, to: step + 1, backup: backup.clone(), error })?;
    }
    let finish = tx.pragma_update(None, "user_version", current).and_then(|()| tx.commit());
    finish.map_err(|error| OpenError::Migration { from, to: current, backup: backup.clone(), error })?;
    eprintln!("flowie: upgraded {} from format {from} to {current}; the old one is in {}", path.display(), backup.display());
    prune_backups(path);
    Ok(())
}

/// Copy the database to `<name>.backup-v<format>-<millis>` beside it.
/// `VACUUM INTO` reads in one transaction, so the copy is consistent and
/// includes what is still in the WAL.
fn backup(conn: &Connection, path: &Path, version: i64) -> Result<PathBuf, OpenError> {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".backup-v{version}-{}", crate::tree::now_millis()));
    let backup = PathBuf::from(name);
    let saved = (|| {
        let target = backup.to_str().ok_or_else(|| rusqlite::Error::InvalidPath(backup.clone()))?;
        conn.execute("VACUUM INTO ?1", [target])?;
        // Opened for writing: Windows only flushes (FlushFileBuffers) a handle with write access.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&backup)
            .and_then(|f| f.sync_all())
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
    })();
    match saved {
        Ok(()) => Ok(backup),
        Err(error) => {
            let _ = std::fs::remove_file(&backup);
            Err(OpenError::Backup { path: backup, error })
        }
    }
}

/// Keep the newest `BACKUPS_KEPT` upgrade backups of `path`, delete the rest.
fn prune_backups(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else { return };
    let prefix = format!("{name}.backup-v");
    let Ok(entries) = std::fs::read_dir(if dir.as_os_str().is_empty() { Path::new(".") } else { dir }) else { return };
    // (millis, path) of each backup; the millis end the name: `v<format>-<millis>`.
    let mut backups: Vec<(i64, PathBuf)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let file = entry.file_name();
            let rest = file.to_str()?.strip_prefix(&prefix)?;
            let (_, millis) = rest.split_once('-')?;
            Some((millis.parse().ok()?, entry.path()))
        })
        .collect();
    backups.sort_by(|a, b| b.cmp(a));
    for (_, old) in backups.into_iter().skip(BACKUPS_KEPT) {
        for suffix in ["", "-wal", "-shm"] {
            let mut file = old.as_os_str().to_owned();
            file.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(file));
        }
    }
}

/// Rename an old-format database (and its WAL and shared-memory files) to
/// `<name>.old-<millis>`, where a newer Flowie leaves it alone.
fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".old-{}", crate::tree::now_millis()));
    let aside = PathBuf::from(aside);
    let mut moved = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let mut from = path.as_os_str().to_owned();
        from.push(suffix);
        let mut to = aside.as_os_str().to_owned();
        to.push(suffix);
        if Path::new(&from).exists() {
            if let Err(error) = std::fs::rename(&from, &to) {
                for (original, moved_to) in moved.iter().rev() {
                    let _ = std::fs::rename(moved_to, original);
                }
                return Err(error);
            }
            moved.push((PathBuf::from(from), PathBuf::from(to)));
        }
    }
    Ok(aside)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = "
        PRAGMA journal_mode = WAL;
        CREATE TABLE IF NOT EXISTS nodes (id INTEGER PRIMARY KEY, text TEXT NOT NULL);
    ";

    fn add_note(tx: &Transaction<'_>) -> rusqlite::Result<()> {
        tx.execute_batch("ALTER TABLE nodes ADD COLUMN note TEXT NOT NULL DEFAULT ''")
    }

    fn fill_note(tx: &Transaction<'_>) -> rusqlite::Result<()> {
        // Needs the column the step before added, so it checks the order.
        tx.execute("UPDATE nodes SET note = 'from ' || text", []).map(drop)
    }

    fn fail(tx: &Transaction<'_>) -> rusqlite::Result<()> {
        tx.execute_batch("SELECT * FROM no_such_table")
    }

    const BASELINE: i64 = 10;

    fn format(migrations: &[Migration]) -> Format<'_> {
        Format { schema: SCHEMA, baseline: BASELINE, migrations }
    }

    /// A database at `version` with the test schema and two rows, left in WAL
    /// mode with rows still in the WAL, as Flowie leaves it while running.
    fn database_at(path: &Path, version: i64) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch("PRAGMA wal_autocheckpoint = 0; INSERT INTO nodes (text) VALUES ('a'), ('b');").unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
        conn
    }

    fn version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    fn texts(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("SELECT text FROM nodes ORDER BY id").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    fn files_with(dir: &Path, part: &str) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_str().unwrap().contains(part))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn new_database_starts_at_the_current_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        let conn = open(&path, &format(&[add_note, fill_note])).unwrap();
        assert_eq!(version(&conn), BASELINE + 2);
        // The real schema and format, too.
        drop(conn);
        let real = super::super::open(&dir.path().join("real.sqlite")).unwrap();
        assert_eq!(version(&real), super::super::FORMAT);
    }

    #[test]
    fn current_database_is_opened_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        drop(database_at(&path, BASELINE + 1));
        let conn = open(&path, &format(&[add_note])).unwrap();
        assert_eq!(texts(&conn), ["a", "b"]);
        assert!(files_with(dir.path(), ".backup-").is_empty());
    }

    #[test]
    fn migrations_run_in_order_after_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        let before = database_at(&path, BASELINE);
        let conn = open(&path, &format(&[add_note, fill_note])).unwrap();
        assert_eq!(version(&conn), BASELINE + 2);
        let notes: Vec<String> =
            conn.prepare("SELECT note FROM nodes ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(notes, ["from a", "from b"]);
        drop(before);

        let backups = files_with(dir.path(), ".backup-");
        assert_eq!(backups.len(), 1);
        let name = backups[0].file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(&format!("flowie.sqlite.backup-v{BASELINE}-")), "{name}");
        // The backup is the database as it was: its rows (WAL included), its
        // format, and no note column yet.
        let backup = Connection::open(&backups[0]).unwrap();
        assert_eq!(texts(&backup), ["a", "b"]);
        assert_eq!(version(&backup), BASELINE);
        assert!(backup.prepare("SELECT note FROM nodes").is_err());
    }

    #[test]
    fn migrations_start_at_the_databases_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        // Already past `fail`: only `add_note` and `fill_note` run.
        drop(database_at(&path, BASELINE + 1));
        let conn = open(&path, &format(&[fail, add_note, fill_note])).unwrap();
        assert_eq!(version(&conn), BASELINE + 3);
        assert_eq!(files_with(dir.path(), ".backup-v11-").len(), 1);
    }

    #[test]
    fn a_failed_migration_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        drop(database_at(&path, BASELINE));
        let error = open(&path, &format(&[add_note, fail])).unwrap_err();
        match &error {
            OpenError::Migration { from, to, backup, .. } => {
                assert_eq!((*from, *to), (BASELINE + 1, BASELINE + 2));
                assert!(backup.exists());
            }
            other => panic!("expected a migration error, got {other:?}"),
        }
        // The first step was rolled back with the second.
        let conn = Connection::open(&path).unwrap();
        assert_eq!(version(&conn), BASELINE);
        assert_eq!(texts(&conn), ["a", "b"]);
        assert!(conn.prepare("SELECT note FROM nodes").is_err());
        // And code that returns rusqlite errors still sees what went wrong.
        let as_sqlite: rusqlite::Error = error.into();
        assert!(as_sqlite.to_string().contains("from format 11 to 12"), "{as_sqlite}");
    }

    #[test]
    fn a_failed_backup_stops_the_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        let conn = database_at(&path, BASELINE);
        // A backup the database can't write: its folder doesn't exist.
        let error = backup(&conn, &dir.path().join("missing").join("flowie.sqlite"), BASELINE).unwrap_err();
        assert!(matches!(error, OpenError::Backup { .. }), "{error:?}");
    }

    #[cfg(unix)]
    #[test]
    fn no_migration_runs_without_a_backup() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("data");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("flowie.sqlite");
        // Kept open so the WAL files stay and the database opens in a read-only folder.
        let before = database_at(&path, BASELINE);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let writable = std::fs::write(folder.join("probe"), b"").is_ok(); // as root
        let result = open(&path, &format(&[add_note]));
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if writable {
            return;
        }
        let error = result.unwrap_err();
        assert!(matches!(error, OpenError::Backup { .. }), "{error:?}");
        assert_eq!(version(&before), BASELINE);
        assert!(before.prepare("SELECT note FROM nodes").is_err());
        assert!(files_with(&folder, ".backup-").is_empty());
    }

    #[test]
    fn release_candidate_database_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        drop(database_at(&path, BASELINE - 1));
        let conn = open(&path, &format(&[add_note])).unwrap();
        assert_eq!(version(&conn), BASELINE + 1);
        assert!(texts(&conn).is_empty());
        let aside = files_with(dir.path(), ".old-");
        let [moved] = aside.as_slice() else { panic!("expected one database set aside, found {aside:?}") };
        assert_eq!(texts(&Connection::open(moved).unwrap()), ["a", "b"]);
        assert!(files_with(dir.path(), ".backup-").is_empty());
    }

    #[test]
    fn newer_database_is_refused_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        drop(database_at(&path, BASELINE + 5));
        let error = open(&path, &format(&[add_note])).unwrap_err();
        assert!(error.is_too_new());
        assert!(matches!(error, OpenError::TooNew { found: 15, current: 11 }), "{error:?}");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(version(&conn), BASELINE + 5);
        assert_eq!(texts(&conn), ["a", "b"]);
        assert!(files_with(dir.path(), ".old-").is_empty());
        assert!(files_with(dir.path(), ".backup-").is_empty());
    }

    #[test]
    fn only_the_newest_backups_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flowie.sqlite");
        for millis in [100, 300, 200, 400, 50] {
            std::fs::write(dir.path().join(format!("flowie.sqlite.backup-v4-{millis}")), b"").unwrap();
        }
        std::fs::write(dir.path().join("flowie.sqlite.backup-v4-50-wal"), b"").unwrap();
        std::fs::write(dir.path().join("other.sqlite.backup-v4-1"), b"").unwrap();
        prune_backups(&path);
        let left: Vec<String> =
            files_with(dir.path(), ".backup-").iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_owned()).collect();
        assert_eq!(
            left,
            ["flowie.sqlite.backup-v4-200", "flowie.sqlite.backup-v4-300", "flowie.sqlite.backup-v4-400", "other.sqlite.backup-v4-1"]
        );
    }
}
