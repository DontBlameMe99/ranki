#![expect(
    clippy::excessive_nesting,
    clippy::redundant_pub_crate,
    reason = "protobuf decoding walks nested Anki messages"
)]
//! Anki's value encodings: ids, guids, field separators and checksums.

use crate::entities;
use ranki_model::FIELD_SEPARATOR;
use ranki_model::Id;
use ranki_model::StoreError;
use sha1::Digest;
use sha1::Sha1;
use std::ops::Range;
use std::process;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use unicode_normalization::UnicodeNormalization;

static GUID_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const BASE91: &[u8; 91] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!#$%&()*+,-./:;<=>?@[]^_`{|}~";

pub fn now_secs() -> Result<i64, StoreError> {
    let millis = now_ms()?;
    millis
        .checked_div(1000)
        .ok_or_else(|| StoreError::Backend("invalid clock".into()))
}

pub fn now_ms() -> Result<i64, StoreError> {
    let duration =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                StoreError::Backend(format!("system clock: {error}"))
            })?;
    i64::try_from(duration.as_millis())
        .map_err(|_| StoreError::Backend("clock overflow".into()))
}

/// Minutes west of UTC in the local timezone, including daylight saving time.
///
/// # Errors
/// Returns an error when the timestamp cannot be represented.
pub fn local_minutes_west(now_secs: i64) -> Result<i32, StoreError> {
    let stamp =
        chrono::DateTime::from_timestamp(now_secs, 0).ok_or_else(|| {
            StoreError::InvalidData(format!("invalid timestamp: {now_secs}"))
        })?;
    Ok(stamp
        .with_timezone(&chrono::Local)
        .offset()
        .utc_minus_local()
        .div_euclid(60))
}

fn conf_value(
    conf: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str::<serde_json::Value>(conf)
        .ok()
        .and_then(|value| value.as_object().cloned())
}

/// True when the collection metadata still declares the original scheduler.
pub fn scheduler_v1_conf(conf: &str) -> bool {
    conf_value(conf).is_some_and(|object| {
        object
            .get("schedVer")
            .and_then(serde_json::Value::as_i64)
            .is_none_or(|version| version < 2)
    })
}

pub fn next_id() -> Result<Id, StoreError> {
    Ok(ranki_model::next_id(now_ms()?))
}
pub fn guid(id: Id) -> Result<String, StoreError> {
    let duration =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                StoreError::Backend(format!("system clock: {error}"))
            })?;
    let nanos = duration
        .as_secs()
        .wrapping_mul(1_000_000_000)
        .wrapping_add(u64::from(duration.subsec_nanos()));
    let seed = nanos
        ^ u64::from(process::id()).rotate_left(32)
        ^ u64::try_from(id).unwrap_or_default().rotate_left(17)
        ^ GUID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(anki_base91(seed))
}

pub fn encode_fields(fields: &[String]) -> String {
    fields.join(&FIELD_SEPARATOR.to_string())
}

pub fn decode_fields(flds: &str) -> Vec<String> {
    flds.split(FIELD_SEPARATOR).map(ToOwned::to_owned).collect()
}

pub fn encode_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        String::new()
    } else {
        format!(" {} ", tags.join(" "))
    }
}

