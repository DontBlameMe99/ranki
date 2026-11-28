//! Anki package import and export.
//!
//! The crate reads and writes `.apkg` and `.colpkg` files directly. Collection
//! changes are applied through SQLite so existing notes, cards and metadata
//! survive an import.

mod error;
mod media;
mod merge;
mod package;
mod schema;

pub use error::FormatError;
use ranki_model::Id;
use ranki_storage::Collection;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use rusqlite::params_from_iter;
use serde_json::Value;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use tempfile::Builder;
use tempfile::NamedTempFile;

/// Counts inserted records from an `.apkg` import.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub notes: usize,
    pub cards: usize,
    pub media: usize,
}

/// Merge an `.apkg` into an existing collection, creating one when needed.
///
/// # Errors
///
/// Returns an error when the package is malformed, media cannot be copied, or
/// the destination collection cannot be opened and updated.
pub fn import_apkg(
    imported_package: &Path,
    collection: &Path,
    media_dir: &Path,
) -> Result<ImportReport, FormatError> {
    let scratch_dir = collection
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    schema::ensure_parent(collection)?;
    let contents = package::read_apkg(imported_package, scratch_dir)?;
    let source_crt = package_creation_time(contents.collection.path())?;
    let copied_media = media::copy_media(media_dir, &contents.media)?;
    let media_count = contents.media.len();

    let mut connection = schema::open_or_create(collection, Some(source_crt))?;
    let incoming = contents.collection.path().to_string_lossy().into_owned();
    connection
        .execute("ATTACH DATABASE ? AS incoming", params![incoming])
        .map_err(error::db_error("cannot open package collection"))?;

    let result = merge::merge_collection(
        &mut connection,
        copied_media.renames(),
        media_count,
    );
    let detach = connection
        .execute_batch("DETACH DATABASE incoming")
        .map_err(error::db_error("cannot close package collection"));
    let report = result?;
    copied_media.keep();
    detach?;
    Ok(report)
}

fn package_creation_time(path: &Path) -> Result<i64, FormatError> {
    let connection = rusqlite::Connection::open(path)
        .map_err(error::db_error("cannot open package collection"))?;
    connection
        .query_row("SELECT crt FROM col WHERE id = 1", [], |row| row.get(0))
        .map_err(error::db_error("cannot read package creation time"))
}

/// Export a collection into an `.apkg` file Anki can import.
///
/// # Errors
///
/// Returns an error when the collection cannot be read, media cannot be read,
/// or the destination package cannot be written.
pub fn export_apkg(
    collection: &Path,
    media_dir: &Path,
    destination: &Path,
) -> Result<(), FormatError> {
    package::write_apkg(collection, media_dir, destination, true)
}

/// Export a collection and publish without replacing a file that appeared
/// after the caller checked the destination.
///
/// # Errors
///
/// Returns an error when the collection or media cannot be read, or the
/// destination already exists.
pub fn export_apkg_new(
    collection: &Path,
    media_dir: &Path,
    destination: &Path,
) -> Result<(), FormatError> {
    package::write_apkg(collection, media_dir, destination, false)
}

/// Export one deck and its subdecks into an `.apkg` file Anki can import.
///
/// # Errors
///
/// Returns an error when the deck does not exist, the collection cannot be
/// read, or the destination package cannot be written.
#[expect(
    clippy::too_many_arguments,
    reason = "collection, media, destination, and deck id are separate inputs"
)]
pub fn export_deck_apkg(
    collection: &Path,
    media_dir: &Path,
    destination: &Path,
    deck_id: Id,
) -> Result<(), FormatError> {
    let snapshot = deck_snapshot(collection)?;
    let subtree = deck_subtree(snapshot.path(), deck_id)?;
    if subtree.is_empty() {
        return Err(FormatError::new(format!(
            "deck does not exist: {deck_id}"
        )));
    }

    let connection = Connection::open(snapshot.path())
        .map_err(error::db_error("cannot open deck export snapshot"))?;
    ranki_storage::register_collations(&connection).map_err(|error| {
        FormatError::with_source(
            "cannot register deck export collations",
            error,
        )
    })?;
    filter_records(&connection, &subtree)?;
    filter_deck_metadata(&connection, &subtree)?;
    let media = referenced_media(&connection, media_dir)?;
    drop(connection);
    package::write_apkg_files(snapshot.path(), &media, destination, true)
}

