//! Workloads that cover the CLI, both servers, the store, scheduling, packages,
//! and the GUI's headless frame path.

#![expect(
    clippy::excessive_nesting,
    reason = "sampling loops and protocol polling stay in one place"
)]
#![expect(
    clippy::too_many_arguments,
    reason = "a workload definition keeps its sampling settings together"
)]
#![expect(
    clippy::too_many_lines,
    reason = "all surfaces are registered in one visible list"
)]

#[cfg(feature = "gui")]
use std::cell::RefCell;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Error as IoError, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{
    AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering,
};
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use ranki_core::FsrsPreset;
use ranki_core::FsrsScheduler;
use ranki_format::export_apkg;
use ranki_format::import_apkg;
use ranki_model::Card;
use ranki_model::DEFAULT_DECK_ID;
use ranki_model::DEFAULT_MODEL_ID;
use ranki_model::NewNote;
use ranki_model::Rating;
#[cfg(feature = "gui")]
use ranki_model::Settings;
use ranki_model::Store;
use ranki_storage::Collection;
#[cfg(feature = "gui")]
use ranki_ui::BenchApp;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;
use thiserror::Error;

pub const METHOD: u32 = 1;
const NOTE_COUNT: usize = 2_000;
const REVIEW_COUNT: usize = 50;

#[derive(Debug, Error)]
pub enum BenchError {
    #[error("io: {0}")]
    Io(#[from] IoError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("store: {0}")]
    Store(#[from] ranki_model::StoreError),
    #[error("format: {0}")]
    Format(#[from] ranki_format::FormatError),
    #[error("{0}")]
    Message(String),
}

type BenchResult<T> = Result<T, BenchError>;

fn message(value: impl Into<String>) -> BenchError {
    BenchError::Message(value.into())
}

#[derive(Debug, Serialize)]
pub struct Workload {
    pub name: String,
    pub criterion: String,
    pub budget_ms: f64,
    pub samples: usize,
    pub repeat: usize,
    pub warmup: usize,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub max_gap_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub method: u32,
    pub runtime: String,
    pub os: String,
    pub cpu: String,
    pub results: Vec<Workload>,
    pub workload_hash: String,
}

struct Monitor {
    stop: Arc<AtomicBool>,
    maximum: Arc<AtomicU64>,
    handle: JoinHandle<()>,
}

impl Monitor {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let maximum = Arc::new(AtomicU64::new(0));
        let thread_stop = Arc::clone(&stop);
        let thread_maximum = Arc::clone(&maximum);
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let start = Instant::now();
                thread::sleep(Duration::from_millis(1));
                let micros = u64::try_from(start.elapsed().as_micros())
                    .unwrap_or(u64::MAX);
                thread_maximum.fetch_max(micros, Ordering::Relaxed);
            }
        });
        Self {
            stop,
            maximum,
            handle,
        }
    }

    fn finish(self) -> f64 {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
        let micros = self
            .maximum
            .load(Ordering::Relaxed)
            .min(u64::from(u32::MAX));
        f64::from(u32::try_from(micros).unwrap_or(u32::MAX)) / 1_000.0
    }
}

#[derive(Default)]
struct Recorder {
    results: Vec<Workload>,
}

impl Recorder {
    fn measure<F>(
        &mut self,
        name: &str,
        criterion: &str,
        budget_ms: f64,
        samples: usize,
        repeat: usize,
        warmup: usize,
        run: F,
    ) -> BenchResult<()>
    where
        F: FnMut() -> BenchResult<()>,
    {
        self.measure_with_setup(
            name,
            criterion,
            budget_ms,
            samples,
            repeat,
            warmup,
            || Ok(()),
            run,
        )
    }

