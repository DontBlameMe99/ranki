#![expect(clippy::unwrap_used, reason = "test fixtures use unwrap for setup")]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use ranki_format::ImportReport;
use ranki_format::export_apkg;
use ranki_format::export_colpkg;
use ranki_format::import_apkg;
use ranki_format::import_colpkg;
use ranki_model::Store;
use ranki_storage::Collection as AnkiCollection;
use rusqlite::Connection;
use rusqlite::params;
use serde_json::json;
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const TEST_SCHEMA: &str = "
CREATE TABLE col (
    id INTEGER PRIMARY KEY, crt INTEGER NOT NULL, mod INTEGER NOT NULL,
    scm INTEGER NOT NULL, ver INTEGER NOT NULL, dty INTEGER NOT NULL,
    usn INTEGER NOT NULL, ls INTEGER NOT NULL, conf TEXT NOT NULL,
    models TEXT NOT NULL, decks TEXT NOT NULL, dconf TEXT NOT NULL,
    tags TEXT NOT NULL
);
CREATE TABLE notes (
    id INTEGER PRIMARY KEY, guid TEXT NOT NULL, mid INTEGER NOT NULL,
    mod INTEGER NOT NULL, usn INTEGER NOT NULL, tags TEXT NOT NULL,
    flds TEXT NOT NULL, sfld TEXT NOT NULL, csum INTEGER NOT NULL,
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

const REAL_FIXTURE: &str = "/home/user/downloads/WorldFlags.apkg";
const DAY_SECS: i64 = 86_400;

#[derive(Clone)]
struct TestNote {
    id: i64,
    guid: String,
    mid: i64,
    fields: String,
}

#[derive(Clone)]
struct TestCard {
    id: i64,
    note_id: i64,
    deck_id: i64,
    ordinal: i32,
}

#[derive(Clone)]
struct Seed {
    notes: Vec<TestNote>,
    cards: Vec<TestCard>,
    revlog: bool,
}

struct Fixture {
    dir: TempDir,
    source: PathBuf,
    media: PathBuf,
    seed: Seed,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.anki2");
        let media = dir.path().join("source-media");
        fs::create_dir_all(&media).unwrap();
        let seed = Seed {
            notes: vec![
                TestNote {
                    id: 1,
                    guid: "guid-a".to_owned(),
                    mid: 1,
                    fields: "front <img src=\"photo.png\">\u{001f}back"
                        .to_owned(),
                },
                TestNote {
                    id: 2,
                    guid: "guid-b".to_owned(),
                    mid: 1,
                    fields: "second\u{001f}answer".to_owned(),
                },
            ],
            cards: vec![
                TestCard {
                    id: 100,
                    note_id: 1,
                    deck_id: 1,
                    ordinal: 0,
                },
                TestCard {
                    id: 101,
                    note_id: 2,
                    deck_id: 1,
                    ordinal: 0,
                },
            ],
            revlog: true,
        };
        write_collection(&source, &seed);
        Self {
            dir,
            source,
            media,
            seed,
        }
    }

    fn package(&self) -> PathBuf {
        let package = self.dir.path().join("deck.apkg");
        export_apkg(&self.source, &self.media, &package).unwrap();
        package
    }

    fn import(
        &self,
        package: &Path,
        name: &str,
    ) -> (PathBuf, PathBuf, ImportReport) {
        let collection = self.dir.path().join(format!("{name}.anki2"));
        let media = self.dir.path().join(format!("{name}-media"));
        let report = import_apkg(package, &collection, &media).unwrap();
        (collection, media, report)
    }
}