fn deck_snapshot(collection: &Path) -> Result<NamedTempFile, FormatError> {
    let bytes = package::snapshot_collection(collection)?;
    let parent = collection
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut snapshot = Builder::new()
        .prefix(".ranki-deck-")
        .suffix(".anki2")
        .tempfile_in(parent)
        .map_err(error::io_error("cannot create deck export snapshot"))?;
    snapshot
        .write_all(&bytes)
        .map_err(error::io_error("cannot write deck export snapshot"))?;
    snapshot
        .flush()
        .map_err(error::io_error("cannot flush deck export snapshot"))?;
    Ok(snapshot)
}

fn deck_subtree(path: &Path, deck_id: Id) -> Result<Vec<Id>, FormatError> {
    let store = Collection::open(path).map_err(|error| {
        FormatError::with_source("cannot read deck export snapshot", error)
    })?;
    store.deck_subtree(deck_id).map_err(|error| {
        FormatError::with_source("cannot read exported deck", error)
    })
}

fn filter_records(
    connection: &Connection,
    subtree: &[Id],
) -> Result<(), FormatError> {
    let list = placeholders(subtree.len());
    connection
        .execute(
            &format!("DELETE FROM cards WHERE did NOT IN ({list})"),
            params_from_iter(subtree.iter().copied()),
        )
        .map_err(error::db_error("cannot filter exported cards"))?;
    connection
        .execute_batch(
            "DELETE FROM revlog WHERE cid NOT IN (SELECT id FROM cards);
             DELETE FROM notes WHERE id NOT IN (SELECT nid FROM cards);
             DELETE FROM graves;",
        )
        .map_err(error::db_error("cannot filter exported records"))?;

    let has_deck_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'decks')",
            [],
            |row| row.get(0),
        )
        .map_err(error::db_error("cannot inspect exported decks"))?;
    if has_deck_table {
        connection
            .execute(
                &format!("DELETE FROM decks WHERE id NOT IN ({list})"),
                params_from_iter(subtree.iter().copied()),
            )
            .map_err(error::db_error("cannot filter exported decks"))?;
    }
    Ok(())
}

fn filter_deck_metadata(
    connection: &Connection,
    subtree: &[Id],
) -> Result<(), FormatError> {
    let raw_metadata: Option<String> = connection
        .query_row("SELECT decks FROM col WHERE id = 1", [], |row| row.get(0))
        .optional()
        .map_err(error::db_error("cannot read exported deck metadata"))?;
    let Some(stored) = raw_metadata.filter(|value| !value.trim().is_empty())
    else {
        return Ok(());
    };
    let mut value: Value = serde_json::from_str(&stored)
        .map_err(error::json_error("cannot read exported deck metadata"))?;
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    object.retain(|key, _| {
        key.parse::<Id>().is_ok_and(|id| subtree.contains(&id))
    });
    let filtered = serde_json::to_string(&value)
        .map_err(error::json_error("cannot write exported deck metadata"))?;
    if filtered == stored {
        return Ok(());
    }
    connection
        .execute("UPDATE col SET decks = ?1 WHERE id = 1", params![filtered])
        .map_err(error::db_error("cannot write exported deck metadata"))?;
    Ok(())
}

fn placeholders(count: usize) -> String {
    (0..count).map(|_| "?").collect::<Vec<_>>().join(", ")
}

