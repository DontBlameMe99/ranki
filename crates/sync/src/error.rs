use std::io;
use std::result;
use thiserror::Error;

/// Result type used by the sync crate.
pub type Result<T> = result::Result<T, SyncError>;

/// Errors produced by the sync layer.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SyncError {
    /// No host key is stored and no credentials were supplied.
    #[error("no sync credentials are available")]
    NoCredentials,

    /// The server rejected the username, password, or host key.
    #[error("sync authentication failed")]
    Auth,

    /// The server rejected the uploaded collection.
    #[error("server rejected the collection: {0}")]
    DatabaseCheck(String),

    /// The server reported a conflict that requires a user decision.
    #[error("sync conflict: {0}")]
    Conflict(String),

    /// The server is newer than this client.
    #[error("client is too old for this server")]
    ClientTooOld,

    /// An unrecoverable server response.
    #[error("server error {status}: {message}")]
    Server {
        /// HTTP status code.
        status: u16,
        /// Short server message.
        message: String,
    },

    /// A server message that stops the sync.
    #[error("server message: {0}")]
    ServerMessage(String),

    /// The local and server clocks differ by more than five minutes.
    #[error("clock skew exceeds 300 seconds")]
    ClockSkew,

    /// The server redirected more times than allowed.
    #[error("too many sync redirects")]
    TooManyRedirects,

    /// A transport or network failure.
    #[error("network error: {0}")]
    Network(String),

    /// A request exceeded its time budget.
    #[error("sync request timed out")]
    Timeout,

    /// A compressed or declared payload exceeded the size limit.
    #[error("sync payload is too large")]
    TooLarge,

    /// A response could not be decoded.
    #[error("invalid sync response: {0}")]
    Decode(String),

    /// SQLite rejected or could not prepare the collection file.
    #[error("collection database error: {0}")]
    Database(String),

    /// A local file operation failed.
    #[error("file error: {0}")]
    Io(String),

    /// The local state file is unusable.
    #[error("sync state error: {0}")]
    State(String),
}

impl SyncError {
    pub(crate) const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Network(_)
                | Self::Timeout
                | Self::Server {
                    status: 500..=599,
                    ..
                }
        )
    }
}

impl From<io::Error> for SyncError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for SyncError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(error.to_string())
    }
}

impl From<rusqlite::Error> for SyncError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}
