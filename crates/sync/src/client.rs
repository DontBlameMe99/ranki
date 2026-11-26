use crate::error::{Result, SyncError};
use crate::state::SyncState;
use crate::transport::{
    META_CLIENT_VERSION, SyncMethod, SyncResponse, SyncTransport,
    normalize_endpoint,
};
use crate::wire::{self, MAX_COLLECTION_BYTES, SYNC_VERSION};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::Builder;

const CLOCK_SKEW_SECONDS: i64 = 300;
const MAX_REDIRECTS: u8 = 3;

/// Username and password used only for a single login call.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    /// Build credentials from caller-provided strings.
    #[must_use]
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Collection metadata returned by `sync/meta`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteStatus {
    /// Server collection modification time in milliseconds.
    pub mod_time: i64,
    /// Server schema modification time in milliseconds.
    pub schema_mod_time: i64,
    /// Server USN.
    pub usn: i32,
    /// Server Unix time in seconds.
    pub server_time: i64,
    /// Whether the remote collection has no cards.
    pub empty: bool,
    /// Server media USN.
    pub media_usn: i32,
    /// Optional server message.
    pub message: Option<String>,
}

/// What [`SyncClient::sync`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    /// No host key and no credentials were supplied.
    Disabled,
    /// Both sides already match the last successful sync.
    NoChanges,
    /// The local collection was uploaded.
    Uploaded,
    /// The remote collection replaced the local one.
    Downloaded,
    /// A one-way sync would risk data loss.
    Conflict {
        /// Reason the automatic sync stopped.
        reason: String,
    },
    /// Transient failures remain after bounded retries.
    RetryLater {
        /// Reason the sync should be retried later.
        reason: String,
    },
}

/// Minimal `AnkiWeb` v11 collection sync client.
///
/// The client stores only the host key and sync metadata in its state file.
/// Media files are never deleted or rewritten.
pub struct SyncClient<T: SyncTransport> {
    transport: T,
    state: SyncState,
    collection: PathBuf,
    media_dir: PathBuf,
    state_path: PathBuf,
}

