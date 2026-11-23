#![expect(
    clippy::redundant_pub_crate,
    reason = "helpers are shared only with the crate root"
)]
//! `AnkiWeb` sync wiring for the command line binary.
//!
//! Credentials come from the environment or a private host-key file. They are
//! never stored in config, printed, or included in a `Debug` implementation.

use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use ranki_model::Config;
use ranki_model::Settings;
use ranki_sync::Credentials;
use ranki_sync::HttpTransport;
use ranki_sync::RetryPolicy;
use ranki_sync::SyncClient;
use ranki_sync::SyncError;
use ranki_sync::SyncOutcome;
use ranki_sync::SyncResponse;
use ranki_sync::SyncState;
use ranki_sync::SyncTransport;
use ranki_sync::TransportConfig;

const PRIVATE_BITS: u32 = 0o077;
const STATE_MODE: u32 = 0o600;

/// Credentials loaded for one process. Passwords are only held for login.
pub(super) struct SyncAuth {
    host_key: Option<String>,
    login: Option<Credentials>,
}

/// Environment values used by [`load_auth`], kept separate for testing.
pub(super) struct SyncEnv {
    pub username: Option<String>,
    pub password: Option<String>,
    pub hkey: Option<String>,
}

/// Builds an HTTP transport with the configured retry settings.
#[must_use]
pub(super) fn http_transport(config: &Config) -> HttpTransport {
    let transport = TransportConfig {
        retry_policy: RetryPolicy {
            retries: config.sync.retries,
            backoff: Duration::from_millis(config.sync.backoff_ms),
        },
        ..TransportConfig::default()
    };
    HttpTransport::with_config(ranki_sync::DEFAULT_ENDPOINT, transport)
}

/// Reads the credential environment variables without ever echoing them.
#[must_use]
pub(super) fn read_env() -> SyncEnv {
    SyncEnv {
        username: env::var("RANKI_SYNC_USERNAME").ok().and_then(non_empty),
        password: env::var("RANKI_SYNC_PASSWORD").ok().and_then(non_empty),
        hkey: env::var("RANKI_SYNC_HKEY").ok().and_then(non_empty),
    }
}

/// Whether a previous login left a host key in the local sync state.
#[must_use]
pub(super) fn has_stored_key(settings: &Settings) -> bool {
    fs::read_to_string(state_path(&settings.collection))
        .ok()
        .and_then(|text| serde_json::from_str::<SyncState>(&text).ok())
        .is_some_and(|state| !state.host_key.is_empty())
}

/// Chooses a stored host key or a one-time username and password login.
#[must_use]
pub(super) fn load_auth(
    settings: &Settings,
    env: &SyncEnv,
) -> Option<SyncAuth> {
    let host_key = env.hkey.clone().or_else(|| {
        settings.sync.hkey_file.as_deref().and_then(read_hkey_file)
    });
    let username = env
        .username
        .clone()
        .or_else(|| settings.sync.username.clone())
        .and_then(non_empty);
    let login = match (username, env.password.clone().and_then(non_empty)) {
        (Some(name), Some(password)) => Some(Credentials::new(name, password)),
        _ => None,
    };
    if host_key.is_none() && login.is_none() {
        return None;
    }
    Some(SyncAuth { host_key, login })
}

/// Path of the sync state file beside a collection.
#[must_use]
pub(super) fn state_path(collection: &Path) -> PathBuf {
    let parent = collection.parent().unwrap_or_else(|| Path::new("."));
    parent.join("ranki-sync.json")
}

/// Adapts an injected transport to the generic sync client.
pub(super) struct TransportRef<'a>(pub &'a mut dyn SyncTransport);

impl SyncTransport for TransportRef<'_> {
    fn host_key(&mut self, body: &[u8]) -> ranki_sync::Result<SyncResponse> {
        self.0.host_key(body)
    }

    fn meta(&mut self, body: &[u8]) -> ranki_sync::Result<SyncResponse> {
        self.0.meta(body)
    }

    fn upload(&mut self, body: &[u8]) -> ranki_sync::Result<SyncResponse> {
        self.0.upload(body)
    }

    fn download(&mut self, body: &[u8]) -> ranki_sync::Result<SyncResponse> {
        self.0.download(body)
    }

    fn set_endpoint(&mut self, endpoint: &str) -> ranki_sync::Result<()> {
        self.0.set_endpoint(endpoint)
    }

    fn set_host_key(&mut self, key: &str) {
        self.0.set_host_key(key);
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.0.retry_policy()
    }
}

/// What the explicit `ranki sync` command should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SyncMode {
    Safe,
    Status,
    FullUpload,
    FullDownload,
}

