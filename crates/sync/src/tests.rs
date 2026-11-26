use crate::{
    Credentials, MockResponse, MockTransport, ORIGINAL_SIZE_HEADER,
    RemoteStatus, RetryPolicy, SyncClient, SyncError, SyncMethod, SyncOutcome,
    SyncResponse, SyncState, wire,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

type TestResult<T = ()> = crate::Result<T>;
type CollectionValues = (i64, i64, u32);
type MetaValues = (i64, i64, bool, i64);

/// Derives a throwaway login value from the test's random temp directory.
///
/// Nothing is hard-coded: the value comes from the per-run directory name, so
/// it also proves the state file never stores the login.
fn fixture_login(directory: &TempDir) -> String {
    directory
        .path()
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Derives a throwaway session key that is not a substring of [`fixture_login`].
fn fixture_key(directory: &TempDir) -> String {
    fixture_login(directory).chars().rev().collect()
}

fn now_secs() -> TestResult<i64> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| SyncError::State(error.to_string()))?
        .as_secs();
    i64::try_from(seconds)
        .map_err(|_| SyncError::State("clock out of range".to_owned()))
}

fn make_collection(
    path: &Path,
    (mod_time, scm, cards): CollectionValues,
) -> TestResult {
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "CREATE TABLE col (id INTEGER PRIMARY KEY, crt INTEGER, mod INTEGER, scm INTEGER, \
         ver INTEGER, dty INTEGER, usn INTEGER, ls INTEGER, conf TEXT, models TEXT, decks TEXT, \
         dconf TEXT, tags TEXT); \
         CREATE TABLE notes (id INTEGER PRIMARY KEY, guid TEXT, usn INTEGER); \
         CREATE TABLE cards (id INTEGER PRIMARY KEY, nid INTEGER, usn INTEGER); \
         CREATE TABLE revlog (id INTEGER PRIMARY KEY, usn INTEGER); \
         CREATE TABLE graves (oid INTEGER, ty INTEGER, usn INTEGER);",
    )?;
    connection.execute(
        "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags) \
         VALUES (1, 0, ?1, ?2, 11, 0, 0, 0, '{}', '{}', '{}', '{}', '{}')",
        params![mod_time, scm],
    )?;
    for id in 0..i64::from(cards) {
        connection.execute(
            "INSERT INTO notes (id, guid, usn) VALUES (?1, ?2, -1)",
            params![id, format!("guid-{id}")],
        )?;
        connection.execute(
            "INSERT INTO cards (id, nid, usn) VALUES (?1, ?2, -1)",
            params![id, id],
        )?;
    }
    Ok(())
}

fn add_unicase_index(path: &Path) -> TestResult {
    let connection = Connection::open(path)?;
    connection.create_collation("unicase", |left, right| {
        left.to_lowercase().cmp(&right.to_lowercase())
    })?;
    connection.execute_batch(
        "CREATE TABLE decks (name TEXT COLLATE unicase); \
         CREATE INDEX ix_decks_name ON decks (name);",
    )?;
    connection.close().map_err(|(_, error)| error)?;
    Ok(())
}

fn write_state(path: &Path, state: &SyncState) -> TestResult {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec(state)?)?;
    Ok(())
}

fn seeded_state(host_key: &str) -> SyncState {
    SyncState {
        host_key: host_key.to_owned(),
        ..SyncState::default()
    }
}

fn meta_response(
    (mod_time, scm, empty, ts): MetaValues,
) -> crate::Result<MockResponse> {
    MockResponse::ok_json(&json!({
        "mod": mod_time,
        "scm": scm,
        "usn": 7,
        "ts": ts,
        "empty": empty,
        "cont": true,
        "media_usn": 3,
    }))
}

fn decode_request(request: &crate::RecordedRequest) -> TestResult<Vec<u8>> {
    let response = SyncResponse {
        body: request.body.clone(),
        ..SyncResponse::default()
    };
    wire::decode(&response)
}

fn assert_status(
    client: &mut SyncClient<MockTransport>,
) -> TestResult<RemoteStatus> {
    client.status()
}

#[test]
fn login_success_stores_only_host_key() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    let key = fixture_key(&directory);
    let transport = MockTransport::new().with_response(
        SyncMethod::HostKey,
        MockResponse::ok_json(&json!({ "key": &key }))?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    let login = fixture_login(&directory);
    client.login(&Credentials::new(&login, &login))?;

    assert_eq!(client.state().host_key, key);
    let text = fs::read_to_string(&state_path)?;
    assert!(text.contains(&key));
    assert!(!text.contains(&login));
    assert_eq!(client.transport().request_count(), 1);
    Ok(())
}

