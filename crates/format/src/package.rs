use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::str;

use rusqlite::Connection;
use rusqlite::MAIN_DB;
use rusqlite::backup::Progress;
use serde::Serialize;
use serde_json::Value;
use tempfile::NamedTempFile;
use zip::CompressionMethod;
use zip::ZipArchive;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;
use zstd::stream::decode_all;

use crate::FormatError;
use crate::error::db_error;
use crate::error::io_error;
use crate::error::json_error;
use crate::error::zip_error;
use crate::media::MediaEntry;
use crate::media::MediaFile;
use crate::media::collect_media_files;
use crate::media::sanitize_filename;
use crate::media::write_new_file;
use crate::schema;

const PACKAGE_META_LEGACY_2: &[u8] = &[0x08, 0x02];

pub struct ApkgPackage {
    pub collection: NamedTempFile,
    pub media: Vec<MediaEntry>,
}

pub fn read_apkg(package: &Path) -> Result<ApkgPackage, FormatError> {
    let file = File::open(package).map_err(io_error(format!(
        "cannot open package {}",
        package.display()
    )))?;
    let mut archive = ZipArchive::new(file).map_err(zip_error(format!(
        "malformed apkg package {}",
        package.display()
    )))?;
    let names = entry_names(&archive)?;

    let Some(collection_name) = choose_collection(&names) else {
        return Err(FormatError::new(
            "package has no supported collection file",
        ));
    };
    let collection_bytes =
        maybe_decompress(read_bytes(&mut archive, collection_name)?)?;
    let mapping = read_media_mapping(&mut archive, &names)?;
    let media = read_media_entries(&mut archive, &mapping)?;
    let collection = write_temp_collection(&collection_bytes)?;

    Ok(ApkgPackage { collection, media })
}

pub fn write_apkg(
    collection: &Path,
    media_dir: &Path,
    destination: &Path,
) -> Result<(), FormatError> {
    schema::ensure_parent(destination)?;
    let database = snapshot_collection(collection)?;
    let media_files = collect_media_files(media_dir)?;

    let file = File::create(destination).map_err(io_error(format!(
        "cannot create package {}",
        destination.display()
    )))?;
    let mut writer = ZipWriter::new(BufWriter::new(file));

    write_zip_entry(&mut writer, "collection.anki21", &database)?;
    let mapping = media_mapping(&media_files);
    write_json_entry(&mut writer, "media", &mapping)?;
    for (index, media) in media_files.iter().enumerate() {
        let bytes = fs::read(&media.path).map_err(io_error(format!(
            "cannot read media file {}",
            media.path.display()
        )))?;
        write_zip_entry(&mut writer, &index.to_string(), &bytes)?;
    }
    write_zip_entry(&mut writer, "meta", PACKAGE_META_LEGACY_2)?;

    let output = writer
        .finish()
        .map_err(zip_error("cannot finish apkg package"))?;
    output.into_inner().map_err(|error| {
        FormatError::with_source(
            format!("cannot flush package {}", destination.display()),
            error.into_error(),
        )
    })?;
    Ok(())
}

pub fn write_colpkg(
    collection: &Path,
    media_dir: &Path,
    destination: &Path,
) -> Result<(), FormatError> {
    write_apkg(collection, media_dir, destination)
}

