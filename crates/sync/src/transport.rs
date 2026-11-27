use crate::error::{Result, SyncError};
use crate::wire::{MAX_COLLECTION_BYTES, SYNC_VERSION};
use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::env::consts;
use std::fmt;
use std::hash::BuildHasher;
use std::process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ureq::Agent;

/// Default `AnkiWeb` endpoint.
pub const DEFAULT_ENDPOINT: &str = "https://sync.ankiweb.net/";

/// Platform token used in `AnkiWeb` client identifiers.
pub const SYNC_PLATFORM: &str = consts::OS;

/// Client identifier sent in the `anki-sync` header.
#[must_use]
pub fn client_version() -> String {
    format!("ranki,0.1.0,1,{SYNC_PLATFORM}")
}

/// Client version reported in `sync/meta`. `AnkiWeb` rejects a `cv` value that
/// does not follow Anki's `anki,<version>,<platform>` shape.
#[must_use]
pub fn meta_client_version() -> String {
    format!("anki,ranki-0.1.0,{SYNC_PLATFORM}")
}

/// Header that carries the uncompressed response length.
pub const ORIGINAL_SIZE_HEADER: &str = "anki-original-size";

const BASE62: &[u8; 62] =
    b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// The four collection endpoints used by this crate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SyncMethod {
    /// Exchange username and password for a host key.
    HostKey,
    /// Read remote collection metadata.
    Meta,
    /// Replace the remote collection with the local one.
    Upload,
    /// Replace the local collection with the remote one.
    Download,
    /// Begin a media download session.
    MediaBegin,
    /// Read the remote media manifest changes.
    MediaChanges,
    /// Download a batch of media files.
    MediaDownload,
}

impl fmt::Display for SyncMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::HostKey => "sync/hostKey",
            Self::Meta => "sync/meta",
            Self::Upload => "sync/upload",
            Self::Download => "sync/download",
            Self::MediaBegin => "msync/begin",
            Self::MediaChanges => "msync/mediaChanges",
            Self::MediaDownload => "msync/downloadFiles",
        };
        formatter.write_str(name)
    }
}

/// Raw response returned by a transport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncResponse {
    /// HTTP status code.
    pub status: u16,
    /// Lowercase response header names and their values.
    pub headers: BTreeMap<String, String>,
    /// Raw response bytes, normally a zstd frame.
    pub body: Vec<u8>,
}

impl SyncResponse {
    /// Return a header value by name, case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// Bounded retry settings used by [`SyncClient`](crate::SyncClient).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Number of retries after the first attempt.
    pub retries: u8,
    /// Delay before the first retry, increased linearly for later retries.
    pub backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            retries: 2,
            backoff: Duration::from_secs(1),
        }
    }
}

/// Timeouts and size limits used by [`HttpTransport`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportConfig {
    /// TCP and TLS connection timeout.
    pub connect_timeout: Duration,
    /// Per-read and per-write timeout.
    pub io_timeout: Duration,
    /// Total budget for one request.
    pub total_timeout: Duration,
    /// Retry settings reported to the client.
    pub retry_policy: RetryPolicy,
    /// Maximum compressed response body.
    pub max_response_bytes: u64,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            io_timeout: Duration::from_secs(30),
            total_timeout: Duration::from_secs(120),
            retry_policy: RetryPolicy::default(),
            max_response_bytes: MAX_COLLECTION_BYTES,
        }
    }
}

