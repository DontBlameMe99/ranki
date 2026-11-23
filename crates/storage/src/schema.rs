//! Anki's legacy `collection.anki2` schema and its default collection data.

use ranki_model::DEFAULT_DECK_ID;
use ranki_model::DEFAULT_MODEL_ID;
use ranki_model::StoreError;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use serde_json::Value;
use serde_json::json;

use crate::records::deck_config_json;

pub const CREATE_STATEMENTS: &[&str] = &[
    "CREATE TABLE col (
        id integer primary key,
        crt integer not null,
        mod integer not null,
        scm integer not null,
        ver integer not null,
        dty integer not null,
        usn integer not null,
        ls integer not null,
        conf text not null,
        models text not null,
        decks text not null,
        dconf text not null,
        tags text not null
    )",
    "CREATE TABLE notes (
        id integer primary key,
        guid text not null,
        mid integer not null,
        mod integer not null,
        usn integer not null,
        tags text not null,
        flds text not null,
        sfld integer not null,
        csum integer not null,
        flags integer not null,
        data text not null
    )",
    "CREATE TABLE cards (
        id integer primary key,
        nid integer not null,
        did integer not null,
        ord integer not null,
        mod integer not null,
        usn integer not null,
        type integer not null,
        queue integer not null,
        due integer not null,
        ivl integer not null,
        factor integer not null,
        reps integer not null,
        lapses integer not null,
        left integer not null,
        odue integer not null,
        odid integer not null,
        flags integer not null,
        data text not null
    )",
    "CREATE TABLE revlog (
        id integer primary key,
        cid integer not null,
        usn integer not null,
        ease integer not null,
        ivl integer not null,
        lastIvl integer not null,
        factor integer not null,
        time integer not null,
        type integer not null
    )",
    "CREATE TABLE graves (
        usn integer not null,
        oid integer not null,
        type integer not null
    )",
    "CREATE INDEX ix_notes_csum ON notes (csum)",
    "CREATE INDEX ix_notes_usn ON notes (usn)",
    "CREATE INDEX ix_cards_nid ON cards (nid)",
    "CREATE INDEX ix_cards_sched ON cards (did, queue, due)",
    "CREATE INDEX ix_cards_usn ON cards (usn)",
    "CREATE INDEX ix_revlog_cid ON revlog (cid)",
    "CREATE INDEX ix_revlog_usn ON revlog (usn)",
];

#[must_use]
pub fn default_col(now_secs: i64, now_ms: i64) -> Value {
    let model = json!({
        "id": DEFAULT_MODEL_ID,
        "name": "Basic",
        "type": 0,
        "mod": now_secs,
        "usn": 0,
        "sortf": 0,
        "did": DEFAULT_DECK_ID,
        "latexPre": "\\documentclass[12pt]{article}\n\\special{papersize=3in,5in}\n\\usepackage[utf8]{inputenc}\n\\pagestyle{empty}\n\\setlength{\\parindent}{0in}\n\\begin{document}\n",
        "latexPost": "\\end{document}",
        "tmpls": [{
            "name": "Card 1",
            "ord": 0,
            "qfmt": "{{Front}}",
            "afmt": "{{FrontSide}}\n\n<hr id=answer>\n\n{{Back}}",
            "bqfmt": "",
            "bafmt": "",
            "did": null,
            "bfc": 0,
            "id": 1_234_567_890_i64
        }],
        "flds": [
            {"name": "Front", "ord": 0, "sticky": false, "rtl": false, "font": "Arial", "size": 20},
            {"name": "Back", "ord": 1, "sticky": false, "rtl": false, "font": "Arial", "size": 20}
        ],
        "css": ".card {\n font-family: arial;\n font-size: 20px;\n text-align: center;\n color: black;\n background-color: white;\n}\n",
        "req": [[0, "any", [0]]]
    });

    let deck = default_deck(DEFAULT_DECK_ID, "Default", now_secs, 0);
    let config = default_deck_config(1, "Default", now_secs, 0);

    json!({
        "id": 1_i64,
        "crt": now_secs,
        "mod": now_ms,
        "scm": now_ms,
        "ver": 11_i64,
        "dty": 0_i64,
        "usn": 0_i64,
        "ls": now_ms,
        "conf": json!({
            "activeDecks": [DEFAULT_DECK_ID],
            "curDeck": DEFAULT_DECK_ID,
            "newSpread": 0,
            "collapseTime": 1200,
            "timeLim": 0,
            "estTimes": true,
            "dueCounts": true,
            "curModel": Value::Null,
            "nextPos": 1_i64,
            "sortType": "noteFld",
            "sortBackwards": false,
            "addToCur": true,
            "dayLearnFirst": false,
            "schedVer": 2,
            "creationOffset": Value::Null,
            "sched2021": true,
        }),
        "models": json!({DEFAULT_MODEL_ID.to_string(): model}),
        "decks": json!({DEFAULT_DECK_ID.to_string(): deck}),
        "dconf": json!({"1": config}),
        "tags": json!({})
    })
}

