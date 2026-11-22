#![expect(clippy::unwrap_used, reason = "test fixtures use unwrap for setup")]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use ranki_format::ImportReport;
use ranki_format::export_apkg;
use ranki_format::import_apkg;
use ranki_format::import_colpkg;
use rusqlite::Connection;
use rusqlite::params;
use serde_json::json;
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const TEST_SCHEMA: &str = "
CREATE TABLE col (
    id INTEGER PRIMARY KEY, crt INTEGER NOT NULL, mod INTEGER NOT NULL,
    scm INTEGER NOT NULL, ver INTEGER NOT NULL, dt REAL NOT NULL,
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
            "INSERT INTO col (id, crt, mod, scm, ver, dt, usn, ls, conf, models, decks, dconf, tags)
             VALUES (1, 0, 0, 0, 11, 0.0, 0, 0, ?, ?, ?, ?, ?)",
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
    let meta = br#"{"version": 1}"#.to_vec();
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