/// Object-safe boundary between the sync state machine and HTTP.
///
/// Every method accepts an already framed zstd body. Responses keep their raw
/// bytes and status headers so the client can decompress and validate them.
pub trait SyncTransport: Send {
    /// Call `sync/hostKey`.
    ///
    /// # Errors
    ///
    /// Returns a transport error on a network or HTTP failure.
    fn host_key(&mut self, body: &[u8]) -> Result<SyncResponse>;
    /// Call `sync/meta`.
    ///
    /// # Errors
    ///
    /// Returns a transport error on a network or HTTP failure.
    fn meta(&mut self, body: &[u8]) -> Result<SyncResponse>;
    /// Call `sync/upload`.
    ///
    /// # Errors
    ///
    /// Returns a transport error on a network or HTTP failure.
    fn upload(&mut self, body: &[u8]) -> Result<SyncResponse>;
    /// Call `sync/download`.
    ///
    /// # Errors
    ///
    /// Returns a transport error on a network or HTTP failure.
    fn download(&mut self, body: &[u8]) -> Result<SyncResponse>;
    /// Call `msync/begin`.
    ///
    /// # Errors
    ///
    /// Returns a transport error on a network or HTTP failure.
    fn media_begin(&mut self, body: &[u8]) -> Result<SyncResponse>;
    /// Call `msync/mediaChanges`.
    ///
    /// # Errors
    ///
    /// Returns a transport error on a network or HTTP failure.
    fn media_changes(&mut self, body: &[u8]) -> Result<SyncResponse>;
    /// Call `msync/downloadFiles`.
    ///
    /// # Errors
    ///
    /// Returns a transport error on a network or HTTP failure.
    fn media_download(&mut self, body: &[u8]) -> Result<SyncResponse>;
    /// Replace the endpoint after a server redirect.
    ///
    /// # Errors
    ///
    /// Returns an error when the endpoint is not a usable HTTPS URL.
    fn set_endpoint(&mut self, endpoint: &str) -> Result<()>;
    /// Replace the host key used in the `anki-sync` header.
    fn set_host_key(&mut self, key: &str);
    /// Retry settings for this transport.
    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }
}

impl<T: SyncTransport + ?Sized> SyncTransport for Box<T> {
    fn host_key(&mut self, body: &[u8]) -> Result<SyncResponse> {
        (**self).host_key(body)
    }

    fn meta(&mut self, body: &[u8]) -> Result<SyncResponse> {
        (**self).meta(body)
    }

    fn upload(&mut self, body: &[u8]) -> Result<SyncResponse> {
        (**self).upload(body)
    }

    fn download(&mut self, body: &[u8]) -> Result<SyncResponse> {
        (**self).download(body)
    }

    fn media_begin(&mut self, body: &[u8]) -> Result<SyncResponse> {
        (**self).media_begin(body)
    }

    fn media_changes(&mut self, body: &[u8]) -> Result<SyncResponse> {
        (**self).media_changes(body)
    }

    fn media_download(&mut self, body: &[u8]) -> Result<SyncResponse> {
        (**self).media_download(body)
    }

    fn set_endpoint(&mut self, endpoint: &str) -> Result<()> {
        (**self).set_endpoint(endpoint)
    }

    fn set_host_key(&mut self, key: &str) {
        (**self).set_host_key(key);
    }

    fn retry_policy(&self) -> RetryPolicy {
        (**self).retry_policy()
    }
}

/// Blocking `AnkiWeb` v11 transport.
pub struct HttpTransport {
    agent: Agent,
    endpoint: String,
    host_key: String,
    client_version: String,
    session_key: String,
    config: TransportConfig,
}

