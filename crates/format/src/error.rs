use std::error::Error;
use std::io::Error as IoError;

use rusqlite::Error as DbError;
use serde_json::Error as JsonError;
use zip::result::ZipError;

/// An error produced while reading or writing an Anki package.
#[derive(Debug, thiserror::Error)]
#[error("{message}{}", source.as_ref().map(|error| format!(": {error}")).unwrap_or_default())]
pub struct FormatError {
    message: String,
    #[source]
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl FormatError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        message: impl Into<String>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

pub fn io_error(
    context: impl Into<String>,
) -> impl FnOnce(IoError) -> FormatError {
    move |error| FormatError::with_source(context, error)
}

pub fn db_error(
    context: impl Into<String>,
) -> impl FnOnce(DbError) -> FormatError {
    move |error| FormatError::with_source(context, error)
}

pub fn zip_error(
    context: impl Into<String>,
) -> impl FnOnce(ZipError) -> FormatError {
    move |error| FormatError::with_source(context, error)
}

pub fn json_error(
    context: impl Into<String>,
) -> impl FnOnce(JsonError) -> FormatError {
    move |error| FormatError::with_source(context, error)
}
