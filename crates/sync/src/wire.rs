use crate::error::{Result, SyncError};
use crate::transport::{ORIGINAL_SIZE_HEADER, SyncResponse};
use serde::Serialize;
use std::io::Read;
use zstd::stream;

/// `AnkiWeb` rejects uploads at or above 300 MiB.
pub const MAX_COLLECTION_BYTES: u64 = 300 * 1024 * 1024;
pub const SYNC_VERSION: u8 = 11;

pub fn frame(bytes: &[u8]) -> Result<Vec<u8>> {
    stream::encode_all(bytes, 1)
        .map_err(|error| SyncError::Decode(error.to_string()))
}

pub fn frame_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(value)?;
    frame(&json)
}

pub fn decode(response: &SyncResponse) -> Result<Vec<u8>> {
    let declared = response
        .header(ORIGINAL_SIZE_HEADER)
        .and_then(|value| value.parse::<u64>().ok());
    let limit = match declared {
        Some(size) if size > MAX_COLLECTION_BYTES => {
            return Err(SyncError::TooLarge);
        }
        Some(size) => size,
        None => MAX_COLLECTION_BYTES,
    };
    let budget = limit.saturating_add(1);
    let decoder = stream::read::Decoder::new(response.body.as_slice())
        .map_err(|error| SyncError::Decode(error.to_string()))?;
    let mut output = Vec::new();
    decoder
        .take(budget)
        .read_to_end(&mut output)
        .map_err(|error| SyncError::Decode(error.to_string()))?;
    if u64::try_from(output.len()).unwrap_or(u64::MAX) > limit {
        return Err(SyncError::TooLarge);
    }
    Ok(output)
}