pub fn extract_colpkg(
    package: &Path,
    destination_dir: &Path,
) -> Result<PathBuf, FormatError> {
    let file = File::open(package).map_err(io_error(format!(
        "cannot open package {}",
        package.display()
    )))?;
    let mut archive = ZipArchive::new(file).map_err(zip_error(format!(
        "malformed colpkg package {}",
        package.display()
    )))?;
    let names = entry_names(&archive)?;
    let Some(collection_name) = choose_collection(&names) else {
        return Err(FormatError::new(
            "package has no supported collection file",
        ));
    };

    fs::create_dir_all(destination_dir).map_err(io_error(format!(
        "cannot create destination {}",
        destination_dir.display()
    )))?;
    let destination = destination_dir.join(collection_name);
    if destination.exists() {
        return Err(FormatError::new(format!(
            "collection already exists: {}",
            destination.display()
        )));
    }
    let collection_bytes =
        maybe_decompress(read_bytes(&mut archive, collection_name)?)?;
    write_new_file(
        &destination,
        &collection_bytes,
        &destination.display().to_string(),
    )?;

    let media_dir = destination_dir.join("media");
    fs::create_dir_all(&media_dir).map_err(io_error(format!(
        "cannot create media directory {}",
        media_dir.display()
    )))?;
    let mapping = read_media_mapping(&mut archive, &names)?;
    for (entry, name) in &mapping {
        extract_media_file(&mut archive, &media_dir, (entry, name))?;
    }
    for entry in names.iter().filter(|name| is_media_entry(name)) {
        let relative = entry.trim_start_matches("media/");
        extract_media_file(&mut archive, &media_dir, (entry, relative))?;
    }

    if names.contains("meta") {
        let meta_path = destination_dir.join("meta");
        if !meta_path.exists() {
            let bytes = read_bytes(&mut archive, "meta")?;
            write_new_file(
                &meta_path,
                &bytes,
                &meta_path.display().to_string(),
            )?;
        }
    }
    Ok(destination)
}

fn entry_names(
    archive: &ZipArchive<File>,
) -> Result<BTreeSet<String>, FormatError> {
    archive
        .file_names()
        .map(|name| {
            name.map(Cow::into_owned)
                .map_err(zip_error("cannot read package entry names"))
        })
        .collect()
}

fn choose_collection(names: &BTreeSet<String>) -> Option<&'static str> {
    if names.contains("collection.anki21b") {
        Some("collection.anki21b")
    } else if names.contains("collection.anki21") {
        Some("collection.anki21")
    } else if names.contains("collection.anki2") {
        Some("collection.anki2")
    } else {
        None
    }
}

fn is_media_entry(name: &str) -> bool {
    name.starts_with("media/") && !name.ends_with('/')
}

fn extract_media_file(
    archive: &mut ZipArchive<File>,
    media_dir: &Path,
    (entry, name): (&str, &str),
) -> Result<(), FormatError> {
    let filename = sanitize_filename(name);
    if filename.is_empty() {
        return Err(FormatError::new(format!(
            "invalid media filename in package: '{name}'"
        )));
    }
    let target = media_dir.join(filename);
    if target.exists() {
        return Ok(());
    }
    let bytes = maybe_decompress(read_bytes(archive, entry)?)?;
    write_new_file(&target, &bytes, &target.display().to_string())
}

fn is_numbered_entry(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
}

fn read_bytes(
    archive: &mut ZipArchive<File>,
    name: &str,
) -> Result<Vec<u8>, FormatError> {
    let mut entry = archive
        .by_name(name)
        .map_err(zip_error(format!("cannot read package entry '{name}'")))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(io_error(format!("cannot read package entry '{name}'")))?;
    Ok(bytes)
}

fn read_media_mapping(
    archive: &mut ZipArchive<File>,
    names: &BTreeSet<String>,
) -> Result<BTreeMap<String, String>, FormatError> {
    if !names.contains("media") {
        if names.iter().any(|name| is_numbered_entry(name)) {
            return Err(FormatError::new(
                "package has media files but no media mapping",
            ));
        }
        return Ok(BTreeMap::new());
    }

    let bytes = maybe_decompress(read_bytes(archive, "media")?)?;
    let mapping = parse_media_mapping(&bytes)?;
    for key in mapping.keys() {
        if !names.contains(key) {
            return Err(FormatError::new(format!(
                "media entry '{key}' is missing from package"
            )));
        }
    }
    for name in names.iter().filter(|name| is_numbered_entry(name)) {
        if !mapping.contains_key(name) {
            return Err(FormatError::new(format!(
                "media entry '{name}' has no media mapping"
            )));
        }
    }
    Ok(mapping)
}

fn parse_media_mapping(
    bytes: &[u8],
) -> Result<BTreeMap<String, String>, FormatError> {
    serde_json::from_slice::<BTreeMap<String, String>>(bytes)
        .map_or_else(|_| parse_anki_media_entries(bytes), Ok)
}

