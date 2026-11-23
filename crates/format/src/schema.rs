use std::fs;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use serde_json::Value;

use crate::FormatError;
use crate::error::db_error;
use crate::error::io_error;
use crate::error::json_error;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS col (
    id INTEGER PRIMARY KEY,
    crt INTEGER NOT NULL,
    mod INTEGER NOT NULL,
    scm INTEGER NOT NULL,
    ver INTEGER NOT NULL,
    dty INTEGER NOT NULL,
    usn INTEGER NOT NULL,
    ls INTEGER NOT NULL,
    conf TEXT NOT NULL,
    models TEXT NOT NULL,
    decks TEXT NOT NULL,
    dconf TEXT NOT NULL,
    tags TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS notes (
    id INTEGER PRIMARY KEY,
    guid TEXT NOT NULL,
    mid INTEGER NOT NULL,
    mod INTEGER NOT NULL,
    usn INTEGER NOT NULL,
    tags TEXT NOT NULL,
    flds TEXT NOT NULL,
    sfld INTEGER NOT NULL,
    csum INTEGER NOT NULL,
    flags INTEGER NOT NULL,
    data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS cards (
    id INTEGER PRIMARY KEY,
    nid INTEGER NOT NULL,
    did INTEGER NOT NULL,
    ord INTEGER NOT NULL,
    mod INTEGER NOT NULL,
    usn INTEGER NOT NULL,
    type INTEGER NOT NULL,
    queue INTEGER NOT NULL,
    due INTEGER NOT NULL,
    ivl INTEGER NOT NULL,
    factor INTEGER NOT NULL,
    reps INTEGER NOT NULL,
    lapses INTEGER NOT NULL,
    left INTEGER NOT NULL,
    odue INTEGER NOT NULL,
    odid INTEGER NOT NULL,
    flags INTEGER NOT NULL,
    data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS revlog (
    id INTEGER PRIMARY KEY,
    cid INTEGER NOT NULL,
    usn INTEGER NOT NULL,
    ease INTEGER NOT NULL,
    ivl INTEGER NOT NULL,
    lastIvl INTEGER NOT NULL,
    factor INTEGER NOT NULL,
    time INTEGER NOT NULL,
    type INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS graves (
    usn INTEGER NOT NULL,
    oid INTEGER NOT NULL,
    type INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_notes_csum ON notes (csum);
CREATE INDEX IF NOT EXISTS ix_notes_usn ON notes (usn);
CREATE INDEX IF NOT EXISTS ix_cards_nid ON cards (nid);
CREATE INDEX IF NOT EXISTS ix_cards_sched ON cards (did, queue, due);
CREATE INDEX IF NOT EXISTS ix_cards_usn ON cards (usn);
CREATE INDEX IF NOT EXISTS ix_revlog_cid ON revlog (cid);
CREATE INDEX IF NOT EXISTS ix_revlog_usn ON revlog (usn);
";

pub fn ensure_parent(path: &Path) -> Result<(), FormatError> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(io_error(format!(
        "cannot create directory {}",
        parent.display()
    )))
}

/// Opens a collection, creating its schema and `col` row when needed.
///
/// Import callers pass the source collection's creation time. A standalone
/// collection gets the current Unix time.
pub fn open_or_create(
    path: &Path,
    crt: Option<i64>,
) -> Result<Connection, FormatError> {
    ensure_parent(path)?;
    let connection = Connection::open(path).map_err(db_error(format!(
        "cannot open collection {}",
        path.display()
    )))?;
    ranki_storage::register_collations(&connection).map_err(|error| {
        FormatError::with_source("cannot register collection collations", error)
    })?;
    connection
        .execute_batch(SCHEMA)
        .map_err(db_error("cannot initialize collection schema"))?;
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM col", [], |row| row.get(0))
        .map_err(db_error("cannot read collection metadata"))?;
    if count == 0 {
        insert_default_col(&connection, crt)?;
    }
    ranki_storage::drop_legacy_schema11_indexes(&connection).map_err(
        |error| {
            FormatError::with_source(
                "cannot remove legacy collection indexes",
                error,
            )
        },
    )?;
    Ok(connection)
}

fn insert_default_col(
    connection: &Connection,
    crt: Option<i64>,
) -> Result<(), FormatError> {
    let creation_time = crt.map_or_else(current_time, Ok)?;
    let col = ranki_storage::default_col(creation_time, current_millis()?);
    let field = |key: &str| -> Result<&Value, FormatError> {
        col.get(key)
            .ok_or_else(|| FormatError::new("default collection is incomplete"))
    };
    let number = |key: &str| -> Result<i64, FormatError> {
        col.get(key)
            .and_then(Value::as_i64)
            .ok_or_else(|| FormatError::new("default collection is incomplete"))
    };
    connection
        .execute(
            "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags)
             VALUES (1, ?1, ?2, ?3, 11, 0, 0, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                creation_time,
                number("mod")?,
                number("scm")?,
                number("ls")?,
                encode_json(field("conf")?)?,
                encode_json(field("models")?)?,
                encode_json(field("decks")?)?,
                encode_json(field("dconf")?)?,
                encode_json(field("tags")?)?,
            ],
        )
        .map_err(db_error("cannot create collection metadata"))?;
    Ok(())
}

fn current_time() -> Result<i64, FormatError> {
    let elapsed =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                FormatError::with_source("cannot read system clock", error)
            })?;
    i64::try_from(elapsed.as_secs())
        .map_err(|_| FormatError::new("system clock out of range"))
}

fn current_millis() -> Result<i64, FormatError> {
    let elapsed =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                FormatError::with_source("cannot read system clock", error)
            })?;
    i64::try_from(elapsed.as_millis())
        .map_err(|_| FormatError::new("system clock out of range"))
}

pub fn encode_json(value: &Value) -> Result<String, FormatError> {
    serde_json::to_string(value)
        .map_err(json_error("cannot encode collection JSON"))
}