impl<T: SyncTransport> SyncClient<T> {
    /// Load local state and bind the client to a collection and media directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the state file is unreadable or the stored
    /// endpoint is not a usable HTTPS URL.
    #[expect(
        clippy::too_many_arguments,
        reason = "paths and transport are supplied together"
    )]
    pub fn new(
        transport: T,
        collection: &Path,
        media_dir: &Path,
        state_path: &Path,
    ) -> Result<Self> {
        let mut transport = transport;
        let state = SyncState::load(state_path)?;
        transport.set_endpoint(&state.endpoint)?;
        transport.set_host_key(&state.host_key);
        Ok(Self {
            transport,
            state,
            collection: collection.to_path_buf(),
            media_dir: media_dir.to_path_buf(),
            state_path: state_path.to_path_buf(),
        })
    }

    /// Transport used by this client.
    #[must_use]
    pub const fn transport(&self) -> &T {
        &self.transport
    }

    /// Current persisted state.
    #[must_use]
    pub const fn state(&self) -> &SyncState {
        &self.state
    }

    /// Collection path supplied by the caller.
    #[must_use]
    pub fn collection(&self) -> &Path {
        &self.collection
    }

    /// Media directory supplied by the caller. It is never modified.
    #[must_use]
    pub fn media_dir(&self) -> &Path {
        &self.media_dir
    }

    /// Exchange credentials for a host key and persist only that key.
    ///
    /// # Errors
    ///
    /// Returns an error when the login request fails or the state file cannot
    /// be written.
    pub fn login(&mut self, credentials: &Credentials) -> Result<()> {
        self.transport.set_host_key("");
        let request = HostKeyRequest {
            u: &credentials.username,
            p: &credentials.password,
        };
        let body = wire::frame_json(&request)?;
        let response = self.send(SyncMethod::HostKey, &body)?;
        let bytes = wire::decode(&response)?;
        let reply: HostKeyReply = serde_json::from_slice(&bytes)?;
        if reply.key.is_empty() {
            return Err(SyncError::Auth);
        }
        let key = reply.key;
        self.state.host_key.clone_from(&key);
        self.state.failures = 0;
        self.transport.set_host_key(&key);
        self.state.save(&self.state_path)?;
        Ok(())
    }

    /// Read and check remote collection metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when credentials are missing, the request fails, the
    /// response is invalid, or the clocks differ by more than five minutes.
    pub fn status(&mut self) -> Result<RemoteStatus> {
        self.require_host_key()?;
        let request = MetaRequest {
            v: SYNC_VERSION,
            cv: META_CLIENT_VERSION,
        };
        let body = wire::frame_json(&request)?;
        let response = self.send(SyncMethod::Meta, &body)?;
        let bytes = wire::decode(&response)?;
        let meta: MetaWire = serde_json::from_slice(&bytes)?;
        let message = meta.msg.clone();
        if !meta.flags.cont.is_on() {
            return Err(SyncError::ServerMessage(
                message.unwrap_or_else(|| "server stopped sync".to_owned()),
            ));
        }
        let limit = u64::try_from(CLOCK_SKEW_SECONDS).unwrap_or_default();
        if now_secs()?.abs_diff(meta.ts) > limit {
            return Err(SyncError::ClockSkew);
        }
        Ok(RemoteStatus {
            mod_time: meta.mod_time,
            schema_mod_time: meta.scm,
            usn: meta.usn,
            server_time: meta.ts,
            empty: meta.flags.empty.is_on(),
            media_usn: meta.media_usn,
            message,
        })
    }

    /// Upload the whole local collection. This is an explicit one-way choice.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection cannot be prepared, the upload
    /// fails, or the state file cannot be written.
    pub fn full_upload(&mut self) -> Result<()> {
        self.require_host_key()?;
        let (info, bytes) = self.prepare_collection()?;
        let body = wire::frame(&bytes)?;
        let response = self.send(SyncMethod::Upload, &body)?;
        let reply = wire::decode(&response)?;
        if String::from_utf8_lossy(&reply).trim() != "OK" {
            return Err(SyncError::ServerMessage(
                "upload was not accepted".to_owned(),
            ));
        }
        self.state.last_mod = Some(info.mod_time);
        self.state.last_scm = Some(info.scm);
        self.record_success()?;
        Ok(())
    }

    /// Download the whole remote collection. This is an explicit one-way choice.
    ///
    /// # Errors
    ///
    /// Returns an error when the download fails, the temporary collection is
    /// invalid, or the atomic replacement fails.
    pub fn full_download(&mut self) -> Result<()> {
        self.require_host_key()?;
        let body = wire::frame(b"{}")?;
        let response = self.send(SyncMethod::Download, &body)?;
        let bytes = wire::decode(&response)?;
        if bytes.is_empty()
            || u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                >= MAX_COLLECTION_BYTES
        {
            return Err(SyncError::TooLarge);
        }
        let parent = self.collection.parent().unwrap_or_else(|| Path::new("."));
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
        let mut file = Builder::new()
            .prefix(".ranki-sync-")
            .suffix(".anki2")
            .tempfile_in(parent)?;
        {
            let writer = file.as_file_mut();
            writer.write_all(&bytes)?;
            writer.flush()?;
        }
        let info = validate_download(file.path())?;
        file.as_file().sync_all()?;
        self.backup_collection()?;
        file.persist(&self.collection)
            .map_err(|error| SyncError::Io(error.to_string()))?;
        self.state.last_mod = Some(info.mod_time);
        self.state.last_scm = Some(info.scm);
        self.record_success()?;
        Ok(())
    }

    /// Run one safe automatic sync.
    ///
    /// Credentials are used for login only when no host key is stored. The
    /// method never chooses a one-way sync when both sides changed or when the
    /// local collection may be newer than the server copy.
    ///
    /// # Errors
    ///
    /// Returns an error when login or metadata retrieval fails for a
    /// non-retryable reason.
    pub fn sync(
        &mut self,
        credentials: Option<Credentials>,
    ) -> Result<SyncOutcome> {
        if self.state.host_key.is_empty() {
            match credentials {
                Some(supplied) => self.login(&supplied)?,
                None => return Ok(SyncOutcome::Disabled),
            }
        }
        match self.decide_and_sync() {
            Ok(outcome) => Ok(outcome),
            Err(error) if error.is_retryable() => {
                self.state.failures = self.state.failures.saturating_add(1);
                self.state.save(&self.state_path)?;
                Ok(SyncOutcome::RetryLater {
                    reason: error.to_string(),
                })
            }
            Err(error) => Err(error),
        }
    }

    fn decide_and_sync(&mut self) -> Result<SyncOutcome> {
        let status = self.status()?;
        let local = read_local_info(&self.collection)?;
        let last_mod = self.state.last_mod;
        let last_scm = self.state.last_scm;

        let Some(last) = last_mod else {
            return self.sync_without_stamp(&status, local);
        };
        let schema_matches = match last_scm {
            Some(scm) => scm == status.schema_mod_time,
            None => local.scm == status.schema_mod_time,
        };
        if !schema_matches {
            return Ok(conflict(
                "schema changed; choose whether to keep the local or AnkiWeb collection",
            ));
        }
        let local_changed = local.mod_time != last;
        let remote_changed = status.mod_time != last;
        if local_changed && remote_changed {
            return Ok(conflict(
                "both collections changed since the last sync",
            ));
        }
        if !local_changed && !remote_changed {
            self.state.failures = 0;
            self.state.last_success = Some(now_secs()?);
            self.state.save(&self.state_path)?;
            return Ok(SyncOutcome::NoChanges);
        }
        if local_changed {
            self.full_upload()?;
            return Ok(SyncOutcome::Uploaded);
        }
        if status.mod_time < local.mod_time {
            return Ok(conflict(
                "local collection is newer than the server copy",
            ));
        }
        self.full_download()?;
        Ok(SyncOutcome::Downloaded)
    }

    fn sync_without_stamp(
        &mut self,
        status: &RemoteStatus,
        local: LocalInfo,
    ) -> Result<SyncOutcome> {
        match (status.empty, local.cards) {
            (_, 0) if status.empty => {
                self.record_success()?;
                Ok(SyncOutcome::NoChanges)
            }
            (false, 0) => {
                self.full_download()?;
                Ok(SyncOutcome::Downloaded)
            }
            _ => Ok(conflict(
                "choose whether to keep the local or AnkiWeb collection",
            )),
        }
    }

    const fn require_host_key(&self) -> Result<()> {
        if self.state.host_key.is_empty() {
            Err(SyncError::NoCredentials)
        } else {
            Ok(())
        }
    }

    fn record_success(&mut self) -> Result<()> {
        self.state.last_success = Some(now_secs()?);
        self.state.failures = 0;
        self.state.save(&self.state_path)
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "request loop keeps redirect and retry state local"
    )]
    fn send(
        &mut self,
        method: SyncMethod,
        body: &[u8],
    ) -> Result<SyncResponse> {
        let mut redirects = 0_u8;
        loop {
            let response = self.attempt(method, body)?;
            if response.status != 308 {
                ensure_success(&response, method)?;
                return Ok(response);
            }
            self.follow_redirect(&response)?;
            redirects = redirects.saturating_add(1);
            if redirects > MAX_REDIRECTS {
                return Err(SyncError::TooManyRedirects);
            }
        }
    }

    fn follow_redirect(&mut self, response: &SyncResponse) -> Result<()> {
        let location = response.header("location").ok_or_else(|| {
            SyncError::Network("redirect response has no location".to_owned())
        })?;
        let endpoint = normalize_endpoint(location)?;
        self.transport.set_endpoint(&endpoint)?;
        self.state.endpoint = endpoint;
        self.state.save(&self.state_path)
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "retry loop keeps attempt state local"
    )]
    fn attempt(
        &mut self,
        method: SyncMethod,
        body: &[u8],
    ) -> Result<SyncResponse> {
        let policy = self.transport.retry_policy();
        let mut attempt = 0_u8;
        loop {
            let result = match method {
                SyncMethod::HostKey => self.transport.host_key(body),
                SyncMethod::Meta => self.transport.meta(body),
                SyncMethod::Upload => self.transport.upload(body),
                SyncMethod::Download => self.transport.download(body),
            };
            let retry = is_retryable_result(&result, method)
                && attempt < policy.retries;
            if !retry {
                return result;
            }
            let delay = policy
                .backoff
                .saturating_mul(u32::from(attempt).saturating_add(1));
            sleep(delay);
            attempt = attempt.saturating_add(1);
        }
    }

    fn prepare_collection(&self) -> Result<(LocalInfo, Vec<u8>)> {
        let mut connection = Connection::open(&self.collection)?;
        register_collations(&connection)?;
        {
            let transaction = connection.transaction()?;
            prepare_transaction(&transaction)?;
            transaction.commit()?;
        }
        let checkpoint: i64 = connection.query_row(
            "PRAGMA wal_checkpoint(TRUNCATE)",
            [],
            |row| row.get(0),
        )?;
        if checkpoint < 0 {
            return Err(SyncError::Database(
                "wal checkpoint was busy".to_owned(),
            ));
        }
        let info = local_info(&connection)?;
        connection
            .close()
            .map_err(|(_, error)| SyncError::Database(error.to_string()))?;
        let bytes = fs::read(&self.collection)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            >= MAX_COLLECTION_BYTES
        {
            return Err(SyncError::TooLarge);
        }
        Ok((info, bytes))
    }

    fn backup_collection(&self) -> Result<()> {
        if !self.collection.exists() {
            return Ok(());
        }
        let backup = self.collection.with_extension("ranki-sync.bak");
        fs::copy(&self.collection, &backup)?;
        Ok(())
    }
}

