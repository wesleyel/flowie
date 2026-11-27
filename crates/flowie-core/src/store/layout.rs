//! A database's layout as SQLite reports it, for the format tests: a database
//! upgraded from each `formats/v*.sql` snapshot must have the layout of a fresh
//! one, and a migration marked `additive` may only add to the layout of the
//! format before it.
//!
//! The layout covers tables (columns, `WITHOUT ROWID`, `STRICT`, foreign keys),
//! indexes (including the automatic ones behind `PRIMARY KEY` and `UNIQUE`,
//! uniqueness, partial or not, key columns), triggers and views. It doesn't see
//! `CHECK` constraints or a column's `COLLATE`, which SQLite only keeps in the
//! `CREATE TABLE` text.

use std::collections::BTreeMap;

use rusqlite::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub decl_type: String,
    pub not_null: bool,
    /// The default as written in the schema, `None` when there is none.
    pub default: Option<String>,
    /// Position in the primary key, from 1; 0 when not part of it.
    pub pk: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKey {
    pub from: String,
    pub table: String,
    pub to: Option<String>,
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub columns: Vec<Column>,
    pub without_rowid: bool,
    pub strict: bool,
    pub foreign_keys: Vec<ForeignKey>,
}

impl Table {
    fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|column| column.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexColumn {
    /// `None` for an expression.
    pub name: Option<String>,
    pub desc: bool,
    pub collation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub table: String,
    pub unique: bool,
    pub partial: bool,
    pub columns: Vec<IndexColumn>,
}

/// A trigger or a view: the table it belongs to and its SQL with whitespace
/// collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub table: String,
    pub sql: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub tables: BTreeMap<String, Table>,
    pub indexes: BTreeMap<String, Index>,
    pub triggers: BTreeMap<String, Definition>,
    pub views: BTreeMap<String, Definition>,
}

impl Layout {
    /// Read the layout of the `main` schema of `conn`.
    pub fn read(conn: &Connection) -> rusqlite::Result<Layout> {
        let mut layout = Layout::default();

        let mut tables = conn.prepare(
            "SELECT name, wr, strict FROM pragma_table_list
             WHERE schema = 'main' AND type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )?;
        let mut rows = tables.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            let table = Table {
                columns: read_columns(conn, &name)?,
                without_rowid: row.get(1)?,
                strict: row.get(2)?,
                foreign_keys: read_foreign_keys(conn, &name)?,
            };
            read_indexes(conn, &name, &mut layout.indexes)?;
            layout.tables.insert(name, table);
        }

