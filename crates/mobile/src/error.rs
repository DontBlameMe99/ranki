//! Errors shared by the mobile screens and platform entry points.

use std::io;
use std::result::Result as StdResult;

use thiserror::Error;

/// Failures surfaced by a mobile action or startup path.
#[derive(Debug, Error)]
pub enum MobileError {
    #[error("{0}")]
    Message(String),
    #[error("storage: {0}")]
    Store(#[from] ranki_model::StoreError),
    #[error("sync: {0}")]
    Sync(#[from] ranki_sync::SyncError),
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

impl From<String> for MobileError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for MobileError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_owned())
    }
}

/// Result type for mobile operations.
pub type Result<T> = StdResult<T, MobileError>;
