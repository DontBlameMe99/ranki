//! Command line entry point for Ranki.

mod config;
mod connect;
mod error;
mod mcp;

use clap::Parser;
use clap::Subcommand;
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
use ranki_model::parse_image_occlusion_field;
use ranki_storage::Collection;
use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fmt::Display;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModelRef {
    Id(Id),
    Name(String),
}

impl FromStr for ModelRef {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.trim().is_empty() {
            return Err("model cannot be empty".to_owned());
        }
        Ok(value
            .parse::<Id>()
            .map_or_else(|_| Self::Name(value.to_owned()), Self::Id))
    }
}

impl PartialEq<i64> for ModelRef {
    fn eq(&self, other: &i64) -> bool {
        matches!(self, Self::Id(id) if id == other)
    }
}

impl Display for ModelRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => write!(formatter, "{id}"),
            Self::Name(name) => write!(formatter, "{name}"),
        }
    }
}
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
    /// Create a deck.
    DeckCreate { name: String },
    /// Rename a deck and its subdecks.
    DeckRename { id: Id, name: String },
    /// Remove a deck and its subdecks.
    DeckRemove {
        id: Id,
        /// Remove non-empty decks without asking again.
        #[arg(long)]
        force: bool,
    },
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
        #[arg(
            long = "model",
            visible_alias = "model-id",
            value_name = "ID_OR_NAME"
        )]
        model: ModelRef,
        #[arg(long = "deck", visible_alias = "deck-id", value_name = "ID")]
        deck_id: Id,
        #[arg(long = "field", value_name = "TEXT")]
        fields: Vec<String>,
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },
    /// Add a built-in Image Occlusion note from an image and cloze markers.
    AddImageOcclusion {
        #[arg(long = "deck", value_name = "ID")]
        deck_id: Id,
        #[arg(long, value_name = "PATH")]
        image: PathBuf,
        #[arg(long = "occlusion", value_name = "MARKER", required = true)]
        occlusions: Vec<String>,
        #[arg(long, value_name = "TEXT")]
        header: Option<String>,
        #[arg(long = "back-extra", value_name = "TEXT")]
        back_extra: Option<String>,
        #[arg(long, value_name = "TEXT")]
        comments: Option<String>,
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },
    /// List note models available in the collection, including stock types.
    Models,
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
    let mut cli = cli;
    if cli.collection.is_none() {
        cli.collection = env::var_os("RANKI_COLLECTION").map(PathBuf::from);
    }
    let has_collection =
        cli.collection.is_some() || config.general.collection.is_some();
    let fallback = if has_collection {
        None
    } else {
        Some(default_collection_path(env::var_os("HOME").as_deref())?)
    };
    let settings = config::resolve_settings(&cli, &config, fallback);
    let scheduler = ranki_core::from_settings(&settings);

    if matches!(cli.command, Some(Command::Mcp)) {
        return mcp::serve(Some(&settings.collection), scheduler);
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
        Command::Connect { port } => {
            connect::serve(collection_path, port, scheduler.as_ref())
        }
        Command::Decks
        | Command::DeckCreate { .. }
        | Command::DeckRename { .. }
        | Command::DeckRemove { .. }
        | Command::Due { .. }
        | Command::Search { .. }
        | Command::AddNote { .. }
        | Command::AddImageOcclusion { .. }
        | Command::Models
        | Command::Answer { .. }
        | Command::Stats { .. } => {
            let mut collection = open_or_create(collection_path)?;
            run_store_command(
                &mut collection,
                command,
                scheduler.as_ref(),
                media_dir,
            )
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the CLI dispatch table maps each subcommand once"
)]
fn run_store_command(
    collection: &mut Collection,
    command: Command,
    scheduler: &dyn Scheduler,
    media_dir: &Path,
) -> CliResult<()> {
    match command {
        Command::Decks => {
            for deck in collection.decks()? {
                println!("{}\t{}", deck.id, deck.name);
            }
            Ok(())
        }
        Command::DeckCreate { .. }
        | Command::DeckRename { .. }
        | Command::DeckRemove { .. } => run_deck_command(collection, command),
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
            model,
            deck_id,
            fields,
            tags,
        } => {
            let model_id = resolve_model(collection, &model)?;
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
        Command::AddImageOcclusion {
            deck_id,
            image,
            occlusions,
            header,
            back_extra,
            comments,
            tags,
        } => add_image_occlusion(
            collection,
            media_dir,
            deck_id,
            &image,
            &occlusions,
            header.as_deref(),
            back_extra.as_deref(),
            comments.as_deref(),
            &tags,
        ),
        Command::Models => {
            for model in collection.models()? {
                println!(
                    "{}\t{}\t{}\t{}",
                    model.id,
                    model.name,
                    model.fields.len(),
                    model.templates.len()
                );
            }
            for model in ranki_model::stock_models() {
                println!(
                    "stock\t{}\t{}\t{}",
                    model.name,
                    model.fields.len(),
                    model.templates.len()
                );
            }
            Ok(())
        }
        Command::Answer { card_id, rating } => {
            let decision = answer_card(
                collection,
                card_id,
                rating,
                now_secs()?,
                scheduler,
            )?;
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
        collection: settings.collection.clone(),
        media_dir: settings.media_dir.clone(),
        theme: settings.theme,
        font_size: settings.font_size,
        review_batch_size: settings.review_batch_size,
        show_keyboard_hints: settings.show_keyboard_hints,
        scheduler: settings.scheduler,
        desired_retention: settings.desired_retention,
    };
    ranki_ui::run(Box::new(collection), options);
    Ok(())
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

fn run_deck_command(
    collection: &mut Collection,
    command: Command,
) -> CliResult<()> {
    match command {
        Command::DeckCreate { name } => {
            let deck = collection.create_deck(&name)?;
            println!("created\t{}\t{}", deck.id, deck.name);
        }
        Command::DeckRename { id, name } => {
            let deck = collection.rename_deck(id, &name)?;
            println!("renamed\t{}\t{}", deck.id, deck.name);
        }
        Command::DeckRemove { id, force } => {
            let deck = deck_by_id(collection, id)?;
            require_force(collection, &deck, force)?;
            collection.remove_deck(id)?;
            println!("removed\t{}\t{}", deck.id, deck.name);
        }
        Command::Gui
        | Command::Decks
        | Command::Due { .. }
        | Command::Search { .. }
        | Command::AddNote { .. }
        | Command::AddImageOcclusion { .. }
        | Command::Models
        | Command::Answer { .. }
        | Command::Stats { .. }
        | Command::Import { .. }
        | Command::Export { .. }
        | Command::ExportColpkg { .. }
        | Command::Check
        | Command::Connect { .. }
        | Command::Mcp => {}
    }
    Ok(())
}

fn require_force(
    collection: &Collection,
    deck: &ranki_model::Deck,
    force: bool,
) -> CliResult<()> {
    if force {
        return Ok(());
    }
    let cards = collection.stats(Some(deck.id), now_secs()?)?.total_cards;
    if cards == 0 {
        return Ok(());
    }
    Err(CliError::Message(format!(
        "deck {} ({}) has {cards} cards; pass --force to remove it",
        deck.id, deck.name
    )))
}
fn deck_by_id(collection: &Collection, id: Id) -> CliResult<ranki_model::Deck> {
    collection
        .decks()?
        .into_iter()
        .find(|deck| deck.id == id)
        .ok_or_else(|| CliError::Message(format!("deck not found: {id}")))
}

fn resolve_model(
    collection: &mut Collection,
    model: &ModelRef,
) -> CliResult<Id> {
    match model {
        ModelRef::Id(id) => Ok(*id),
        ModelRef::Name(name) => {
            let existing = collection.models()?;
            if let Some(model) = existing
                .iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(name))
            {
                return Ok(model.id);
            }
            Ok(collection.ensure_model(name)?.id)
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the image occlusion command maps one flag per note field"
)]
fn add_image_occlusion(
    collection: &mut Collection,
    media_dir: &Path,
    deck_id: Id,
    image: &Path,
    occlusions: &[String],
    header: Option<&str>,
    back_extra: Option<&str>,
    comments: Option<&str>,
    tags: &[String],
) -> CliResult<()> {
    let occlusion = occlusions.join("<br>");
    let parsed = parse_image_occlusion_field(&occlusion);
    if parsed.shapes.is_empty() {
        return Err(CliError::Message(
            "no image-occlusion markers found".to_owned(),
        ));
    }
    let model_id = resolve_model(
        collection,
        &ModelRef::Name("Image Occlusion".to_owned()),
    )?;
    let model = collection
        .models()?
        .into_iter()
        .find(|entry| entry.id == model_id)
        .ok_or_else(|| {
            CliError::Message("image occlusion model missing".into())
        })?;
    if model.fields.len() != 5 {
        return Err(CliError::Message(format!(
            "image occlusion model needs 5 fields, found {}",
            model.fields.len()
        )));
    }
    let media_name = image
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| CliError::Message("invalid image file name".into()))?;
    if media_name.is_empty() || media_name.contains(['"', '<', '>']) {
        return Err(CliError::Message("unsafe image file name".into()));
    }
    fs::create_dir_all(media_dir)?;
    fs::copy(image, media_dir.join(media_name))?;
    let fields = vec![
        occlusion,
        format!("<img src=\"{media_name}\">"),
        header.unwrap_or_default().to_owned(),
        back_extra.unwrap_or_default().to_owned(),
        comments.unwrap_or_default().to_owned(),
    ];
    let note = NewNote {
        model_id,
        deck_id,
        fields,
        tags: tags.to_vec(),
    };
    for id in collection.add_note(&note)? {
        println!("{id}");
    }
    Ok(())
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
    reason = "the shared answer helper needs store, card, rating, time, and scheduler"
)]
pub(crate) fn answer_card<S: Store>(
    store: &mut S,
    card_id: Id,
    rating: Rating,
    now: i64,
    scheduler: &dyn Scheduler,
) -> CliResult<ReviewDecision> {
    let card = store.card(card_id)?.ok_or_else(|| {
        CliError::Message(format!("card not found: {card_id}"))
    })?;
    let collection_start_secs = store.collection_created()?;
    let decision =
        scheduler.schedule(&card, rating, now, collection_start_secs);
    store.apply_review(&decision)?;
    Ok(decision)
}

