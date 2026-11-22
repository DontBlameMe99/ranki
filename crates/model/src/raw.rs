//! Hex encoding used to keep raw Anki config blobs in schema-11 JSON.

/// Encodes bytes as lowercase hex.
#[must_use]
pub fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let value = usize::from(*byte);
        let high = DIGITS.get(value >> 4).copied().unwrap_or(b'?');
        let low = DIGITS.get(value & 0x0f).copied().unwrap_or(b'?');
        output.push(char::from(high));
        output.push(char::from(low));
    }
    output
}

/// Decodes hex produced by [`encode_hex`].
#[must_use]
pub fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len().saturating_div(2));
    for pair in bytes.as_chunks::<2>().0 {
        let high = (pair.first().copied()?).checked_sub(b'0')?;
        let low = (pair.get(1).copied()?).checked_sub(b'0')?;
        let high_value = hex_digit(high)?;
        let low_value = hex_digit(low)?;
        output.push(high_value.saturating_mul(16).checked_add(low_value)?);
    }
    Some(output)
}

const fn hex_digit(value: u8) -> Option<u8> {
    match value {
        0..=9 => Some(value),
        17..=22 => Some(value.saturating_sub(17).saturating_add(10)),
        49..=54 => Some(value.saturating_sub(49).saturating_add(10)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::decode_hex;
    use super::encode_hex;

    #[test]
    fn round_trips_raw_config_bytes() {
        let bytes = vec![0, 1, 9, 10, 15, 16, 255];
        let encoded = encode_hex(&bytes);
        assert_eq!(encoded, "0001090a0f10ff");
        assert_eq!(decode_hex(&encoded), Some(bytes));
        assert_eq!(decode_hex("abc"), None);
    }
}