fn write_collection(path: &Path, seed: &Seed) {
    let _ = fs::remove_file(path);
    let connection = Connection::open(path).unwrap();
    connection.execute_batch(TEST_SCHEMA).unwrap();
    let conf =
        json!({"activeDecks": [1], "curDeck": 1, "curModel": 1}).to_string();
    let models = json!({"1": {"id": 1, "name": "Basic", "did": 1}}).to_string();
    let decks =
        json!({"1": {"id": 1, "name": "Default", "conf": 1}}).to_string();
    let dconf = json!({"1": {"id": 1, "name": "Default"}}).to_string();
    let tags = json!({}).to_string();
    connection
        .execute(
            "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags)
             VALUES (1, 0, 0, 0, 11, 0, 0, 0, ?, ?, ?, ?, ?)",
            params![conf, models, decks, dconf, tags],
        )
        .unwrap();

    for note in &seed.notes {
        let sort_field = note.fields.split('\u{001f}').next().unwrap_or("");
        connection
            .execute(
                "INSERT INTO notes (id, guid, mid, mod, usn, tags, flds, sfld, csum, flags, data)
                 VALUES (?, ?, ?, 0, 0, '', ?, ?, 0, 0, '')",
                params![note.id, note.guid, note.mid, note.fields, sort_field],
            )
            .unwrap();
    }
    for card in &seed.cards {
        connection
            .execute(
                "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor,
                     reps, lapses, left, odue, odid, flags, data)
                 VALUES (?, ?, ?, ?, 0, 0, 0, 0, 1, 0, 2500, 0, 0, 0, 0, 0, 0, '')",
                params![card.id, card.note_id, card.deck_id, card.ordinal],
            )
            .unwrap();
    }
    if seed.revlog {
        connection
            .execute(
                "INSERT INTO revlog (id, cid, usn, ease, ivl, lastIvl, factor, time, type)
                 VALUES (1000, 100, 0, 3, 1, 0, 2500, 1200, 0)",
                [],
            )
            .unwrap();
    }
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn strings(connection: &Connection, sql: &str) -> Vec<String> {
    let mut statement = connection.prepare(sql).unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|value| value.unwrap())
        .collect()
}

fn query_string(connection: &Connection, sql: &str, value: &str) -> String {
    connection
        .query_row(sql, params![value], |row| row.get(0))
        .unwrap()
}

fn query_id(connection: &Connection, sql: &str, value: &str) -> i64 {
    connection
        .query_row(sql, params![value], |row| row.get(0))
        .unwrap()
}

fn build_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    for (name, bytes) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn entry_bytes(archive: &mut zip::ZipArchive<File>, name: &str) -> Vec<u8> {
    let mut entry = archive.by_name(name).unwrap();
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).unwrap();
    bytes
}

fn unix_seconds() -> i64 {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    i64::try_from(elapsed.as_secs()).unwrap()
}

fn write_review_source(path: &Path, crt: i64, due: i64) {
    let seed = Seed {
        notes: vec![TestNote {
            id: 1,
            guid: "review-guid".to_owned(),
            mid: 1,
            fields: "question\u{001f}answer".to_owned(),
        }],
        cards: vec![TestCard {
            id: 100,
            note_id: 1,
            deck_id: 1,
            ordinal: 0,
        }],
        revlog: false,
    };
    write_collection(path, &seed);
    let connection = Connection::open(path).unwrap();
    connection
        .execute("UPDATE col SET crt = ?1 WHERE id = 1", [crt])
        .unwrap();
    connection
        .execute(
            "UPDATE cards SET type = 2, queue = 2, ivl = 10, due = ?1 WHERE id = 100",
            [due],
        )
        .unwrap();
}

fn set_collection_crt(path: &Path, crt: i64) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute("UPDATE col SET crt = ?1 WHERE id = 1", [crt])
        .unwrap();
}

#[test]
fn roundtrip_notes_cards_and_review_log() {
    let fixture = Fixture::new();
    let package = fixture.package();
    let (collection, _media, report) = fixture.import(&package, "fresh");

    assert_eq!(
        report,
        ImportReport {
            notes: 2,
            cards: 2,
            media: 0
        }
    );
    let connection = Connection::open(collection).unwrap();
    assert_eq!(count(&connection, "notes"), 2);
    assert_eq!(count(&connection, "cards"), 2);
    assert_eq!(count(&connection, "revlog"), 1);
    let guids = strings(&connection, "SELECT guid FROM notes ORDER BY guid");
    assert_eq!(guids, vec!["guid-a", "guid-b"]);
}

#[test]
fn fresh_import_preserves_source_crt_and_due_scale() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.anki2");
    let now = unix_seconds();
    let source_days = 20_i64;
    let source_crt = now.saturating_sub(DAY_SECS.saturating_mul(source_days));
    let source_due = source_days.saturating_add(5);
    write_review_source(&source, source_crt, source_due);

    let package = dir.path().join("review.apkg");
    export_apkg(&source, &dir.path().join("media"), &package).unwrap();
    let collection = dir.path().join("fresh.anki2");
    import_apkg(&package, &collection, &dir.path().join("fresh-media"))
        .unwrap();

    let connection = Connection::open(&collection).unwrap();
    let stored_crt: i64 = connection
        .query_row("SELECT crt FROM col WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored_crt, source_crt);
    drop(connection);

    let store = AnkiCollection::open(&collection).unwrap();
    let due = store.due_cards(None, 10, now).unwrap();
    assert_eq!(due.as_slice(), &[]);
    let card = store.card(100).unwrap().unwrap();
    assert_eq!(card.due, source_due);
}