impl fmt::Debug for HttpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpTransport")
            .field("endpoint", &self.endpoint)
            .field("host_key", &"<redacted>")
            .field("client_version", &self.client_version)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport {
    /// Build a transport for the default `AnkiWeb` endpoint.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(DEFAULT_ENDPOINT, TransportConfig::default())
    }

    /// Build a transport with custom timeouts and retry settings.
    #[must_use]
    pub fn with_config(
        endpoint: impl Into<String>,
        config: TransportConfig,
    ) -> Self {
        let agent: Agent = Agent::config_builder()
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .timeout_connect(Some(config.connect_timeout))
            .timeout_per_read(Some(config.io_timeout))
            .timeout_per_write(Some(config.io_timeout))
            .timeout_global(Some(config.total_timeout))
            .build()
            .into();
        Self {
            agent,
            endpoint: endpoint.into(),
            host_key: String::new(),
            client_version: client_version(),
            session_key: session_key(),
            config,
        }
    }

    /// Use a different client identifier in the `anki-sync` header.
    #[must_use]
    pub fn with_client_version(
        mut self,
        client_version: impl Into<String>,
    ) -> Self {
        self.client_version = client_version.into();
        self
    }

    fn request(&self, method: SyncMethod, body: &[u8]) -> Result<SyncResponse> {
        let endpoint = normalize_endpoint(&self.endpoint)?;
        let url = format!("{endpoint}{method}");
        let header = serde_json::json!({
            "v": SYNC_VERSION,
            "k": self.host_key,
            "c": self.client_version,
            "s": self.session_key,
        })
        .to_string();
        let response = self
            .agent
            .post(url)
            .header("anki-sync", header)
            .content_type("application/octet-stream")
            .send(body)
            .map_err(|error| map_http_error(&error))?;
        let status = u16::from(response.status());
        let mut headers = BTreeMap::new();
        for (name, value) in response.headers() {
            let header_value =
                value.to_str().map(str::to_owned).unwrap_or_default();
            headers.insert(name.as_str().to_ascii_lowercase(), header_value);
        }
        let body = response
            .into_body()
            .with_config()
            .limit(self.config.max_response_bytes)
            .read_to_vec()
            .map_err(|error| map_http_error(&error))?;
        Ok(SyncResponse {
            status,
            headers,
            body,
        })
    }
}

impl SyncTransport for HttpTransport {
    fn host_key(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.request(SyncMethod::HostKey, body)
    }

    fn meta(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.request(SyncMethod::Meta, body)
    }

    fn upload(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.request(SyncMethod::Upload, body)
    }

    fn download(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.request(SyncMethod::Download, body)
    }

    fn media_begin(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.request(SyncMethod::MediaBegin, body)
    }

    fn media_changes(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.request(SyncMethod::MediaChanges, body)
    }

    fn media_download(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.request(SyncMethod::MediaDownload, body)
    }

    fn set_endpoint(&mut self, endpoint: &str) -> Result<()> {
        self.endpoint = normalize_endpoint(endpoint)?;
        Ok(())
    }

    fn set_host_key(&mut self, key: &str) {
        key.clone_into(&mut self.host_key);
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.config.retry_policy
    }
}

/// Normalize and validate an HTTPS endpoint.
///
/// # Errors
///
/// Returns an error when the endpoint is not a usable HTTPS URL.
pub fn normalize_endpoint(raw: &str) -> Result<String> {
    let endpoint = raw.trim();
    if !endpoint.starts_with("https://")
        || endpoint.contains('?')
        || endpoint.contains('#')
    {
        return Err(SyncError::Network("invalid sync endpoint".to_owned()));
    }
    if endpoint.ends_with('/') {
        Ok(endpoint.to_owned())
    } else {
        Ok(format!("{endpoint}/"))
    }
}

fn map_http_error(error: &ureq::Error) -> SyncError {
    if matches!(error, ureq::Error::Timeout(_)) {
        return SyncError::Timeout;
    }
    if let ureq::Error::Io(io_error) = &error {
        return SyncError::Network(io_error.to_string());
    }
    if matches!(
        error,
        ureq::Error::HostNotFound
            | ureq::Error::ConnectionFailed
            | ureq::Error::RedirectFailed
    ) {
        return SyncError::Network("connection failed".to_owned());
    }
    SyncError::Network("http request failed".to_owned())
}

fn session_key() -> String {
    let hasher = RandomState::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seed = nanos ^ u128::from(process::id());
    base62(hasher.hash_one(seed))
}

fn base62(mut value: u64) -> String {
    if value == 0 {
        return "0".to_owned();
    }
    let mut buffer = [0_u8; 11];
    let mut index = buffer.len();
    while value > 0 {
        let digit = value.checked_rem(62).unwrap_or_default();
        index = index.saturating_sub(1);
        let character = BASE62
            .get(usize::try_from(digit).unwrap_or_default())
            .copied();
        if let Some(slot) = buffer.get_mut(index) {
            *slot = character.unwrap_or(b'0');
        }
        value = value.checked_div(62).unwrap_or_default();
    }
    let bytes = buffer.iter().skip(index).copied().collect::<Vec<u8>>();
    String::from_utf8(bytes).unwrap_or_default()
}
