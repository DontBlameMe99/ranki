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
    sfld TEXT NOT NULL,
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
                encode_json(&default_conf())?,
                encode_json(&default_models())?,
                encode_json(&default_decks())?,
                encode_json(&default_dconf())?,
                encode_json(&Value::Object(serde_json::Map::new()))?,
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

pub fn encode_json(value: &Value) -> Result<String, FormatError> {
    serde_json::to_string(value)
        .map_err(json_error("cannot encode collection JSON"))
}

fn default_conf() -> Value {
    json!({
        "activeDecks": [1],
        "curDeck": 1,
        "curModel": 1,
        "nextPos": 1,
        "sortBackwards": false,
        "sortType": "noteFld",
        "fsrs": true,
        "schedVer": 2,
        "creationOffset": 0,
        "lastFsrsOptimize": 0,
        "loadBalancerEnabled": true,
        "collapseTime": 1200,
        "timeLim": 0,
        "estTimes": true,
        "dueCounts": true,
        "newSpread": 0,
        "dayLearnFirst": false,
        "addToCur": true
    })
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

const FSRS_PARAMS6: [f64; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722,
    0.1666, 0.796, 1.4835, 0.0614, 0.2629, 1.6483, 0.6014, 1.8729, 0.5425,
    0.0912, 0.0658, 0.1542,
];

fn default_dconf() -> Value {
    let config = json!({
        "id": 1,
        "name": "Default",
        "mod": 0,
        "usn": 0,
        "maxTaken": 60,
        "autoplay": true,
        "timer": 0,
        "replayq": true,
        "new": {
            "bury": false,
            "delays": [1.0, 10.0],
            "initialFactor": 2500,
            "ints": [1, 4, 0],
            "order": 1,
            "perDay": 20
        },
        "rev": {
            "bury": false,
            "ease4": 1.3,
            "hardFactor": 1.2,
            "ivlFct": 1.0,
            "maxIvl": 36500,
            "perDay": 200
        },
        "lapse": {
            "delays": [10.0],
            "leechAction": 1,
            "leechFails": 8,
            "minInt": 1,
            "mult": 0.0
        },
        "dyn": false,
        "newMix": 0,
        "newPerDayMinimum": 0,
        "interdayLearningMix": 0,
        "reviewOrder": 0,
        "newSortOrder": 0,
        "newGatherPriority": 0,
        "buryInterdayLearning": false,
        "stopTimerOnAnswer": false,
        "secondsToShowQuestion": 0.0,
        "secondsToShowAnswer": 0.0,
        "questionAction": 0,
        "answerAction": 0,
        "waitForAudio": true,
        "easyDaysPercentages": [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        "fsrsWeights": [],
        "fsrsParams5": [],
        "fsrsParams6": FSRS_PARAMS6,
        "desiredRetention": 0.9,
        "sm2Retention": 0.9,
        "weightSearch": "",
        "ignoreRevlogsBeforeDate": ""
    });
    let mut configs = serde_json::Map::new();
    let _ = configs.insert("1".to_owned(), config);
    Value::Object(configs)
}
