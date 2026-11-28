use std::fs;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use serde_json::Value;
use serde_json::json;

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
CREATE INDEX IF NOT EXISTS ix_notes_guid ON notes (guid);
CREATE INDEX IF NOT EXISTS ix_cards_nid ON cards (nid);
CREATE INDEX IF NOT EXISTS ix_revlog_cid ON revlog (cid);
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
    let mut connection = Connection::open(path).map_err(db_error(format!(
        "cannot open collection {}",
        path.display()
    )))?;
    ranki_storage::register_collations(&connection).map_err(|error| {
        FormatError::with_source("cannot register collection collations", error)
    })?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(db_error("cannot configure collection journal"))?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(db_error("cannot configure collection synchronization"))?;
    let transaction = connection
        .transaction()
        .map_err(db_error("cannot initialize collection schema"))?;
    transaction
        .execute_batch(SCHEMA)
        .map_err(db_error("cannot initialize collection schema"))?;
    let count: i64 = transaction
        .query_row("SELECT COUNT(*) FROM col", [], |row| row.get(0))
        .map_err(db_error("cannot read collection metadata"))?;
    if count == 0 {
        insert_default_col(&transaction, crt)?;
    }
    transaction
        .commit()
        .map_err(db_error("cannot commit collection schema"))?;
    Ok(connection)
}

fn insert_default_col(
    connection: &Connection,
    crt: Option<i64>,
) -> Result<(), FormatError> {
    let creation_time = crt.map_or_else(current_time, Ok)?;
    connection
        .execute(
            "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags)
             VALUES (1, ?1, 0, 0, 11, 0, 0, 0, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                creation_time,
                encode_json(&ranki_storage::schema11_default_conf(Some(0)))?,
                encode_json(&default_models())?,
                encode_json(&default_decks())?,
                encode_json(&ranki_storage::schema11_default_deck_config(0))?,
                encode_json(&Value::Object(serde_json::Map::new()))?,
            ],
        )
        .map_err(db_error("cannot create collection metadata"))?;
    Ok(())
}

pub fn current_time() -> Result<i64, FormatError> {
    let elapsed =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                FormatError::with_source("cannot read system clock", error)
            })?;
    i64::try_from(elapsed.as_secs())
        .map_err(|_| FormatError::new("system clock out of range"))
}

pub fn encode_json(value: &Value) -> Result<String, FormatError> {
    serde_json::to_string(value)
        .map_err(json_error("cannot encode collection JSON"))
}

fn default_models() -> Value {
    let model = json!({
        "id": 1,
        "name": "Basic",
        "type": 0,
        "mod": 0,
        "usn": 0,
        "sortf": 0,
        "did": 1,
        "tmpls": [{
            "name": "Card 1",
            "qfmt": "{{Front}}",
            "afmt": "{{FrontSide}}\n<hr id=answer>\n{{Back}}",
            "bqfmt": "",
            "bafmt": "",
            "did": null,
            "ord": 0,
            "bfont": "Arial",
            "bsize": 0,
            "id": 1000
        }],
        "flds": [
            {"name": "Front", "ord": 0, "sticky": false, "rtl": false, "font": "Arial", "size": 20, "id": 1000},
            {"name": "Back", "ord": 1, "sticky": false, "rtl": false, "font": "Arial", "size": 20, "id": 1001},
        ],
        "css": ".card { font-family: arial; font-size: 20px; text-align: center; }",
        "latexPre": "",
        "latexPost": "",
        "latexSvg": false,
        "req": [[0, "any", [0]]],
    });
    let mut models = serde_json::Map::new();
    models.insert("1".to_owned(), model);
    Value::Object(models)
}

fn default_decks() -> Value {
    let deck = json!({
        "id": 1,
        "name": "Default",
        "mod": 0,
        "usn": 0,
        "conf": 1,
        "desc": "",
        "dyn": 0,
        "collapsed": false,
        "browserCollapsed": false,
        "lrnToday": [0, 0],
        "revToday": [0, 0],
        "newToday": [0, 0],
        "timeToday": [0, 0],
        "extendNew": 0,
        "extendRev": 0,
    });
    let mut decks = serde_json::Map::new();
    decks.insert("1".to_owned(), deck);
    Value::Object(decks)
}

#[cfg(test)]
mod tests {
    use super::open_or_create;

    #[test]
    fn created_collections_use_integer_sort_field_and_shared_defaults() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("collection.anki2");
        let connection =
            open_or_create(&path, Some(1_700_000_000)).expect("create");
        let affinity: String = connection
            .prepare("PRAGMA table_info(notes)")
            .expect("prepare")
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .expect("query")
            .filter_map(Result::ok)
            .find_map(|(name, kind)| (name == "sfld").then_some(kind))
            .expect("sfld column");
        assert_eq!(affinity, "INTEGER");
        let conf: String = connection
            .query_row("SELECT conf FROM col WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("conf");
        let parsed: serde_json::Value =
            serde_json::from_str(&conf).expect("JSON");
        assert_eq!(parsed, ranki_storage::schema11_default_conf(Some(0)));
    }
}
