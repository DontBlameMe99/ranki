//! Command line entry point for Ranki.

mod config;
mod connect;
mod mcp;

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

static MANUAL_LOG_SEQUENCE: AtomicU64 = AtomicU64::new(0);
use clap::Parser;
use clap::Subcommand;
use ranki_core::FsrsScheduler;
use ranki_core::Scheduler;
use ranki_format::export_apkg;
use ranki_format::export_colpkg;
use ranki_format::import_apkg;
use ranki_model::Card;
use ranki_model::Id;
use ranki_model::NewNote;
use ranki_model::Rating;
use ranki_model::RevLog;
use ranki_model::RevLogKind;
use ranki_model::ReviewDecision;
use ranki_model::Store;
use ranki_storage::Collection;
use thiserror::Error;

/// Errors surfaced by the binary and its two built-in interfaces.
#[derive(Debug, Error)]
pub(crate) enum CliError {
    #[error("{0}")]
    Message(String),
    #[error("storage: {0}")]
    Store(#[from] ranki_model::StoreError),
    #[error("format: {0}")]
    Format(#[from] ranki_format::FormatError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

impl From<CliError> for String {
    fn from(error: CliError) -> Self {
        error.to_string()
    }
}

pub(crate) type CliResult<T> = Result<T, CliError>;

#[derive(Debug, Parser)]
#[command(name = "ranki", version, about = "An Anki-compatible flashcard app")]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    collection: Option<PathBuf>,
    #[arg(long, global = true, value_name = "PATH")]
    media_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open the review window.
    Gui,
    /// List decks in the collection.
    Decks,
    /// Show cards that are due.
    Due {
        #[arg(long, value_name = "ID")]
        deck: Option<Id>,
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Find cards with an Anki-style query.
    Search { query: String },
    /// Add a note and its cards.
    AddNote {
        #[arg(long = "model", visible_alias = "model-id", value_name = "ID")]
        model_id: Id,
        #[arg(long = "deck", visible_alias = "deck-id", value_name = "ID")]
        deck_id: Id,
        #[arg(long = "field", value_name = "TEXT")]
        fields: Vec<String>,
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },
    /// Answer a card with an ease rating from 1 to 4.
    Answer {
        card_id: Id,
        #[arg(value_parser = parse_rating)]
        rating: Rating,
    },
    /// Show card counts for a deck.
    Stats {
        #[arg(long, value_name = "ID")]
        deck: Option<Id>,
    },
    /// Import an Anki package into the collection.
    Import { package: PathBuf },
    /// Export the collection as an Anki package.
    Export { package: PathBuf },
    /// Export the collection as an Anki colpkg.
    ExportColpkg { package: PathBuf },
    /// Check that the collection can be read.
    Check,
    /// Serve the AnkiConnect-compatible JSON-RPC API.
    Connect {
        #[arg(long, default_value_t = 8765)]
        port: u16,
    },
    /// Serve the Model Context Protocol over stdio.
    Mcp,
}

fn parse_rating(value: &str) -> Result<Rating, String> {
    let code = value.parse::<u8>().map_err(|_| {
        format!("invalid rating '{value}', expected 1, 2, 3, or 4")
    })?;
    Rating::from_code(code).ok_or_else(|| {
        format!("invalid rating '{value}', expected 1, 2, 3, or 4")
    })
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .without_time()
        .init();
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ranki: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> CliResult<()> {
    let config = config::load();
    let fallback = match &cli.collection {
        Some(path) => path.clone(),
        None => default_collection_path(env::var_os("HOME").as_deref())?,
    };
    let settings = config::resolve_settings(&cli, &config, fallback);

    if matches!(cli.command, Some(Command::Mcp)) {
        return mcp::serve(Some(&settings.collection));
    }
    let collection_path = settings.collection.as_path();
    let media_dir = settings.media_dir.as_path();
    let command = cli.command.unwrap_or(Command::Gui);
    match command {
        Command::Mcp => Ok(()),
        Command::Gui => open_gui(&settings),
        Command::Import { package } => {
            import_package(collection_path, media_dir, &package)
        }
        Command::Export { package } => {
            export_package(collection_path, media_dir, &package)
        }
        Command::ExportColpkg { package } => {
            export_colpkg_package(collection_path, media_dir, &package)
        }
        Command::Check => check(collection_path),
        Command::Connect { port } => connect::serve(collection_path, port),
        Command::Decks
        | Command::Due { .. }
        | Command::Search { .. }
        | Command::AddNote { .. }
        | Command::Answer { .. }
        | Command::Stats { .. } => {
            let mut collection = open_or_create(collection_path)?;
            run_store_command(&mut collection, command)
        }
    }
}

fn run_store_command(
    collection: &mut Collection,
    command: Command,
) -> CliResult<()> {
    match command {
        Command::Decks => {
            for deck in collection.decks()? {
                println!("{}\t{}", deck.id, deck.name);
            }
            Ok(())
        }
        Command::Due { deck, limit } => {
            let cards =
                collection.due_cards(deck, limit.unwrap_or(20), now_secs()?)?;
            for card in cards {
                println!("{}\t{}\t{}", card.id, card.deck_id, card.due);
            }
            Ok(())
        }
        Command::Search { query } => {
            for id in collection.find_cards(&query)? {
                println!("{id}");
            }
            Ok(())
        }
        Command::AddNote {
            model_id,
            deck_id,
            fields,
            tags,
        } => {
            let note = NewNote {
                model_id,
                deck_id,
                fields,
                tags,
            };
            for id in collection.add_note(&note)? {
                println!("{id}");
            }
            Ok(())
        }
        Command::Answer { card_id, rating } => {
            let decision =
                answer_card(collection, card_id, rating, now_secs()?)?;
            println!("{}\t{}", card_id, decision.card.due);
            Ok(())
        }
        Command::Stats { deck } => {
            let stats = collection.stats(deck, now_secs()?)?;
            println!(
                "new={} learning={} review={} total={}",
                stats.new_cards,
                stats.learning_cards,
                stats.review_cards,
                stats.total_cards
            );
            Ok(())
        }
        Command::Gui
        | Command::Import { .. }
        | Command::Export { .. }
        | Command::ExportColpkg { .. }
        | Command::Check
        | Command::Connect { .. }
        | Command::Mcp => Ok(()),
    }
}

fn open_gui(settings: &config::Settings) -> CliResult<()> {
    let collection = open_or_create(&settings.collection)?;
    let options = ranki_ui::Options {
        window_title: String::from("Ranki"),
        media_dir: Some(settings.media_dir.clone()),
        light_theme: settings.theme == ranki_ui::ThemeChoice::Light,
        theme: settings.theme,
        font_size: settings.font_size,
        review_batch_size: settings.review_batch_size,
        show_keyboard_hints: settings.show_keyboard_hints,
        scheduler: settings.scheduler,
        desired_retention: settings.desired_retention,
    };
    ranki_ui::run(Box::new(collection), options)
        .map_err(|error| CliError::Message(error.to_string()))
}

fn import_package(
    path: &Path,
    media_dir: &Path,
    package: &Path,
) -> CliResult<()> {
    let report = import_apkg(package, path, media_dir)?;
    println!(
        "imported notes={} cards={} media={}",
        report.notes, report.cards, report.media
    );
    Ok(())
}

fn export_package(
    path: &Path,
    media_dir: &Path,
    package: &Path,
) -> CliResult<()> {
    let collection = open_or_create(path)?;
    collection.checkpoint()?;
    collection.close()?;
    export_apkg(path, media_dir, package)?;
    Ok(())
}

fn export_colpkg_package(
    path: &Path,
    media_dir: &Path,
    package: &Path,
) -> CliResult<()> {
    let collection = open_or_create(path)?;
    collection.checkpoint()?;
    collection.close()?;
    export_colpkg(path, media_dir, package)?;
    Ok(())
}

pub(crate) fn default_collection_path(
    home: Option<&OsStr>,
) -> CliResult<PathBuf> {
    let home_dir = home.ok_or_else(|| {
        CliError::Message("HOME is not set; pass --collection PATH".into())
    })?;
    Ok(Path::new(home_dir).join(".local/share/Anki2/main/collection.anki2"))
}

pub(crate) fn media_dir_for(collection: &Path) -> PathBuf {
    let parent = collection.parent().unwrap_or_else(|| Path::new("."));
    parent.join("collection.media")
}

pub(crate) fn open_or_create(path: &Path) -> CliResult<Collection> {
    if path.exists() {
        return Ok(Collection::open(path)?);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(Collection::create(path)?)
}

pub(crate) fn now_secs() -> CliResult<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CliError::Message(error.to_string()))?;
    i64::try_from(duration.as_secs())
        .map_err(|_| CliError::Message("timestamp too large".into()))
}

pub(crate) fn now_ms() -> CliResult<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CliError::Message(error.to_string()))?;
    i64::try_from(duration.as_millis())
        .map_err(|_| CliError::Message("timestamp too large".into()))
}

#[expect(
    clippy::too_many_arguments,
    reason = "store, card id, rating and time are all needed"
)]
pub(crate) fn answer_card<S: Store>(
    store: &mut S,
    card_id: Id,
    rating: Rating,
    now: i64,
) -> CliResult<ReviewDecision> {
    let card = store.card(card_id)?.ok_or_else(|| {
        CliError::Message(format!("card not found: {card_id}"))
    })?;
    let collection_start_secs = store.collection_created()?;
    let decision = FsrsScheduler::default().schedule(
        &card,
        rating,
        now,
        collection_start_secs,
    );
    store.apply_review(&decision)?;
    Ok(decision)
}