pub fn decode_tags(tags: &str) -> Vec<String> {
    tags.split([' ', '\u{3000}'])
        .filter(|tag| !tag.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Anki strips HTML and resolves entities before computing sort fields and checksums.
#[must_use]
pub fn sort_field(fields: &[String], index: usize) -> String {
    sort_field_with(fields, index, true)
}

/// Computes a sort field with explicit note-text normalization.
pub fn sort_field_with(
    fields: &[String],
    index: usize,
    normalize_text: bool,
) -> String {
    let field = fields.get(index).map_or("", String::as_str);
    strip_html_preserving_media_filenames_with(field, normalize_text)
}

/// Computes the first-field checksum used by Anki.
#[must_use]
pub fn checksum(field: &str) -> i32 {
    checksum_with(field, true)
}

/// Computes a checksum with explicit note-text normalization.
pub fn checksum_with(field: &str, normalize_text: bool) -> i32 {
    let text =
        strip_html_preserving_media_filenames_with(field, normalize_text);
    checksum_text(&text)
}

/// Text used by the checksum algorithm stored before this change.
#[must_use]
pub fn legacy_field_text(field: &str) -> String {
    decode_entities(&remove_tags(field))
}

/// Checksum of a field saved by the previous storage version.
#[must_use]
pub fn legacy_checksum(field: &str) -> i32 {
    checksum_text(&legacy_field_text(field))
}

fn checksum_text(text: &str) -> i32 {
    let digest = Sha1::digest(text.as_bytes());
    let bytes = digest.get(..4).unwrap_or_default();
    let unsigned = u32::from_be_bytes([
        bytes.first().copied().unwrap_or(0),
        bytes.get(1).copied().unwrap_or(0),
        bytes.get(2).copied().unwrap_or(0),
        bytes.get(3).copied().unwrap_or(0),
    ]);
    signed_i32(unsigned)
}

/// Removes the control characters Anki rejects and optionally normalizes NFC.
pub fn normalize_field(field: &mut String, normalize_text: bool) {
    if field.chars().any(invalid_char_for_field) {
        field.retain(|character| !invalid_char_for_field(character));
    }
    if normalize_text && !unicode_normalization::is_nfc(field) {
        *field = field.nfc().collect();
    }
}

const fn invalid_char_for_field(character: char) -> bool {
    character.is_ascii_control() && character != '\n' && character != '\t'
}

/// Converts one field to the text Anki uses for display, sorting, and checking.
///
/// Media references remain as filenames so `<img src="x.png">` and
/// `[sound:x.mp3]` both become `x.png` and `x.mp3` respectively.
#[must_use]
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "Anki's text helper exposes both independent flags"
)]
pub fn html_to_text_line(
    html: &str,
    preserve_media_filenames: bool,
    normalize_text: bool,
) -> String {
    let mut field = html.to_owned();
    normalize_field(&mut field, normalize_text);
    let spaced = replace_persistent_spacers(&field);
    let plain = remove_type_tags(&spaced);
    let with_sound = replace_sound_tags(&plain, preserve_media_filenames);
    let stripped = if preserve_media_filenames {
        strip_html_preserving_media_filenames(&with_sound)
    } else {
        strip_html(&with_sound)
    };
    stripped.trim().to_owned()
}

#[must_use]
pub fn csum_value(checksum: i32) -> i64 {
    i64::from(checksum) & i64::from(u32::MAX)
}

pub fn signed_i32(unsigned: u32) -> i32 {
    if unsigned > 2_147_483_647 {
        i32::try_from(
            i64::from(unsigned)
                .saturating_sub(i64::from(u32::MAX))
                .saturating_sub(1),
        )
        .unwrap_or(i32::MIN)
    } else {
        i32::try_from(unsigned).unwrap_or(i32::MAX)
    }
}

pub fn as_u32(value: i64) -> Result<u32, StoreError> {
    u32::try_from(value).map_err(|_| {
        StoreError::InvalidData(format!("value out of range: {value}"))
    })
}

pub fn as_u16(value: i64) -> Result<u16, StoreError> {
    u16::try_from(value).map_err(|_| {
        StoreError::InvalidData(format!("value out of range: {value}"))
    })
}

#[must_use]
pub fn decode_deck_name(name: &str) -> String {
    name.replace('\u{001f}', "::")
}

#[must_use]
pub fn encode_deck_name(name: &str) -> String {
    name.replace("::", "\u{001f}")
}

#[must_use]
pub fn normal_deck_kind(config_id: i64) -> Vec<u8> {
    let mut normal = vec![0x08];
    append_varint(&mut normal, u64::try_from(config_id).unwrap_or(1));
    let mut bytes = vec![0x0a];
    append_varint(&mut bytes, u64::try_from(normal.len()).unwrap_or(2));
    bytes.extend_from_slice(&normal);
    bytes
}

fn varint_field(bytes: &mut Vec<u8>, field: u32, value: u64) {
    let tag = u64::from(field).checked_shl(3).unwrap_or(0);
    append_varint(bytes, tag);
    append_varint(bytes, value);
}

fn string_field(bytes: &mut Vec<u8>, field: u32, value: &str) {
    let tag = u64::from(field).checked_shl(3).unwrap_or(0) | 2;
    append_varint(bytes, tag);
    append_varint(bytes, u64::try_from(value.len()).unwrap_or(0));
    bytes.extend_from_slice(value.as_bytes());
}

pub(crate) fn append_varint(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        let byte = u8::try_from(value & 0x7f).unwrap_or(0) | 0x80;
        bytes.push(byte);
        value >>= 7;
    }
    bytes.push(u8::try_from(value & 0x7f).unwrap_or(0));
}
#[must_use]
pub fn strip_html(input: &str) -> String {
    decode_entities(&remove_tags(input)).replace('\u{a0}', " ")
}