pub(crate) fn save_card<S: Store>(store: &mut S, card: Card) -> CliResult<()> {
    let log_id = ranki_model::next_id(now_ms()?);
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
            model,
            deck_id,
            fields,
            tags,
        }) = cli.command
        else {
            panic!("wrong command");
        };
        assert_eq!(model, 1);
        assert_eq!(deck_id, 2);
        assert_eq!(fields, ["front", "back"]);
        assert_eq!(tags, ["one", "two"]);
    }

    #[test]
    fn add_note_accepts_stock_model_names() {
        let cli = parse(&[
            "ranki",
            "add-note",
            "--model",
            "Basic (and reversed card)",
            "--deck",
            "1",
            "--field",
            "front",
            "--field",
            "back",
        ]);
        assert!(matches!(
            cli.command,
            Some(Command::AddNote {
                model: super::ModelRef::Name(name),
                ..
            }) if name == "Basic (and reversed card)"
        ));
    }

    #[test]
    fn add_image_occlusion_collects_markers_and_fields() {
        let cli = parse(&[
            "ranki",
            "add-image-occlusion",
            "--deck",
            "1",
            "--image",
            "/tmp/diagram.png",
            "--occlusion",
            "{{c1::image-occlusion:rect:left=.1:top=.2:width=.3:height=.4}}",
            "--occlusion",
            "{{c2::image-occlusion:ellipse:left=.5:top=.6:width=.2:height=.2}}",
            "--header",
            "Diagram",
            "--back-extra",
            "More",
            "--tag",
            "io",
        ]);
        let Some(Command::AddImageOcclusion {
            image,
            occlusions,
            header,
            back_extra,
            tags,
            ..
        }) = cli.command
        else {
            panic!("wrong command");
        };
        assert_eq!(image, Path::new("/tmp/diagram.png"));
        assert_eq!(occlusions.len(), 2);
        assert!(occlusions[1].contains("c2"));
        assert_eq!(header.as_deref(), Some("Diagram"));
        assert_eq!(back_extra.as_deref(), Some("More"));
        assert_eq!(tags, ["io"]);
    }

    #[test]
    fn models_command_is_available() {
        assert!(matches!(
            parse(&["ranki", "models"]).command,
            Some(Command::Models)
        ));
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