fn referenced_media(
    connection: &Connection,
    media_dir: &Path,
) -> Result<Vec<media::MediaFile>, FormatError> {
    let files = media::collect_media_files(media_dir)?;
    let mut statement = connection
        .prepare("SELECT DISTINCT mid FROM notes")
        .map_err(error::db_error("cannot read exported notes"))?;
    let model_ids = statement
        .query_map([], |row| row.get::<_, Id>(0))
        .map_err(error::db_error("cannot read exported notes"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error::db_error("cannot read exported notes"))?;
    drop(statement);

    let mut statement = connection
        .prepare("SELECT flds FROM notes")
        .map_err(error::db_error("cannot read exported notes"))?;
    let mut fields = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(error::db_error("cannot read exported notes"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error::db_error("cannot read exported notes"))?;
    drop(statement);
    fields.extend(model_text(connection, &model_ids)?);

    let references = fields.join("\u{1f}");
    Ok(files
        .into_iter()
        .filter(|file| {
            let encoded = media::percent_encode(&file.name);
            contains_reference(&references, &file.name)
                || contains_reference(&references, &encoded)
        })
        .collect())
}
fn model_text(
    connection: &Connection,
    model_ids: &[Id],
) -> Result<Vec<String>, FormatError> {
    let raw_models: Option<String> = connection
        .query_row("SELECT models FROM col WHERE id = 1", [], |row| row.get(0))
        .optional()
        .map_err(error::db_error("cannot read exported note types"))?;
    let Some(stored) = raw_models.filter(|value| !value.trim().is_empty())
    else {
        return table_model_text(connection, model_ids);
    };
    let mut value: Value = serde_json::from_str(&stored)
        .map_err(error::json_error("cannot read exported note types"))?;
    let Some(object) = value.as_object_mut() else {
        return table_model_text(connection, model_ids);
    };
    object.retain(|key, _| {
        key.parse::<Id>().is_ok_and(|id| model_ids.contains(&id))
    });
    if object.is_empty() {
        return table_model_text(connection, model_ids);
    }
    let filtered = serde_json::to_string(&value)
        .map_err(error::json_error("cannot write exported note types"))?;
    Ok(vec![filtered])
}

fn table_model_text(
    connection: &Connection,
    model_ids: &[Id],
) -> Result<Vec<String>, FormatError> {
    let has_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'notetypes')",
            [],
            |row| row.get(0),
        )
        .map_err(error::db_error("cannot inspect exported note types"))?;
    if !has_table {
        return Ok(Vec::new());
    }
    let models =
        ranki_storage::models_from_tables_with_prefix(connection, "main")
            .map_err(|error| {
                FormatError::with_source(
                    "cannot read exported note types",
                    error,
                )
            })?;
    let mut texts = Vec::new();
    for model in models
        .into_iter()
        .filter(|model| model_ids.contains(&model.id))
    {
        texts.push(model.css);
        for template in model.templates {
            texts.push(template.question_format);
            texts.push(template.answer_format);
        }
    }
    Ok(texts)
}

fn contains_reference(field: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut offset = 0;
    while let Some(found) = field[offset..].find(name) {
        let start = offset.saturating_add(found);
        let end = start.saturating_add(name.len());
        let before = field[..start].chars().next_back();
        let after = field[end..].chars().next();
        if before.is_none_or(|character| !is_filename_char(character))
            && after.is_none_or(|character| !is_filename_char(character))
        {
            return true;
        }
        offset = field[start..].chars().next().map_or(end, |character| {
            start.saturating_add(character.len_utf8())
        });
    }
    false
}

fn is_filename_char(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '-' | '_' | '.')
}

/// Export a collection into a `.colpkg` backup Anki can import.
///
/// # Errors
///
/// Returns an error when the collection or media cannot be read, or the
/// destination package cannot be written.
pub fn export_colpkg(
    collection: &Path,
    media_dir: &Path,
    destination: &Path,
) -> Result<(), FormatError> {
    package::write_colpkg(collection, media_dir, destination, true)
}

/// Export a colpkg and publish without replacing an existing destination.
///
/// # Errors
///
/// Returns an error when the collection or media cannot be read, or the
/// destination already exists.
pub fn export_colpkg_new(
    collection: &Path,
    media_dir: &Path,
    destination: &Path,
) -> Result<(), FormatError> {
    package::write_colpkg(collection, media_dir, destination, false)
}

/// Extract a `.colpkg` backup without overwriting existing files.
///
/// # Errors
///
/// Returns an error when the package is malformed or the destination already
/// contains a collection file.
pub fn import_colpkg(
    imported_package: &Path,
    destination_dir: &Path,
) -> Result<PathBuf, FormatError> {
    package::extract_colpkg(imported_package, destination_dir)
}
