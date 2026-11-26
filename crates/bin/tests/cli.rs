//! End-to-end coverage through the built binary.

#![expect(clippy::unwrap_used, reason = "test assertions")]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use ranki_model::Store;
use ranki_storage::Collection;
use tempfile::TempDir;

struct Isolated {
    data: PathBuf,
    config: PathBuf,
}

impl Isolated {
    fn new(dir: &TempDir, name: &str) -> Self {
        let root = dir.path().join(name);
        let data = root.join("xdg/data");
        let config = root.join("xdg/config");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(config.join("ranki")).unwrap();
        Self { data, config }
    }

    fn collection(&self) -> PathBuf {
        self.data.join("ranki/collection.anki2")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ranki"));
        command
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", &self.config);
        command
    }

    fn run(&self, args: &[&str]) -> String {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "command failed: {:?}\n{}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

#[test]
fn full_cli_workflow_exports_and_reimports_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    let source = Isolated::new(&dir, "source");
    let destination = Isolated::new(&dir, "destination");
    let source_collection = source.collection();

    let decks = source.run(&["decks"]);
    assert!(decks.contains("Default"), "{decks}");
    assert!(source_collection.is_file());

    let note_id = source.run(&[
        "add-note",
        "--model",
        "1",
        "--deck",
        "1",
        "--field",
        "integration front",
        "--field",
        "integration back",
        "--tag",
        "cli-test",
    ]);
    let card_id = note_id.trim().parse::<i64>().unwrap();
    assert!(card_id > 0);

    let found = source.run(&["search", "integration"]);
    assert_eq!(found.trim(), card_id.to_string());

    let due = source.run(&["due", "--limit", "5"]);
    assert!(due.contains(&card_id.to_string()), "{due}");

    source.run(&["answer", &card_id.to_string(), "3"]);

    let stats = source.run(&["stats"]);
    assert!(stats.contains("total=1"), "{stats}");

    let package = dir.path().join("backup.apkg");
    let package_arg = package.to_str().unwrap();
    source.run(&["export", package_arg]);
    assert!(package.is_file());

    let destination_collection = destination.collection();
    let report = destination.run(&["import", package_arg]);
    assert!(report.contains("notes=1"), "{report}");
    assert!(destination_collection.is_file());

    let fresh_stats = destination.run(&["stats"]);
    assert!(fresh_stats.contains("total=1"), "{fresh_stats}");
    assert_eq!(
        destination.run(&["search", "integration"]).trim(),
        found.trim()
    );

    let store = Collection::open(&destination_collection).unwrap();
    let imported = store.find_cards("integration").unwrap();
    assert_eq!(imported.len(), 1);
    let note = store
        .note(store.card(imported[0]).unwrap().unwrap().note_id)
        .unwrap()
        .unwrap();
    assert!(note.fields.iter().any(|field| field == "integration front"));
    assert!(note.tags.iter().any(|tag| tag == "cli-test"));
    drop(store);

    assert_eq!(source.run(&["check"]).trim(), "ok");
    assert_eq!(destination.run(&["check"]).trim(), "ok");
}

#[test]
fn missing_collection_is_created_only_in_the_isolated_data_root() {
    let dir = tempfile::tempdir().unwrap();
    let app = Isolated::new(&dir, "app");
    let path = app.collection();
    assert!(!path.exists());
    app.run(&["check"]);
    assert!(path.is_file());
    assert_eq!(path.parent(), Some(app.data.join("ranki").as_path()));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn invalid_rating_fails_without_creating_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    let app = Isolated::new(&dir, "app");
    let output = app.command().args(["answer", "1", "9"]).output().unwrap();
    assert!(!output.status.success());
    assert!(!app.collection().exists());
}

#[test]
fn collection_works_without_home_when_data_home_is_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let app = Isolated::new(&dir, "app");
    let output = app
        .command()
        .env_remove("HOME")
        .env_remove("RANKI_COLLECTION")
        .args(["decks"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(app.collection().is_file());
}

#[test]
fn collection_works_without_home_with_isolated_config() {
    let dir = tempfile::tempdir().unwrap();
    let app = Isolated::new(&dir, "app");
    fs::write(
        app.config.join("ranki/config.toml"),
        "[ui]\ntheme = 'dark'\n",
    )
    .unwrap();
    let output = app
        .command()
        .env_remove("HOME")
        .env_remove("RANKI_COLLECTION")
        .args(["decks"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(app.collection().is_file());
}

#[test]
fn legacy_path_values_leave_an_anki_profile_untouched() {
    let dir = tempfile::tempdir().unwrap();
    for case in ["config", "environment"] {
        let app = Isolated::new(&dir, case);
        let decoy = dir.path().join(format!("{case}-anki"));
        fs::create_dir_all(&decoy).unwrap();
        let decoy_collection = decoy.join("collection.anki2");
        let decoy_media = decoy.join("collection.media");

        if case == "config" {
            fs::write(
                app.config.join("ranki/config.toml"),
                format!(
                    "[general]\ncollection = '{}'\nmedia_dir = '{}'\n",
                    decoy_collection.display(),
                    decoy_media.display()
                ),
            )
            .unwrap();
        }

        let mut command = app.command();
        if case == "environment" {
            command.env("RANKI_COLLECTION", &decoy_collection);
        }
        let output = command.args(["decks"]).output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(app.collection().is_file(), "{case}");
        assert!(!decoy_collection.exists(), "{case}");
        assert!(!decoy_media.exists(), "{case}");
        assert_eq!(fs::read_dir(&decoy).unwrap().count(), 0, "{case}");
    }
}

#[test]
fn unknown_config_keys_are_reported_by_full_path() {
    let dir = tempfile::tempdir().unwrap();
    let app = Isolated::new(&dir, "app");
    fs::write(
        app.config.join("ranki/config.toml"),
        "[ui]\nthem = 'dark'\n",
    )
    .unwrap();
    let output = app.command().args(["decks"]).output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ui.them"), "{stderr}");
}

#[test]
fn deck_create_rename_and_remove_commands_update_the_collection() {
    let dir = tempfile::tempdir().unwrap();
    let app = Isolated::new(&dir, "app");
    let path = app.collection();

    let created = app.run(&["deck-create", "Study::Languages"]);
    let fields: Vec<_> = created.trim().split('\t').collect();
    assert_eq!(fields[0], "created");
    assert_eq!(fields[2], "Study::Languages");
    let id: i64 = fields[1].parse().unwrap();
    assert!(id > 0);

    let renamed = app.run(&["deck-rename", &id.to_string(), "Learning"]);
    assert_eq!(renamed.trim(), format!("renamed\t{id}\tLearning"));
    assert!(app.run(&["decks"]).contains("Learning"));

    let card_id = app
        .run(&[
            "add-note",
            "--model",
            "1",
            "--deck",
            &id.to_string(),
            "--field",
            "deck command",
            "--field",
            "answer",
        ])
        .trim()
        .parse::<i64>()
        .unwrap();
    assert!(card_id > 0);

    let failure = app
        .command()
        .args(["deck-remove", &id.to_string()])
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(
        String::from_utf8_lossy(&failure.stderr).contains("--force"),
        "{}",
        String::from_utf8_lossy(&failure.stderr)
    );

    let removed = app.run(&["deck-remove", &id.to_string(), "--force"]);
    assert_eq!(removed.trim(), format!("removed\t{id}\tLearning"));
    let decks = app.run(&["decks"]);
    assert!(!decks.contains("Learning"), "{decks}");

    let store = Collection::open(&path).unwrap();
    assert_eq!(store.find_cards("deck command").unwrap(), [] as [i64; 0]);
}

#[test]
fn deck_remove_refuses_to_delete_the_default_deck() {
    let dir = tempfile::tempdir().unwrap();
    let app = Isolated::new(&dir, "app");
    let output = app
        .command()
        .args(["deck-remove", "1", "--force"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Default deck"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(app.run(&["decks"]).contains("Default"));
}