        let mut others = conn.prepare("SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('trigger', 'view')")?;
        let mut rows = others.query([])?;
        while let Some(row) = rows.next()? {
            let kind: String = row.get(0)?;
            let name: String = row.get(1)?;
            let sql: String = row.get(3)?;
            let definition = Definition { table: row.get(2)?, sql: collapse_whitespace(&sql) };
            if kind == "trigger" {
                layout.triggers.insert(name, definition);
            } else {
                layout.views.insert(name, definition);
            }
        }
        Ok(layout)
    }

    /// Every way `self` differs from `other`, one line each; empty when the two
    /// are the same.
    pub fn differences(&self, other: &Layout) -> Vec<String> {
        let mut out = Vec::new();
        compare_maps("table", &self.tables, &other.tables, &mut out);
        compare_maps("index", &self.indexes, &other.indexes, &mut out);
        compare_maps("trigger", &self.triggers, &other.triggers, &mut out);
        compare_maps("view", &self.views, &other.views, &mut out);
        out
    }

    /// Why going from `old` to `self` isn't additive: an older build that knows
    /// only `old` could fail or lose data writing to a database laid out as
    /// `self`. Empty when it is additive.
    pub fn additive_problems(&self, old: &Layout) -> Vec<String> {
        let mut out = Vec::new();

        for (name, old_table) in &old.tables {
            let Some(table) = self.tables.get(name) else {
                out.push(format!("table {name} was dropped"));
                continue;
            };
            if table.without_rowid != old_table.without_rowid || table.strict != old_table.strict {
                out.push(format!("table {name} changed WITHOUT ROWID or STRICT"));
            }
            if table.foreign_keys != old_table.foreign_keys {
                out.push(format!("table {name} changed its foreign keys"));
            }
            for old_column in &old_table.columns {
                match table.column(&old_column.name) {
                    None => out.push(format!("column {name}.{} was dropped", old_column.name)),
                    Some(column) if column != old_column => {
                        out.push(format!("column {name}.{} changed: {old_column:?} -> {column:?}", old_column.name))
                    }
                    Some(_) => {}
                }
            }
            for column in &table.columns {
                if old_table.column(&column.name).is_some() {
                    continue;
                }
                if column.not_null && column.default.is_none() {
                    out.push(format!("new column {name}.{} is NOT NULL without a default, so older builds' inserts fail", column.name));
                }
            }
        }

        for (name, table) in &self.tables {
            if old.tables.contains_key(name) {
                continue;
            }
            for key in &table.foreign_keys {
                if old.tables.contains_key(&key.table) {
                    out.push(format!("new table {name} has a foreign key to {}, which older builds don't know about", key.table));
                }
            }
        }

        // Upserts in older builds name their conflict targets, so a unique index
        // they rely on must stay; a new one on an existing table can reject rows
        // they write.
        for (name, old_index) in &old.indexes {
            if !old_index.unique {
                continue;
            }
            match self.indexes.get(name) {
                None => out.push(format!("unique index {name} was dropped")),
                Some(index) if index != old_index => out.push(format!("unique index {name} changed")),
                Some(_) => {}
            }
        }
        for (name, index) in &self.indexes {
            let is_new = !old.indexes.contains_key(name);
            if is_new && index.unique && old.tables.contains_key(&index.table) {
                out.push(format!("new unique index {name} on existing table {}", index.table));
            }
        }

        // A trigger runs on older builds' writes too, which don't expect it.
        for (name, trigger) in &self.triggers {
            match old.triggers.get(name) {
                None => out.push(format!("new trigger {name}")),
                Some(old_trigger) if old_trigger != trigger => out.push(format!("trigger {name} changed")),
                Some(_) => {}
            }
        }
        for name in old.triggers.keys() {
            if !self.triggers.contains_key(name) {
                out.push(format!("trigger {name} was dropped"));
            }
        }
        for (name, old_view) in &old.views {
            match self.views.get(name) {
                None => out.push(format!("view {name} was dropped")),
                Some(view) if view != old_view => out.push(format!("view {name} changed")),
                Some(_) => {}
            }
        }

        out
    }
}

fn read_columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<Column>> {
    let mut stmt = conn.prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid")?;
    let mut rows = stmt.query([table])?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        columns.push(Column { name: row.get(0)?, decl_type: row.get(1)?, not_null: row.get(2)?, default: row.get(3)?, pk: row.get(4)? });
    }
    Ok(columns)
}

fn read_foreign_keys(conn: &Connection, table: &str) -> rusqlite::Result<Vec<ForeignKey>> {
    let mut stmt =
        conn.prepare("SELECT \"from\", \"table\", \"to\", on_update, on_delete FROM pragma_foreign_key_list(?1) ORDER BY id, seq")?;
    let mut rows = stmt.query([table])?;
    let mut keys = Vec::new();
    while let Some(row) = rows.next()? {
        keys.push(ForeignKey { from: row.get(0)?, table: row.get(1)?, to: row.get(2)?, on_update: row.get(3)?, on_delete: row.get(4)? });
    }
    Ok(keys)
}

fn read_indexes(conn: &Connection, table: &str, into: &mut BTreeMap<String, Index>) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT name, \"unique\", partial FROM pragma_index_list(?1)")?;
    let mut rows = stmt.query([table])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let index = Index { table: table.to_owned(), unique: row.get(1)?, partial: row.get(2)?, columns: read_index_columns(conn, &name)? };
        into.insert(name, index);
    }
    Ok(())
}

fn read_index_columns(conn: &Connection, index: &str) -> rusqlite::Result<Vec<IndexColumn>> {
    // Only the key columns: a WITHOUT ROWID table's indexes also list the
    // primary key columns they carry, and other indexes the rowid.
    let mut stmt = conn.prepare("SELECT name, \"desc\", coll FROM pragma_index_xinfo(?1) WHERE key = 1 ORDER BY seqno")?;
    let mut rows = stmt.query([index])?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        columns.push(IndexColumn { name: row.get(0)?, desc: row.get(1)?, collation: row.get(2)? });
    }
    Ok(columns)
}