#[test]
fn login_failure_leaves_no_state() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    let transport = MockTransport::new().with_response(
        SyncMethod::HostKey,
        MockResponse::error(403, "bad login"),
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    assert_eq!(
        client.login(&Credentials::new(
            fixture_login(&directory),
            fixture_login(&directory)
        )),
        Err(SyncError::Auth)
    );
    assert!(!state_path.exists());
    assert_eq!(client.state().host_key, "");
    Ok(())
}

#[test]
fn status_rejects_clock_skew() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let skewed = now_secs()?.saturating_add(10_000);
    let transport = MockTransport::new().with_response(
        SyncMethod::Meta,
        meta_response((100, 100, false, skewed))?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    assert!(matches!(client.status(), Err(SyncError::ClockSkew)));
    Ok(())
}

#[test]
fn full_upload_sends_checkpointed_collection() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 2))?;
    let seeded_key = fixture_key(&directory);
    write_state(&state_path, &seeded_state(&seeded_key))?;
    let transport = MockTransport::new().with_response(
        SyncMethod::Upload,
        MockResponse::ok_bytes(b"OK".to_vec())?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    client.full_upload()?;

    let requests = client.transport().requests();
    assert_eq!(requests.len(), 1);
    let request = requests
        .first()
        .ok_or_else(|| SyncError::State("missing upload request".to_owned()))?;
    assert_eq!(request.method, SyncMethod::Upload);
    assert_eq!(request.host_key, seeded_key);
    let sent = decode_request(request)?;
    assert_eq!(sent, fs::read(&collection)?);
    let connection = Connection::open(&collection)?;
    let usn: i32 = connection.query_row(
        "SELECT usn FROM col WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let pending: i64 = connection.query_row(
        "SELECT count(*) FROM cards WHERE usn = -1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(usn, 1);
    assert_eq!(pending, 0);
    assert!(client.state().last_mod.is_some());
    Ok(())
}

#[test]
fn full_download_replaces_collection_and_keeps_media() -> TestResult {
    let directory = TempDir::new()?;
    let source = directory.path().join("source.anki2");
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&source, (900, 200, 1))?;
    make_collection(&collection, (100, 100, 0))?;
    fs::create_dir_all(&media)?;
    fs::write(media.join("image.png"), b"image")?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let transport = MockTransport::new().with_response(
        SyncMethod::Download,
        MockResponse::ok_bytes(fs::read(&source)?)?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    client.full_download()?;

    let connection = Connection::open(&collection)?;
    let (mod_time, scm, last_sync): (i64, i64, i64) = connection.query_row(
        "SELECT mod, scm, ls FROM col WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let cards: i64 =
        connection
            .query_row("SELECT count(*) FROM cards", [], |row| row.get(0))?;
    drop(connection);
    assert_eq!(mod_time, 900);
    assert_eq!(scm, 200);
    assert_eq!(last_sync, mod_time);
    assert_eq!(cards, 1);
    assert_eq!(fs::read(media.join("image.png"))?, b"image");
    assert!(collection.with_extension("ranki-sync.bak").exists());
    assert_eq!(client.state().last_mod, Some(900));
    Ok(())
}

#[test]
fn sync_refuses_conflict_without_writing() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (200, 100, 1))?;
    let before = fs::read(&collection)?;
    let state = SyncState {
        host_key: fixture_key(&directory),
        last_mod: Some(100),
        last_scm: Some(100),
        ..SyncState::default()
    };
    write_state(&state_path, &state)?;
    let transport = MockTransport::new().with_response(
        SyncMethod::Meta,
        meta_response((300, 100, false, now_secs()?))?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    let outcome = client.sync(None)?;

    assert!(matches!(outcome, SyncOutcome::Conflict { .. }));
    let requests = client.transport().requests();
    assert_eq!(requests.len(), 1);
    let request = requests
        .first()
        .ok_or_else(|| SyncError::State("missing meta request".to_owned()))?;
    assert_eq!(request.method, SyncMethod::Meta);
    assert_eq!(fs::read(&collection)?, before);
    Ok(())
}

#[test]
fn meta_request_uses_an_supported_client_version() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let transport = MockTransport::new().with_response(
        SyncMethod::Meta,
        meta_response((100, 100, false, now_secs()?))?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    client.status()?;

    let requests = client.transport().requests();
    let request = requests
        .first()
        .ok_or_else(|| SyncError::State("missing meta request".to_owned()))?;
    let body = decode_request(request)?;
    let meta: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(meta["v"], wire::SYNC_VERSION);
    assert_eq!(meta["cv"], crate::META_CLIENT_VERSION);
    let version = meta["cv"]
        .as_str()
        .ok_or_else(|| SyncError::State("missing client version".to_owned()))?;
    assert!(version.starts_with("anki,"));
    assert!(version.split(',').count() >= 3);
    Ok(())
}

#[test]
fn local_cards_with_an_empty_remote_need_a_choice() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 2))?;
    let before = fs::read(&collection)?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let transport = MockTransport::new().with_response(
        SyncMethod::Meta,
        meta_response((100, 100, true, now_secs()?))?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    let outcome = client.sync(None)?;

    assert!(matches!(outcome, SyncOutcome::Conflict { .. }));
    assert_eq!(client.transport().requests().len(), 1);
    assert_eq!(fs::read(&collection)?, before);
    assert_eq!(client.state().last_mod, None);
    Ok(())
}

