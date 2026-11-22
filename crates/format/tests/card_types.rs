#![expect(clippy::expect_used, reason = "test fixtures panic when setup fails")]
//! Import and export tests for full schema-18 note types.

use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use ranki_model::NewNote;
use ranki_model::NoteKind;
use ranki_model::Store;
use ranki_storage::Collection as AnkiCollection;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

const LOCAL_COLLECTION: &str =
    "/home/user/.local/share/Anki2/main/collection.anki2";

fn import_local(dir: &TempDir, name: &str) -> Option<PathBuf> {
    if !Path::new(LOCAL_COLLECTION).exists() {
        eprintln!(
            "skipping: real schema-18 fixture is missing: {LOCAL_COLLECTION}"
        );
        return None;
    }
    let source = dir.path().join(format!("{name}-source.anki2"));
    fs::copy(LOCAL_COLLECTION, &source).expect("copy collection");
    let media = dir.path().join(format!("{name}-source-media"));
    fs::create_dir_all(&media).expect("media dir");
    let package = dir.path().join(format!("{name}.apkg"));
    ranki_format::export_apkg(&source, &media, &package).expect("export");
    let collection = dir.path().join(format!("{name}.anki2"));
    ranki_format::import_apkg(
        &package,
        &collection,
        &dir.path().join(format!("{name}-media")),
    )
    .expect("import");
    Some(collection)
}

fn models_json(path: &Path) -> Value {
    let connection = Connection::open(path).expect("open collection");
    let text: String = connection
        .query_row("SELECT models FROM col WHERE id = 1", [], |row| row.get(0))
        .expect("models column");
    serde_json::from_str(&text).expect("parse models")
}

fn zip_collection(package: &Path, destination: &Path) {
    let mut archive =
        zip::ZipArchive::new(File::open(package).expect("package"))
            .expect("zip");
    let mut entry = archive
        .by_name("collection.anki21")
        .expect("collection entry");
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read entry");
    drop(entry);
    fs::write(destination, bytes).expect("write extracted collection");
}

#[test]
fn schema18_import_preserves_full_models() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(collection) = import_local(&dir, "full") else {
        return;
    };
    let store = AnkiCollection::open(&collection).expect("open");
    let models = store.models().expect("models");
    assert!(models.len() > 1, "expected several decoded note types");

    let basic = models
        .iter()
        .find(|model| {
            model.name == "Basic" && model.fields == ["Front", "Back"]
        })
        .expect("Basic");
    assert_eq!(basic.templates.len(), 1);
    assert_eq!(basic.templates[0].question_format, "{{Front}}");
    assert!(basic.templates[0].answer_format.contains("<hr id=answer>"));
    assert_ne!(basic.css, "");
    assert_eq!(basic.templates[0].ord, 0);

    let cloze = models
        .iter()
        .find(|model| model.name.contains("Cloze"))
        .expect("Cloze");
    assert_eq!(cloze.kind, NoteKind::Cloze);
    assert!(
        cloze.templates[0]
            .question_format
            .contains("{{cloze:Text}}")
    );

    let json = models_json(&collection);
    assert!(
        json.as_object().is_some_and(|object| object.len() > 1),
        "col.models must contain full entries"
    );
}

#[test]
fn export_roundtrip_preserves_raw_model_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(collection) = import_local(&dir, "raw") else {
        return;
    };
    let before = models_json(&collection);
    let raw = before
        .as_object()
        .and_then(|object| {
            object.values().find_map(|model| {
                model
                    .as_object()
                    .and_then(|entry| entry.get("_rankiRawConfig"))
                    .and_then(Value::as_str)
            })
        })
        .expect("raw config");

    let media = dir.path().join("raw-media");
    let package = dir.path().join("export.apkg");
    ranki_format::export_apkg(&collection, &media, &package).expect("export");

    let extracted = dir.path().join("exported.anki2");
    zip_collection(&package, &extracted);
    let after = models_json(&extracted);
    let raw_after = after
        .as_object()
        .and_then(|object| {
            object.values().find_map(|model| {
                model
                    .as_object()
                    .and_then(|entry| entry.get("_rankiRawConfig"))
                    .and_then(Value::as_str)
            })
        })
        .expect("raw config after export");
    assert_eq!(raw_after, raw);
}

#[expect(
    clippy::excessive_nesting,
    reason = "the fixture clears one model entry in place"
)]
#[test]
fn worldflags_model_without_templates_imports_without_crashing() {
    let source = Path::new("/home/user/downloads/WorldFlags.apkg");
    if !source.exists() {
        eprintln!(
            "skipping: real WorldFlags fixture is missing: {}",
            source.display()
        );
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let collection = dir.path().join("worldflags.anki2");
    ranki_format::import_apkg(
        source,
        &collection,
        &dir.path().join("worldflags-media"),
    )
    .expect("import");
    {
        let connection =
            Connection::open(&collection).expect("open collection");
        let text: String = connection
            .query_row("SELECT models FROM col WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("models");
        let mut models: Value =
            serde_json::from_str(&text).expect("parse models");
        let mut cleared = false;
        if let Some(object) = models.as_object_mut() {
            for entry in object.values_mut() {
                if entry.get("name").and_then(Value::as_str) == Some("Basic_")
                    && let Some(fields) = entry.as_object_mut()
                {
                    let _ =
                        fields.insert("tmpls".into(), serde_json::json!([]));
                    cleared = true;
                }
            }
        }
        assert!(cleared, "WorldFlags collection has no Basic_ model");
        connection
            .execute(
                "UPDATE col SET models = ?1 WHERE id = 1",
                rusqlite::params![
                    serde_json::to_string(&models).expect("encode")
                ],
            )
            .expect("write models");
    }
    let mut store = AnkiCollection::open(&collection).expect("open");
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