#[must_use]
pub fn strip_html_preserving_media_filenames(html: &str) -> String {
    strip_html_preserving_media_filenames_with(html, false)
}

#[must_use]
pub fn strip_html_preserving_media_filenames_with(
    html: &str,
    normalize_text: bool,
) -> String {
    let mut field = html.to_owned();
    normalize_field(&mut field, normalize_text);
    let with_media = replace_media_tags(&field);
    let stripped = remove_tags(&with_media);
    decode_entities(&stripped).replace('\u{a0}', " ")
}

fn replace_persistent_spacers(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(character) = rest.chars().next() {
        if character == '\n' {
            output.push(' ');
            rest = rest.get(1..).unwrap_or("");
            continue;
        }
        if character == '<' {
            if let Some(length) = br_spacer_len(rest) {
                output.push(' ');
                rest = rest.get(length..).unwrap_or("");
                continue;
            }
            if starts_with_ci(rest, "<div>") {
                output.push(' ');
                rest = rest.get(5..).unwrap_or("");
                continue;
            }
        }
        output.push(character);
        rest = rest.get(character.len_utf8()..).unwrap_or("");
    }
    output
}

fn br_spacer_len(rest: &str) -> Option<usize> {
    if !starts_with_ci(rest, "<br") {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut index = 3usize;
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index = index.saturating_add(1);
    }
    if bytes.get(index) == Some(&b'/') {
        index = index.saturating_add(1);
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index = index.saturating_add(1);
        }
    }
    if bytes.get(index) == Some(&b'>') {
        Some(index.saturating_add(1))
    } else {
        None
    }
}

fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn remove_type_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("[[type:") {
        let (prefix, tail) = rest.split_at(start);
        output.push_str(prefix);
        if let Some(end) = tail.find("]]") {
            let (_, after) = tail.split_at(end.saturating_add(2));
            rest = after;
        } else {
            output.push_str(tail);
            return output;
        }
    }
    output.push_str(rest);
    output
}

fn replace_sound_tags(input: &str, preserve_filename: bool) -> String {
    let needle = "[sound:";
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(needle) {
        let (prefix, tail) = rest.split_at(start);
        output.push_str(prefix);
        let after_open = tail.get(needle.len()..).unwrap_or("");
        if let Some(end) = after_open.find(']') {
            let (filename, after) = after_open.split_at(end);
            if preserve_filename {
                output.push_str(filename);
            }
            rest = after.get(1..).unwrap_or("");
        } else {
            output.push_str(tail);
            return output;
        }
    }
    output.push_str(rest);
    output
}

fn replace_media_tags(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let (prefix, tail) = rest.split_at(start);
        output.push_str(prefix);
        let Some(end) = tag_end(tail) else {
            output.push_str(tail);
            return output;
        };
        let (tag, after) = tail.split_at(end);
        if let Some(filename) = media_filename(tag) {
            output.push(' ');
            output.push_str(filename);
            output.push(' ');
        } else {
            output.push_str(tag);
        }
        rest = after;
    }
    output.push_str(rest);
    output
}

fn tag_end(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'<') {
        return None;
    }
    let mut quote = None;
    for index in 1..bytes.len() {
        let byte = *bytes.get(index)?;
        if let Some(marker) = quote {
            if byte == marker {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'>' => return Some(index.saturating_add(1)),
            _ => {}
        }
    }
    None
}

fn media_filename(tag: &str) -> Option<&str> {
    let inner = tag.strip_prefix('<')?.strip_suffix('>')?;
    let bytes = inner.as_bytes();
    let mut index = 0usize;
    skip_whitespace(bytes, &mut index);
    if bytes.get(index) == Some(&b'/') {
        return None;
    }
    let name_start = index;
    while bytes.get(index).is_some_and(|byte| {
        !byte.is_ascii_whitespace() && *byte != b'>' && *byte != b'/'
    }) {
        index = index.saturating_add(1);
    }
    let name = inner.get(name_start..index)?;
    if !["img", "audio", "video", "object", "source"]
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
    {
        return None;
    }
    loop {
        skip_whitespace(bytes, &mut index);
        match bytes.get(index) {
            None | Some(b'>' | b'/') => return None,
            Some(_) => {}
        }
        let attribute_start = index;
        while bytes.get(index).is_some_and(|byte| {
            !byte.is_ascii_whitespace() && *byte != b'=' && *byte != b'>'
        }) {
            index = index.saturating_add(1);
        }
        let attribute = inner.get(attribute_start..index)?;
        skip_whitespace(bytes, &mut index);
        if bytes.get(index) != Some(&b'=') {
            continue;
        }
        index = index.saturating_add(1);
        skip_whitespace(bytes, &mut index);
        let value = if let Some(b'"' | b'\'') = bytes.get(index) {
            let quote = *bytes.get(index)?;
            let value_start = index.saturating_add(1);
            index = value_start;
            while bytes.get(index).is_some_and(|byte| *byte != quote) {
                index = index.saturating_add(1);
            }
            let value = inner.get(value_start..index)?;
            if bytes.get(index) == Some(&quote) {
                index = index.saturating_add(1);
            }
            value
        } else {
            let value_start = index;
            while bytes.get(index).is_some_and(|byte| {
                !byte.is_ascii_whitespace() && *byte != b'>'
            }) {
                index = index.saturating_add(1);
            }
            inner.get(value_start..index)?
        };
        if attribute.eq_ignore_ascii_case("src")
            || attribute.eq_ignore_ascii_case("data")
        {
            return Some(value);
        }
    }
}

