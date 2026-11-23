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

/// Builds an empty schema-18 `decks.common` blob.
///
/// `usn` and `mtime_secs` are columns of the `decks` table, not fields of
/// `Deck.Common`, so a new deck has no per-day counters to record.
#[must_use]
pub const fn deck_common() -> Vec<u8> {
    Vec::new()
}

/// Builds a schema-18 `decks.kind` blob for a normal deck.
///
/// `Deck.KindContainer.normal` only needs the deck config id here; Anki
/// rejects a deck row with an empty kind as `invalid deck kind`.
#[must_use]
pub fn deck_kind(config_id: i64) -> Vec<u8> {
    let mut normal = Vec::new();
    varint_field(&mut normal, 1, u64::try_from(config_id).unwrap_or(1));
    let mut bytes = Vec::new();
    message_field(&mut bytes, 1, &normal);
    bytes
}

/// Builds a schema-18 `notetypes.config` blob: kind, CSS, stock kind.
#[must_use]
pub fn notetype_config(
    kind: i32,
    stock_kind: Option<i32>,
    css: &str,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    varint_field(&mut bytes, 1, u64::try_from(kind).unwrap_or(0));
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

pub fn varint_field(bytes: &mut Vec<u8>, field: u32, value: u64) {
    let tag = u64::from(field).checked_shl(3).unwrap_or(0);
    append_varint(bytes, tag);
    append_varint(bytes, value);
}

pub fn message_field(bytes: &mut Vec<u8>, field: u32, body: &[u8]) {
    let tag = u64::from(field).checked_shl(3).unwrap_or(0) | 2;
    append_varint(bytes, tag);
    append_varint(bytes, u64::try_from(body.len()).unwrap_or(0));
    bytes.extend_from_slice(body);
}

pub fn string_field(bytes: &mut Vec<u8>, field: u32, value: &str) {
    let tag = u64::from(field).checked_shl(3).unwrap_or(0) | 2;
    append_varint(bytes, tag);
    append_varint(bytes, u64::try_from(value.len()).unwrap_or(0));
    bytes.extend_from_slice(value.as_bytes());
}

pub fn float_field(bytes: &mut Vec<u8>, field: u32, value: f32) {
    let tag = u64::from(field).checked_shl(3).unwrap_or(0) | 5;
    append_varint(bytes, tag);
    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
}

pub fn floats_field(bytes: &mut Vec<u8>, field: u32, values: &[f32]) {
    let mut payload = Vec::with_capacity(values.len().saturating_mul(4));
    for value in values {
        payload.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    message_field(bytes, field, &payload);
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

/// A decoded protobuf field value from an Anki binary column.
#[derive(Clone, Copy, Debug)]
pub enum ProtoValue<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Float(f32),
}

/// Decodes every field of a protobuf message.
///
/// Unknown wire types are rejected instead of skipped so a caller never
/// silently reads a truncated Anki blob.
#[must_use]
pub fn protobuf_fields(bytes: &[u8]) -> Option<Vec<(u32, ProtoValue<'_>)>> {
    let mut fields = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let tag = read_varint(bytes, &mut pos)?;
        let field = u32::try_from(tag >> 3).ok()?;
        let value = match tag & 0x07 {
            0 => ProtoValue::Varint(read_varint(bytes, &mut pos)?),
            1 => {
                let _ = bytes.get(pos..pos.checked_add(8)?)?;
                pos = pos.checked_add(8)?;
                continue;
            }
            2 => {
                let length =
                    usize::try_from(read_varint(bytes, &mut pos)?).ok()?;
                let end = pos.checked_add(length)?;
                let slice = bytes.get(pos..end)?;
                pos = end;
                ProtoValue::Bytes(slice)
            }
            5 => {
                let slice = bytes.get(pos..pos.checked_add(4)?)?;
                let array: [u8; 4] = slice.try_into().ok()?;
                pos = pos.checked_add(4)?;
                ProtoValue::Float(f32::from_bits(u32::from_le_bytes(array)))
            }
            _ => return None,
        };
        fields.push((field, value));
    }
    Some(fields)
}

#[must_use]
pub fn proto_varint(
    fields: &[(u32, ProtoValue<'_>)],
    wanted: u32,
) -> Option<u64> {
    fields.iter().find_map(|(field, value)| match value {
        ProtoValue::Varint(number) if *field == wanted => Some(*number),
        ProtoValue::Varint(_) | ProtoValue::Bytes(_) | ProtoValue::Float(_) => {
            None
        }
    })
}

#[must_use]
pub fn proto_string(
    fields: &[(u32, ProtoValue<'_>)],
    wanted: u32,
) -> Option<String> {
    proto_bytes(fields, wanted)
        .and_then(|bytes| String::from_utf8(bytes.to_vec()).ok())
}

#[must_use]
pub fn proto_bytes<'a>(
    fields: &[(u32, ProtoValue<'a>)],
    wanted: u32,
) -> Option<&'a [u8]> {
    fields.iter().find_map(|(field, value)| match value {
        ProtoValue::Bytes(bytes) if *field == wanted => Some(*bytes),
        ProtoValue::Varint(_) | ProtoValue::Bytes(_) | ProtoValue::Float(_) => {
            None
        }
    })
}

/// Every length-delimited value for a repeated message field.
#[must_use]
pub fn proto_messages<'a>(
    fields: &[(u32, ProtoValue<'a>)],
    wanted: u32,
) -> Vec<&'a [u8]> {
    fields
        .iter()
        .filter_map(|(field, value)| match value {
            ProtoValue::Bytes(bytes) if *field == wanted => Some(*bytes),
            ProtoValue::Varint(_)
            | ProtoValue::Bytes(_)
            | ProtoValue::Float(_) => None,
        })
        .collect()
}

#[must_use]
pub fn proto_float(
    fields: &[(u32, ProtoValue<'_>)],
    wanted: u32,
) -> Option<f32> {
    fields.iter().find_map(|(field, value)| match value {
        ProtoValue::Float(number) if *field == wanted => Some(*number),
        ProtoValue::Varint(_) | ProtoValue::Bytes(_) | ProtoValue::Float(_) => {
            None
        }
    })
}

/// A repeated float field, which Anki encodes either packed or unpacked.
#[must_use]
pub fn proto_floats(fields: &[(u32, ProtoValue<'_>)], wanted: u32) -> Vec<f32> {
    let mut values = Vec::new();
    for (field, value) in fields {
        if *field != wanted {
            continue;
        }
        match value {
            ProtoValue::Float(number) => values.push(*number),
            ProtoValue::Bytes(bytes) => packed_floats(&mut values, bytes),
            ProtoValue::Varint(_) => {}
        }
    }
    values
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

fn packed_floats(values: &mut Vec<f32>, bytes: &[u8]) {
    let (chunks, _) = bytes.as_chunks::<4>();
    for chunk in chunks {
        values.push(f32::from_bits(u32::from_le_bytes(*chunk)));
    }
}

fn remove_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('<') {
        output.push_str(rest.get(..start).unwrap_or(""));
        let tail = rest.get(start..).unwrap_or("");
        if let Some(end) = tail.find('>') {
            let tag = tail.get(..end.saturating_add(1)).unwrap_or("");
            push_media_reference(&mut output, tag);
            rest = tail.get(end.saturating_add(1)..).unwrap_or("");
        } else {
            output.push_str(tail);
            return output;
        }
    }
    output.push_str(rest);
    output
}

fn push_media_reference(output: &mut String, tag: &str) {
    if let Some(name) = media_reference(tag) {
        output.push(' ');
        output.push_str(name);
        output.push(' ');
    }
}

/// Returns the `src`/`data` filename of an HTML media tag.
///
/// Anki keeps these filenames when it strips HTML for sort fields and
/// checksums so two notes that differ only by their image stay distinct.
fn media_reference(tag: &str) -> Option<&str> {
    let body = tag.strip_prefix('<')?;
    let name_end = body
        .find(|character: char| character.is_whitespace() || character == '>')
        .unwrap_or(body.len());
    let name = body.get(..name_end)?;
    if !matches!(
        name.to_ascii_lowercase().as_str(),
        "img" | "audio" | "video" | "object" | "source"
    ) {
        return None;
    }
    let lowercase = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(rest) = lowercase.get(search..) {
        let src = rest.find("src=");
        let data = rest.find("data=");
        let data_first = match (src, data) {
            (Some(left), Some(right)) => right < left,
            (None, Some(_)) => true,
            (_, None) => false,
        };
        let (offset, prefix) = if data_first { (data?, 5) } else { (src?, 4) };
        let value_start = search.saturating_add(offset).saturating_add(prefix);
        let after = tag.get(value_start..)?;
        if let Some(quote) = after
            .chars()
            .next()
            .filter(|character| *character == '"' || *character == '\'')
        {
            let inner = after.get(1..)?;
            let end = inner.find(quote)?;
            return inner.get(..end);
        }
        let end = after
            .find(|character: char| {
                character.is_whitespace() || character == '>'
            })
            .unwrap_or(after.len());
        let value = after.get(..end)?;
        if !value.is_empty() {
            return Some(value);
        }
        search = value_start;
    }
    None
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
    use super::checksum;
    use super::csum_value;
    use super::sort_field;

    #[test]
    fn checksum_matches_anki_reference_values() {
        assert_eq!(csum_value(checksum("test")), 2_840_236_005);
        assert_eq!(csum_value(checksum("今日")), 1_464_653_051);
    }

    #[test]
    fn sort_field_keeps_media_filenames_like_anki() {
        let field = String::from("<img src=\"Abkhazia.png\" />");
        assert_eq!(sort_field(&[field], 0), " Abkhazia.png ");
        let single = String::from("<img src='a b.png'>");
        assert_eq!(sort_field(&[single], 0), " a b.png ");
        let nested = String::from("<b>hi</b><audio src=\"clip.mp3\"></audio>");
        assert_eq!(sort_field(&[nested], 0), "hi clip.mp3 ");
        let object = String::from("<object data=\"diagram.svg\"></object>");
        assert_eq!(sort_field(&[object], 0), " diagram.svg ");
        let bare = String::from("<object data=diagram.svg></object>");
        assert_eq!(sort_field(&[bare], 0), " diagram.svg ");
        let plain = String::from("<b>hello</b> world");
        assert_eq!(sort_field(&[plain], 0), "hello world");
    }
}