#[derive(Serialize)]
struct HostKeyRequest<'a> {
    u: &'a str,
    p: &'a str,
}

#[derive(Deserialize)]
struct HostKeyReply {
    key: String,
}

#[derive(Serialize)]
struct MetaRequest<'a> {
    v: u8,
    cv: &'a str,
}

#[derive(Deserialize)]
struct MetaWire {
    #[serde(rename = "mod")]
    mod_time: i64,
    scm: i64,
    usn: i32,
    ts: i64,
    #[serde(flatten)]
    flags: MetaFlags,
    #[serde(default)]
    msg: Option<String>,
    #[serde(rename = "media_usn", default)]
    media_usn: i32,
}

#[derive(Clone, Copy, Deserialize)]
struct MetaFlags {
    #[serde(default)]
    empty: SyncFlag,
    #[serde(default = "cont_true")]
    cont: SyncFlag,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(from = "bool")]
enum SyncFlag {
    On,
    #[default]
    Off,
}

impl From<bool> for SyncFlag {
    fn from(value: bool) -> Self {
        if value { Self::On } else { Self::Off }
    }
}

impl SyncFlag {
    const fn is_on(self) -> bool {
        matches!(self, Self::On)
    }
}

const fn cont_true() -> SyncFlag {
    SyncFlag::On
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LocalInfo {
    mod_time: i64,
    scm: i64,
    cards: u64,
}

fn conflict(reason: &str) -> SyncOutcome {
    SyncOutcome::Conflict {
        reason: reason.to_owned(),
    }
}

fn is_retryable_result(
    result: &Result<SyncResponse>,
    method: SyncMethod,
) -> bool {
    match result {
        Ok(response) => {
            is_server_error(response.status) && method != SyncMethod::Upload
        }
        Err(error) => error.is_retryable() && method != SyncMethod::Upload,
    }
}

fn ensure_success(response: &SyncResponse, method: SyncMethod) -> Result<()> {
    if (200..300).contains(&response.status) {
        Ok(())
    } else {
        Err(map_status(response.status, &response.body, method))
    }
}

fn prepare_transaction(transaction: &Transaction<'_>) -> Result<()> {
    if table_exists(transaction, "graves")? {
        transaction.execute("DELETE FROM graves", [])?;
    }
    for table in ["notes", "cards", "revlog"] {
        if table_exists(transaction, table)? {
            let statement =
                format!("UPDATE {table} SET usn = 0 WHERE usn = -1");
            transaction.execute(&statement, [])?;
        }
    }
    let now = now_ms()?;
    transaction.execute(
        "UPDATE col SET usn = usn + 1, scm = ?1, ls = ?1 WHERE id = 1",
        params![now],
    )?;
    Ok(())
}

fn read_local_info(path: &Path) -> Result<LocalInfo> {
    if !path.exists() {
        return Ok(LocalInfo::default());
    }
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    register_collations(&connection)?;
    local_info(&connection)
}

fn register_collations(connection: &Connection) -> Result<()> {
    connection.create_collation("unicase", |left, right| {
        left.to_lowercase().cmp(&right.to_lowercase())
    })?;
    Ok(())
}

fn local_info(connection: &Connection) -> Result<LocalInfo> {
    let (mod_time, scm): (i64, i64) = connection.query_row(
        "SELECT mod, scm FROM col WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let cards = if table_exists(connection, "cards")? {
        let count: i64 =
            connection.query_row("SELECT count(*) FROM cards", [], |row| {
                row.get(0)
            })?;
        u64::try_from(count).unwrap_or_default()
    } else {
        0
    };
    Ok(LocalInfo {
        mod_time,
        scm,
        cards,
    })
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool> {
    let found = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()?;
    Ok(found.is_some())
}

fn validate_download(path: &Path) -> Result<LocalInfo> {
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    register_collations(&connection)?;
    let check: String =
        connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if !check.eq_ignore_ascii_case("ok") {
        return Err(SyncError::Database(
            "downloaded collection failed quick_check".to_owned(),
        ));
    }
    let info = local_info(&connection)?;
    connection.execute("UPDATE col SET ls = mod WHERE id = 1", [])?;
    connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        row.get::<_, i64>(0)
    })?;
    connection
        .close()
        .map_err(|(_, error)| SyncError::Database(error.to_string()))?;
    Ok(info)
}