fn parse_anki_media_entries(
    bytes: &[u8],
) -> Result<BTreeMap<String, String>, FormatError> {
    let mut mapping = BTreeMap::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (tag, after_tag) = read_varint(bytes, offset)?;
        if tag != 0x0A {
            return Err(invalid_media_mapping());
        }
        let (raw_length, start) = read_varint(bytes, after_tag)?;
        let length =
            usize::try_from(raw_length).map_err(|_| invalid_media_mapping())?;
        let end = start
            .checked_add(length)
            .ok_or_else(invalid_media_mapping)?;
        let message =
            bytes.get(start..end).ok_or_else(invalid_media_mapping)?;
        let name = parse_media_name(message)?;
        let key = mapping.len().to_string();
        mapping.insert(key, name);
        offset = end;
    }
    Ok(mapping)
}

#[expect(
    clippy::excessive_nesting,
    reason = "protobuf field handling is clearer inline"
)]
fn parse_media_name(message: &[u8]) -> Result<String, FormatError> {
    let mut offset = 0;
    let mut name = None;
    while offset < message.len() {
        let (tag, after_tag) = read_varint(message, offset)?;
        match tag & 0x07 {
            0 => offset = read_varint(message, after_tag)?.1,
            2 => {
                let (raw_length, start) = read_varint(message, after_tag)?;
                let length = usize::try_from(raw_length)
                    .map_err(|_| invalid_media_mapping())?;
                let end = start
                    .checked_add(length)
                    .ok_or_else(invalid_media_mapping)?;
                if end > message.len() {
                    return Err(invalid_media_mapping());
                }
                if tag == 0x0A {
                    let bytes = message
                        .get(start..end)
                        .ok_or_else(invalid_media_mapping)?;
                    let value = str::from_utf8(bytes)
                        .map_err(|_| invalid_media_mapping())?;
                    name = Some(value.to_owned());
                }
                offset = end;
            }
            _ => return Err(invalid_media_mapping()),
        }
    }
    name.ok_or_else(invalid_media_mapping)
}

fn read_varint(
    bytes: &[u8],
    mut offset: usize,
) -> Result<(u64, usize), FormatError> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    loop {
        let byte = *bytes.get(offset).ok_or_else(invalid_media_mapping)?;
        offset = offset.checked_add(1).ok_or_else(invalid_media_mapping)?;
        let part = u64::from(byte & 0x7F)
            .checked_shl(shift)
            .ok_or_else(invalid_media_mapping)?;
        value = value.checked_add(part).ok_or_else(invalid_media_mapping)?;
        if byte & 0x80 == 0 {
            return Ok((value, offset));
        }
        shift = shift.checked_add(7).ok_or_else(invalid_media_mapping)?;
    }
}

fn invalid_media_mapping() -> FormatError {
    FormatError::new("invalid media mapping in package")
}

fn read_media_entries(
    archive: &mut ZipArchive<File>,
    mapping: &BTreeMap<String, String>,
) -> Result<Vec<MediaEntry>, FormatError> {
    let mut media = Vec::new();
    for (key, name) in mapping {
        let bytes = maybe_decompress(read_bytes(archive, key)?)?;
        media.push(MediaEntry {
            name: name.clone(),
            bytes,
        });
    }
    Ok(media)
}

const ZSTD_MAGIC: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD];

pub fn maybe_decompress(bytes: Vec<u8>) -> Result<Vec<u8>, FormatError> {
    if bytes.starts_with(ZSTD_MAGIC) {
        decode_all(bytes.as_slice())
            .map_err(io_error("cannot decompress zstd package data"))
    } else {
        Ok(bytes)
    }
}

fn write_temp_collection(bytes: &[u8]) -> Result<NamedTempFile, FormatError> {
    let mut file = NamedTempFile::new()
        .map_err(io_error("cannot create temporary collection"))?;
    file.write_all(bytes)
        .map_err(io_error("cannot write temporary collection"))?;
    file.flush()
        .map_err(io_error("cannot flush temporary collection"))?;
    Ok(file)
}

