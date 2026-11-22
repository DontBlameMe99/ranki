use std::borrow::Cow;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use crate::FormatError;

use crate::error::io_error;

/// A media file stored in a package.
#[derive(Debug, Clone)]
pub struct MediaEntry {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// A filename change that must be applied to note fields.
#[derive(Debug, Clone)]
pub struct MediaRename {
    pub from: String,
    pub to: String,
}

/// A media file found in a media directory during export.
#[derive(Debug, Clone)]
pub struct MediaFile {
    pub name: String,
    pub path: PathBuf,
}

pub fn sanitize_filename(name: &str) -> String {
    let trimmed = name.trim();
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let cleaned: String = base
        .trim_matches(|character| character == '.' || character == ' ')
        .chars()
        .map(|character| {
            if is_allowed_media_char(character) {
                character
            } else {
                '_'
            }
        })
        .collect();

    if cleaned.is_empty()
        || cleaned == "."
        || cleaned == ".."
        || is_reserved_name(&cleaned)
    {
        String::new()
    } else {
        cleaned
    }
}

fn is_allowed_media_char(character: char) -> bool {
    character.is_alphanumeric()
        || matches!(
            character,
            '-' | '_'
                | '.'
                | ' '
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '+'
                | ','
                | '\''
                | '!'
                | '~'
        )
}

fn is_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let base =
        stem.trim_end_matches(|character: char| character.is_ascii_digit());
    matches!(base, "CON" | "PRN" | "AUX" | "NUL" | "COM" | "LPT")
}

pub fn collect_media_files(
    media_dir: &Path,
) -> Result<Vec<MediaFile>, FormatError> {
    if !media_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let entries = fs::read_dir(media_dir).map_err(io_error(format!(
        "cannot read media directory {}",
        media_dir.display()
    )))?;
    for directory_entry in entries {
        let entry = directory_entry.map_err(io_error(format!(
            "cannot read media directory {}",
            media_dir.display()
        )))?;
        let file_type = entry.file_type().map_err(io_error(format!(
            "cannot inspect media file {}",
            entry.path().display()
        )))?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            return Err(FormatError::new(format!(
                "media filename is not valid UTF-8: {}",
                entry.path().display()
            )));
        };
        files.push(MediaFile {
            name: name.to_owned(),
            path: entry.path(),
        });
    }
    files.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(files)
}

#[expect(
    clippy::excessive_nesting,
    reason = "media copying has a fixed set of filename and collision cases"
)]
pub fn copy_media(
    media_dir: &Path,
    entries: &[MediaEntry],
) -> Result<Vec<MediaRename>, FormatError> {
    fs::create_dir_all(media_dir).map_err(io_error(format!(
        "cannot create media directory {}",
        media_dir.display()
    )))?;

    let mut renames = Vec::new();
    for entry in entries {
        let preferred = sanitize_filename(&entry.name);
        if preferred.is_empty() {
            return Err(FormatError::new(format!(
                "invalid media filename in package: '{}'",
                entry.name
            )));
        }

        let final_name = unique_name(media_dir, &preferred, &entry.bytes)?;
        let destination = media_dir.join(&final_name);
        if !destination.exists() {
            write_new_file(&destination, &entry.bytes)?;
        }

        let mut sources = vec![entry.name.as_str()];
        if !sources.contains(&preferred.as_str()) {
            sources.push(preferred.as_str());
        }
        for source in sources {
            if source != final_name {
                renames.push(MediaRename {
                    from: source.to_owned(),
                    to: final_name.clone(),
                });
            }
        }
    }
    Ok(renames)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), FormatError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error(format!(
            "cannot create media file {}",
            path.display()
        )))?;
    file.write_all(bytes).map_err(io_error(format!(
        "cannot write media file {}",
        path.display()
    )))
}

fn unique_name(
    media_dir: &Path,
    preferred: &str,
    bytes: &[u8],
) -> Result<String, FormatError> {
    let mut candidate = preferred.to_owned();
    let mut suffix = 1_u32;
    loop {
        let path = media_dir.join(&candidate);
        match fs::read(&path) {
            Ok(existing) if existing == bytes => return Ok(candidate),
            Ok(_) => candidate = numbered_name(preferred, suffix),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(candidate);
            }
            Err(error) => {
                return Err(FormatError::with_source(
                    format!("cannot inspect media file {}", path.display()),
                    error,
                ));
            }
        }
        suffix = suffix.checked_add(1).ok_or_else(|| {
            FormatError::new(format!(
                "cannot find a free media filename for '{preferred}'"
            ))
        })?;
    }
}

fn numbered_name(preferred: &str, suffix: u32) -> String {
    match preferred.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => {
            format!("{stem}_{suffix}.{extension}")
        }
        _ => format!("{preferred}_{suffix}"),
    }
}

pub fn rewrite_references<'a>(
    text: &'a str,
    renames: &[MediaRename],
) -> Cow<'a, str> {
    let mut result = Cow::Borrowed(text);
    for rename in renames {
        if rename.from == rename.to {
            continue;
        }
        result = Cow::Owned(result.replace(&rename.from, &rename.to));
        let encoded_from = percent_encode(&rename.from);
        let encoded_to = percent_encode(&rename.to);
        if encoded_from != rename.from {
            result = Cow::Owned(result.replace(&encoded_from, &encoded_to));
        }
    }
    result
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let byte = *byte;
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
        {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}