    fn measure_with_setup<F, S>(
        &mut self,
        name: &str,
        criterion: &str,
        budget_ms: f64,
        samples: usize,
        repeat: usize,
        warmup: usize,
        mut setup: S,
        mut run: F,
    ) -> BenchResult<()>
    where
        S: FnMut() -> BenchResult<()>,
        F: FnMut() -> BenchResult<()>,
    {
        if env::var_os("RANKI_BENCH_VERBOSE").is_some() {
            eprintln!("benchmarking {name}");
        }
        for _ in 0..warmup {
            setup()
                .map_err(|error| message(format!("{name} setup: {error}")))?;
            run().map_err(|error| message(format!("{name}: {error}")))?;
        }
        let monitor = Monitor::start();
        let mut times = Vec::with_capacity(samples.saturating_mul(repeat));
        for _ in 0..samples {
            for _ in 0..repeat {
                setup().map_err(|error| {
                    message(format!("{name} setup: {error}"))
                })?;
                let start = Instant::now();
                run().map_err(|error| message(format!("{name}: {error}")))?;
                times.push(start.elapsed().as_secs_f64() * 1_000.0);
            }
            thread::sleep(Duration::from_millis(2));
        }
        let max_gap_ms = monitor.finish();
        times.sort_by(f64::total_cmp);
        self.results.push(Workload {
            name: name.to_owned(),
            criterion: criterion.to_owned(),
            budget_ms,
            samples,
            repeat,
            warmup,
            median_ms: rank(&times, 1, 2),
            p95_ms: rank(&times, 95, 100),
            max_gap_ms,
        });
        Ok(())
    }
}

fn rank(times: &[f64], numerator: usize, denominator: usize) -> f64 {
    let length = times.len();
    if length == 0 {
        return 0.0;
    }
    let index = length
        .saturating_mul(numerator)
        .div_ceil(denominator)
        .saturating_sub(1)
        .min(length.saturating_sub(1));
    times.get(index).copied().unwrap_or(0.0)
}

struct Fixture {
    directory: TempDir,
    collection: PathBuf,
    media: PathBuf,
    package: PathBuf,
    created: i64,
    now: i64,
    cards: Vec<Card>,
    counter: AtomicUsize,
}

impl Fixture {
    fn new() -> BenchResult<Self> {
        let directory = tempfile::tempdir()?;
        let collection = directory.path().join("collection.anki2");
        let media = directory.path().join("collection.media");
        fs::create_dir_all(&media)?;
        let mut store = Collection::create(&collection)?;
        let created = store.collection_created()?;
        let mut cards = Vec::with_capacity(NOTE_COUNT);
        for index in 0..NOTE_COUNT {
            let ids = store.add_note(&NewNote {
                model_id: DEFAULT_MODEL_ID,
                deck_id: DEFAULT_DECK_ID,
                fields: vec![
                    format!("Common front {index} unique{index}"),
                    format!("Common back {index}"),
                ],
                tags: vec!["performance".to_owned()],
            })?;
            let id = *ids
                .first()
                .ok_or_else(|| message("add_note produced no card"))?;
            let card = store
                .card(id)?
                .ok_or_else(|| message("add_note card is missing"))?;
            cards.push(card);
        }
        drop(store);
        let package = directory.path().join("seed.apkg");
        export_apkg(&collection, &media, &package)?;
        let now = created.saturating_add(86_400_i64.saturating_mul(2));
        Ok(Self {
            directory,
            collection,
            media,
            package,
            created,
            now,
            cards,
            counter: AtomicUsize::new(0),
        })
    }

    fn unique_path(&self, prefix: &str, suffix: &str) -> PathBuf {
        let index = self.counter.fetch_add(1, Ordering::Relaxed);
        let name = if suffix.is_empty() {
            format!("{prefix}-{index}")
        } else {
            format!("{prefix}-{index}.{suffix}")
        };
        self.directory.path().join(name)
    }

    #[cfg(feature = "gui")]
    fn options(&self) -> Settings {
        Settings {
            paths: ranki_model::DataPaths::new(self.directory.path()),
            ..Settings::default()
        }
    }

    #[cfg(feature = "gui")]
    fn app(&self) -> BenchResult<BenchApp> {
        let store = Box::new(Collection::open(&self.collection)?);
        Ok(BenchApp::new(store, &self.options()))
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> BenchResult<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn rpc(port: u16, body: &str) -> BenchResult<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    stream.flush()?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    if !response.contains("\"result\":") || response.contains("\"error\":\"") {
        return Err(message(format!(
            "unexpected AnkiConnect response: {response}"
        )));
    }
    Ok(response)
}

struct ConnectServer {
    _child: ChildGuard,
    port: u16,
}

impl ConnectServer {
    fn start(binary: &Path, data_dir: &Path) -> BenchResult<Self> {
        let port = free_port()?;
        let child = Command::new(binary)
            .env("RANKI_DATA_DIR", data_dir)
            .args(["connect", "--port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let server = Self {
            _child: ChildGuard(child),
            port,
        };
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            if Instant::now() >= deadline {
                return Err(message("AnkiConnect server did not start"));
            }
            thread::sleep(Duration::from_millis(10));
        }
        let response = rpc(port, "{\"action\":\"version\",\"version\":6}")?;
        if !response.contains("\"result\":6") {
            return Err(message("AnkiConnect version handshake failed"));
        }
        Ok(server)
    }

