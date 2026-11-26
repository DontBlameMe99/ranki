use crate::error::{Result, SyncError};
use crate::transport::{
    DEFAULT_ENDPOINT, ORIGINAL_SIZE_HEADER, RetryPolicy, SyncMethod,
    SyncResponse, SyncTransport, normalize_endpoint,
};
use crate::wire;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

/// A request observed by [`MockTransport`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedRequest {
    /// Endpoint called by the client.
    pub method: SyncMethod,
    /// Framed request body.
    pub body: Vec<u8>,
    /// Endpoint recorded at call time.
    pub endpoint: String,
    /// Host key recorded at call time.
    pub host_key: String,
}

/// Scripted response for [`MockTransport`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MockResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: BTreeMap<String, String>,
    /// Raw response body.
    pub body: Vec<u8>,
}

impl MockResponse {
    /// Build a successful zstd-framed JSON response.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or zstd framing fails.
    pub fn ok_json<T: Serialize>(value: &T) -> Result<Self> {
        let json = serde_json::to_vec(value)?;
        Self::ok_bytes(json)
    }

    /// Build a successful zstd-framed response around raw bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when zstd framing fails.
    pub fn ok_bytes(raw: impl Into<Vec<u8>>) -> Result<Self> {
        let bytes = raw.into();
        let body = wire::frame(&bytes)?;
        let mut headers = BTreeMap::new();
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_string();
        headers.insert(ORIGINAL_SIZE_HEADER.to_owned(), size);
        Ok(Self {
            status: 200,
            headers,
            body,
        })
    }

    /// Build an error response with a plain text body.
    #[must_use]
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            headers: BTreeMap::new(),
            body: message.into().into_bytes(),
        }
    }

    /// Build a response without framing it.
    #[must_use]
    pub const fn raw(
        status: u16,
        body: Vec<u8>,
        headers: BTreeMap<String, String>,
    ) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    /// Add or replace a response header.
    #[must_use]
    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }
}

impl From<MockResponse> for SyncResponse {
    fn from(response: MockResponse) -> Self {
        Self {
            status: response.status,
            headers: response.headers,
            body: response.body,
        }
    }
}

/// In-memory transport with scripted responses and failures.
#[derive(Clone, Debug)]
pub struct MockTransport {
    responses: BTreeMap<SyncMethod, VecDeque<MockResponse>>,
    failures: BTreeMap<SyncMethod, VecDeque<SyncError>>,
    requests: Vec<RecordedRequest>,
    endpoint: String,
    host_key: String,
    retry_policy: RetryPolicy,
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl MockTransport {
    /// Create an empty transport.
    #[must_use]
    pub fn new() -> Self {
        Self {
            responses: BTreeMap::new(),
            failures: BTreeMap::new(),
            requests: Vec::new(),
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            host_key: String::new(),
            retry_policy: RetryPolicy {
                retries: 0,
                backoff: Duration::ZERO,
            },
        }
    }

    /// Queue a response for a method. Responses are consumed in insertion order.
    #[must_use]
    pub fn with_response(
        mut self,
        method: SyncMethod,
        response: MockResponse,
    ) -> Self {
        self.responses
            .entry(method)
            .or_default()
            .push_back(response);
        self
    }

    /// Queue a transport failure for a method. Failures take priority over responses.
    #[must_use]
    pub fn with_failure(
        mut self,
        method: SyncMethod,
        error: SyncError,
    ) -> Self {
        self.failures.entry(method).or_default().push_back(error);
        self
    }

    /// Override retry behavior, mainly to make failure tests deterministic.
    #[must_use]
    pub const fn with_retry_policy(
        mut self,
        retry_policy: RetryPolicy,
    ) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    /// Requests seen so far.
    #[must_use]
    pub fn requests(&self) -> &[RecordedRequest] {
        &self.requests
    }

    /// Number of requests seen so far.
    #[must_use]
    pub const fn request_count(&self) -> usize {
        self.requests.len()
    }

    fn dispatch(
        &mut self,
        method: SyncMethod,
        body: &[u8],
    ) -> Result<SyncResponse> {
        self.requests.push(RecordedRequest {
            method,
            body: body.to_vec(),
            endpoint: self.endpoint.clone(),
            host_key: self.host_key.clone(),
        });
        if let Some(error) =
            self.failures.get_mut(&method).and_then(VecDeque::pop_front)
        {
            return Err(error);
        }
        self.responses
            .get_mut(&method)
            .and_then(VecDeque::pop_front)
            .map(SyncResponse::from)
            .ok_or_else(|| {
                SyncError::Network("no scripted response".to_owned())
            })
    }
}

impl SyncTransport for MockTransport {
    fn host_key(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.dispatch(SyncMethod::HostKey, body)
    }

    fn meta(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.dispatch(SyncMethod::Meta, body)
    }

    fn upload(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.dispatch(SyncMethod::Upload, body)
    }

    fn download(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.dispatch(SyncMethod::Download, body)
    }

    fn media_begin(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.dispatch(SyncMethod::MediaBegin, body)
    }

    fn media_changes(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.dispatch(SyncMethod::MediaChanges, body)
    }

    fn media_download(&mut self, body: &[u8]) -> Result<SyncResponse> {
        self.dispatch(SyncMethod::MediaDownload, body)
    }

    fn set_endpoint(&mut self, endpoint: &str) -> Result<()> {
        self.endpoint = normalize_endpoint(endpoint)?;
        Ok(())
    }

    fn set_host_key(&mut self, key: &str) {
        key.clone_into(&mut self.host_key);
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy
    }
}
