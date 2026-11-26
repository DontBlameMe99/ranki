//! `AnkiWeb` media download used after a full collection download.

use crate::error::{Result, SyncError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{Cursor, ErrorKind, Read, Write};
use std::path::{Component, Path};
use tempfile::Builder;

/// Anki downloads at most 25 files in one media ZIP.
pub const DOWNLOAD_BATCH: usize = 25;

#[derive(Serialize)]
pub struct BeginRequest {
    pub v: String,
}

#[derive(Deserialize)]
pub struct BeginReply {
    pub usn: i32,
}

#[derive(Serialize)]
pub struct ChangesRequest {
    #[serde(rename = "lastUsn")]
    pub last_usn: i32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(from = "MediaChangeWire")]
pub struct MediaChange {
    pub fname: String,
    pub usn: i32,
    pub sha1: String,
}

#[derive(Deserialize)]
struct MediaChangeWire(String, i32, String);

impl From<MediaChangeWire> for MediaChange {
    fn from(value: MediaChangeWire) -> Self {
        let MediaChangeWire(fname, usn, sha1) = value;
        Self { fname, usn, sha1 }
    }
}

#[derive(Serialize)]
pub struct DownloadRequest {
    pub files: Vec<String>,
}

#[derive(Deserialize)]
struct JsonReply<T> {
    data: Option<T>,
    #[serde(default)]
    err: String,
}

impl<T: DeserializeOwned> JsonReply<T> {
    fn into_data(self) -> Result<T> {
        if !self.err.is_empty() {
            return Err(SyncError::ServerMessage(self.err));
        }
        self.data.ok_or_else(|| {
            SyncError::Decode("media response has no data".to_owned())
        })
    }
}

pub fn parse_begin(bytes: &[u8]) -> Result<i32> {
    let reply: JsonReply<BeginReply> = serde_json::from_slice(bytes)?;
    Ok(reply.into_data()?.usn)
}

pub fn parse_changes(bytes: &[u8]) -> Result<Vec<MediaChange>> {
    let reply: JsonReply<Vec<MediaChange>> = serde_json::from_slice(bytes)?;
    reply.into_data()
}

/// Accepts a single plain filename, never a path or the media ZIP metadata.
pub fn safe_media_name(name: &str) -> Option<&str> {
    if name == "_meta" || name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    let mut components = path.components();
    let single = matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none();
    single.then_some(name)
}

/// Reports whether the local file already has the server's content.
pub fn local_matches_sha1(
    media_dir: &Path,
    name: &str,
    remote_sha1: &str,
) -> Result<bool> {
    let path = media_dir.join(name);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };
    Ok(sha1_hex(&bytes).eq_ignore_ascii_case(remote_sha1))
}

fn sha1_hex(bytes: &[u8]) -> String {
    let digest = Sha1::digest(bytes);
    let mut text = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Writes the files from an `AnkiWeb` media ZIP into the media directory.
pub fn extract_files(media_dir: &Path, zip: &[u8]) -> Result<usize> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|error| SyncError::Decode(error.to_string()))?;
    let map: HashMap<String, String> = {
        let mut meta = archive
            .by_name("_meta")
            .map_err(|error| SyncError::Decode(error.to_string()))?;
        serde_json::from_reader(&mut meta)?
    };
    if map.len() > DOWNLOAD_BATCH {
        return Err(SyncError::Decode(
            "media ZIP has too many files".to_owned(),
        ));
    }
    fs::create_dir_all(media_dir)?;

    let mut written: usize = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| SyncError::Decode(error.to_string()))?;
        let stored_name = entry
            .name()
            .map_err(|error| SyncError::Decode(error.to_string()))?
            .into_owned();
        if stored_name == "_meta" {
            continue;
        }
        let Some(real_name) = map.get(&stored_name) else {
            return Err(SyncError::Decode(
                "media ZIP has an unmapped file".to_owned(),
            ));
        };
        let Some(name) = safe_media_name(real_name) else {
            return Err(SyncError::Decode(
                "media ZIP has an unsafe filename".to_owned(),
            ));
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if bytes.is_empty() {
            return Err(SyncError::Decode(
                "media ZIP has an empty file".to_owned(),
            ));
        }
        let mut file = Builder::new()
            .prefix(".ranki-media-")
            .tempfile_in(media_dir)?;
        file.as_file_mut().write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(media_dir.join(name))
            .map_err(|error| SyncError::Io(error.to_string()))?;
        written = written.saturating_add(1);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MockResponse, SyncResponse, wire};
    use tempfile::TempDir;
    use zip::write::FileOptions;

    fn framed(value: &serde_json::Value) -> Vec<u8> {
        let response = MockResponse::ok_json(value).expect("frame response");
        wire::decode(&SyncResponse::from(response)).expect("decode response")
    }

    fn media_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
        let options: FileOptions<'_, '_, ()> = FileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut map = HashMap::new();
        for (index, (name, bytes)) in entries.iter().enumerate() {
            let stored = index.to_string();
            map.insert(stored.clone(), (*name).to_owned());
            zip.start_file(stored, options).expect("start file");
            zip.write_all(bytes).expect("write file");
        }
        let meta = serde_json::to_vec(&map).expect("encode map");
        zip.start_file("_meta", options).expect("start meta");
        zip.write_all(&meta).expect("write meta");
        zip.finish().expect("finish zip").into_inner()
    }

    #[test]
    fn parses_media_json_wrappers() {
        let begin = framed(&serde_json::json!({
            "data": { "usn": 4, "sk": "host-key" }, "err": ""
        }));
        assert_eq!(parse_begin(&begin).expect("begin"), 4);

        let changes = framed(&serde_json::json!({
            "data": [["flag.png", 4, "abc"]],
            "err": ""
        }));
        assert_eq!(
            parse_changes(&changes).expect("changes"),
            [MediaChange {
                fname: "flag.png".to_owned(),
                usn: 4,
                sha1: "abc".to_owned(),
            }]
        );

        let error = framed(&serde_json::json!({ "err": "server said no" }));
        assert_eq!(
            parse_begin(&error).unwrap_err(),
            SyncError::ServerMessage("server said no".to_owned())
        );
    }

    #[test]
    fn rejects_media_paths_outside_the_directory() {
        assert_eq!(safe_media_name("flag.png"), Some("flag.png"));
        assert_eq!(safe_media_name("../flag.png"), None);
        assert_eq!(safe_media_name("flags/flag.png"), None);
        assert_eq!(safe_media_name("_meta"), None);
    }

    #[test]
    fn extracts_named_files_from_a_media_zip() {
        let dir = TempDir::new().expect("tempdir");
        let zip = media_zip(&[("flag.png", b"image bytes")]);
        assert_eq!(extract_files(dir.path(), &zip).expect("extract"), 1);
        assert_eq!(
            fs::read(dir.path().join("flag.png")).expect("read file"),
            b"image bytes"
        );
        assert!(
            local_matches_sha1(
                dir.path(),
                "flag.png",
                &sha1_hex(b"image bytes")
            )
            .expect("hash")
        );
    }

    #[test]
    fn rejects_unsafe_names_inside_a_media_zip() {
        let dir = TempDir::new().expect("tempdir");
        let zip = media_zip(&[("../flag.png", b"image bytes")]);
        assert!(extract_files(dir.path(), &zip).is_err());
        assert!(!dir.path().join("flag.png").exists());
    }
}