#[test]
fn local_changes_upload_when_remote_is_unchanged() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (200, 100, 1))?;
    write_state(
        &state_path,
        &SyncState {
            host_key: fixture_key(&directory),
            last_mod: Some(100),
            last_scm: Some(100),
            ..SyncState::default()
        },
    )?;
    let transport = MockTransport::new()
        .with_response(
            SyncMethod::Meta,
            meta_response((100, 100, false, now_secs()?))?,
        )
        .with_response(
            SyncMethod::Upload,
            MockResponse::ok_bytes(b"OK".to_vec())?,
        );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    let outcome = client.sync(None)?;

    assert_eq!(outcome, SyncOutcome::Uploaded);
    let requests = client.transport().requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, SyncMethod::Meta);
    assert_eq!(requests[1].method, SyncMethod::Upload);
    assert_eq!(client.state().last_mod, Some(200));
    assert!(client.state().last_scm.is_some_and(|scm| scm > 100));
    Ok(())
}

#[test]
fn two_empty_collections_need_no_choice() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 0))?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let transport = MockTransport::new()
        .with_response(
            SyncMethod::Meta,
            meta_response((200, 200, true, now_secs()?))?,
        )
        .with_response(
            SyncMethod::Meta,
            meta_response((200, 200, true, now_secs()?))?,
        );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    assert_eq!(client.sync(None)?, SyncOutcome::NoChanges);
    assert_eq!(client.sync(None)?, SyncOutcome::NoChanges);
    assert_eq!(client.transport().requests().len(), 2);
    assert_eq!(client.state().last_mod, None);
    assert_eq!(client.state().last_scm, None);
    Ok(())
}

#[test]
fn local_info_reads_schema18_collations() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    add_unicase_index(&collection)?;
    write_state(
        &state_path,
        &SyncState {
            host_key: fixture_key(&directory),
            last_mod: Some(100),
            last_scm: Some(100),
            ..SyncState::default()
        },
    )?;
    let transport = MockTransport::new().with_response(
        SyncMethod::Meta,
        meta_response((100, 100, false, now_secs()?))?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    assert_eq!(client.sync(None)?, SyncOutcome::NoChanges);
    Ok(())
}

#[test]
fn empty_local_collection_downloads_a_populated_remote() -> TestResult {
    let directory = TempDir::new()?;
    let source = directory.path().join("source.anki2");
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&source, (900, 500, 1))?;
    make_collection(&collection, (100, 100, 0))?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let transport = MockTransport::new()
        .with_response(
            SyncMethod::Meta,
            meta_response((900, 500, false, now_secs()?))?,
        )
        .with_response(
            SyncMethod::Download,
            MockResponse::ok_bytes(fs::read(&source)?)?,
        );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    let outcome = client.sync(None)?;

    assert_eq!(outcome, SyncOutcome::Downloaded);
    let connection = Connection::open(&collection)?;
    let cards: i64 =
        connection
            .query_row("SELECT count(*) FROM cards", [], |row| row.get(0))?;
    assert_eq!(cards, 1);
    Ok(())
}