pub(crate) fn save_card<S: Store>(store: &mut S, card: Card) -> CliResult<()> {
    let sequence = MANUAL_LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let offset = i64::try_from(sequence).unwrap_or(i64::MAX);
    let log_id = now_ms()?.saturating_add(offset);
    let log = RevLog {
        id: log_id,
        card_id: card.id,
        usn: 0,
        ease: Rating::Good,
        interval: card.interval,
        last_interval: 0,
        factor: card.factor,
        time_ms: 0,
        kind: RevLogKind::Manual,
    };
    store.apply_review(&ReviewDecision { card, revlog: log })?;
    Ok(())
}

fn check(path: &Path) -> CliResult<()> {
    let collection = open_or_create(path)?;
    collection.decks()?;
    collection.models()?;
    collection.stats(None, now_secs()?)?;
    collection.collection_modified()?;
    collection.checkpoint()?;
    collection.close()?;
    println!("ok");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use clap::Parser;

    use super::Cli;
    use super::Command;
    use super::default_collection_path;
    use super::media_dir_for;
    use super::parse_rating;
    use ranki_model::Rating;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn no_subcommand_selects_gui() {
        assert!(matches!(
            parse(&["ranki"]).command,
            None | Some(Command::Gui)
        ));
    }

    #[test]
    fn collection_flag_is_global() {
        let cli = parse(&["ranki", "--collection", "/tmp/demo.anki2", "decks"]);
        assert_eq!(
            cli.collection.as_deref(),
            Some(Path::new("/tmp/demo.anki2"))
        );
        assert!(matches!(cli.command, Some(Command::Decks)));
    }

    #[test]
    fn media_dir_flag_is_global() {
        let cli = parse(&["ranki", "mcp", "--media-dir", "/tmp/media"]);
        assert_eq!(cli.media_dir.as_deref(), Some(Path::new("/tmp/media")));
        assert!(matches!(cli.command, Some(Command::Mcp)));
    }

    #[test]
    fn export_colpkg_uses_global_collection_and_media_paths() {
        let cli = parse(&[
            "ranki",
            "--collection",
            "/tmp/demo.anki2",
            "--media-dir",
            "/tmp/media",
            "export-colpkg",
            "/tmp/backup.colpkg",
        ]);
        assert_eq!(
            cli.collection.as_deref(),
            Some(Path::new("/tmp/demo.anki2"))
        );
        assert_eq!(cli.media_dir.as_deref(), Some(Path::new("/tmp/media")));
        assert!(matches!(
            cli.command,
            Some(Command::ExportColpkg { package })
                if package == Path::new("/tmp/backup.colpkg")
        ));
    }

    #[test]
    fn add_note_collects_repeated_fields_and_tags() {
        let cli = parse(&[
            "ranki", "add-note", "--model", "1", "--deck", "2", "--field",
            "front", "--field", "back", "--tag", "one", "--tag", "two",
        ]);
        let Some(Command::AddNote {
            model_id,
            deck_id,
            fields,
            tags,
        }) = cli.command
        else {
            panic!("wrong command");
        };
        assert_eq!((model_id, deck_id), (1, 2));
        assert_eq!(fields, ["front", "back"]);
        assert_eq!(tags, ["one", "two"]);
    }

    #[test]
    fn due_accepts_optional_deck_and_limit() {
        let cli = parse(&["ranki", "due", "--deck", "7", "--limit", "5"]);
        assert!(matches!(
            cli.command,
            Some(Command::Due {
                deck: Some(7),
                limit: Some(5)
            })
        ));
    }

    #[test]
    fn answer_only_accepts_ratings_one_to_four() {
        for (input, expected) in [
            ("1", Rating::Again),
            ("2", Rating::Hard),
            ("3", Rating::Good),
            ("4", Rating::Easy),
        ] {
            assert_eq!(parse_rating(input).unwrap(), expected);
        }
        assert!(parse_rating("0").is_err());
        assert!(parse_rating("5").is_err());
        assert!(parse_rating("good").is_err());
    }

    #[test]
    fn connect_defaults_to_port_8765() {
        let cli = parse(&["ranki", "connect"]);
        assert!(matches!(cli.command, Some(Command::Connect { port: 8765 })));
    }

    #[test]
    fn default_paths_follow_anki_layout() {
        let collection =
            default_collection_path(Some(OsStr::new("/home/user"))).unwrap();
        assert_eq!(
            collection,
            Path::new("/home/user/.local/share/Anki2/main/collection.anki2")
        );
        assert_eq!(
            media_dir_for(&collection),
            Path::new("/home/user/.local/share/Anki2/main/collection.media")
        );
        assert!(default_collection_path(None).is_err());
    }
}
