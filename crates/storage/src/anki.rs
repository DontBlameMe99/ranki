//! Anki's value encodings: ids, guids, field separators and checksums.

use ranki_model::FIELD_SEPARATOR;
use ranki_model::Id;
use ranki_model::StoreError;
use sha1::Digest;
use sha1::Sha1;
use std::process;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const DAY_SECS: i64 = 86_400;
const BASE91: &[u8; 91] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!#$%&()*+,./:;<=>?@[]^_`{|}~\"";

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
        .wrapping_add(u64::from(duration.subsec_nanos()))
        .wrapping_add(u64::from(process::id()));
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&id.to_be_bytes());
    bytes.extend_from_slice(&nanos.to_be_bytes());
    Ok(base91(&bytes))
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
pub fn deck_common(usn: i32, modified: i64) -> Vec<u8> {
    let mut bytes = vec![0x08];
    append_varint(&mut bytes, u64::try_from(usn).unwrap_or(0));
    bytes.push(0x18);
    append_varint(&mut bytes, u64::try_from(modified).unwrap_or(0));
    bytes
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

fn base91(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in bytes {
        buffer |= u32::from(*byte).wrapping_shl(u32::from(bits));
        bits = bits.saturating_add(8);
        while bits > 13 {
            push_base91(&mut output, buffer);
            buffer >>= 13;
            bits = bits.saturating_sub(13);
        }
    }
    if bits > 0 {
        push_base91(&mut output, buffer);
    }
    output
}
fn push_base91(output: &mut String, buffer: u32) {
    let value = buffer & 8191;
    let index = usize::try_from(value % 91).unwrap_or(0);
    let character = BASE91.get(index).copied().unwrap_or(b'?');
    output.push(char::from(character));
    if value > 90 {
        let second =
            usize::try_from(value.checked_div(91).unwrap_or(0)).unwrap_or(0);
        let character = BASE91.get(second).copied().unwrap_or(b'?');
        output.push(char::from(character));
    }
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
