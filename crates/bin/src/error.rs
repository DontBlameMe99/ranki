//! Stable error codes shared by the MCP and `AnkiConnect` transports.

use std::fmt;
use std::io;

use ranki_model::StoreError;

use crate::CliError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    NotFound,
    InvalidData,
    Backend,
    Unsupported,
    Format,
    Io,
    Json,
    Message,
}

impl ErrorCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::InvalidData => "invalid_data",
            Self::Backend => "backend",
            Self::Unsupported => "unsupported",
            Self::Format => "format",
            Self::Io => "io",
            Self::Json => "json",
            Self::Message => "message",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceError {
    code: ErrorCode,
    message: String,
}

impl InterfaceError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Message, message)
    }

    pub(crate) const fn code(&self) -> ErrorCode {
        self.code
    }

    pub(crate) fn into_parts(self) -> (ErrorCode, String) {
        (self.code, self.message)
    }
}

impl fmt::Display for InterfaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<StoreError> for InterfaceError {
    fn from(error: StoreError) -> Self {
        let code = match &error {
            StoreError::NotFound(_) => ErrorCode::NotFound,
            StoreError::InvalidData(_) => ErrorCode::InvalidData,
            StoreError::Backend(_) => ErrorCode::Backend,
            StoreError::Unsupported(_) => ErrorCode::Unsupported,
        };
        Self::new(code, error.to_string())
    }
}

impl From<ranki_format::FormatError> for InterfaceError {
    fn from(error: ranki_format::FormatError) -> Self {
        Self::new(ErrorCode::Format, error.to_string())
    }
}

impl From<io::Error> for InterfaceError {
    fn from(error: io::Error) -> Self {
        Self::new(ErrorCode::Io, error.to_string())
    }
}

impl From<serde_json::Error> for InterfaceError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(ErrorCode::Json, error.to_string())
    }
}

impl From<CliError> for InterfaceError {
    fn from(source: CliError) -> Self {
        match source {
            CliError::Message(message) => Self::message(message),
            CliError::Store(store_error) => store_error.into(),
            CliError::Format(format_error) => format_error.into(),
            CliError::Json(json_error) => json_error.into(),
            CliError::Io(io_error) => io_error.into(),
        }
    }
}

impl From<String> for InterfaceError {
    fn from(error: String) -> Self {
        Self::message(error)
    }
}

impl From<&str> for InterfaceError {
    fn from(error: &str) -> Self {
        Self::message(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, InterfaceError};
    use crate::CliError;

    #[test]
    fn store_errors_keep_their_kind() {
        for (error, expected) in [
            (
                ranki_model::StoreError::NotFound("card".into()),
                ErrorCode::NotFound,
            ),
            (
                ranki_model::StoreError::InvalidData("bad".into()),
                ErrorCode::InvalidData,
            ),
            (
                ranki_model::StoreError::Backend("sqlite".into()),
                ErrorCode::Backend,
            ),
            (
                ranki_model::StoreError::Unsupported("nope".into()),
                ErrorCode::Unsupported,
            ),
        ] {
            let mapped = InterfaceError::from(error);
            assert_eq!(mapped.code(), expected);
            assert_ne!(mapped.to_string(), "");
        }
    }

    #[test]
    fn higher_level_errors_keep_their_source_kind() {
        let mapped = InterfaceError::from(CliError::Store(
            ranki_model::StoreError::NotFound("card".into()),
        ));
        assert_eq!(mapped.code(), ErrorCode::NotFound);
        let mapped = InterfaceError::from(CliError::Format(
            ranki_format::FormatError::new("bad package"),
        ));
        assert_eq!(mapped.code(), ErrorCode::Format);
        let mapped = InterfaceError::from(CliError::Message("oops".into()));
        assert_eq!(mapped.code(), ErrorCode::Message);
        assert_eq!(mapped.into_parts().1, "oops");
    }
}
