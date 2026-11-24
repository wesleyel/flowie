use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use flowie_core::markup::{rewrite_links, LinkEdit};
use flowie_core::{store, BlockKind, Node, NodeId, ROOT};
use rusqlite::{backup::NO_PROGRESS, params, Connection, OpenFlags, MAIN_DB};
use serde::Deserialize;
use tempfile::NamedTempFile;

const USAGE: &str = "One-time Flowie migration: 1.0.0-rc.3 to 1.0.0-rc.4 only.\n\
    Usage: migrate-rc3-to-rc4 <rc3-database> <new-rc4-database>\n\
           migrate-rc3-to-rc4 --replace <rc3-database>\n\n\
    Copy mode uses a read-only source and requires a new destination.\n\
    For --replace, quit Flowie first. The tool saves a backup beside the database before replacing it.";
const RC3_NODE_COLUMNS: &str = "id:INTEGER,parent:INTEGER,rank:REAL,text:TEXT,note:TEXT,completed:INTEGER,collapsed:INTEGER,created:INTEGER,modified:INTEGER,kind:INTEGER,mirror:INTEGER";
const RC3_TRASH_COLUMNS: &str = "seq:INTEGER,root:INTEGER,deleted_at:INTEGER,label:TEXT,payload:TEXT";
const RC3_META_COLUMNS: &str = "key:TEXT,value:TEXT";
const RC4_FORMAT: i64 = 3;
const LEGACY_VIRTUAL_BIT: u64 = 1 << 62;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
type IdMap = HashMap<u64, NodeId>;

#[derive(Clone, Deserialize)]
struct LegacyNode {
    id: u64,
    parent: u64,
    rank: f64,
    text: String,
    note: String,
    completed: bool,
    collapsed: bool,
    created: i64,
    modified: i64,
    #[serde(default)]
    kind: BlockKind,
    #[serde(default)]
    mirror: u64,
}

impl LegacyNode {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get::<_, i64>(0)? as u64,
            parent: row.get::<_, i64>(1)? as u64,
            rank: row.get(2)?,
            text: row.get(3)?,
            note: row.get(4)?,
            completed: row.get(5)?,
            collapsed: row.get(6)?,
            created: row.get(7)?,
            modified: row.get(8)?,
            kind: BlockKind::from_u8(row.get(9)?),
            mirror: row.get::<_, i64>(10)? as u64,
        })
    }

    fn migrate(self, ids: &mut IdMap) -> Node {
        let mut relink = |text: &str| {
            rewrite_links(text, |url| match url.strip_prefix("flowie:").and_then(|id| id.parse::<u64>().ok()) {
                Some(id) => LinkEdit::Url(format!("flowie:{}", remap(ids, id))),
                None => LinkEdit::Keep,
            })
        };
        let text = if self.kind == BlockKind::Code { self.text } else { relink(&self.text) };
        let note = relink(&self.note);
        Node {
            id: remap(ids, self.id),
            parent: remap(ids, self.parent),
            rank: self.rank,
            text,
            note,
            completed: self.completed,
            collapsed: self.collapsed,
            created: self.created,
            modified: self.modified,
            kind: self.kind,
            mirror: remap(ids, self.mirror),
            image: String::new(),
        }
    }
}

struct LegacyTrash {
    seq: i64,
    root: u64,
    deleted_at: i64,
    label: String,
    nodes: Vec<LegacyNode>,
}

fn remap(ids: &mut IdMap, id: u64) -> NodeId {
    *ids.entry(id).or_insert_with(NodeId::random)
}

fn ids_in_sibling_tiebreak_order(nodes: &[LegacyNode], trash: &[LegacyTrash]) -> IdMap {
    let old_ids: BTreeSet<_> = nodes.iter().chain(trash.iter().flat_map(|entry| &entry.nodes)).map(|node| node.id).collect();
    let mut new_ids = BTreeSet::new();
    while new_ids.len() < old_ids.len() {
        new_ids.insert(NodeId::random());
    }
    old_ids.into_iter().zip(new_ids).chain([(0, ROOT)]).collect()
}

fn migrate_meta(ids: &mut IdMap, key: &str, value: String) -> Option<String> {
    let mut id = |value: &str| value.parse::<u64>().ok().map(|id| remap(ids, id));
    match key {
        "next_id" => None,
        "zoom" | "journal" => id(&value).map(|id| id.to_string()),
        "starred" => Some(value.split(',').filter_map(id).map(|id| id.to_string()).collect::<Vec<_>>().join(",")),
        "scroll" => {
            let (page, rest) = value.split_once(':')?;
            let (row, offset) = rest.split_once(':')?;
            let row = row.parse::<u64>().ok()?;
            if row & LEGACY_VIRTUAL_BIT != 0 {
                return None;
            }
            Some(format!("{}:{}:{offset}", id(page)?, remap(ids, row)))
        }
        _ => Some(value),
    }
}

