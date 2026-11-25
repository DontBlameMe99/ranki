//! Anki's value encodings: ids, guids, field separators and checksums.

use ranki_model::FIELD_SEPARATOR;
use ranki_model::Id;
use ranki_model::StoreError;
use sha1::Digest;
use sha1::Sha1;
use std::process;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const DAY_SECS: i64 = 86_400;
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

pub fn day_number(
    now_secs: i64,
    collection_created: i64,
) -> Result<i64, StoreError> {
    now_secs
        .checked_sub(collection_created)
        .map(|elapsed| elapsed.div_euclid(DAY_SECS))
        .ok_or_else(|| StoreError::InvalidData("invalid timestamp".into()))
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
    tags.split_whitespace().map(ToOwned::to_owned).collect()
}

/// Anki strips HTML and resolves entities before computing sort fields and checksums.
pub fn sort_field(fields: &[String], index: usize) -> String {
    fields
        .get(index)
        .map_or_else(String::new, |value| strip_html(value))
}

pub fn checksum(field: &str) -> i32 {
    let digest = Sha1::digest(strip_html(field).as_bytes());
    let bytes = digest.get(..4).unwrap_or_default();
    let unsigned = u32::from_be_bytes([
        bytes.first().copied().unwrap_or(0),
        bytes.get(1).copied().unwrap_or(0),
        bytes.get(2).copied().unwrap_or(0),
        bytes.get(3).copied().unwrap_or(0),
    ]);
    signed_i32(unsigned)
}

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

/// Builds a schema-18 `notetypes.config` blob: kind, CSS, stock kind.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "the schema-18 config fields are written together"
)]
pub fn notetype_config(
    kind: i32,
    sort_field_index: u16,
    stock_kind: Option<i32>,
    css: &str,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    varint_field(&mut bytes, 1, u64::try_from(kind).unwrap_or(0));
    varint_field(&mut bytes, 2, u64::from(sort_field_index));
    string_field(&mut bytes, 3, css);
    if let Some(stock) = stock_kind {
        varint_field(&mut bytes, 9, u64::try_from(stock).unwrap_or(0));
    }
    bytes
}

/// Builds a schema-18 `fields.config` blob holding Anki's field tag.
#[must_use]
pub fn field_config(tag: Option<u32>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(value) = tag {
        varint_field(&mut bytes, 10, u64::from(value));
    }
    bytes
}

/// Builds a schema-18 `templates.config` blob: question and answer formats.
#[must_use]
pub fn template_config(question: &str, answer: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    string_field(&mut bytes, 1, question);
    string_field(&mut bytes, 2, answer);
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

fn append_varint(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        let byte = u8::try_from(value & 0x7f).unwrap_or(0) | 0x80;
        bytes.push(byte);
        value >>= 7;
    }
    bytes.push(u8::try_from(value & 0x7f).unwrap_or(0));
}
pub fn strip_html(input: &str) -> String {
    let without_tags = remove_tags(input);
    decode_entities(&without_tags)
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
        output.push_str(rest.get(..start).unwrap_or(""));
        let tail = rest.get(start..).unwrap_or("");
        if let Some(end) = tail.find('>') {
            rest = tail.get(end.saturating_add(1)..).unwrap_or("");
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
        output.push_str(rest.get(..start).unwrap_or(""));
        let tail = rest.get(start..).unwrap_or("");
        let Some(end) = tail.find(';') else {
            output.push_str(tail);
            return output;
        };
        let entity = tail.get(1..end).unwrap_or("");
        output.push(decode_entity(entity).unwrap_or('&'));
        rest = tail.get(end.saturating_add(1)..).unwrap_or("");
    }
    output.push_str(rest);
    output
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" | "#160" => Some('\u{00a0}'),
        _ => entity
            .strip_prefix('#')
            .and_then(|value| value.parse::<u32>().ok())
            .and_then(char::from_u32),
    }
}

#[cfg(test)]
mod tests {
    use super::anki_base91;

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
}
