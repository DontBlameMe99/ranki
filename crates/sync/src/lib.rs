//! Minimal `AnkiWeb` v11 sync for Ranki.
//!
//! [`SyncClient`] implements login, metadata checks, full upload, full download,
//! and a conservative automatic [`SyncClient::sync`]. It refuses one-way
//! changes when both sides changed or when the local collection may be newer.
//! Media files are never deleted or rewritten.
//!
//! HTTP lives behind the object-safe [`SyncTransport`] trait. [`HttpTransport`]
//! uses a blocking client, sends the `anki-sync` header fields `v`, `k`, `c`,
//! and `s`, disables redirects, and applies bounded timeouts and retries.
//! [`MockTransport`] provides scripted responses and failures for tests.
//!
//! Requests and responses are raw zstd-framed bytes. The client owns framing,
//! decompression, status checks, and payload limits. The state file under
//! `collection`'s directory stores only the host key and sync metadata, with
//! mode `0600`. Credentials are passed to [`SyncClient::login`] or
//! [`SyncClient::sync`] and are never written to disk or diagnostics.
//!
//! Wire details follow `docs/ankiweb-sync-research.md`. Delta sync, media sync,
//! and merge behavior are intentionally out of scope. Callers must choose
//! [`SyncClient::full_upload`] or [`SyncClient::full_download`] explicitly when
//! [`SyncOutcome::Conflict`] is returned.

mod client;
mod error;
mod mock;
mod state;
mod transport;
mod wire;

pub use client::{Credentials, RemoteStatus, SyncClient, SyncOutcome};
pub use error::{Result, SyncError};
pub use mock::{MockResponse, MockTransport, RecordedRequest};
pub use state::{SyncState, state_path};
pub use transport::{
    DEFAULT_CLIENT_VERSION, DEFAULT_ENDPOINT, HttpTransport,
    ORIGINAL_SIZE_HEADER, RetryPolicy, SyncMethod, SyncResponse, SyncTransport,
    TransportConfig,
};

#[cfg(test)]
mod tests;