    fn find_cards(&self) -> BenchResult<()> {
        let body = "{\"action\":\"findCards\",\"version\":6,\"params\":{\"query\":\"Common\"}}";
        let response = rpc(self.port, body)?;
        if !response.contains("\"result\":[") {
            return Err(message("AnkiConnect search returned no cards"));
        }
        Ok(())
    }
}

struct McpSession {
    _child: ChildGuard,
    stdin: ChildStdin,
    lines: Receiver<BenchResult<String>>,
    next_id: u64,
}

impl McpSession {
    fn start(binary: &Path, data_dir: &Path) -> BenchResult<Self> {
        let spawned = Command::new(binary)
            .env("RANKI_DATA_DIR", data_dir)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut child = ChildGuard(spawned);
        let stdin = child
            .0
            .stdin
            .take()
            .ok_or_else(|| message("MCP stdin is not piped"))?;
        let stdout = child
            .0
            .stdout
            .take()
            .ok_or_else(|| message("MCP stdout is not piped"))?;
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {
                        if sender.send(Ok(line)).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        let mut session = Self {
            _child: child,
            stdin,
            lines,
            next_id: 0,
        };
        let initialize = json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "ranki-bench", "version": "1.0.0"}
            }
        });
        let response = session.request(initialize)?;
        if response.get("result").is_none() {
            return Err(message(format!("MCP initialize failed: {response}")));
        }
        session.notify(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )?;
        Ok(session)
    }

    fn notify(&mut self, value: &Value) -> BenchResult<()> {
        writeln!(self.stdin, "{}", serde_json::to_string(value)?)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn request(&mut self, value: Value) -> BenchResult<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut request_value = value;
        if let Some(object) = request_value.as_object_mut() {
            object.insert("id".to_owned(), json!(id));
        }
        writeln!(self.stdin, "{}", serde_json::to_string(&request_value)?)?;
        self.stdin.flush()?;
        for _ in 0..100 {
            let line =
                self.lines.recv_timeout(Duration::from_secs(5)).map_err(
                    |_| message("MCP server did not answer within 5 seconds"),
                )??;
            let Ok(response) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if response.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(response);
            }
        }
        Err(message("MCP server did not answer the request"))
    }

    fn list_tools(&mut self) -> BenchResult<()> {
        let response =
            self.request(json!({"jsonrpc": "2.0", "method": "tools/list"}))?;
        let tools = response
            .get("result")
            .and_then(|result| result.get("tools"))
            .and_then(Value::as_array)
            .ok_or_else(|| message("MCP tools/list returned no tools"))?;
        if tools.len() < 10 {
            return Err(message("MCP tool list is incomplete"));
        }
        Ok(())
    }

    fn list_decks(&mut self) -> BenchResult<()> {
        let response = self.request(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {"name": "list_decks", "arguments": {}}
        }))?;
        let text = response
            .get("result")
            .map(Value::to_string)
            .unwrap_or_default();
        if !text.contains("Default") {
            return Err(message("MCP list_decks returned no decks"));
        }
        Ok(())
    }
}

fn first_line_containing(path: &str, needle: &str) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .find(|line| line.contains(needle))
        .map_or_else(|| "unknown".to_owned(), |line| line.trim().to_owned())
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "rustc unknown".to_owned())
}

