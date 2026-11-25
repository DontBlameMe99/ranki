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

const FSRS_PARAMS6: [f64; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722,
    0.1666, 0.796, 1.4835, 0.0614, 0.2629, 1.6483, 0.6014, 1.8729, 0.5425,
    0.0912, 0.0658, 0.1542,
];

#[expect(
    clippy::too_many_lines,
    reason = "the default collection JSON mirrors Anki's schema 11 defaults"
)]
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
        "dyn": 0,
        "collapsed": false,
        "browserCollapsed": false,
        "lrnToday": [0, 0],
        "revToday": [0, 0],
        "newToday": [0, 0],
        "timeToday": [0, 0],
        "extendNew": 0,
        "extendRev": 0
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

    json!({
        "id": 1_i64,
        "crt": now_secs,
        "mod": now_ms,
        "scm": now_ms,
        "ver": 11_i64,
        "dty": 0_i64,
        "usn": 0_i64,
        "ls": now_ms,
        "conf": json!({"nextPos": 1_i64, "estimateReps": 0_i64, "autoUpdateEnabled": true, "fsrs": true, "schedVer": 2_i64, "creationOffset": 0_i64, "lastFsrsOptimize": 0_i64, "loadBalancerEnabled": true}),
        "models": json!({DEFAULT_MODEL_ID.to_string(): model}),
        "decks": json!({DEFAULT_DECK_ID.to_string(): deck}),
        "dconf": json!({"1": config}),
        "tags": json!({})
    })
}
