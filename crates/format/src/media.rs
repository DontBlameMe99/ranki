use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use unicode_normalization::UnicodeNormalization;

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

/// The maximum media filename length Anki enforces, in bytes.
const MAX_FILENAME_BYTES: usize = 120;
const MAX_EXTENSION_BYTES: usize = 10;

/// Adjusts a package media name the way Anki's `normalize_filename` does.
///
/// Disallowed characters are removed, names are NFC, non-breaking spaces
/// become regular spaces, Windows device names get an underscore, a trailing
/// space or dot gets an underscore, and long names are truncated.
///
/// Two deliberate differences: a path component is reduced to its basename so
/// a package cannot write outside the media directory, and unassigned code
/// points are kept because Ranki has no Unicode category table.
#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut cleaned: String = base
        .nfc()
        .filter(|character| !is_disallowed_media_char(*character))
        .map(|character| {
            if character == '\u{a0}' {
                ' '
            } else {
                character
            }
        })
        .collect();
    cleaned = prefix_windows_device_name(&cleaned);
    if cleaned.ends_with(' ') || cleaned.ends_with('.') {
        cleaned.push('_');
    }
    let truncated = truncate_filename(&cleaned);
    if truncated.is_empty() || truncated == "." || truncated == ".." {
        String::new()
    } else {
        truncated
    }
}

const fn is_disallowed_media_char(character: char) -> bool {
    matches!(
        character,
        '[' | ']' | '<' | '>' | ':' | '"' | '/' | '?' | '*' | '^' | '\\' | '|'
    ) || character.is_ascii_control()
}

fn prefix_windows_device_name(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    for device in ["CON", "PRN", "AUX", "NUL"] {
        if upper == device || upper.starts_with(&format!("{device}.")) {
            let original = name.get(..device.len()).unwrap_or(device);
            let rest = name.get(device.len()..).unwrap_or_default();
            return format!("{original}_{rest}");
        }
    }
    for prefix in ["COM", "LPT"] {
        let Some(rest) = upper.strip_prefix(prefix) else {
            continue;
        };
        let mut characters = rest.chars();
        let Some(digit) = characters.next() else {
            continue;
        };
        if matches!(digit, '1'..='9')
            && (characters.next().is_none()
                || rest.starts_with(&format!("{digit}.")))
        {
            let original =
                name.get(..prefix.len().saturating_add(1)).unwrap_or(prefix);
            let tail = name
                .get(prefix.len().saturating_add(1)..)
                .unwrap_or_default();
            return format!("{original}_{tail}");
        }
    }
    name.to_owned()
}

fn truncate_filename(name: &str) -> String {
    if name.len() <= MAX_FILENAME_BYTES {
        return name.to_owned();
    }
    let (stem, extension) = name
        .rsplit_once('.')
        .map_or((name, ""), |(stem, extension)| (stem, extension));
    let shortened_extension =
        truncated_to_char_boundary(extension, MAX_EXTENSION_BYTES);
    let stem_limit = MAX_FILENAME_BYTES
        .saturating_sub(shortened_extension.len())
        .saturating_sub(2);
    let shortened_stem = truncated_to_char_boundary(stem, stem_limit);
    let mut truncated = if shortened_extension.is_empty() {
        shortened_stem.to_owned()
    } else {
        format!("{shortened_stem}.{shortened_extension}")
    };
    if truncated.ends_with(' ') || truncated.ends_with('.') {
        truncated.push('_');
    }
    truncated
}