fn map_status(status: u16, body: &[u8], method: SyncMethod) -> SyncError {
    let message = server_message(body);
    match status {
        401 | 403 => SyncError::Auth,
        400 => SyncError::DatabaseCheck(message),
        409 => SyncError::Conflict(message),
        501 => SyncError::ClientTooOld,
        500..=599 => SyncError::Server { status, message },
        _ => {
            if method == SyncMethod::Upload && message.is_empty() {
                SyncError::ServerMessage("upload was not accepted".to_owned())
            } else {
                SyncError::ServerMessage(message)
            }
        }
    }
}

fn server_message(body: &[u8]) -> String {
    String::from_utf8_lossy(body)
        .trim()
        .chars()
        .take(256)
        .collect()
}

fn is_server_error(status: u16) -> bool {
    (500..600).contains(&status)
}

fn unix_duration() -> Result<Duration> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| SyncError::State(error.to_string()))
}

fn now_secs() -> Result<i64> {
    let seconds = unix_duration()?.as_secs();
    i64::try_from(seconds)
        .map_err(|_| SyncError::State("clock is out of range".to_owned()))
}

fn now_ms() -> Result<i64> {
    let millis = unix_duration()?.as_millis();
    i64::try_from(millis)
        .map_err(|_| SyncError::State("clock is out of range".to_owned()))
}
