#![expect(clippy::unwrap_used, reason = "test fixtures use unwrap for setup")]

use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use ranki_format::export_apkg;
use ranki_format::import_apkg;
use ranki_storage::Collection;
use rusqlite::Connection;
use serde_json::json;
use zip::ZipArchive;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn temp_packages(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy().starts_with(".ranki-export-")
            })
        })
        .collect();
    paths.sort();
    paths
}

fn build_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    for (name, bytes) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

#[cfg(unix)]
#[test]
fn failed_export_leaves_no_partial_package_and_retry_succeeds() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.anki2");
    {
        let _store = Collection::create(&source).unwrap();
    }
    let media = dir.path().join("media");
    fs::create_dir_all(&media).unwrap();
    let blocked = media.join("blocked.png");
    fs::write(&blocked, b"blocked").unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o0)).unwrap();
    if fs::read(&blocked).is_ok() {
        eprintln!(
            "skipping unreadable media check: process can read mode 000 files"
        );
        return;
    }

    let destination = dir.path().join("deck.apkg");
    let error = export_apkg(&source, &media, &destination).unwrap_err();
    assert!(
        error.to_string().contains("cannot read media file"),
        "{error}"
    );
    assert!(!destination.exists());
    let leftovers = temp_packages(dir.path());
    assert!(leftovers.is_empty(), "{leftovers:?}");

    fs::write(&destination, b"old package").unwrap();
    let _error = export_apkg(&source, &media, &destination).unwrap_err();
    assert_eq!(fs::read(&destination).unwrap(), b"old package".to_vec());
    let leftovers = temp_packages(dir.path());
    assert!(leftovers.is_empty(), "{leftovers:?}");

    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o644)).unwrap();
    export_apkg(&source, &media, &destination).unwrap();
    assert!(ZipArchive::new(File::open(&destination).unwrap()).is_ok());
    let leftovers = temp_packages(dir.path());
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn failed_import_removes_only_media_created_by_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.anki2");
    {
        let _store = Collection::create(&source).unwrap();
    }
    {
        let connection = Connection::open(&source).unwrap();
        connection
            .execute("UPDATE col SET decks = ?1 WHERE id = 1", ["not-json"])
            .unwrap();
    }

    let package = dir.path().join("broken.apkg");
    let mapping = json!({"0": "fresh.png", "1": "shared.png"})
        .to_string()
        .into_bytes();
    build_zip(
        &package,
        &[
            ("collection.anki21", fs::read(&source).unwrap()),
            ("media", mapping),
            ("0", b"fresh bytes".to_vec()),
            ("1", b"shared bytes".to_vec()),
        ],
    );

    let media = dir.path().join("media");
    fs::create_dir_all(&media).unwrap();
    fs::write(media.join("shared.png"), b"shared bytes").unwrap();
    fs::write(media.join("keep.png"), b"keep").unwrap();

    let error =
        import_apkg(&package, &dir.path().join("imported.anki2"), &media)
            .unwrap_err();
    assert!(
        error.to_string().contains("invalid collection decks JSON"),
        "{error}"
    );

    assert!(!media.join("fresh.png").exists());
    assert_eq!(
        fs::read(media.join("shared.png")).unwrap(),
        b"shared bytes".to_vec()
    );
    assert_eq!(fs::read(media.join("keep.png")).unwrap(), b"keep".to_vec());
}

#[test]
fn failed_media_copy_removes_files_written_before_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.anki2");
    {
        let _store = Collection::create(&source).unwrap();
    }
    let package = dir.path().join("bad-name.apkg");
    let mapping = json!({"0": "photo.png", "1": ".."})
        .to_string()
        .into_bytes();
    build_zip(
        &package,
        &[
            ("collection.anki21", fs::read(&source).unwrap()),
            ("media", mapping),
            ("0", b"photo".to_vec()),
            ("1", b"ignored".to_vec()),
        ],
    );

    let media = dir.path().join("media");
    let error =
        import_apkg(&package, &dir.path().join("imported.anki2"), &media)
            .unwrap_err();
    assert!(
        error.to_string().contains("invalid media filename"),
        "{error}"
    );

    assert!(!media.join("photo.png").exists());
    let entries: Vec<_> = fs::read_dir(&media).unwrap().collect();
    assert!(entries.is_empty(), "{entries:?}");
}