/// A schema-11 deck entry with every field Anki's serde model requires.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "id, name, clock and sync stamp mirror the stored columns"
)]
pub fn default_deck(id: i64, name: &str, now_secs: i64, usn: i32) -> Value {
    json!({
        "id": id,
        "name": name,
        "usn": usn,
        "mod": now_secs,
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
    })
}

/// A schema-11 deck config with Anki's required fields and defaults.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "id, name, clock and sync stamp mirror the stored columns"
)]
pub fn default_deck_config(
    id: i64,
    name: &str,
    now_secs: i64,
    usn: i32,
) -> Value {
    let mut config = deck_config_json(&[]);
    if let Some(object) = config.as_object_mut() {
        let _ = object.insert(String::from("id"), Value::from(id));
        let _ = object.insert(String::from("name"), Value::from(name));
        let _ = object.insert(String::from("mod"), Value::from(now_secs));
        let _ = object.insert(String::from("usn"), Value::from(usn));
        let _ = object.insert(String::from("waitForAudio"), Value::Bool(true));
        if let Some(new) = object.get_mut("new").and_then(Value::as_object_mut)
        {
            let _ = new.insert(String::from("delays"), json!([1.0, 10.0]));
            let _ = new.insert(String::from("perDay"), json!(20));
        }
        if let Some(rev) = object.get_mut("rev").and_then(Value::as_object_mut)
        {
            let _ = rev.insert(String::from("perDay"), json!(200));
        }
        if let Some(lapse) =
            object.get_mut("lapse").and_then(Value::as_object_mut)
        {
            let _ = lapse.insert(String::from("delays"), json!([10.0]));
            let _ = lapse.insert(String::from("leechAction"), json!(1));
        }
        let _ = object.insert(
            String::from("easyDaysPercentages"),
            json!([1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
        );
    }
    config
}

/// Drops indexes a previous Ranki release wrote into schema 11.
///
/// Those index names belong to Anki's schema 15/18 upgrades, and Anki's
/// upgrade fails when it tries to create them again.
///
/// # Errors
///
/// Returns an error when the schema version or index list cannot be read.
pub fn drop_legacy_schema11_indexes(
    connection: &Connection,
) -> Result<(), StoreError> {
    let (version, legacy): (Option<i64>, i64) = connection
        .query_row(
            "SELECT ver, (SELECT COUNT(*) FROM sqlite_master WHERE type = \
             'index' AND name IN ('idx_notes_mid', 'idx_cards_odid', \
             'idx_graves_pending')) FROM col WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| {
            StoreError::Backend(format!("cannot read schema version: {error}"))
        })?
        .unwrap_or((None, 0));
    if version != Some(11) || legacy == 0 {
        return Ok(());
    }
    connection
        .execute_batch(
            "DROP INDEX IF EXISTS idx_notes_mid;
             DROP INDEX IF EXISTS idx_cards_odid;
             DROP INDEX IF EXISTS idx_graves_pending;",
        )
        .map_err(|error| {
            StoreError::Backend(format!(
                "cannot remove legacy schema indexes: {error}"
            ))
        })
}