fn skip_whitespace(bytes: &[u8], index: &mut usize) {
    while bytes.get(*index).is_some_and(u8::is_ascii_whitespace) {
        *index = index.saturating_add(1);
    }
}

/// Reads a length-delimited protobuf field from Anki 25's `config` blobs.
pub fn protobuf_string(bytes: &[u8], wanted: u32) -> Option<String> {
    let wanted_field = u64::from(wanted);
    let mut pos = 0usize;
    while pos < bytes.len() {
        let tag = read_varint(bytes, &mut pos)?;
        let field = tag >> 3;
        match (tag & 7, field == wanted_field) {
            (2, true) => {
                let length =
                    usize::try_from(read_varint(bytes, &mut pos)?).ok()?;
                let end = pos.checked_add(length)?;
                let slice = bytes.get(pos..end)?;
                return String::from_utf8(slice.to_vec()).ok();
            }
            (2, false) => {
                let length =
                    usize::try_from(read_varint(bytes, &mut pos)?).ok()?;
                pos = pos.checked_add(length)?;
            }
            (0, _) => {
                read_varint(bytes, &mut pos)?;
            }
            (1, _) => pos = pos.checked_add(8)?,
            (5, _) => pos = pos.checked_add(4)?,
            _ => return None,
        }
    }
    None
}
/// Reads a varint field from an Anki protobuf config blob.
///
/// Length-delimited and fixed-width fields are skipped so callers can read
/// fields that occur after them.
pub fn protobuf_varint(bytes: &[u8], wanted: u32) -> Option<u64> {
    let wanted_field = u64::from(wanted);
    let mut pos = 0usize;
    while pos < bytes.len() {
        let tag = read_varint(bytes, &mut pos)?;
        let field = tag >> 3;
        match (tag & 7, field == wanted_field) {
            (0, true) => return read_varint(bytes, &mut pos),
            (0, false) => {
                let _ = read_varint(bytes, &mut pos)?;
            }
            (2, _) => {
                let length =
                    usize::try_from(read_varint(bytes, &mut pos)?).ok()?;
                pos = pos.checked_add(length)?;
            }
            (1, _) => pos = pos.checked_add(8)?,
            (5, _) => pos = pos.checked_add(4)?,
            _ => return None,
        }
    }
    None
}

/// Reads Anki's repeated float fields, such as learning steps in a deck preset.
#[expect(
    clippy::excessive_nesting,
    reason = "protobuf wire types are handled together"
)]
#[must_use]
pub fn protobuf_floats(bytes: &[u8], wanted: u32) -> Vec<f32> {
    let wanted_field = u64::from(wanted);
    let mut result = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let Some(tag) = read_varint(bytes, &mut pos) else {
            break;
        };
        let matched = tag >> 3 == wanted_field;
        match tag & 7 {
            0 => {
                if read_varint(bytes, &mut pos).is_none() {
                    break;
                }
            }
            1 => {
                let Some(next) = pos.checked_add(8) else {
                    break;
                };
                pos = next;
            }
            2 => {
                let Some(length) = read_varint(bytes, &mut pos)
                    .and_then(|length| usize::try_from(length).ok())
                else {
                    break;
                };
                let Some(end) = pos.checked_add(length) else {
                    break;
                };
                let Some(slice) = bytes.get(pos..end) else {
                    break;
                };
                if matched {
                    push_floats(&mut result, slice);
                }
                pos = end;
            }
            5 => {
                let Some(end) = pos.checked_add(4) else {
                    break;
                };
                let Some(slice) = bytes.get(pos..end) else {
                    break;
                };
                if matched {
                    push_floats(&mut result, slice);
                }
                pos = end;
            }
            _ => break,
        }
    }
    result
}

