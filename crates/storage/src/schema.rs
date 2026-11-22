//! Anki's legacy `collection.anki2` schema and its default collection data.

use ranki_model::DEFAULT_DECK_ID;
use ranki_model::DEFAULT_MODEL_ID;
use serde_json::Value;
use serde_json::json;

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
        oid integer not null,
        type integer not null,
        usn integer not null,
        PRIMARY KEY (oid, type)
    ) WITHOUT ROWID",
    "CREATE INDEX ix_notes_csum ON notes (csum)",
    "CREATE INDEX ix_notes_usn ON notes (usn)",
    "CREATE INDEX idx_notes_mid ON notes (mid)",
    "CREATE INDEX ix_cards_nid ON cards (nid)",
    "CREATE INDEX ix_cards_sched ON cards (did, queue, due)",
    "CREATE INDEX ix_cards_usn ON cards (usn)",
    "CREATE INDEX idx_cards_odid ON cards (odid) WHERE odid != 0",
    "CREATE INDEX ix_revlog_cid ON revlog (cid)",
    "CREATE INDEX ix_revlog_usn ON revlog (usn)",
    "CREATE INDEX idx_graves_pending ON graves (usn)",
];

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

    let deck = json!({
        "id": DEFAULT_DECK_ID,
        "name": "Default",
        "usn": 0,
        "mod": now_secs,
        "conf": 1,
        "desc": "",
        "dyn": 0
    });

    let config = json!({
        "id": 1,
        "name": "Default",
        "usn": 0,
        "mod": now_secs,
        "maxTaken": 60,
        "autoplay": true,
        "timer": 0,
        "replayq": true,
        "new": {"perDay": 20, "delays": [1, 10], "separate": true, "order": 0},
        "rev": {"perDay": 200, "fuzz": 0.05, "maxInterval": 36500},
        "lap": {"ease4": 1.3, "ivlFct": 1.0, "maxIvl": 36500},
        "ease4": 2.5,
        "minSpace": 1,
        "maxIvl": 36500,
        "bury": true,
        "dyn": false,
        "newMix": 0,
        "newPerDayMinimum": 0,
        "interdayLearningMix": 0,
        "buryNew": true,
        "buryRev": true,
        "suspendNew": false,
        "newOrder": 0
    });

    json!({
        "id": 1_i64,
        "crt": now_secs,
        "mod": now_ms,
        "scm": now_ms,
        "ver": 18_i64,
        "dty": 0_i64,
        "usn": 0_i64,
        "ls": now_ms,
        "conf": json!({"nextPos": 1_i64, "estimateReps": 0_i64, "autoUpdateEnabled": true}),
        "models": json!({DEFAULT_MODEL_ID.to_string(): model}),
        "decks": json!({DEFAULT_DECK_ID.to_string(): deck}),
        "dconf": json!({"1": config}),
        "tags": json!({})
    })
}