fn workload_hash() -> String {
    const OFFSET: u64 = 14_695_981_039_346_656_037;
    const PRIME: u64 = 1_099_511_628_211;
    let mut hash = OFFSET;
    for source in [
        include_str!("measure.rs"),
        include_str!("main.rs"),
        include_str!("../Cargo.toml"),
    ] {
        for byte in source.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{hash:016x}")
}

#[cfg(feature = "gui")]
fn measure_gui(fixture: &Fixture, recorder: &mut Recorder) -> BenchResult<()> {
    let app = RefCell::new(fixture.app()?);
    recorder.measure(
        "gui/frame/idle",
        "render one review frame without a native window within 100 ms",
        100.0,
        10,
        1,
        3,
        || {
            app.borrow_mut().frame(None);
            Ok(())
        },
    )?;
    recorder.measure_with_setup(
        "gui/frame/reveal",
        "render the answer reveal frame within 100 ms",
        100.0,
        10,
        1,
        2,
        || {
            app.borrow_mut().reset();
            Ok(())
        },
        || {
            let mut revealed = app.borrow_mut();
            revealed.frame(Some(ranki_ui::BenchKey::Space));
            revealed.frame(None);
            Ok(())
        },
    )
}
pub fn run() -> BenchResult<Report> {
    let fixture = Fixture::new()?;
    let mut recorder = Recorder::default();

    let fsrs = FsrsScheduler::new(FsrsPreset::default());
    let mut index = 0usize;
    let cards = &fixture.cards;
    let created = fixture.created;
    let now = fixture.now;
    let fsrs_sink = AtomicI64::new(0);
    recorder.measure(
        "core/fsrs/schedule",
        "one FSRS scheduling decision within 10 ms",
        10.0,
        10,
        20,
        3,
        || {
            let card = cards
                .get(index.checked_rem(cards.len()).unwrap_or(0))
                .ok_or_else(|| message("missing card"))?;
            let decision = fsrs.schedule(card, Rating::Good, now, created);
            if decision.card.id != card.id {
                return Err(message("FSRS scheduler changed the card id"));
            }
            fsrs_sink.fetch_add(1, Ordering::Relaxed);
            index = index.saturating_add(1);
            Ok(())
        },
    )?;
    if fsrs_sink.load(Ordering::Relaxed) == 0 {
        return Err(message("FSRS scheduler produced no intervals"));
    }
    let collection = &fixture.collection;
    recorder.measure(
        "storage/collection/open",
        "open an existing collection within 50 ms",
        50.0,
        10,
        1,
        2,
        || {
            let store = Collection::open(collection)?;
            drop(store);
            Ok(())
        },
    )?;
    let store = Collection::open(collection)?;
    recorder.measure(
        "storage/search/common",
        "search 2,000 notes for a common term within 50 ms",
        50.0,
        10,
        2,
        3,
        || {
            let found = store.find_cards("Common")?;
            if found.len() != NOTE_COUNT {
                return Err(message(
                    "common search returned the wrong number of cards",
                ));
            }
            Ok(())
        },
    )?;
    recorder.measure(
        "storage/search/missing",
        "search 2,000 notes for a missing term within 20 ms",
        20.0,
        10,
        5,
        3,
        || {
            let found = store.find_cards("zzq")?;
            if !found.is_empty() {
                return Err(message("missing search returned cards"));
            }
            Ok(())
        },
    )?;
    recorder.measure(
        "storage/due/50",
        "load 50 due cards within 50 ms",
        50.0,
        10,
        2,
        3,
        || {
            let due = store.due_cards(None, REVIEW_COUNT, now)?;
            if due.len() != REVIEW_COUNT {
                return Err(message(
                    "due query returned the wrong number of cards",
                ));
            }
            Ok(())
        },
    )?;
    recorder.measure(
        "storage/stats",
        "compute collection statistics within 20 ms",
        20.0,
        10,
        5,
        3,
        || {
            let stats = store.stats(None, now)?;
            if stats.total_cards != NOTE_COUNT {
                return Err(message("stats returned the wrong card count"));
            }
            Ok(())
        },
    )?;
    drop(store);

    let mut store = Collection::open(collection)?;
    recorder.measure(
        "storage/review/write",
        "persist one scheduled review within 100 ms",
        100.0,
        10,
        20,
        3,
        || {
            let fixture_card = fixture
                .cards
                .get(index.checked_rem(fixture.cards.len()).unwrap_or(0))
                .ok_or_else(|| message("missing fixture card"))?;
            let card = store
                .card(fixture_card.id)?
                .ok_or_else(|| message("review card is missing"))?;
            let decision = fsrs.schedule(&card, Rating::Good, now, created);
            store.apply_review(&decision)?;
            index = index.saturating_add(1);
            Ok(())
        },
    )?;
    drop(store);

    let media = &fixture.media;
    recorder.measure(
        "format/export-apkg",
        "export the fixture package within 500 ms",
        500.0,
        5,
        1,
        2,
        || {
            let target = fixture.unique_path("export", "apkg");
            export_apkg(collection, media, &target)?;
            fs::remove_file(&target)?;
            Ok(())
        },
    )?;
    recorder.measure(
        "format/import-apkg",
        "import the fixture package within 1,000 ms",
        1_000.0,
        5,
        1,
        2,
        || {
            let target = fixture.unique_path("import", "anki2");
            let target_media = fixture.unique_path("import-media", "");
            let report = import_apkg(&fixture.package, &target, &target_media)?;
            if report.notes != NOTE_COUNT {
                return Err(message("import returned the wrong note count"));
            }
            fs::remove_file(&target)?;
            fs::remove_dir_all(&target_media)?;
            Ok(())
        },
    )?;
    #[cfg(feature = "gui")]
    measure_gui(&fixture, &mut recorder)?;

    let binary =
        env::var_os("RANKI_BIN").map(PathBuf::from).ok_or_else(|| {
            message("RANKI_BIN must point at the release ranki binary")
        })?;
    if !binary.is_file() {
        return Err(message(format!(
            "RANKI_BIN does not exist: {}",
            binary.display()
        )));
    }
    let data_dir = fixture.directory.path();
    recorder.measure(
        "process/cli/decks",
        "start ranki and list decks within 500 ms",
        500.0,
        7,
        1,
        2,
        || {
            let output = Command::new(&binary)
                .env("RANKI_DATA_DIR", data_dir)
                .args(["decks"])
                .output()?;
            if !output.status.success()
                || !String::from_utf8_lossy(&output.stdout).contains("Default")
            {
                return Err(message("ranki decks failed"));
            }
            Ok(())
        },
    )?;
    recorder.measure(
        "process/cli/search",
        "start ranki and search the fixture within 500 ms",
        500.0,
        7,
        1,
        2,
        || {
            let output = Command::new(&binary)
                .env("RANKI_DATA_DIR", data_dir)
                .args(["search", "unique42"])
                .output()?;
            if !output.status.success() {
                return Err(message("ranki search failed"));
            }
            Ok(())
        },
    )?;

    recorder.measure(
        "connect/startup",
        "start AnkiConnect and answer the first request within 1,000 ms",
        1_000.0,
        5,
        1,
        1,
        || {
            let server = ConnectServer::start(&binary, data_dir)?;
            drop(server);
            Ok(())
        },
    )?;
    let connect = ConnectServer::start(&binary, data_dir)?;
    recorder.measure(
        "connect/roundtrip/version",
        "answer an AnkiConnect version request within 100 ms",
        100.0,
        10,
        1,
        3,
        || {
            let response =
                rpc(connect.port, "{\"action\":\"version\",\"version\":6}")?;
            if !response.contains("\"result\":6") {
                return Err(message("version request failed"));
            }
            Ok(())
        },
    )?;
    recorder.measure(
        "connect/roundtrip/find-cards",
        "answer an AnkiConnect search within 100 ms",
        100.0,
        10,
        1,
        3,
        || connect.find_cards(),
    )?;
    drop(connect);

    recorder.measure(
        "mcp/startup",
        "start the MCP server and finish initialize within 1,000 ms",
        1_000.0,
        5,
        1,
        1,
        || {
            let session = McpSession::start(&binary, data_dir)?;
            drop(session);
            Ok(())
        },
    )?;
    let mut mcp = McpSession::start(&binary, data_dir)?;
    recorder.measure(
        "mcp/roundtrip/tools-list",
        "answer an MCP tools/list request within 100 ms",
        100.0,
        10,
        1,
        3,
        || mcp.list_tools(),
    )?;
    recorder.measure(
        "mcp/roundtrip/list-decks",
        "answer an MCP list_decks call within 100 ms",
        100.0,
        10,
        1,
        3,
        || mcp.list_decks(),
    )?;
    drop(mcp);

    Ok(Report {
        method: METHOD,
        runtime: rustc_version(),
        os: fs::read_to_string("/proc/sys/kernel/osrelease").map_or_else(
            |_| env::consts::OS.to_owned(),
            |text| format!("Linux {}", text.trim()),
        ),
        cpu: first_line_containing("/proc/cpuinfo", "model name"),
        results: recorder.results,
        workload_hash: workload_hash(),
    })
}