#[test]
fn existing_import_shifts_review_due_to_destination_scale() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("existing.anki2");
    let created = AnkiCollection::create(&destination).unwrap();
    let destination_crt = created.collection_created().unwrap();
    drop(created);

    let source_days = 20_i64;
    let source_crt =
        destination_crt.saturating_sub(DAY_SECS.saturating_mul(source_days));
    let source_due = source_days.saturating_add(5);
    let source = dir.path().join("source.anki2");
    write_review_source(&source, source_crt, source_due);

    let package = dir.path().join("review.apkg");
    export_apkg(&source, &dir.path().join("media"), &package).unwrap();
    import_apkg(&package, &destination, &dir.path().join("shifted-media"))
        .unwrap();

    let store = AnkiCollection::open(&destination).unwrap();
    let card = store.card(100).unwrap().unwrap();
    assert_eq!(card.due, 5);
    let due = store.due_cards(None, 10, destination_crt).unwrap();
    assert_eq!(due.as_slice(), &[]);
}

#[test]
fn existing_import_uses_day_indices_when_crt_times_differ() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("existing.anki2");
    let created = AnkiCollection::create(&destination).unwrap();
    drop(created);

    let now = unix_seconds();
    let half_day = DAY_SECS.saturating_div(2);
    let destination_crt = now.saturating_sub(half_day);
    let source_crt = now
        .saturating_sub(DAY_SECS.saturating_mul(20))
        .saturating_sub(half_day.saturating_add(DAY_SECS.saturating_div(4)));
    set_collection_crt(&destination, destination_crt);

    let source = dir.path().join("source.anki2");
    write_review_source(&source, source_crt, 20);

    let package = dir.path().join("review.apkg");
    export_apkg(&source, &dir.path().join("media"), &package).unwrap();
    import_apkg(&package, &destination, &dir.path().join("shifted-media"))
        .unwrap();

    let connection = Connection::open(&destination).unwrap();
    let due: i64 = connection
        .query_row("SELECT due FROM cards WHERE id = 100", [], |row| row.get(0))
        .unwrap();
    assert_eq!(due, 0);
    drop(connection);

    let store = AnkiCollection::open(&destination).unwrap();
    let due_cards = store.due_cards(None, 10, now).unwrap();
    assert!(due_cards.iter().any(|card| card.id == 100));
}

fn write_review_shift_source(path: &Path, crt: i64, due: i64) {
    let notes = [
        ("suspended-review", "suspended\u{001f}answer"),
        ("odue-review", "odue\u{001f}answer"),
        ("new-card", "new\u{001f}answer"),
        ("learning-card", "learning\u{001f}answer"),
        ("no-original-due", "no-original\u{001f}answer"),
    ];
    let seed = Seed {
        notes: notes
            .iter()
            .enumerate()
            .map(|(index, (guid, fields))| TestNote {
                id: i64::try_from(index).unwrap().saturating_add(1),
                guid: (*guid).to_owned(),
                mid: 1,
                fields: (*fields).to_owned(),
            })
            .collect(),
        cards: (1_i64..=5)
            .map(|id| TestCard {
                id: id.saturating_add(99),
                note_id: id,
                deck_id: 1,
                ordinal: 0,
            })
            .collect(),
        revlog: false,
    };
    write_collection(path, &seed);
    let connection = Connection::open(path).unwrap();
    connection
        .execute("UPDATE col SET crt = ?1 WHERE id = 1", [crt])
        .unwrap();
    connection
        .execute(
            "UPDATE cards SET type = 2, queue = -1, due = ?1, odue = ?2 WHERE id = 100",
            params![due, due.saturating_add(2)],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE cards SET type = 2, queue = 2, due = ?1, odue = ?2 WHERE id = 101",
            params![due, due.saturating_add(5)],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE cards SET type = 0, queue = 0, due = 42, odue = 99 WHERE id = 102",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE cards SET type = 1, queue = 1, due = 999999, odue = 888888 WHERE id = 103",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE cards SET type = 2, queue = 2, due = ?1, odue = 0 WHERE id = 104",
            [due],
        )
        .unwrap();
}

type CardRowData = (i64, i64, i64, i64, i64);

