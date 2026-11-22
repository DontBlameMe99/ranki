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

use std::path::Path;
use std::path::PathBuf;

use rusqlite::params;

pub use error::FormatError;

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
    let contents = package::read_apkg(imported_package)?;
    let renames = media::copy_media(media_dir, &contents.media)?;
    let media_count = contents.media.len();

    let mut connection = schema::open_or_create(collection)?;
    let incoming = contents.collection.path().to_string_lossy().into_owned();
    connection
        .execute("ATTACH DATABASE ? AS incoming", params![incoming])
        .map_err(error::db_error("cannot open package collection"))?;

    let result =
        merge::merge_collection(&mut connection, &renames, media_count);
    let detach = connection
        .execute_batch("DETACH DATABASE incoming")
        .map_err(error::db_error("cannot close package collection"));
    let report = result?;
    detach?;
    Ok(report)
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
    package::write_apkg(collection, media_dir, destination)
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
