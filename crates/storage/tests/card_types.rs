#![expect(clippy::expect_used, reason = "test fixtures panic when setup fails")]
//! Card-type compatibility tests for decoded note types and card generation.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use ranki_model::NewNote;
use ranki_model::NoteKind;
use ranki_model::StockModelKind;
use ranki_model::Store;
use ranki_model::parse_image_occlusion_field;
use ranki_storage::Collection;
use ranki_storage::register_collations;
use rusqlite::Connection;
use rusqlite::params;
use rusqlite::types::Value as SqlValue;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

const SCHEMA18: &str = "
CREATE TABLE col (
    id INTEGER PRIMARY KEY, crt INTEGER NOT NULL, mod INTEGER NOT NULL,
    scm INTEGER NOT NULL, ver INTEGER NOT NULL, dty INTEGER NOT NULL,
    usn INTEGER NOT NULL, ls INTEGER NOT NULL, conf TEXT NOT NULL,
    models TEXT NOT NULL, decks TEXT NOT NULL, dconf TEXT NOT NULL,
    tags TEXT NOT NULL
);
CREATE TABLE notetypes (
    id INTEGER PRIMARY KEY, name TEXT NOT NULL COLLATE unicase,
    mtime_secs INTEGER NOT NULL, usn INTEGER NOT NULL, config BLOB NOT NULL
);
CREATE TABLE fields (
    ntid INTEGER NOT NULL, ord INTEGER NOT NULL, name TEXT NOT NULL COLLATE unicase,
    config BLOB NOT NULL, PRIMARY KEY (ntid, ord)
) WITHOUT ROWID;
CREATE TABLE templates (
    ntid INTEGER NOT NULL, ord INTEGER NOT NULL, name TEXT NOT NULL COLLATE unicase,
    mtime_secs INTEGER NOT NULL, usn INTEGER NOT NULL, config BLOB NOT NULL,
    PRIMARY KEY (ntid, ord)
) WITHOUT ROWID;
CREATE TABLE notes (
    id INTEGER PRIMARY KEY, guid TEXT NOT NULL, mid INTEGER NOT NULL,
    mod INTEGER NOT NULL, usn INTEGER NOT NULL, tags TEXT NOT NULL,
    flds TEXT NOT NULL, sfld INTEGER NOT NULL, csum INTEGER NOT NULL,
    flags INTEGER NOT NULL, data TEXT NOT NULL
);
CREATE TABLE cards (
    id INTEGER PRIMARY KEY, nid INTEGER NOT NULL, did INTEGER NOT NULL,
    ord INTEGER NOT NULL, mod INTEGER NOT NULL, usn INTEGER NOT NULL,
    type INTEGER NOT NULL, queue INTEGER NOT NULL, due INTEGER NOT NULL,
    ivl INTEGER NOT NULL, factor INTEGER NOT NULL, reps INTEGER NOT NULL,
    lapses INTEGER NOT NULL, left INTEGER NOT NULL, odue INTEGER NOT NULL,
    odid INTEGER NOT NULL, flags INTEGER NOT NULL, data TEXT NOT NULL
);
CREATE TABLE revlog (
    id INTEGER PRIMARY KEY, cid INTEGER NOT NULL, usn INTEGER NOT NULL,
    ease INTEGER NOT NULL, ivl INTEGER NOT NULL, lastIvl INTEGER NOT NULL,
    factor INTEGER NOT NULL, time INTEGER NOT NULL, type INTEGER NOT NULL
);
CREATE TABLE graves (usn INTEGER NOT NULL, oid INTEGER NOT NULL, type INTEGER NOT NULL);
";

fn fixture() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("collection.anki2");
    Collection::create(&path).expect("create");
    (dir, path)
}

fn schema18(path: &Path) -> Connection {
    let _ = fs::remove_file(path);
    let conn = Connection::open(path).expect("open");
    register_collations(&conn).expect("collations");
    conn.execute_batch(SCHEMA18).expect("schema");
    conn.execute(
        "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags)
         VALUES (1, 0, 0, 0, 18, 0, 0, 0, ?1, '', ?2, '{}', '{}')",
        params![
            json!({"nextPos": 1}).to_string(),
            json!({"1": {"id": 1, "name": "Default", "conf": 1}}).to_string(),
        ],
    )
    .expect("col");
    conn
}

fn pb_varint(field: u32, value: u64) -> Vec<u8> {
    let mut bytes = vec![u8::try_from(field.saturating_mul(8)).expect("tag")];
    let mut rest = value;
    while rest >= 0x80 {
        let byte = u8::try_from(rest & 0x7f).expect("byte") | 0x80;
        bytes.push(byte);
        rest >>= 7;
    }
    bytes.push(u8::try_from(rest).expect("byte"));
    bytes
}