/// Runs one automatic sync. A missing credential is a silent no-op in the
/// sync client, which leaves any existing state-file host key usable.
///
/// # Errors
/// Returns sync errors for login, transport, or local file failures.
pub(super) fn sync_once<T: SyncTransport>(
    transport: T,
    settings: &Settings,
    auth: Option<&SyncAuth>,
) -> Result<SyncOutcome, SyncError> {
    let mut client = build_client(transport, settings, auth)?;
    client.sync(auth.and_then(|credentials| credentials.login.clone()))
}

/// Performs the explicit `ranki sync` operation and returns its display text.
///
/// # Errors
/// Returns sync errors for failed requests or local file operations.
#[expect(
    clippy::too_many_arguments,
    reason = "transport, settings, credentials and mode are distinct inputs"
)]
pub(super) fn manual_sync<T: SyncTransport>(
    transport: T,
    settings: &Settings,
    auth: Option<&SyncAuth>,
    mode: SyncMode,
) -> Result<String, SyncError> {
    let mut client = build_client(transport, settings, auth)?;
    match mode {
        SyncMode::Status => {
            let info = client.status()?;
            Ok(format!(
                "remote mod={} scm={} usn={} empty={}",
                info.mod_time, info.schema_mod_time, info.usn, info.empty
            ))
        }
        SyncMode::FullUpload => {
            client.full_upload()?;
            Ok(String::from("uploaded"))
        }
        SyncMode::FullDownload => {
            client.full_download()?;
            Ok(String::from("downloaded"))
        }
        SyncMode::Safe => Ok(outcome_text(
            &client
                .sync(auth.and_then(|credentials| credentials.login.clone()))?,
        )),
    }
}

fn build_client<T: SyncTransport>(
    transport: T,
    settings: &Settings,
    auth: Option<&SyncAuth>,
) -> Result<SyncClient<T>, SyncError> {
    let state_path = state_path(&settings.collection);
    if let Some(key) =
        auth.and_then(|credentials| credentials.host_key.as_deref())
    {
        store_host_key(&state_path, key)
            .map_err(|error| SyncError::State(error.to_string()))?;
    }
    SyncClient::new(
        transport,
        &settings.collection,
        &settings.media_dir,
        &state_path,
    )
}

/// Prints a warning only for outcomes that need user attention.
pub(super) fn warn_outcome(outcome: &SyncOutcome) {
    match outcome {
        SyncOutcome::Conflict { reason } => {
            eprintln!("ranki: sync: conflict: {reason}");
        }
        SyncOutcome::RetryLater { reason } => {
            eprintln!("ranki: sync: retry later: {reason}");
        }
        SyncOutcome::Disabled
        | SyncOutcome::NoChanges
        | SyncOutcome::Uploaded
        | SyncOutcome::Downloaded => {}
    }
}