fn check_rc3(connection: &Connection) -> Result<()> {
    let unsupported = "Expected a Flowie 1.0.0-rc.3 database with the legacy integer-ID schema";
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let tables: String = connection.query_row(
        "SELECT group_concat(name) FROM (SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name)",
        [],
        |row| Ok(row.get::<_, Option<String>>(0)?.unwrap_or_default()),
    )?;
    if version != 0 || tables != "meta,nodes,trash" {
        return Err(unsupported.into());
    }
    for (table, expected) in [("nodes", RC3_NODE_COLUMNS), ("trash", RC3_TRASH_COLUMNS), ("meta", RC3_META_COLUMNS)] {
        let columns: String = connection.query_row(
            "SELECT group_concat(name || ':' || type) FROM (SELECT name, type FROM pragma_table_info(?1) ORDER BY cid)",
            [table],
            |row| row.get(0),
        )?;
        if columns != expected {
            return Err(unsupported.into());
        }
    }
    Ok(())
}

fn require_unused_destination(destination: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = destination.as_os_str().to_owned();
        name.push(suffix);
        match fs::symlink_metadata(&name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => return Err(format!("Destination already exists: {}", Path::new(&name).display()).into()),
        }
    }
    Ok(())
}

fn build_rc4(snapshot: &Connection, parent: &Path) -> Result<(NamedTempFile, usize, usize)> {
    check_rc3(snapshot)?;

    let mut statement =
        snapshot.prepare("SELECT id, parent, rank, text, note, completed, collapsed, created, modified, kind, mirror FROM nodes")?;
    let nodes = statement.query_map([], LegacyNode::read)?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut statement = snapshot.prepare("SELECT seq, deleted_at, label, payload FROM trash ORDER BY seq")?;
    let trash = statement
        .query_and_then([], |row| -> Result<_> {
            let seq = row.get(0)?;
            let payload: String = row.get(3)?;
            let nodes: Vec<LegacyNode> = serde_json::from_str(&payload).map_err(|error| format!("Invalid Trash entry {seq}: {error}"))?;
            let root = nodes.first().map(|node| node.id).ok_or_else(|| format!("Empty Trash entry {seq}"))?;
            Ok(LegacyTrash { seq, root, deleted_at: row.get(1)?, label: row.get(2)?, nodes })
        })?
        .collect::<Result<Vec<_>>>()?;
    let mut ids = ids_in_sibling_tiebreak_order(&nodes, &trash);
    let nodes: Vec<_> = nodes.into_iter().map(|node| node.migrate(&mut ids)).collect();
    let mut statement = snapshot.prepare("SELECT key, value FROM meta")?;
    let meta = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut metadata = Vec::new();
    for entry in meta {
        let (key, value) = entry?;
        if let Some(value) = migrate_meta(&mut ids, &key, value) {
            metadata.push((key, value));
        }
    }

    let temporary = tempfile::Builder::new().prefix(".flowie-rc4-").tempfile_in(parent)?;
    let mut output = store::open(temporary.path())?;
    let version: i64 = output.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != RC4_FORMAT {
        return Err("This tool must be run with the Flowie 1.0.0-rc.4 database schema".into());
    }
    output.pragma_update(None, "journal_mode", "DELETE")?;
    store::write_batch(&mut output, &nodes, &[], &metadata)?;
    let batch = output.transaction()?;
    for entry in &trash {
        let root = remap(&mut ids, entry.root);
        let nodes: Vec<_> = entry.nodes.iter().cloned().map(|node| node.migrate(&mut ids)).collect();
        batch.execute(
            "INSERT INTO trash (seq, root, deleted_at, label, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![entry.seq, root, entry.deleted_at, entry.label, serde_json::to_string(&nodes)?],
        )?;
    }
    batch.commit()?;
    output.close().map_err(|(_, error)| error)?;
    temporary.as_file().sync_all()?;
    Ok((temporary, nodes.len(), trash.len()))
}

fn migrate(source: &Path, destination: &Path) -> Result<(usize, usize)> {
    require_unused_destination(destination)?;
    let parent = destination.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut input = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let snapshot = input.transaction()?;
    let (temporary, nodes, trash) = build_rc4(&snapshot, parent)?;
    temporary.persist_noclobber(destination)?;
    Ok((nodes, trash))
}

