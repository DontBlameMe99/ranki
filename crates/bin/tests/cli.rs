//! End-to-end coverage through the built binary.

#![expect(clippy::unwrap_used, reason = "test assertions")]

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use ranki_model::Store;
use ranki_storage::Collection;
use tempfile::TempDir;

fn run(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ranki"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "command failed: {:?}\n{}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn collection_arg(dir: &TempDir, name: &str) -> (PathBuf, String) {
    let path = dir.path().join(name);
    (path.clone(), path.to_str().unwrap().to_owned())
}

#[test]
fn full_cli_workflow_exports_and_reimports_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    let (source, source_arg) = collection_arg(&dir, "collection.anki2");

    let decks = run(&["--collection", &source_arg, "decks"]);
    assert!(decks.contains("Default"), "{decks}");
    assert!(source.is_file());

    let note_id = run(&[
        "--collection",
        &source_arg,
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

    let found = run(&["--collection", &source_arg, "search", "integration"]);
    assert_eq!(found.trim(), card_id.to_string());

    let due = run(&["--collection", &source_arg, "due", "--limit", "5"]);
    assert!(due.contains(&card_id.to_string()), "{due}");

    run(&[
        "--collection",
        &source_arg,
        "answer",
        &card_id.to_string(),
        "3",
    ]);

    let stats = run(&["--collection", &source_arg, "stats"]);
    assert!(stats.contains("total=1"), "{stats}");

    let package = dir.path().join("backup.apkg");
    let package_arg = package.to_str().unwrap();
    run(&["--collection", &source_arg, "export", package_arg]);
    assert!(package.is_file());

    let (destination, destination_arg) =
        collection_arg(&dir, "fresh/collection.anki2");
    let report =
        run(&["--collection", &destination_arg, "import", package_arg]);
    assert!(report.contains("notes=1"), "{report}");
    assert!(destination.is_file());

    let fresh_stats = run(&["--collection", &destination_arg, "stats"]);
    assert!(fresh_stats.contains("total=1"), "{fresh_stats}");
    assert_eq!(
        run(&["--collection", &destination_arg, "search", "integration"])
            .trim(),
        found.trim()
    );

    let store = Collection::open(&destination).unwrap();
    let imported = store.find_cards("integration").unwrap();
    assert_eq!(imported.len(), 1);
    let note = store
        .note(store.card(imported[0]).unwrap().unwrap().note_id)
        .unwrap()
        .unwrap();
    assert!(note.fields.iter().any(|field| field == "integration front"));
    assert!(note.tags.iter().any(|tag| tag == "cli-test"));
    drop(store);

    assert_eq!(run(&["--collection", &source_arg, "check"]).trim(), "ok");
    assert_eq!(
        run(&["--collection", &destination_arg, "check"]).trim(),
        "ok"
    );
}

#[test]
fn missing_collection_is_created_only_at_the_requested_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/collection.anki2");
    let path_arg = path.to_str().unwrap();
    assert!(!path.exists());
    run(&["--collection", path_arg, "check"]);
    assert!(path.is_file());
    assert_eq!(path.parent(), Some(dir.path().join("nested").as_path()));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn invalid_rating_fails_without_creating_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collection.anki2");
    let output = Command::new(env!("CARGO_BIN_EXE_ranki"))
        .args(["--collection", path.to_str().unwrap(), "answer", "1", "9"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!Path::new(&path).exists());
}

#[test]
fn collection_works_without_home_when_supplied_by_environment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("env.anki2");
    let output = Command::new(env!("CARGO_BIN_EXE_ranki"))
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("RANKI_COLLECTION", path.to_str().unwrap())
        .args(["decks"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(path.is_file());
}

#[test]
fn collection_works_without_home_when_supplied_by_config() {
    let dir = tempfile::tempdir().unwrap();
    let config_home = dir.path().join("config");
    fs::create_dir_all(config_home.join("ranki")).unwrap();
    let path = dir.path().join("configured.anki2");
    fs::write(
        config_home.join("ranki/config.toml"),
        format!("[general]\ncollection = '{}'\n", path.display()),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ranki"))
        .env_remove("HOME")
        .env_remove("RANKI_COLLECTION")
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .args(["decks"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(path.is_file());
}

#[test]
fn unknown_config_keys_are_reported_by_full_path() {
    let dir = tempfile::tempdir().unwrap();
    let config_home = dir.path().join("config");
    fs::create_dir_all(config_home.join("ranki")).unwrap();
    fs::write(
        config_home.join("ranki/config.toml"),
        "[ui]\nthem = 'dark'\n",
    )
    .unwrap();
    let (_path, collection) = collection_arg(&dir, "collection.anki2");
    let output = Command::new(env!("CARGO_BIN_EXE_ranki"))
        .env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
        .args(["--collection", &collection, "decks"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ui.them"), "{stderr}");
}

#[test]
fn deck_create_rename_and_remove_commands_update_the_collection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collection.anki2");
    let arg = path.to_str().unwrap();

    let created =
        run(&["--collection", arg, "deck-create", "Study::Languages"]);
    let fields: Vec<_> = created.trim().split('\t').collect();
    assert_eq!(fields[0], "created");
    assert_eq!(fields[2], "Study::Languages");
    let id: i64 = fields[1].parse().unwrap();
    assert!(id > 0);

    let renamed = run(&[
        "--collection",
        arg,
        "deck-rename",
        &id.to_string(),
        "Learning",
    ]);
    assert_eq!(renamed.trim(), format!("renamed\t{id}\tLearning"));
    assert!(run(&["--collection", arg, "decks"]).contains("Learning"));

    let card_id = run(&[
        "--collection",
        arg,
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

    let failure = Command::new(env!("CARGO_BIN_EXE_ranki"))
        .args(["--collection", arg, "deck-remove", &id.to_string()])
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(
        String::from_utf8_lossy(&failure.stderr).contains("--force"),
        "{}",
        String::from_utf8_lossy(&failure.stderr)
    );

    let removed = run(&[
        "--collection",
        arg,
        "deck-remove",
        &id.to_string(),
        "--force",
    ]);
    assert_eq!(removed.trim(), format!("removed\t{id}\tLearning"));
    let decks = run(&["--collection", arg, "decks"]);
    assert!(!decks.contains("Learning"), "{decks}");

    let store = Collection::open(&path).unwrap();
    assert_eq!(store.find_cards("deck command").unwrap(), [] as [i64; 0]);
}

#[test]
fn deck_remove_refuses_to_delete_the_default_deck() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collection.anki2");
    let arg = path.to_str().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ranki"))
        .args(["--collection", arg, "deck-remove", "1", "--force"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Default deck"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(run(&["--collection", arg, "decks"]).contains("Default"));
}