fn pb_length(value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut rest = value;
    while rest >= 0x80 {
        bytes.push(u8::try_from(rest & 0x7f).expect("byte") | 0x80);
        rest >>= 7;
    }
    bytes.push(u8::try_from(rest).expect("byte"));
    bytes
}

fn pb_string(field: u32, text: &str) -> Vec<u8> {
    let mut bytes = vec![
        u8::try_from(field.saturating_mul(8).saturating_add(2)).expect("tag"),
    ];
    bytes.extend_from_slice(&pb_length(
        u64::try_from(text.len()).expect("length"),
    ));
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

#[expect(
    clippy::too_many_arguments,
    reason = "fixture rows mirror the normalized Anki tables"
)]
fn insert_model(
    conn: &Connection,
    id: i64,
    name: &str,
    config: Vec<u8>,
    fields: &[(&str, Vec<u8>)],
    templates: &[(&str, Vec<u8>)],
) {
    conn.execute(
        "INSERT INTO notetypes (id, name, mtime_secs, usn, config) VALUES (?1, ?2, 0, 0, ?3)",
        params![id, name, SqlValue::Blob(config)],
    )
    .expect("notetype");
    for (ord, (field, config)) in fields.iter().enumerate() {
        conn.execute(
            "INSERT INTO fields (ntid, ord, name, config) VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                i64::try_from(ord).expect("ord"),
                field,
                SqlValue::Blob(config.clone())
            ],
        )
        .expect("field");
    }
    for (ord, (name, config)) in templates.iter().enumerate() {
        conn.execute(
            "INSERT INTO templates (ntid, ord, name, mtime_secs, usn, config)
             VALUES (?1, ?2, ?3, 0, 0, ?4)",
            params![
                id,
                i64::try_from(ord).expect("ord"),
                name,
                SqlValue::Blob(config.clone())
            ],
        )
        .expect("template");
    }
}

fn ordinals(store: &Collection, ids: &[i64]) -> Vec<u16> {
    let mut values = ids
        .iter()
        .map(|id| store.card(*id).expect("card").expect("exists").ordinal)
        .collect::<Vec<_>>();
    values.sort_unstable();
    values
}