fn truncated_to_char_boundary(value: &str, mut limit: usize) -> &str {
    if limit >= value.len() {
        return value;
    }
    while !value.is_char_boundary(limit) {
        limit = limit.saturating_sub(1);
    }
    value.get(..limit).unwrap_or_default()
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

    let mut cache = MediaCache::new(media_dir);
    let mut renames = Vec::new();
    for entry in entries {
        let preferred = sanitize_filename(&entry.name);
        if preferred.is_empty() {
            return Err(FormatError::new(format!(
                "invalid media filename in package: '{}'",
                entry.name
            )));
        }

        let hash = hash_bytes(&entry.bytes);
        let final_name = unique_name(&preferred, hash, &mut cache)?;
        let destination = media_dir.join(&final_name);
        if !cache.contains(&final_name) {
            write_new_file(&destination, &entry.bytes)?;
            cache.record(&final_name, hash);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContentHash {
    length: usize,
    value: u64,
}

fn hash_bytes(bytes: &[u8]) -> ContentHash {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    ContentHash {
        length: bytes.len(),
        value: hasher.finish(),
    }
}

struct MediaCache {
    directory: PathBuf,
    files: HashMap<String, Option<ContentHash>>,
}

impl MediaCache {
    fn new(directory: &Path) -> Self {
        Self {
            directory: directory.to_path_buf(),
            files: HashMap::new(),
        }
    }

    fn hash(&mut self, name: &str) -> Result<Option<ContentHash>, FormatError> {
        if let Some(cached) = self.files.get(name) {
            return Ok(*cached);
        }
        let path = self.directory.join(name);
        let hash = match fs::read(&path) {
            Ok(bytes) => Some(hash_bytes(&bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(FormatError::with_source(
                    format!("cannot inspect media file {}", path.display()),
                    error,
                ));
            }
        };
        let _ = self.files.insert(name.to_owned(), hash);
        Ok(hash)
    }

    fn contains(&self, name: &str) -> bool {
        self.files.get(name).is_some_and(Option::is_some)
    }

    fn record(&mut self, name: &str, hash: ContentHash) {
        let _ = self.files.insert(name.to_owned(), Some(hash));
    }
}

fn unique_name(
    preferred: &str,
    hash: ContentHash,
    cache: &mut MediaCache,
) -> Result<String, FormatError> {
    let mut candidate = preferred.to_owned();
    let mut suffix = 0_u32;
    loop {
        let available = cache
            .hash(&candidate)?
            .is_none_or(|existing| existing == hash);
        if available {
            return Ok(candidate);
        }
        suffix = suffix.checked_add(1).ok_or_else(|| {
            FormatError::new(format!(
                "cannot find a free media filename for '{preferred}'"
            ))
        })?;
        candidate = numbered_name(preferred, suffix);
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

#[derive(Debug, Default)]
struct MatchNode {
    children: HashMap<u8, Self>,
    replacement: Option<String>,
}

/// Matches every raw and percent-encoded media rename in one pass.
#[derive(Debug, Default)]
pub struct ReferenceMatcher {
    root: MatchNode,
}

impl ReferenceMatcher {
    pub fn new(renames: &[MediaRename]) -> Self {
        let mut root = MatchNode::default();
        for rename in renames {
            Self::insert_rename(&mut root, rename);
        }
        Self { root }
    }

    fn insert_rename(root: &mut MatchNode, rename: &MediaRename) {
        Self::insert(root, &rename.from, &rename.to);
        let encoded_from = percent_encode(&rename.from);
        if encoded_from != rename.from {
            let encoded_to = percent_encode(&rename.to);
            Self::insert(root, &encoded_from, &encoded_to);
        }
    }

    fn insert(root: &mut MatchNode, from: &str, to: &str) {
        if from.is_empty() {
            return;
        }
        let mut node = root;
        for byte in from.as_bytes() {
            node = node.children.entry(*byte).or_default();
        }
        node.replacement = Some(to.to_owned());
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "trie scanning has a fixed set of branches"
    )]
    fn longest_match<'a>(
        &'a self,
        bytes: &[u8],
        start: usize,
    ) -> Option<(usize, &'a str)> {
        let mut node = &self.root;
        let mut best = None;
        let mut scan = start;
        while let Some(byte) = bytes.get(scan).copied() {
            let Some(next) = node.children.get(&byte) else {
                break;
            };
            node = next;
            scan = scan.checked_add(1)?;
            if let Some(replacement) = node.replacement.as_deref() {
                best = Some((scan, replacement));
            }
        }
        best
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "single-pass rewriting has a fixed set of branches"
    )]
    pub fn rewrite<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let bytes = text.as_bytes();
        let mut rewritten: Option<String> = None;
        let mut copied_until = 0;
        let mut cursor = 0;
        while cursor < bytes.len() {
            if let Some((end, replacement)) = self.longest_match(bytes, cursor)
            {
                let output = rewritten
                    .get_or_insert_with(|| String::with_capacity(text.len()));
                if let Some(unchanged) = text.get(copied_until..cursor) {
                    output.push_str(unchanged);
                }
                output.push_str(replacement);
                copied_until = end;
                cursor = end;
            } else {
                let step = text
                    .get(cursor..)
                    .and_then(|rest| rest.chars().next())
                    .map_or(1, char::len_utf8);
                cursor = match cursor.checked_add(step) {
                    Some(value) => value,
                    None => break,
                };
            }
        }
        rewritten.map_or(Cow::Borrowed(text), |mut output| {
            if let Some(tail) = text.get(copied_until..) {
                output.push_str(tail);
            }
            Cow::Owned(output)
        })
    }
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

#[cfg(test)]
mod tests {
    use super::Cow;
    use super::MediaRename;
    use super::ReferenceMatcher;

    #[test]
    fn longest_source_wins_and_repeats_are_rewritten() {
        let renames = vec![
            MediaRename {
                from: "pic:a.png".to_owned(),
                to: "pic_a_1.png".to_owned(),
            },
            MediaRename {
                from: "pic:a.png.bak".to_owned(),
                to: "backup.png".to_owned(),
            },
        ];
        let matcher = ReferenceMatcher::new(&renames);
        let text =
            "pic:a.png pic:a.png.bak pic%3Aa.png pic%3Aa.png.bak pic:a.png";
        let expected =
            "pic_a_1.png backup.png pic_a_1.png backup.png pic_a_1.png";

        assert_eq!(matcher.rewrite(text).as_ref(), expected);
    }

    #[test]
    fn matcher_leaves_unrelated_text_borrowed() {
        let renames = vec![MediaRename {
            from: "pic:a.png".to_owned(),
            to: "pic_a_1.png".to_owned(),
        }];
        let matcher = ReferenceMatcher::new(&renames);
        let text = "no media here";

        assert!(matches!(matcher.rewrite(text), Cow::Borrowed(_)));
    }

    #[test]
    fn sanitized_names_follow_anki_rules() {
        assert_eq!(super::sanitize_filename("CON.txt"), "CON_.txt");
        assert_eq!(super::sanitize_filename("COM3.mp3"), "COM3_.mp3");
        assert_eq!(super::sanitize_filename("COM10.mp3"), "COM10.mp3");
        assert_eq!(super::sanitize_filename("a[b]<c>.png"), "abc.png");
        assert_eq!(super::sanitize_filename("pic\u{a0}one.png"), "pic one.png");
        assert_eq!(super::sanitize_filename("starts. "), "starts. _");
        assert_eq!(super::sanitize_filename("../escape.png"), "escape.png");
        assert_eq!(
            super::sanitize_filename("na\u{ef}ve r\u{e9}sum\u{e9}.png"),
            "na\u{ef}ve r\u{e9}sum\u{e9}.png"
        );
    }

    #[test]
    fn long_media_names_are_truncated_to_anki_limit() {
        let long = format!("{}.jpeg", "a".repeat(200));
        let sanitized = super::sanitize_filename(&long);
        assert!(sanitized.len() <= 120, "{}", sanitized.len());
        assert!(sanitized.rsplit_once('.').is_some_and(|(_, extension)| {
            extension.eq_ignore_ascii_case("jpeg")
        }));
    }

    #[test]
    fn nfd_names_become_nfc() {
        let decomposed = "nai\u{308}ve.png";
        assert_eq!(super::sanitize_filename(decomposed), "na\u{ef}ve.png");
    }
}
