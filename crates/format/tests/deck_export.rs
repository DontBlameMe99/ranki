use std::fs;

use ranki_format::export_deck_apkg;
use ranki_format::import_apkg;
use ranki_model::NewNote;
use ranki_model::Store as _;
use ranki_storage::{Collection, register_collations};
use rusqlite::{Connection, params};

use serde_json::Value as Json;
use serde_json::json;
fn note(deck_id: i64, front: &str, back: &str) -> NewNote {
    NewNote {
        model_id: 1,
        deck_id,
        fields: vec![front.to_owned(), back.to_owned()],
        tags: Vec::new(),
    }
}

#[test]
fn deck_export_keeps_the_subtree_and_only_its_media() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.anki2");
    let media = dir.path().join("source.media");
    fs::create_dir_all(&media).unwrap();
    let mut store = Collection::create(&source).unwrap();
    let language = store.create_deck("Language").unwrap();
    let french = store.create_deck("Language::French").unwrap();
    let other = store.create_deck("Other").unwrap();

    store
        .add_note(&note(language.id, "<img src=shared.png>", "root"))
        .unwrap();
    store
        .add_note(&note(language.id, "<img src=my_pic.png>", "overlap"))
        .unwrap();
    store
        .add_note(&note(french.id, "<img src=french%20photo.png>", "french"))
        .unwrap();
    store
        .add_note(&note(other.id, "<img src=other.png>", "other"))
        .unwrap();
    fs::write(media.join("shared.png"), b"shared").unwrap();
    fs::write(media.join("my_pic.png"), b"my_pic").unwrap();
    fs::write(media.join("pic.png"), b"pic").unwrap();
    fs::write(media.join("french photo.png"), b"french").unwrap();
    fs::write(media.join("other.png"), b"other").unwrap();
    fs::write(media.join("template_bg.png"), b"background").unwrap();
    fs::write(media.join("template_icon.png"), b"icon").unwrap();

    let connection = Connection::open(&source).unwrap();
    let raw_models: String = connection
        .query_row("SELECT models FROM col WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    let mut models: Json = serde_json::from_str(&raw_models).unwrap();
    models["1"]["css"] = json!(".card { background: url(template_bg.png); }");
    models["1"]["tmpls"][0]["qfmt"] =
        json!("<img src=template_icon.png>{{Front}}");
    let updated = serde_json::to_string(&models).unwrap();
    connection
        .execute("UPDATE col SET models = ?1 WHERE id = 1", [updated])
        .unwrap();
    drop(connection);

    let package = dir.path().join("language.apkg");
    export_deck_apkg(&source, &media, &package, language.id).unwrap();

    let destination = dir.path().join("imported.anki2");
    let imported_media = dir.path().join("imported.media");
    let report = import_apkg(&package, &destination, &imported_media).unwrap();
    assert_eq!(report.notes, 3);
    assert_eq!(report.cards, 3);

    let imported = Collection::open(&destination).unwrap();
    let mut names = imported
        .decks()
        .unwrap()
        .into_iter()
        .map(|deck| deck.name)
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["Language", "Language::French"]);

    let connection = Connection::open(&destination).unwrap();
    let cards: i64 = connection
        .query_row("SELECT COUNT(*) FROM cards", [], |row| row.get(0))
        .unwrap();
    assert_eq!(cards, 3);

    let mut files = fs::read_dir(&imported_media)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    assert_eq!(
        files,
        [
            "french photo.png",
            "my_pic.png",
            "shared.png",
            "template_bg.png",
            "template_icon.png",
        ]
    );
}

#[test]
fn deck_export_registers_schema18_collations() {
    let language_id;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.anki2");
    let media = dir.path().join("source.media");
    fs::create_dir_all(&media).unwrap();
    {
        let mut store = Collection::create(&source).unwrap();
        let language = store.create_deck("Language").unwrap();
        language_id = language.id;
        let french = store.create_deck("Language::French").unwrap();
        let other = store.create_deck("Other").unwrap();
        store.add_note(&note(language.id, "root", "one")).unwrap();
        store.add_note(&note(french.id, "child", "two")).unwrap();

        let connection = Connection::open(&source).unwrap();
        register_collations(&connection).unwrap();
        connection
            .execute("UPDATE col SET decks = '', ver = 18 WHERE id = 1", [])
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE decks (
                     id INTEGER PRIMARY KEY NOT NULL,
                     name TEXT NOT NULL COLLATE unicase,
                     mtime_secs INTEGER NOT NULL,
                     usn INTEGER NOT NULL,
                     common BLOB NOT NULL,
                     kind BLOB NOT NULL);
                 CREATE UNIQUE INDEX idx_decks_name ON decks (name);",
            )
            .unwrap();
        for deck in [language, french, other] {
            connection
                .execute(
                    "INSERT INTO decks VALUES (?1, ?2, 0, 0, x'', x'')",
                    params![deck.id, deck.name.replace("::", "\u{1f}")],
                )
                .unwrap();
        }
    }

    let package = dir.path().join("language.apkg");
    export_deck_apkg(&source, &media, &package, language_id).unwrap();

    let destination = dir.path().join("imported.anki2");
    let imported_media = dir.path().join("imported.media");
    let report = import_apkg(&package, &destination, &imported_media).unwrap();
    assert_eq!(report.notes, 2);
    assert_eq!(report.cards, 2);
}

#[test]
fn exporting_an_unknown_deck_fails() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.anki2");
    let media = dir.path().join("source.media");
    fs::create_dir_all(&media).unwrap();
    let _ = Collection::create(&source).unwrap();

    let error = export_deck_apkg(
        &source,
        &media,
        &dir.path().join("missing.apkg"),
        404,
    )
    .expect_err("missing deck must fail");
    assert!(error.to_string().contains("deck does not exist: 404"));
    assert!(!dir.path().join("missing.apkg").exists());
}