fn push_floats(result: &mut Vec<f32>, bytes: &[u8]) {
    if !bytes.len().is_multiple_of(4) {
        return;
    }
    let (chunks, _) = bytes.as_chunks::<4>();
    for &[a, b, c, d] in chunks {
        result.push(f32::from_le_bytes([a, b, c, d]));
    }
}

/// Reads the preset id from a schema-18 deck's `kind` blob.
#[must_use]
pub fn deck_kind_config_id(kind: &[u8]) -> Option<i64> {
    let normal = protobuf_bytes(kind, 1)?;
    i64::try_from(protobuf_varint(&normal, 1)?).ok()
}

/// Replaces the preset id in a schema-18 deck's `kind` blob.
#[must_use]
pub fn with_deck_kind_config_id(
    kind: &[u8],
    config_id: i64,
) -> Option<Vec<u8>> {
    let range = protobuf_field_range(kind, 1, 2)?;
    let mut normal = kind.get(range.clone())?.to_vec();
    let value = protobuf_field_range(&normal, 1, 0)?;
    let mut id = Vec::new();
    append_varint(&mut id, u64::try_from(config_id).ok()?);
    normal.splice(value, id);

    let mut length_start = range.start.checked_sub(1)?;
    while kind.get(length_start)? & 0x80 != 0 {
        length_start = length_start.checked_sub(1)?;
    }
    let mut rewritten = kind.get(..length_start)?.to_vec();
    append_varint(&mut rewritten, u64::try_from(normal.len()).ok()?);
    rewritten.extend(normal);
    rewritten.extend_from_slice(kind.get(range.end..)?);
    Some(rewritten)
}

#[expect(
    clippy::excessive_nesting,
    reason = "protobuf wire types are handled together"
)]
fn protobuf_field_range(
    bytes: &[u8],
    wanted: u32,
    wire: u64,
) -> Option<Range<usize>> {
    let wanted_field = u64::from(wanted);
    let mut pos = 0usize;
    while pos < bytes.len() {
        let tag = read_varint(bytes, &mut pos)?;
        let matched = tag >> 3 == wanted_field && tag & 7 == wire;
        let mut start = pos;
        match tag & 7 {
            0 => {
                let _ = read_varint(bytes, &mut pos)?;
                if matched {
                    return Some(start..pos);
                }
            }
            1 => {
                pos = pos.checked_add(8)?;
                if matched {
                    return Some(start..pos);
                }
            }
            2 => {
                let length =
                    usize::try_from(read_varint(bytes, &mut pos)?).ok()?;
                start = pos;
                pos = pos.checked_add(length)?;
                if matched {
                    return Some(start..pos);
                }
            }
            5 => {
                pos = pos.checked_add(4)?;
                if matched {
                    return Some(start..pos);
                }
            }
            _ => return None,
        }
        if pos > bytes.len() {
            return None;
        }
    }
    None
}

/// Reads a nested message or `bytes` field from an Anki protobuf blob.
#[must_use]
pub fn protobuf_bytes(bytes: &[u8], wanted: u32) -> Option<Vec<u8>> {
    let wanted_field = u64::from(wanted);
    let mut pos = 0usize;
    while pos < bytes.len() {
        let tag = read_varint(bytes, &mut pos)?;
        let field = tag >> 3;
        match (tag & 7, field == wanted_field) {
            (2, true) => {
                let length =
                    usize::try_from(read_varint(bytes, &mut pos)?).ok()?;
                let end = pos.checked_add(length)?;
                let slice = bytes.get(pos..end)?;
                return Some(slice.to_vec());
            }
            (2, false) => {
                let length =
                    usize::try_from(read_varint(bytes, &mut pos)?).ok()?;
                pos = pos.checked_add(length)?;
            }
            (0, _) => {
                let _ = read_varint(bytes, &mut pos)?;
            }
            (1, _) => pos = pos.checked_add(8)?,
            (5, _) => pos = pos.checked_add(4)?,
            _ => return None,
        }
    }
    None
}