#[test]
fn stock_models_are_installed_with_fresh_ids() {
    let (_dir, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let first = store.ensure_model(StockModelKind::Cloze).expect("install");
    assert_ne!(first.id, 0);
    assert_eq!(first.kind, NoteKind::Cloze);
    let again = store.ensure_model(StockModelKind::Cloze).expect("existing");
    assert_eq!(again.id, first.id);
    let models = store.models().expect("models");
    let installed = models
        .iter()
        .find(|model| model.id == first.id)
        .expect("stored");
    assert_eq!(installed.fields, ["Text", "Back Extra"]);
    assert_eq!(installed.field_tags, [Some(0), Some(1)]);
}

#[test]
fn reversed_stock_model_creates_two_cards_with_explicit_ordinals() {
    let (_dir, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let model = store
        .ensure_model(StockModelKind::BasicAndReversed)
        .expect("model");
    let note = NewNote {
        model_id: model.id,
        deck_id: 1,
        fields: vec!["front".into(), "back".into()],
        tags: Vec::new(),
    };
    let ids = store.add_note(&note).expect("add");
    assert_eq!(ids.len(), 2);
    assert_eq!(ordinals(&store, &ids), [0, 1]);
}

#[test]
fn cloze_generates_one_card_per_distinct_positive_ordinal() {
    let (_dir, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let model = store.ensure_model(StockModelKind::Cloze).expect("model");
    let note = NewNote {
        model_id: model.id,
        deck_id: 1,
        fields: vec![
            "{{c3::three}} {{c1::one::hint}} {{c0::never}}".into(),
            "extra".into(),
        ],
        tags: Vec::new(),
    };
    let ids = store.add_note(&note).expect("add");
    assert_eq!(ids.len(), 2);
    assert_eq!(ordinals(&store, &ids), [0, 2]);
}

#[test]
fn cloze_positions_follow_the_generated_cards_not_the_ordinals() {
    let (_dir, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let model = store.ensure_model(StockModelKind::Cloze).expect("model");
    let high = NewNote {
        model_id: model.id,
        deck_id: 1,
        fields: vec!["{{c3::only}}".into(), "extra".into()],
        tags: Vec::new(),
    };
    let low = NewNote {
        fields: vec!["{{c1::first}}".into(), "extra".into()],
        ..high.clone()
    };
    let first = store.add_note(&high).expect("add");
    assert_eq!(ordinals(&store, &first), [2]);
    let second = store.add_note(&low).expect("add");
    assert_eq!(ordinals(&store, &second), [0]);
    let first_card = store.card(first[0]).expect("card").expect("stored");
    let second_card = store.card(second[0]).expect("card").expect("stored");
    assert_eq!(second_card.due, first_card.due.saturating_add(1));
}

#[test]
fn image_occlusion_markers_survive_field_encoding() {
    let (_dir, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let model = store
        .ensure_model(StockModelKind::ImageOcclusion)
        .expect("model");
    let occlusion = concat!(
        "{{c1::image-occlusion:rect:left=.1:top=.2:width=.3:height=.4}}<br>",
        "{{c2::image-occlusion:text:text=label:fs=14}}",
    )
    .to_owned();
    let note = NewNote {
        model_id: model.id,
        deck_id: 1,
        fields: vec![
            occlusion.clone(),
            "<img src=\"diagram.png\">".into(),
            String::new(),
            String::new(),
            String::new(),
        ],
        tags: Vec::new(),
    };
    let ids = store.add_note(&note).expect("add");
    assert_eq!(ordinals(&store, &ids), [0, 1]);
    let note_id = store.card(ids[0]).expect("card").expect("exists").note_id;
    let stored = store.note(note_id).expect("note").expect("exists");
    assert_eq!(stored.fields[0], occlusion);
    assert_eq!(stored.fields[1], "<img src=\"diagram.png\">");
    let parsed = parse_image_occlusion_field(&stored.fields[0]);
    assert_eq!(parsed.shapes.len(), 2);
}

#[test]
fn every_stock_kind_installs_once_and_persists() {
    let (_dir, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let mut ids = Vec::new();
    for kind in StockModelKind::ALL {
        let model = store.ensure_model(kind).expect("install");
        assert!(model.id > 0);
        assert_eq!(model.name, kind.name());
        assert_eq!(store.ensure_model(kind).expect("existing").id, model.id);
        ids.push(model.id);
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4);
    let names = store
        .models()
        .expect("models")
        .into_iter()
        .map(|model| model.name)
        .collect::<Vec<_>>();
    for kind in StockModelKind::ALL {
        assert!(names.iter().any(|name| name == kind.name()));
    }
}

#[test]
fn schema18_destination_gets_notetype_table_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("modern-install.anki2");
    drop(schema18(&path));
    let mut store = Collection::open(&path).expect("open");
    let model = store
        .ensure_model(StockModelKind::ImageOcclusion)
        .expect("install");
    let stored = store
        .models()
        .expect("models")
        .into_iter()
        .find(|entry| entry.id == model.id)
        .expect("stored");
    assert!(stored.is_image_occlusion());
    assert_eq!(stored.fields.len(), 5);
    assert_eq!(
        stored.field_tags,
        [Some(0), Some(1), Some(2), Some(3), Some(4)]
    );
    assert_eq!(stored.templates.len(), 1);
    assert!(
        stored.templates[0]
            .question_format
            .contains("{{cloze:Occlusion}}")
    );
    assert_eq!(
        store
            .ensure_model(StockModelKind::ImageOcclusion)
            .expect("again")
            .id,
        model.id
    );

    let conn = Connection::open(&path).expect("open");
    register_collations(&conn).expect("collations");
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM notetypes", [], |row| row.get(0))
        .expect("count");
    assert_eq!(rows, 1);
    let fields: i64 = conn
        .query_row("SELECT COUNT(*) FROM fields", [], |row| row.get(0))
        .expect("count");
    assert_eq!(fields, 5);
    let models: String = conn
        .query_row("SELECT models FROM col WHERE id = 1", [], |row| row.get(0))
        .expect("models");
    assert_eq!(models, "");
}

#[test]
fn schema18_models_keep_kind_stock_kind_tags_ords_and_raw_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("modern.anki2");
    let conn = schema18(&path);
    let mut config = pb_varint(1, 1);
    config.extend(pb_varint(9, 5));
    config.extend(pb_string(3, ".cloze { color: blue; }"));
    insert_model(
        &conn,
        42,
        "Cloze",
        config.clone(),
        &[("Text", Vec::new()), ("Back Extra", Vec::new())],
        &[("Cloze", {
            let mut blob = pb_string(1, "{{cloze:Text}}");
            blob.extend(pb_string(2, "{{cloze:Text}}<br>{{Back Extra}}"));
            blob
        })],
    );
    drop(conn);

    let store = Collection::open(&path).expect("open");
    let models = store.models().expect("models");
    let model = models.iter().find(|model| model.id == 42).expect("model");
    assert_eq!(model.kind, NoteKind::Cloze);
    assert_eq!(model.original_stock_kind, Some(5));
    assert_eq!(model.field_tags, [Some(0), Some(1)]);
    assert_eq!(model.templates[0].ord, 0);
    assert_eq!(model.css, ".cloze { color: blue; }");
    assert_eq!(model.raw_config.as_deref(), Some(config.as_slice()));
}

#[test]
fn cloze_tags_keep_the_extra_field_when_the_text_field_is_not_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cloze-order.anki2");
    let conn = schema18(&path);
    insert_model(
        &conn,
        7,
        "Cloze",
        {
            let mut config = pb_varint(1, 1);
            config.extend(pb_string(3, ".cloze {}"));
            config
        },
        &[("Back Extra", Vec::new()), ("Text", Vec::new())],
        &[("Cloze", {
            let mut blob = pb_string(1, "{{cloze:Text}}");
            blob.extend(pb_string(2, "{{cloze:Text}}<br>{{Back Extra}}"));
            blob
        })],
    );
    drop(conn);

    let store = Collection::open(&path).expect("open");
    let models = store.models().expect("models");
    let model = models.iter().find(|model| model.id == 7).expect("model");
    assert_eq!(model.fields, ["Back Extra", "Text"]);
    assert_eq!(model.field_tags, [Some(1), Some(0)]);
}

#[test]
fn model_with_no_templates_adds_a_note_without_crashing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("worldflags.anki2");
    let conn = schema18(&path);
    insert_model(
        &conn,
        1_342_703_682_212,
        "Basic_",
        pb_string(3, ".card {}"),
        &[("Front", Vec::new()), ("Back", Vec::new())],
        &[],
    );
    drop(conn);

    let mut store = Collection::open(&path).expect("open");
    let model = store
        .models()
        .expect("models")
        .into_iter()
        .find(|model| model.name == "Basic_")
        .expect("Basic_");
    assert_eq!(model.templates, [] as [ranki_model::CardTemplate; 0]);
    let note = NewNote {
        model_id: model.id,
        deck_id: 1,
        fields: vec!["flag".into(), "answer".into()],
        tags: Vec::new(),
    };
    assert_eq!(store.add_note(&note).expect("add"), [] as [i64; 0]);
}

#[test]
fn models_to_json_keeps_raw_config_for_export() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("export.anki2");
    let conn = schema18(&path);
    let mut config = pb_varint(1, 0);
    config.extend(pb_string(3, ".card {}"));
    insert_model(
        &conn,
        7,
        "Basic",
        config.clone(),
        &[("Front", Vec::new()), ("Back", Vec::new())],
        &[("Card 1", pb_string(1, "{{Front}}"))],
    );
    drop(conn);

    let conn = Connection::open(&path).expect("reopen");
    register_collations(&conn).expect("collations");
    let models = ranki_storage::models_from_tables_with_prefix(&conn, "main")
        .expect("models");
    let json = ranki_storage::models_to_json(&models);
    let entry = json.get("7").expect("entry");
    assert_eq!(entry.get("type").and_then(Value::as_i64), Some(0));
    assert_eq!(
        entry
            .get("_rankiRawConfig")
            .and_then(Value::as_str)
            .map(ranki_model::decode_hex),
        Some(Some(config))
    );
}

#[test]
fn schema18_notes_use_the_config_table_for_new_positions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("modern-notes.anki2");
    drop(schema18(&path));
    let conn = Connection::open(&path).expect("open");
    conn.execute_batch(
        "UPDATE col SET conf = '';
         CREATE TABLE config (KEY text NOT NULL PRIMARY KEY, usn integer NOT NULL,
             mtime_secs integer NOT NULL, val blob NOT NULL) WITHOUT ROWID;
         INSERT INTO config (key, usn, mtime_secs, val) VALUES ('nextPos', 0, 0, CAST('7' AS BLOB));",
    )
    .expect("config table");
    drop(conn);

    let mut store = Collection::open(&path).expect("open");
    let model = store
        .ensure_model(StockModelKind::Basic)
        .expect("install basic");
    let cards = store
        .add_note(&NewNote {
            model_id: model.id,
            deck_id: 1,
            fields: vec!["front".to_owned(), "back".to_owned()],
            tags: Vec::new(),
        })
        .expect("add note writes to a schema-18 collection");
    let card = store.card(cards[0]).expect("card").expect("stored");
    assert_eq!(card.due, 7);

    let conn = Connection::open(&path).expect("open");
    let stored: Vec<u8> = conn
        .query_row("SELECT val FROM config WHERE key = 'nextPos'", [], |row| {
            row.get(0)
        })
        .expect("nextPos");
    assert_eq!(stored, b"8".to_vec());
    let collection_json: String = conn
        .query_row("SELECT conf FROM col WHERE id = 1", [], |row| row.get(0))
        .expect("conf");
    assert!(collection_json.is_empty(), "col.conf must stay untouched");
}