fn collapse_whitespace(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut pending_space = false;
    for ch in sql.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Add a line to `out` for each name only one side has and each one whose
/// value differs.
fn compare_maps<T: PartialEq + std::fmt::Debug>(
    kind: &str,
    actual: &BTreeMap<String, T>,
    expected: &BTreeMap<String, T>,
    out: &mut Vec<String>,
) {
    for (name, value) in actual {
        match expected.get(name) {
            None => out.push(format!("unexpected {kind} {name}")),
            Some(wanted) if wanted != value => out.push(format!("{kind} {name} is {value:?}, expected {wanted:?}")),
            Some(_) => {}
        }
    }
    for name in expected.keys() {
        if !actual.contains_key(name) {
            out.push(format!("missing {kind} {name}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_of(sql: &str) -> Layout {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(sql).unwrap();
        Layout::read(&conn).unwrap()
    }

    const OLD: &str = "
        CREATE TABLE nodes (id BLOB PRIMARY KEY, text TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    ";

    #[test]
    fn reads_what_the_format_tests_compare() {
        let layout = layout_of(
            "CREATE TABLE nodes (id BLOB PRIMARY KEY, parent BLOB NOT NULL, rank TEXT NOT NULL DEFAULT 'a0') WITHOUT ROWID;
             CREATE UNIQUE INDEX nodes_rank ON nodes(parent, rank DESC) WHERE rank != '';
             CREATE TRIGGER nodes_touch AFTER UPDATE ON nodes BEGIN SELECT 1; END;",
        );
        let nodes = &layout.tables["nodes"];
        assert!(nodes.without_rowid);
        assert_eq!(nodes.columns[2].default.as_deref(), Some("'a0'"));
        let index = &layout.indexes["nodes_rank"];
        assert!(index.unique && index.partial);
        assert_eq!(index.columns.len(), 2, "only the key columns, not the primary key it carries");
        assert!(index.columns[1].desc);
        assert!(layout.indexes.contains_key("sqlite_autoindex_nodes_1"));
        assert_eq!(layout.triggers["nodes_touch"].sql, "CREATE TRIGGER nodes_touch AFTER UPDATE ON nodes BEGIN SELECT 1; END");
    }

    #[test]
    fn differences_name_what_changed() {
        let old = layout_of(OLD);
        assert!(old.differences(&layout_of(OLD)).is_empty());
        // The same columns, but a rowid table.
        let rowid = layout_of(
            "CREATE TABLE nodes (id BLOB PRIMARY KEY, text TEXT NOT NULL);
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        );
        let differences = rowid.differences(&old);
        assert!(differences.iter().any(|d| d.starts_with("table nodes is")), "{differences:?}");
    }

    #[test]
    fn adding_tables_and_columns_with_defaults_is_additive() {
        let new = layout_of(&format!(
            "{OLD}
             ALTER TABLE nodes ADD COLUMN note TEXT NOT NULL DEFAULT '';
             ALTER TABLE nodes ADD COLUMN deleted_at INTEGER;
             CREATE INDEX nodes_text ON nodes(text);
             CREATE TABLE outbox (id BLOB PRIMARY KEY, mask INTEGER NOT NULL) WITHOUT ROWID;"
        ));
        assert_eq!(new.additive_problems(&layout_of(OLD)), Vec::<String>::new());
    }

    #[test]
    fn breaking_changes_are_not_additive() {
        let old = layout_of(OLD);
        let cases = [
            ("ALTER TABLE nodes DROP COLUMN text", "column nodes.text was dropped"),
            ("DROP TABLE meta", "table meta was dropped"),
            ("CREATE UNIQUE INDEX nodes_text ON nodes(text)", "new unique index nodes_text"),
            ("CREATE TRIGGER t AFTER INSERT ON meta BEGIN SELECT 1; END", "new trigger t"),
        ];
        for (change, problem) in cases {
            let problems = layout_of(&format!("{OLD}{change};")).additive_problems(&old);
            assert!(problems.iter().any(|p| p.starts_with(problem)), "{change}: {problems:?}");
        }
        // SQLite refuses `ADD COLUMN … NOT NULL` without a default, so this
        // only happens when a migration rebuilds the table.
        let rebuilt = layout_of(
            "CREATE TABLE nodes (id BLOB PRIMARY KEY, text TEXT NOT NULL, kind INTEGER NOT NULL) WITHOUT ROWID;
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        );
        let problems = rebuilt.additive_problems(&old);
        assert!(problems.iter().any(|p| p.starts_with("new column nodes.kind is NOT NULL")), "{problems:?}");
    }
}