/// Reads every nested message with the given field number.
#[must_use]
pub fn protobuf_messages(bytes: &[u8], wanted: u32) -> Vec<Vec<u8>> {
    let wanted_field = u64::from(wanted);
    let mut messages = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let Some(tag) = read_varint(bytes, &mut pos) else {
            break;
        };
        let length = match (tag & 7, tag >> 3 == wanted_field) {
            (2, true) => {
                let Some(length) =
                    usize::try_from(read_varint(bytes, &mut pos).unwrap_or(0))
                        .ok()
                else {
                    break;
                };
                length
            }
            (2, false) => {
                let Some(length) =
                    usize::try_from(read_varint(bytes, &mut pos).unwrap_or(0))
                        .ok()
                else {
                    break;
                };
                pos = pos.saturating_add(length);
                continue;
            }
            (0, _) => {
                if read_varint(bytes, &mut pos).is_none() {
                    break;
                }
                continue;
            }
            (1, _) => {
                pos = pos.saturating_add(8);
                continue;
            }
            (5, _) => {
                pos = pos.saturating_add(4);
                continue;
            }
            _ => break,
        };
        let Some(message) = bytes.get(pos..pos.saturating_add(length)) else {
            break;
        };
        messages.push(message.to_vec());
        pos = pos.saturating_add(length);
    }
    messages
}
/// Reads every value of a repeated varint field, packed or unpacked.
#[must_use]
pub fn protobuf_varints(bytes: &[u8], wanted: u32) -> Vec<u64> {
    let wanted_field = u64::from(wanted);
    let mut values = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let Some(tag) = read_varint(bytes, &mut pos) else {
            break;
        };
        match (tag & 7, tag >> 3 == wanted_field) {
            (0, true) => {
                let Some(value) = read_varint(bytes, &mut pos) else {
                    break;
                };
                values.push(value);
            }
            (2, true) => {
                let Some(length) =
                    usize::try_from(read_varint(bytes, &mut pos).unwrap_or(0))
                        .ok()
                else {
                    break;
                };
                let Some(raw) = bytes.get(pos..pos.saturating_add(length))
                else {
                    break;
                };
                let mut offset = 0usize;
                while offset < raw.len() {
                    let Some(value) = read_varint(raw, &mut offset) else {
                        break;
                    };
                    values.push(value);
                }
                pos = pos.saturating_add(length);
            }
            (2, false) => {
                let Some(length) =
                    usize::try_from(read_varint(bytes, &mut pos).unwrap_or(0))
                        .ok()
                else {
                    break;
                };
                pos = pos.saturating_add(length);
            }
            (0, false) => {
                if read_varint(bytes, &mut pos).is_none() {
                    break;
                }
            }
            (1, _) => pos = pos.saturating_add(8),
            (5, _) => pos = pos.saturating_add(4),
            _ => break,
        }
        if pos > bytes.len() {
            break;
        }
    }
    values
}

/// Reads a 32-bit float field from an Anki protobuf blob.
#[must_use]
pub fn protobuf_f32(bytes: &[u8], wanted: u32) -> Option<f32> {
    let wanted_field = u64::from(wanted);
    let mut pos = 0usize;
    while pos < bytes.len() {
        let tag = read_varint(bytes, &mut pos)?;
        let field = tag >> 3;
        match (tag & 7, field == wanted_field) {
            (5, true) => {
                let raw = bytes.get(pos..pos.saturating_add(4))?;
                let array: [u8; 4] = raw.try_into().ok()?;
                return Some(f32::from_le_bytes(array));
            }
            (5, false) => pos = pos.saturating_add(4),
            (0, _) => {
                let _ = read_varint(bytes, &mut pos)?;
            }
            (2, _) => {
                let length =
                    usize::try_from(read_varint(bytes, &mut pos)?).ok()?;
                pos = pos.saturating_add(length);
            }
            (1, _) => pos = pos.saturating_add(8),
            _ => return None,
        }
    }
    None
}

/// Reads every value of a repeated float field, packed or unpacked.
#[must_use]
pub fn protobuf_f32s(bytes: &[u8], wanted: u32) -> Vec<f32> {
    let wanted_field = u64::from(wanted);
    let mut values = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let Some(tag) = read_varint(bytes, &mut pos) else {
            break;
        };
        let field = tag >> 3;
        match (tag & 7, field == wanted_field) {
            (5, true) => {
                let Some(raw) = bytes.get(pos..pos.saturating_add(4)) else {
                    break;
                };
                if let Ok(array) = raw.try_into() {
                    values.push(f32::from_le_bytes(array));
                }
                pos = pos.saturating_add(4);
            }
            (2, true) => {
                let Some(length) =
                    usize::try_from(read_varint(bytes, &mut pos).unwrap_or(0))
                        .ok()
                else {
                    break;
                };
                let Some(raw) = bytes.get(pos..pos.saturating_add(length))
                else {
                    break;
                };
                values.extend(
                    raw.as_chunks::<4>()
                        .0
                        .iter()
                        .map(|chunk| f32::from_le_bytes(*chunk)),
                );
                pos = pos.saturating_add(length);
            }
            (5, false) => pos = pos.saturating_add(4),
            (0, _) => {
                if read_varint(bytes, &mut pos).is_none() {
                    break;
                }
            }
            (2, false) => {
                let Some(length) =
                    usize::try_from(read_varint(bytes, &mut pos).unwrap_or(0))
                        .ok()
                else {
                    break;
                };
                pos = pos.saturating_add(length);
            }
            (1, _) => pos = pos.saturating_add(8),
            _ => break,
        }
        if pos > bytes.len() {
            break;
        }
    }
    values
}