/// Human-readable result of an automatic sync.
#[must_use]
pub(super) fn outcome_text(outcome: &SyncOutcome) -> String {
    match outcome {
        SyncOutcome::Disabled => String::from("sync disabled"),
        SyncOutcome::NoChanges => String::from("no changes"),
        SyncOutcome::Uploaded => String::from("uploaded"),
        SyncOutcome::Downloaded => String::from("downloaded"),
        SyncOutcome::Conflict { reason } => format!("conflict: {reason}"),
        SyncOutcome::RetryLater { reason } => format!("retry later: {reason}"),
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn read_hkey_file(path: &Path) -> Option<String> {
    let mode = fs::metadata(path).ok()?.permissions().mode();
    if mode & PRIVATE_BITS != 0 {
        eprintln!(
            "ranki: sync: ignoring {}; host-key files must be mode 0600",
            path.display()
        );
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    non_empty(text.trim().to_owned())
}

fn store_host_key(path: &Path, key: &str) -> io::Result<()> {
    let mut state = if path.exists() {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<SyncState>(&text).ok())
            .unwrap_or_default()
    } else {
        SyncState::default()
    };
    key.clone_into(&mut state.host_key);
    let text = serde_json::to_string(&state).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(STATE_MODE)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use ranki_model::NewNote;
    use ranki_sync::MockResponse;
    use ranki_sync::MockTransport;
    use ranki_sync::SyncMethod;

    use super::SyncAuth;
    use super::SyncEnv;
    use super::SyncMode;
    use super::TransportRef;
    use super::load_auth;
    use super::manual_sync;
    use super::outcome_text;
    use super::state_path;
    use super::sync_once;
    use ranki_model::Store;

    fn settings(path: &Path) -> ranki_model::Settings {
        ranki_model::Settings {
            collection: path.join("collection.anki2"),
            media_dir: path.join("collection.media"),
            sync: ranki_model::SyncSettings {
                enabled: true,
                ..ranki_model::SyncSettings::default()
            },
            ..ranki_model::Settings::default()
        }
    }

    fn now_secs() -> i64 {
        i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        )
        .unwrap_or(0)
    }

    fn meta(empty: bool) -> MockResponse {
        MockResponse::ok_json(&serde_json::json!({
            "mod": 1_700_000_000_000_i64,
            "scm": 1_700_000_000_000_i64,
            "usn": 7,
            "ts": now_secs(),
            "empty": empty,
        }))
        .unwrap()
    }

    fn login() -> MockResponse {
        MockResponse::ok_json(&serde_json::json!({"key": "cached-hkey"}))
            .unwrap()
    }

    fn collection_with_cards(path: &Path) {
        let mut collection =
            ranki_storage::Collection::create(&path.join("collection.anki2"))
                .unwrap();
        collection
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![String::from("front"), String::from("back")],
                tags: Vec::new(),
            })
            .unwrap();
        collection.checkpoint().unwrap();
        collection.close().unwrap();
    }

    fn host_key_auth() -> SyncAuth {
        SyncAuth {
            host_key: Some(String::from("stored-hkey")),
            login: None,
        }
    }

    fn seed_state(settings: &ranki_model::Settings) {
        let state = serde_json::json!({
            "version": 1,
            "endpoint": ranki_sync::DEFAULT_ENDPOINT,
            "host_key": "stored-hkey",
            "last_mod": null,
            "last_scm": 1_700_000_000_000_i64,
            "last_usn": 0,
            "last_media_usn": 0,
            "last_success": null,
            "failures": 0,
        });
        fs::write(state_path(&settings.collection), state.to_string()).unwrap();
    }

    #[test]
    fn env_credentials_are_loaded() {
        let settings = settings(Path::new("/tmp/ranki-env"));
        let env = SyncEnv {
            username: Some(String::from("alice")),
            password: Some(String::from("secret")),
            hkey: Some(String::from("env-hkey")),
        };
        let auth = load_auth(&settings, &env).unwrap();
        assert_eq!(auth.host_key.as_deref(), Some("env-hkey"));
        assert!(auth.login.is_some());
    }

    #[test]
    fn username_falls_back_to_config() {
        let mut settings = settings(Path::new("/tmp/ranki-env"));
        settings.sync.username = Some(String::from("config-user"));
        let env = SyncEnv {
            username: None,
            password: Some(String::from("secret")),
            hkey: None,
        };
        assert!(load_auth(&settings, &env).unwrap().login.is_some());
    }

    #[test]
    fn no_credentials_are_skipped() {
        let template = settings(Path::new("/tmp/ranki-env"));
        let env = SyncEnv {
            username: None,
            password: None,
            hkey: None,
        };
        assert!(load_auth(&template, &env).is_none());
        let auth = SyncAuth {
            host_key: None,
            login: None,
        };
        let directory = tempfile::tempdir().unwrap();
        let settings = settings(directory.path());
        let mut transport = MockTransport::new();
        assert_eq!(
            sync_once(TransportRef(&mut transport), &settings, Some(&auth))
                .unwrap(),
            ranki_sync::SyncOutcome::Disabled
        );
        assert_eq!(transport.request_count(), 0);
    }

    #[test]
    fn host_key_file_must_be_private() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = settings(directory.path());
        let path = directory.path().join("hkey");
        fs::write(&path, "file-hkey\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        settings.sync.hkey_file = Some(path.clone());
        let auth = load_auth(
            &settings,
            &SyncEnv {
                username: None,
                password: None,
                hkey: None,
            },
        )
        .unwrap();
        assert_eq!(auth.host_key.as_deref(), Some("file-hkey"));

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            load_auth(
                &settings,
                &SyncEnv {
                    username: None,
                    password: None,
                    hkey: None,
                },
            )
            .is_none()
        );
    }

    #[test]
    fn mock_login_caches_only_the_host_key() {
        let directory = tempfile::tempdir().unwrap();
        let settings = settings(directory.path());
        let env = SyncEnv {
            username: Some(String::from("alice")),
            password: Some(String::from("secret")),
            hkey: None,
        };
        let auth = load_auth(&settings, &env).unwrap();
        let mut transport = MockTransport::new()
            .with_response(SyncMethod::HostKey, login())
            .with_response(SyncMethod::Meta, meta(true));
        sync_once(TransportRef(&mut transport), &settings, Some(&auth))
            .unwrap();
        assert_eq!(transport.requests()[0].method, SyncMethod::HostKey);
        assert_eq!(transport.requests()[0].host_key, "");
        let state =
            fs::read_to_string(state_path(&settings.collection)).unwrap();
        assert!(state.contains("cached-hkey"));
        assert!(!state.contains("secret"));
    }

    #[test]
    fn mock_upload_sends_the_collection() {
        let directory = tempfile::tempdir().unwrap();
        collection_with_cards(directory.path());
        let settings = settings(directory.path());
        seed_state(&settings);
        let mut transport = MockTransport::new()
            .with_response(SyncMethod::Meta, meta(true))
            .with_response(
                SyncMethod::Upload,
                MockResponse::ok_bytes(b"OK".to_vec()).unwrap(),
            );
        let outcome = sync_once(
            TransportRef(&mut transport),
            &settings,
            Some(&host_key_auth()),
        )
        .unwrap();
        assert_eq!(outcome_text(&outcome), "uploaded");
        let methods: Vec<SyncMethod> = transport
            .requests()
            .iter()
            .map(|request| request.method)
            .collect();
        assert_eq!(methods, [SyncMethod::Meta, SyncMethod::Upload]);
    }

    #[test]
    fn mock_download_replaces_an_empty_collection() {
        let source = tempfile::tempdir().unwrap();
        collection_with_cards(source.path());
        let bytes = fs::read(source.path().join("collection.anki2")).unwrap();

        let target = tempfile::tempdir().unwrap();
        let settings = settings(target.path());
        seed_state(&settings);
        let mut transport = MockTransport::new()
            .with_response(SyncMethod::Meta, meta(false))
            .with_response(
                SyncMethod::Download,
                MockResponse::ok_bytes(bytes).unwrap(),
            );
        let outcome = sync_once(
            TransportRef(&mut transport),
            &settings,
            Some(&host_key_auth()),
        )
        .unwrap();
        assert_eq!(outcome_text(&outcome), "downloaded");
        assert!(settings.collection.exists());
        let methods: Vec<SyncMethod> = transport
            .requests()
            .iter()
            .map(|request| request.method)
            .collect();
        assert_eq!(methods, [SyncMethod::Meta, SyncMethod::Download]);
    }

    #[test]
    fn mock_conflict_never_overwrites_the_local_collection() {
        let directory = tempfile::tempdir().unwrap();
        collection_with_cards(directory.path());
        let settings = settings(directory.path());
        let before = fs::metadata(&settings.collection).unwrap().len();
        let mut transport =
            MockTransport::new().with_response(SyncMethod::Meta, meta(false));
        let outcome = sync_once(
            TransportRef(&mut transport),
            &settings,
            Some(&host_key_auth()),
        )
        .unwrap();
        assert!(matches!(outcome, ranki_sync::SyncOutcome::Conflict { .. }));
        assert_eq!(transport.request_count(), 1);
        assert_eq!(fs::metadata(&settings.collection).unwrap().len(), before);
    }

    #[test]
    fn manual_status_reads_remote_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let settings = settings(directory.path());
        let mut transport =
            MockTransport::new().with_response(SyncMethod::Meta, meta(false));
        let text = manual_sync(
            TransportRef(&mut transport),
            &settings,
            Some(&host_key_auth()),
            SyncMode::Status,
        )
        .unwrap();
        assert!(text.starts_with("remote mod=1700000000000"));
        assert_eq!(transport.request_count(), 1);
    }

    #[test]
    fn manual_full_upload_is_an_explicit_escape_hatch() {
        let directory = tempfile::tempdir().unwrap();
        collection_with_cards(directory.path());
        let settings = settings(directory.path());
        let mut transport = MockTransport::new().with_response(
            SyncMethod::Upload,
            MockResponse::ok_bytes(b"OK".to_vec()).unwrap(),
        );
        let text = manual_sync(
            TransportRef(&mut transport),
            &settings,
            Some(&host_key_auth()),
            SyncMode::FullUpload,
        )
        .unwrap();
        assert_eq!(text, "uploaded");
        assert_eq!(transport.requests()[0].method, SyncMethod::Upload);
    }

    #[test]
    fn state_file_sits_beside_the_collection() {
        let path = PathBuf::from("/data/collection.anki2");
        assert_eq!(state_path(&path), PathBuf::from("/data/ranki-sync.json"));
    }

    #[test]
    fn stored_host_key_enables_automatic_sync() {
        let directory = tempfile::tempdir().unwrap();
        let settings = settings(directory.path());
        assert!(!super::has_stored_key(&settings));
        seed_state(&settings);
        assert!(super::has_stored_key(&settings));
    }
}