fn snapshot_collection(collection: &Path) -> Result<Vec<u8>, FormatError> {
    if !collection.is_file() {
        return Err(FormatError::new(format!(
            "collection does not exist: {}",
            collection.display()
        )));
    }
    let source = Connection::open(collection).map_err(db_error(format!(
        "cannot open collection {}",
        collection.display()
    )))?;
    let snapshot = NamedTempFile::new()
        .map_err(io_error("cannot create collection snapshot"))?;
    source
        .backup(MAIN_DB, snapshot.path(), None::<fn(Progress)>)
        .map_err(db_error("cannot snapshot collection"))?;
    drop(source);
    let snapshot_db = Connection::open(snapshot.path())
        .map_err(db_error("cannot open collection snapshot"))?;
    ranki_storage::register_collations(&snapshot_db).map_err(|error| {
        FormatError::with_source("cannot register collection collations", error)
    })?;
    normalize_pending_usns(&snapshot_db)?;
    normalize_col_metadata(&snapshot_db)?;
    drop(snapshot_db);
    fs::read(snapshot.path())
        .map_err(io_error("cannot read collection snapshot"))
}
fn normalize_pending_usns(conn: &Connection) -> Result<(), FormatError> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(db_error("cannot inspect collection snapshot"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(db_error("cannot inspect collection snapshot"))?;
    let mut tables = Vec::new();
    for row in rows {
        tables
            .push(row.map_err(db_error("cannot inspect collection snapshot"))?);
    }
    drop(statement);
    for table in tables {
        if [
            "notes",
            "cards",
            "revlog",
            "graves",
            "config",
            "notetypes",
            "templates",
            "decks",
            "deck_config",
            "tags",
            "col",
        ]
        .contains(&table.as_str())
        {
            conn.execute(
                &format!("UPDATE {table} SET usn = 0 WHERE usn < 0"),
                [],
            )
            .map_err(db_error("cannot normalize pending sync ids"))?;
        }
    }
    Ok(())
}

fn normalize_col_metadata(conn: &Connection) -> Result<(), FormatError> {
    for key in ["conf", "models", "decks", "dconf", "tags"] {
        let raw: Option<String> = conn
            .query_row(
                &format!("SELECT {key} FROM col WHERE id = 1"),
                [],
                |row| row.get(0),
            )
            .map_err(db_error("cannot read collection metadata"))?;
        let Some(value) = raw else {
            continue;
        };
        let serialized = if value.contains("\"usn\":-1") {
            normalize_metadata(&value)?
        } else {
            value
        };
        conn.execute(
            &format!("UPDATE col SET {key} = ?1 WHERE id = 1"),
            [serialized],
        )
        .map_err(db_error("cannot normalize pending sync ids"))?;
    }
    Ok(())
}

fn normalize_metadata(raw: &str) -> Result<String, FormatError> {
    let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
        return Ok(raw.to_owned());
    };
    serde_json::to_string(&normalize_usn(parsed))
        .map_err(json_error("cannot encode collection metadata"))
}

fn normalize_usn(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            if map
                .get("usn")
                .is_some_and(|entry| entry.as_i64() == Some(-1))
            {
                let _ = map.insert("usn".to_owned(), Value::from(0));
            }
            Value::Object(
                map.into_iter()
                    .map(|(key, entry)| (key, normalize_usn(entry)))
                    .collect(),
            )
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(normalize_usn).collect())
        }
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)) => other,
    }
}

fn media_mapping(media_files: &[MediaFile]) -> BTreeMap<String, String> {
    media_files
        .iter()
        .enumerate()
        .map(|(index, file)| (index.to_string(), file.name.clone()))
        .collect()
}

fn zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

fn write_zip_entry(
    writer: &mut ZipWriter<BufWriter<File>>,
    name: &str,
    bytes: &[u8],
) -> Result<(), FormatError> {
    writer
        .start_file(name, zip_options())
        .map_err(zip_error(format!("cannot add package entry '{name}'")))?;
    writer
        .write_all(bytes)
        .map_err(io_error(format!("cannot write package entry '{name}'")))?;
    Ok(())
}

fn write_json_entry<T: Serialize>(
    writer: &mut ZipWriter<BufWriter<File>>,
    name: &str,
    value: &T,
) -> Result<(), FormatError> {
    let bytes = serde_json::to_vec(value)
        .map_err(json_error(format!("cannot encode {name}")))?;
    write_zip_entry(writer, name, &bytes)
}