/// Writes a protobuf varint field.
pub(crate) fn put_varint(bytes: &mut Vec<u8>, field: u32, value: u64) {
    varint_field(bytes, field, value);
}

/// Writes a protobuf length-delimited field.
pub(crate) fn put_bytes(bytes: &mut Vec<u8>, field: u32, value: &[u8]) {
    string_field(bytes, field, "");
    bytes.truncate(bytes.len().saturating_sub(1));
    append_varint(bytes, u64::try_from(value.len()).unwrap_or(0));
    bytes.extend_from_slice(value);
}

/// Writes a protobuf fixed32 field, used for floats.
pub(crate) fn put_f32(bytes: &mut Vec<u8>, field: u32, value: f32) {
    let tag = u64::from(field)
        .checked_shl(3)
        .unwrap_or(0)
        .checked_add(5)
        .unwrap_or(5);
    append_varint(bytes, tag);
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Writes a repeated float field as a packed protobuf field.
pub(crate) fn put_f32s(bytes: &mut Vec<u8>, field: u32, values: &[f32]) {
    if values.is_empty() {
        return;
    }
    let mut payload = Vec::with_capacity(values.len().saturating_mul(4));
    for value in values {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    put_bytes(bytes, field, &payload);
}

fn read_varint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(*pos)?;
        *pos = pos.checked_add(1)?;
        result |= u64::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift = shift.checked_add(7)?;
    }
}

fn anki_base91(mut value: u64) -> String {
    let mut reversed = String::new();
    while value > 0 {
        let digit = usize::try_from(value % 91).unwrap_or(0);
        reversed.push(char::from(BASE91.get(digit).copied().unwrap_or(b'?')));
        value /= 91;
    }
    reversed.chars().rev().collect()
}

fn remove_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('<') {
        let (prefix, tail) = rest.split_at(start);
        output.push_str(prefix);
        let lowered = tail.to_ascii_lowercase();
        let mut skipped = None;
        for (open, close) in [
            ("<!--", "-->"),
            ("<style", "</style>"),
            ("<script", "</script>"),
        ] {
            if lowered.starts_with(open) {
                if let Some(end) = lowered.find(close) {
                    skipped = Some(end.saturating_add(close.len()));
                }
                break;
            }
        }
        if let Some(length) = skipped {
            rest = tail.get(length..).unwrap_or("");
            continue;
        }
        if let Some(end) = tail.find('>') {
            let (_, after) = tail.split_at(end.saturating_add(1));
            rest = after;
        } else {
            output.push_str(tail);
            return output;
        }
    }
    output.push_str(rest);
    output
}

fn decode_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        let (prefix, tail) = rest.split_at(start);
        output.push_str(prefix);
        let after = tail.get(1..).unwrap_or("");
        let Some(end) = after.find(';') else {
            output.push_str(tail);
            return output;
        };
        let entity = after.get(..end).unwrap_or("");
        if entity.is_empty()
            || entity.len() > 32
            || entity.chars().any(char::is_whitespace)
        {
            output.push('&');
            rest = after;
            continue;
        }
        if let Some(decoded) = decode_entity(entity) {
            if decoded == '\u{a0}' {
                output.push(' ');
            } else {
                output.push(decoded);
            }
            rest = after.get(end.saturating_add(1)..).unwrap_or("");
        } else {
            output.push('&');
            rest = after;
        }
    }
    output.push_str(rest);
    output
}

fn decode_entity(entity: &str) -> Option<char> {
    if let Some(digits) = entity.strip_prefix('#') {
        let (radix, numeric) = digits
            .strip_prefix('x')
            .or_else(|| digits.strip_prefix('X'))
            .map_or((10, digits), |hex| (16, hex));
        let code = u32::from_str_radix(numeric, radix).ok()?;
        return char::from_u32(code);
    }
    entities::named(entity).and_then(|value| value.chars().next())
}