fn replace(source: &Path) -> Result<(PathBuf, usize, usize)> {
    let source = source.canonicalize()?;
    let parent = source.parent().ok_or("Expected a database file")?;
    let mut backup_name = source.file_name().ok_or("Expected a database file")?.to_owned();
    backup_name.push(format!(".rc3-backup-{}", flowie_core::tree::now_millis()));
    let backup_path = parent.join(backup_name);
    let mut input = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    input.busy_timeout(Duration::ZERO)?;
    input.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    input.pragma_update(None, "synchronous", "FULL")?;
    input
        .execute_batch("BEGIN EXCLUSIVE; COMMIT;")
        .map_err(|error| format!("Could not lock the database for replacement. Quit Flowie and close other connections first. {error}"))?;
    let (converted, nodes, trash) = build_rc4(&input, parent)?;

    let backup = tempfile::Builder::new().prefix(".flowie-rc3-").tempfile_in(parent)?;
    input.backup(MAIN_DB, backup.path(), NO_PROGRESS)?;
    Connection::open(backup.path())?.pragma_update(None, "journal_mode", "DELETE")?;
    backup.as_file().sync_all()?;
    require_unused_destination(&backup_path)?;
    backup.persist_noclobber(&backup_path)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;

    input
        .restore(MAIN_DB, converted.path(), NO_PROGRESS)
        .map_err(|error| format!("Could not replace the database. Backup at {}. {error}", backup_path.display()))?;
    Ok((backup_path, nodes, trash))
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        [flag, source] if flag == "--replace" => replace(Path::new(source)).map(|(backup, nodes, trash)| {
            println!("Migrated {nodes} bullets and {trash} Trash entries in {}", Path::new(source).display());
            println!("RC3 backup saved to {}", backup.display());
        }),
        [source, destination] => migrate(Path::new(source), Path::new(destination)).map(|(nodes, trash)| {
            println!("Migrated {nodes} bullets and {trash} Trash entries to {}", Path::new(destination).display());
        }),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = result {
        eprintln!("Migration failed: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_db(path: &Path) -> Connection {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE nodes (id INTEGER PRIMARY KEY, parent INTEGER, rank REAL, text TEXT, note TEXT,
                   completed INTEGER, collapsed INTEGER, created INTEGER, modified INTEGER, kind INTEGER, mirror INTEGER);
                 CREATE TABLE trash (seq INTEGER PRIMARY KEY AUTOINCREMENT, root INTEGER, deleted_at INTEGER, label TEXT, payload TEXT);
                 CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
                 PRAGMA journal_mode = WAL;
                 PRAGMA wal_autocheckpoint = 0;
                 INSERT INTO nodes VALUES (1, 0, 1.0, 'Live', '[Deleted](flowie:2)', 0, 0, 10, 11, 0, 0);
                 INSERT INTO meta VALUES ('starred', '1');",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO trash VALUES (7, 4611686018427387906, 99, 'Deleted', ?1)",
                [r#"[{"id":2,"parent":1,"rank":1.0,"text":"Deleted","note":"","completed":false,"collapsed":false,"created":10,"modified":11}]"#],
            )
            .unwrap();
        connection
    }

    #[test]
    fn replace_preserves_uncheckpointed_rows_in_output_and_backup() {
        let work = tempfile::tempdir().unwrap();
        let original = work.path().join("original.sqlite");
        let connection = legacy_db(&original);
        let source = work.path().join("flowie.sqlite");
        fs::copy(&original, &source).unwrap();
        fs::copy(work.path().join("original.sqlite-wal"), work.path().join("flowie.sqlite-wal")).unwrap();
        let without_wal = work.path().join("main-only.sqlite");
        fs::copy(&source, &without_wal).unwrap();
        let row_count = |database: &Connection| database.query_row::<i64, _, _>("SELECT count(*) FROM nodes", [], |row| row.get(0));
        assert_eq!(row_count(&Connection::open(without_wal).unwrap()).unwrap(), 0);

        let (backup, nodes, trash) = replace(&source).unwrap();
        assert_eq!((nodes, trash), (1, 1));
        assert_eq!(backup.parent(), source.canonicalize().unwrap().parent());
        let backup_db = Connection::open(&backup).unwrap();
        check_rc3(&backup_db).unwrap();
        assert_eq!(row_count(&backup_db).unwrap(), 1);
        assert_eq!(backup_db.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0)).unwrap(), "delete");
        let updated = Connection::open(&source).unwrap();
        assert_eq!(updated.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(), RC4_FORMAT);
        let (nodes, metadata) = store::load(&updated).unwrap();
        let node = &nodes[0];
        assert!(metadata.contains(&("starred".to_string(), node.id.to_string())));
        let deleted = updated
            .query_row("SELECT root, payload FROM trash WHERE seq = 7", [], |row| Ok((row.get::<_, NodeId>(0)?, row.get::<_, String>(1)?)))
            .unwrap();
        assert_eq!(node.note, format!("[Deleted](flowie:{})", deleted.0));
        let deleted_nodes: Vec<Node> = serde_json::from_str(&deleted.1).unwrap();
        assert_eq!((deleted_nodes[0].id, deleted_nodes[0].parent), (deleted.0, node.id));
        drop(connection);
    }

    #[test]
    fn replace_refuses_a_database_held_open_in_wal_mode() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("flowie.sqlite");
        let connection = legacy_db(&source);
        let error = replace(&source).unwrap_err();
        assert!(error.to_string().contains("locked"), "{error}");
        check_rc3(&connection).unwrap();
        assert!(fs::read_dir(work.path()).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().contains("backup")));
    }
}