fn card_rows(path: &Path) -> Vec<CardRowData> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT id, type, queue, due, odue FROM cards ORDER BY id")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

#[test]
fn existing_import_shifts_suspended_and_original_due_for_reviews() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("existing.anki2");
    let created = AnkiCollection::create(&destination).unwrap();
    drop(created);

    let now = unix_seconds();
    let destination_crt = now.saturating_sub(DAY_SECS.saturating_div(2));
    set_collection_crt(&destination, destination_crt);
    let source_crt =
        destination_crt.saturating_sub(DAY_SECS.saturating_mul(20));
    let source_due = 20_i64.saturating_add(5);

    let source = dir.path().join("source.anki2");
    write_review_shift_source(&source, source_crt, source_due);

    let package = dir.path().join("reviews.apkg");
    export_apkg(&source, &dir.path().join("media"), &package).unwrap();
    import_apkg(&package, &destination, &dir.path().join("shifted-media"))
        .unwrap();

    assert_eq!(
        card_rows(&destination),
        vec![
            (100, 2, -1, 5, 7),
            (101, 2, 2, 5, 10),
            (102, 0, 0, 42, 99),
            (103, 1, 1, 999_999, 888_888),
            (104, 2, 2, 5, 0),
        ]
    );
}

#[test]
fn created_collection_uses_anki_dty_column() {
    let fixture = Fixture::new();
    let package = fixture.package();
    let (collection, _, _) = fixture.import(&package, "created");

    let connection = Connection::open(collection).unwrap();
    let columns =
        strings(&connection, "SELECT name FROM pragma_table_info('col')");
    assert!(columns.iter().any(|name| name == "dty"));
    assert!(!columns.iter().any(|name| name == "dt"));
}

#[test]
fn export_uses_expected_zip_entries() {
    let fixture = Fixture::new();
    fs::write(fixture.media.join("photo.png"), b"image").unwrap();
    let package = fixture.package();

    let mut archive =
        zip::ZipArchive::new(File::open(package).unwrap()).unwrap();
    let names: BTreeSet<String> = archive
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    for expected in ["collection.anki21", "media", "0", "meta"] {
        assert!(names.contains(expected), "missing entry {expected}");
    }

    assert_eq!(entry_bytes(&mut archive, "meta"), vec![0x08, 0x02]);
    assert!(archive.by_name("collection.anki21").is_ok());
    assert!(!names.contains("collection.anki2"));
    assert!(!names.contains("collection.anki21b"));
    let mapping: BTreeMap<String, String> =
        serde_json::from_slice(&entry_bytes(&mut archive, "media")).unwrap();
    assert_eq!(mapping.get("0").map(String::as_str), Some("photo.png"));
    assert_eq!(entry_bytes(&mut archive, "0"), b"image".to_vec());
}