#[cfg(test)]
mod tests {
    use super::anki_base91;
    use super::checksum;
    use super::html_to_text_line;
    use super::sort_field;

    #[test]
    fn base91_matches_anki_vectors() {
        assert_eq!(anki_base91(0), "");
        assert_eq!(anki_base91(1), "b");
        assert_eq!(anki_base91(1_234_567_890), "saAKk");
        assert_eq!(anki_base91(u64::MAX), "Rj&Z5m[>Zp");
    }

    #[test]
    fn normal_deck_kind_declares_a_normal_deck() {
        assert_eq!(super::normal_deck_kind(1), vec![0x0a, 0x02, 0x08, 0x01]);
    }
    #[test]
    fn html_to_text_matches_anki_vectors() {
        assert_eq!(html_to_text_line("plain", true, true), "plain");
        assert_eq!(
            html_to_text_line(
                "<b>hello</b><br>world<div>again</div>",
                true,
                true
            ),
            "hello world again"
        );
        assert_eq!(
            html_to_text_line(
                "<img src=photo.png> [sound:voice.mp3]",
                true,
                true
            ),
            "photo.png  voice.mp3"
        );
        assert_eq!(
            html_to_text_line(
                "<img src=photo.png> [sound:voice.mp3]",
                false,
                true
            ),
            ""
        );
        assert_eq!(
            html_to_text_line(
                "<audio src='a.mp3'><source data=b.ogg>",
                true,
                true
            ),
            "a.mp3  b.ogg"
        );
        assert_eq!(
            html_to_text_line(
                "<!--hidden--><style>x</style><script>y</script>kept",
                true,
                true
            ),
            "kept"
        );
        assert_eq!(
            html_to_text_line("before <b>after", true, true),
            "before after"
        );
        assert_eq!(
            html_to_text_line("before <b after", true, true),
            "before <b after"
        );
        assert_eq!(html_to_text_line("[[type:cloze]]word", true, true), "word");
    }

    #[test]
    fn entities_and_whitespace_match_anki() {
        assert_eq!(
            html_to_text_line("&amp; &lt; &gt; &quot; &apos;", true, true),
            "& < > \" '"
        );
        assert_eq!(
            html_to_text_line("&Alpha; &#65; &#x1F600;", true, true),
            "Α A 😀"
        );
        assert_eq!(html_to_text_line("a&nbsp;b&#160;c", true, true), "a b c");
        assert_eq!(html_to_text_line("a\u{a0}b", true, true), "a b");
        assert_eq!(html_to_text_line("&unknown;", true, true), "&unknown;");
        assert_eq!(html_to_text_line("A&nbsp;B [sound:x]", false, true), "A B");
    }

    #[test]
    fn normalization_and_checksums_use_the_same_field_text() {
        let decomposed = "e\u{301}";
        assert_eq!(html_to_text_line(decomposed, true, true), "é");
        assert_eq!(html_to_text_line(decomposed, true, false), decomposed);
        assert_eq!(html_to_text_line("a\u{0001}b\nc", true, true), "ab c");
        assert_eq!(checksum("\u{0001}<b>A</b>"), checksum("A"));
        let fields = vec![String::from("<b>A</b>"), String::from("B")];
        assert_eq!(sort_field(&fields, 0), "A");
        assert_eq!(sort_field(&fields, 1), "B");
        assert_eq!(sort_field(&fields, 9), "");
    }
    #[test]
    fn checksums_and_sort_fields_follow_anki_stripping() {
        let fields = vec![
            String::from("a<br>b"),
            String::from("[sound:x]"),
            String::from("<img src=\"sound.png\">y"),
            String::from("a&nbsp;b"),
        ];
        assert_eq!(sort_field(&fields, 0), "ab");
        assert_eq!(sort_field(&fields, 1), "[sound:x]");
        assert_eq!(sort_field(&fields, 2), " sound.png y");
        assert_eq!(sort_field(&fields, 3), "a b");
        assert_eq!(checksum("a<br>b"), checksum("ab"));
        assert_ne!(checksum("[sound:x]"), checksum("x"));
        assert_eq!(
            checksum("<img src=\"sound.png\">y"),
            checksum(" sound.png y")
        );
        assert_ne!(
            checksum("<img src=\"sound.png\">y"),
            checksum("sound.png y")
        );
    }
}