#[test]
fn offline_meta_failure_is_retried() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let transport = MockTransport::new()
        .with_failure(SyncMethod::Meta, SyncError::Timeout)
        .with_response(
            SyncMethod::Meta,
            meta_response((100, 100, false, now_secs()?))?,
        )
        .with_retry_policy(RetryPolicy {
            retries: 2,
            backoff: Duration::ZERO,
        });
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    let status = assert_status(&mut client)?;

    assert_eq!(status.mod_time, 100);
    assert_eq!(client.transport().request_count(), 2);
    Ok(())
}

#[test]
fn no_credentials_disables_sync_without_requests() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    let mut client = SyncClient::new(
        MockTransport::new(),
        &collection,
        &media,
        &state_path,
    )?;

    assert_eq!(client.sync(None)?, SyncOutcome::Disabled);
    assert_eq!(client.transport().request_count(), 0);
    Ok(())
}

#[test]
fn login_writes_state_with_mode_0600() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    let key = fixture_key(&directory);
    let transport = MockTransport::new().with_response(
        SyncMethod::HostKey,
        MockResponse::ok_json(&json!({ "key": &key }))?,
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    client.login(&Credentials::new(
        fixture_login(&directory),
        fixture_login(&directory),
    ))?;

    let metadata = fs::metadata(&state_path)?;
    assert_eq!(metadata.mode() & 0o777, 0o600);
    Ok(())
}

#[test]
fn zstd_frame_roundtrips_with_size_header() -> TestResult {
    let payload = b"zstd payload".to_vec();
    let response = SyncResponse {
        status: 200,
        headers: BTreeMap::from([(
            ORIGINAL_SIZE_HEADER.to_owned(),
            payload.len().to_string(),
        )]),
        body: wire::frame(&payload)?,
    };

    assert_eq!(wire::decode(&response)?, payload);
    Ok(())
}

#[test]
fn transport_failure_is_propagated() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let transport = MockTransport::new().with_failure(
        SyncMethod::Meta,
        SyncError::Network("offline".to_owned()),
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    assert!(matches!(client.status(), Err(SyncError::Network(_))));
    assert_eq!(client.transport().request_count(), 1);
    Ok(())
}

#[test]
fn corrupt_download_keeps_old_collection() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    let before = fs::read(&collection)?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let mut headers = BTreeMap::new();
    headers.insert(ORIGINAL_SIZE_HEADER.to_owned(), "4".to_owned());
    let transport = MockTransport::new().with_response(
        SyncMethod::Download,
        MockResponse::raw(200, b"nope".to_vec(), headers),
    );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    assert!(matches!(client.full_download(), Err(SyncError::Decode(_))));
    assert_eq!(fs::read(&collection)?, before);
    Ok(())
}

#[test]
fn retryable_sync_failure_returns_retry_later() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let transport = MockTransport::new()
        .with_failure(SyncMethod::Meta, SyncError::Timeout)
        .with_retry_policy(RetryPolicy {
            retries: 0,
            backoff: Duration::ZERO,
        });
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    assert!(matches!(client.sync(None)?, SyncOutcome::RetryLater { .. }));
    assert_eq!(client.state().failures, 1);
    Ok(())
}

#[test]
fn redirect_updates_endpoint() -> TestResult {
    let directory = TempDir::new()?;
    let collection = directory.path().join("collection.anki2");
    let media = directory.path().join("media");
    let state_path = directory.path().join("sync.json");
    make_collection(&collection, (100, 100, 1))?;
    write_state(&state_path, &seeded_state(&fixture_key(&directory)))?;
    let mut headers = BTreeMap::new();
    headers.insert(
        "location".to_owned(),
        "https://sync.ankiweb.net/".to_owned(),
    );
    let transport = MockTransport::new()
        .with_response(
            SyncMethod::Meta,
            MockResponse::raw(308, Vec::new(), headers),
        )
        .with_response(
            SyncMethod::Meta,
            meta_response((100, 100, false, now_secs()?))?,
        );
    let mut client =
        SyncClient::new(transport, &collection, &media, &state_path)?;

    client.status()?;

    assert_eq!(client.state().endpoint, "https://sync.ankiweb.net/");
    let requests = client.transport().requests();
    assert_eq!(requests.len(), 2);
    let second = requests.get(1).ok_or_else(|| {
        SyncError::State("missing redirected request".to_owned())
    })?;
    assert_eq!(second.endpoint, "https://sync.ankiweb.net/");
    Ok(())
}