#[test]
fn malformed_zip_has_clear_error() {
    let fixture = Fixture::new();
    let package = fixture.dir.path().join("broken.apkg");
    fs::write(&package, b"not a zip").unwrap();

    let error = import_apkg(
        &package,
        &fixture.dir.path().join("broken.anki2"),
        &fixture.dir.path().join("broken-media"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("malformed apkg package"));
}

#[test]
fn missing_media_mapping_has_clear_error() {
    let fixture = Fixture::new();
    let package = fixture.dir.path().join("no-mapping.apkg");
    build_zip(
        &package,
        &[
            ("collection.anki21", fs::read(&fixture.source).unwrap()),
            ("0", b"image".to_vec()),
        ],
    );

    let error = import_apkg(
        &package,
        &fixture.dir.path().join("no-mapping.anki2"),
        &fixture.dir.path().join("no-mapping-media"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("no media mapping"));
}

#[test]
fn missing_media_entry_has_clear_error() {
    let fixture = Fixture::new();
    let package = fixture.dir.path().join("missing-media.apkg");
    let mapping = json!({"0": "photo.png"}).to_string().into_bytes();
    build_zip(
        &package,
        &[
            ("collection.anki21", fs::read(&fixture.source).unwrap()),
            ("media", mapping),
        ],
    );

    let error = import_apkg(
        &package,
        &fixture.dir.path().join("missing.anki2"),
        &fixture.dir.path().join("missing-media"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("missing from package"));
}

#[test]
fn duplicate_guid_in_package_has_clear_error() {
    let fixture = Fixture::new();
    let mut seed = fixture.seed.clone();
    seed.notes[1].guid = "guid-a".to_owned();
    write_collection(&fixture.source, &seed);
    let package = fixture.package();

    let error = import_apkg(
        &package,
        &fixture.dir.path().join("duplicate.anki2"),
        &fixture.dir.path().join("duplicate-media"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate guid in package"));
}

#[test]
fn importing_twice_skips_existing_notes() {
    let fixture = Fixture::new();
    let package = fixture.package();
    let (collection, media, first) = fixture.import(&package, "twice");
    assert_eq!(first.notes, 2);

    let second = import_apkg(&package, &collection, &media).unwrap();
    assert_eq!(second.notes, 0);
    assert_eq!(second.cards, 0);
    let connection = Connection::open(collection).unwrap();
    assert_eq!(count(&connection, "notes"), 2);
    assert_eq!(count(&connection, "cards"), 2);
}

#[test]
fn media_names_are_sanitized_and_rewritten() {
    let fixture = Fixture::new();
    fs::write(fixture.media.join("bad:name?.png"), b"pixel").unwrap();
    let mut seed = fixture.seed.clone();
    seed.notes[0].fields =
        "<img src=\"bad:name?.png\"> front\u{001f}back".to_owned();
    write_collection(&fixture.source, &seed);
    let package = fixture.package();

    let (collection, media, report) = fixture.import(&package, "sanitized");
    assert_eq!(report.media, 1);
    assert!(media.join("bad_name_.png").exists());
    let connection = Connection::open(collection).unwrap();
    let fields = query_string(
        &connection,
        "SELECT flds FROM notes WHERE guid = ?",
        "guid-a",
    );
    assert!(fields.contains("bad_name_.png"));
    assert!(!fields.contains("bad:name?.png"));
}

#[test]
fn media_collision_uses_new_name_and_rewrites_notes() {
    let fixture = Fixture::new();
    fs::write(fixture.media.join("photo.png"), b"new").unwrap();
    let package = fixture.package();

    let collection = fixture.dir.path().join("collision.anki2");
    let media = fixture.dir.path().join("collision-media");
    fs::create_dir_all(&media).unwrap();
    fs::write(media.join("photo.png"), b"old").unwrap();
    import_apkg(&package, &collection, &media).unwrap();

    assert_eq!(fs::read(media.join("photo.png")).unwrap(), b"old".to_vec());
    assert_eq!(
        fs::read(media.join("photo_1.png")).unwrap(),
        b"new".to_vec()
    );
    let connection = Connection::open(collection).unwrap();
    let fields = query_string(
        &connection,
        "SELECT flds FROM notes WHERE guid = ?",
        "guid-a",
    );
    assert!(fields.contains("photo_1.png"));
}

#[test]
fn id_collisions_do_not_lose_existing_notes() {
    let fixture = Fixture::new();
    let destination = fixture.dir.path().join("existing.anki2");
    write_collection(
        &destination,
        &Seed {
            notes: vec![TestNote {
                id: 1,
                guid: "existing".to_owned(),
                mid: 1,
                fields: "keep me\u{001f}here".to_owned(),
            }],
            cards: vec![TestCard {
                id: 100,
                note_id: 1,
                deck_id: 1,
                ordinal: 0,
            }],
            revlog: false,
        },
    );
    let package = fixture.package();

    let report = import_apkg(
        &package,
        &destination,
        &fixture.dir.path().join("merged-media"),
    )
    .unwrap();
    assert_eq!(report.notes, 2);
    let connection = Connection::open(destination).unwrap();
    assert_eq!(count(&connection, "notes"), 3);
    assert_eq!(count(&connection, "cards"), 3);
    assert_eq!(
        query_id(
            &connection,
            "SELECT id FROM notes WHERE guid = ?",
            "existing"
        ),
        1
    );
    let imported_id =
        query_id(&connection, "SELECT id FROM notes WHERE guid = ?", "guid-a");
    assert_ne!(imported_id, 1);
    assert_eq!(
        query_id(
            &connection,
            "SELECT COUNT(*) FROM cards WHERE nid = ?",
            &imported_id.to_string()
        ),
        1
    );
    assert_eq!(
        strings(
            &connection,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"
        )
        .len(),
        5
    );
    assert!(fixture.source.exists());
}

#[test]
fn colpkg_extracts_without_overwriting() {
    let fixture = Fixture::new();
    let package = fixture.dir.path().join("backup.colpkg");
    let meta = vec![0x08, 0x02];
    build_zip(
        &package,
        &[
            ("collection.anki2", fs::read(&fixture.source).unwrap()),
            ("media/photo.png", b"image".to_vec()),
            ("meta", meta),
        ],
    );

    let destination = fixture.dir.path().join("colpkg");
    let collection = import_colpkg(&package, &destination).unwrap();
    assert_eq!(collection, destination.join("collection.anki2"));
    assert!(collection.exists());
    assert_eq!(
        fs::read(destination.join("media/photo.png")).unwrap(),
        b"image".to_vec()
    );

    fs::write(destination.join("media/photo.png"), b"changed").unwrap();
    let error = import_colpkg(&package, &destination).unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert_eq!(
        fs::read(destination.join("media/photo.png")).unwrap(),
        b"changed".to_vec()
    );
}

#[test]
fn colpkg_extracts_numbered_media_mapping() {
    let fixture = Fixture::new();
    let package = fixture.dir.path().join("mapped.colpkg");
    let mapping = json!({"0": "photo.png", "1": "../bad:name?.png"})
        .to_string()
        .into_bytes();
    build_zip(
        &package,
        &[
            ("collection.anki21", fs::read(&fixture.source).unwrap()),
            ("0", b"image".to_vec()),
            ("1", b"pixel".to_vec()),
            ("media", mapping),
            ("meta", vec![0x08, 0x02]),
        ],
    );

    let destination = fixture.dir.path().join("mapped");
    let collection = import_colpkg(&package, &destination).unwrap();
    assert_eq!(collection, destination.join("collection.anki21"));
    assert_eq!(
        fs::read(destination.join("media/photo.png")).unwrap(),
        b"image".to_vec()
    );
    assert_eq!(
        fs::read(destination.join("media/bad_name_.png")).unwrap(),
        b"pixel".to_vec()
    );
}

#[test]
fn colpkg_roundtrip_preserves_notes_cards_and_media() {
    let fixture = Fixture::new();
    fs::write(fixture.media.join("photo.png"), b"image").unwrap();
    let package = fixture.dir.path().join("backup.colpkg");
    export_colpkg(&fixture.source, &fixture.media, &package).unwrap();

    let destination = fixture.dir.path().join("restored");
    let collection = import_colpkg(&package, &destination).unwrap();
    let connection = Connection::open(collection).unwrap();
    assert_eq!(count(&connection, "notes"), 2);
    assert_eq!(count(&connection, "cards"), 2);
    assert_eq!(count(&connection, "revlog"), 1);
    let guids = strings(&connection, "SELECT guid FROM notes ORDER BY guid");
    assert_eq!(guids, vec!["guid-a", "guid-b"]);
    assert_eq!(
        fs::read(destination.join("media/photo.png")).unwrap(),
        b"image".to_vec()
    );
}

#[test]
fn invalid_media_mapping_has_clear_error() {
    let fixture = Fixture::new();
    let package = fixture.dir.path().join("bad-mapping.apkg");
    build_zip(
        &package,
        &[
            ("collection.anki21", fs::read(&fixture.source).unwrap()),
            ("media", b"not json".to_vec()),
        ],
    );

    let error = import_apkg(
        &package,
        &fixture.dir.path().join("bad-mapping.anki2"),
        &fixture.dir.path().join("bad-mapping-media"),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid media mapping in package")
    );
}

#[test]
fn real_world_apkg_roundtrip_when_available() {
    if !Path::new(REAL_FIXTURE).exists() {
        eprintln!("skipping real-world apkg test: {REAL_FIXTURE} is absent");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("WorldFlags.apkg");
    fs::copy(REAL_FIXTURE, &package).unwrap();
    let collection = dir.path().join("flags.anki2");
    let media = dir.path().join("flags-media");
    let report = import_apkg(&package, &collection, &media).unwrap();

    assert!(report.notes > 200);
    assert!(report.cards > 200);
    assert!(fs::read_dir(&media).unwrap().count() > 0);

    let exported = dir.path().join("roundtrip.apkg");
    export_apkg(&collection, &media, &exported).unwrap();
    let mut archive =
        zip::ZipArchive::new(File::open(&exported).unwrap()).unwrap();
    assert!(archive.by_name("collection.anki21").is_ok());

    let fresh = dir.path().join("fresh.anki2");
    let fresh_media = dir.path().join("fresh-media");
    let second = import_apkg(&exported, &fresh, &fresh_media).unwrap();
    assert_eq!(second.notes, report.notes);
    assert_eq!(second.cards, report.cards);
}
