PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS nodes (
    id        BLOB    PRIMARY KEY,
    parent    BLOB    NOT NULL,
    rank      TEXT    NOT NULL,   -- fractional index (crate::rank), BINARY order
    text      TEXT    NOT NULL DEFAULT '',
    note      TEXT    NOT NULL DEFAULT '',
    completed INTEGER NOT NULL DEFAULT 0,
    collapsed INTEGER NOT NULL DEFAULT 0,
    created   INTEGER NOT NULL,
    modified  INTEGER NOT NULL,
    kind      INTEGER NOT NULL DEFAULT 0,
    mirror    BLOB    NOT NULL,
    image     TEXT    NOT NULL DEFAULT '',
    star      TEXT    NOT NULL DEFAULT '',  -- fractional index in the starred list; '' = not starred
    deleted_at INTEGER,                     -- moved to the Trash then (ms); NULL = in the outline
    clocks     BLOB    NOT NULL DEFAULT x'', -- crate::hlc::Clocks; empty = derived from created/modified
    moved_from BLOB    NOT NULL DEFAULT x'', -- parent before the last move; empty = the parent
    extra      TEXT    NOT NULL DEFAULT '',  -- fields from a newer Flowie, as JSON; '' = none
    local_seq  INTEGER NOT NULL DEFAULT 0    -- order of writes here, for other processes to catch up
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS nodes_parent ON nodes(parent);
CREATE INDEX IF NOT EXISTS nodes_local_seq ON nodes(local_seq);
CREATE TABLE IF NOT EXISTS outbox (     -- records changed here and not sent yet (sync)
    id         BLOB    PRIMARY KEY,
    dirty_mask INTEGER NOT NULL         -- which synced fields changed (crate::hlc::Reg bits)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS purged (     -- records removed for good, and when (crate::hlc::Hlc)
    id BLOB PRIMARY KEY,
    at BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS sync_state ( -- per sync backend: where pulling left off, and the account
    remote  TEXT PRIMARY KEY,
    cursor  BLOB,
    account TEXT
);
CREATE TABLE IF NOT EXISTS bases (      -- the sync backend's version of each record last seen here
    id     BLOB PRIMARY KEY,
    etag   TEXT NOT NULL,
    record TEXT NOT NULL                -- flowie_sync::NodeRecord as JSON
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS lost (       -- text another device's edit won over (conflict copies)
    id    BLOB    NOT NULL,             -- the node
    field TEXT    NOT NULL,             -- 'text' or 'note'
    value TEXT    NOT NULL,             -- what lost
    at    INTEGER NOT NULL              -- when it was noticed (ms)
);
CREATE TABLE IF NOT EXISTS blobs (   -- image files in the image folder
    name    TEXT    PRIMARY KEY,     -- <sha256>.<ext>
    mime    TEXT    NOT NULL,
    width   INTEGER NOT NULL,
    height  INTEGER NOT NULL,
    size    INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS imports (
    root        BLOB    PRIMARY KEY,
    package_id  TEXT    NOT NULL,
    title       TEXT    NOT NULL,
    author_id   TEXT    NOT NULL,
    author_name TEXT    NOT NULL,
    exported_at INTEGER NOT NULL,
    imported_at INTEGER NOT NULL,
    seen_at     INTEGER,
    source_ids  TEXT    NOT NULL DEFAULT '[]'
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS import_bases (   -- each imported bullet as the package had it
    node      BLOB PRIMARY KEY,
    root      BLOB NOT NULL,                -- the imported root it came with
    base_hash TEXT NOT NULL                 -- package::content_hash at the last import
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS meta (   -- this device only (LOCAL_META)
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS prefs (  -- synced preferences (PREFS)
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    at    BLOB NOT NULL DEFAULT x''   -- when it was last written (crate::hlc::Hlc)
);
