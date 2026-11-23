//! Command line entry point for Ranki.

mod config;
mod connect;
mod error;
mod mcp;
mod optimize;
mod sync;

use clap::Parser;
use clap::Subcommand;
use ranki_core::FsrsParams;
use ranki_core::FsrsPreset;
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
use ranki_model::StockModelKind;
use ranki_model::Store;
use ranki_model::parse_image_occlusion_field;
use ranki_storage::Collection;
use ranki_sync::SyncTransport;
use ranki_ui::occlusion::{
    MaskShape, NormalizedRect, OcclusionMask, occlusion_field,
};
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
use std::sync::Arc;
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "mutually exclusive sync flags map directly onto CLI switches"
)]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    collection: Option<PathBuf>,
    #[arg(long, global = true, value_name = "PATH")]
    media_dir: Option<PathBuf>,
    #[arg(long, global = true, conflicts_with = "no_sync")]
    sync: bool,
    #[arg(long, global = true, conflicts_with = "sync")]
    no_sync: bool,
    #[arg(long, global = true, conflicts_with = "no_sync_auto")]
    sync_auto: bool,
    #[arg(long, global = true, conflicts_with = "sync_auto")]
    no_sync_auto: bool,
    #[arg(long, global = true, value_name = "USER")]
    sync_username: Option<String>,
    #[arg(long, global = true, value_name = "PATH")]
    sync_hkey_file: Option<PathBuf>,
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
        #[arg(
            long = "occlusion",
            value_name = "MARKER",
            required_unless_present_any = ["rects", "ellipses"]
        )]
        occlusions: Vec<String>,
        #[arg(
            long = "rect",
            value_name = "LEFT,TOP,WIDTH,HEIGHT",
            value_parser = parse_rect
        )]
        rects: Vec<MaskRect>,
        #[arg(
            long = "ellipse",
            value_name = "LEFT,TOP,WIDTH,HEIGHT",
            value_parser = parse_rect
        )]
        ellipses: Vec<MaskRect>,
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
    /// Sync with `AnkiWeb` or show remote status.
    Sync {
        /// Show remote metadata without changing either collection.
        #[arg(long, conflicts_with_all = ["full_upload", "full_download"])]
        status: bool,
        /// Replace the remote collection with the local one.
        #[arg(long, conflicts_with_all = ["status", "full_download"])]
        full_upload: bool,
        /// Replace the local collection with the remote one.
        #[arg(long, conflicts_with_all = ["status", "full_upload"])]
        full_download: bool,
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
    let mut config = config::load();
    let mut transport = sync::http_transport(&config);
    run_with_sync(cli, &mut config, &mut transport)
}

/// CLI entry point with an injectable sync transport. Tests never reach HTTP.
#[expect(
    clippy::too_many_lines,
    reason = "CLI dispatch maps each subcommand once"
)]
fn run_with_sync(
    cli: Cli,
    config: &mut config::Config,
    transport: &mut dyn SyncTransport,
) -> CliResult<()> {
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
    let settings = config::resolve_settings(&cli, config, fallback);
    let manual_sync = matches!(cli.command, Some(Command::Sync { .. }));
    let auth = if manual_sync || settings.sync.enabled && settings.sync.auto {
        sync::load_auth(&settings, &sync::read_env())
    } else {
        None
    };
    let auto_sync = settings.sync.enabled
        && settings.sync.auto
        && (auth.is_some() || sync::has_stored_key(&settings))
        && !manual_sync;

    if auto_sync {
        match sync::sync_once(
            sync::TransportRef(transport),
            &settings,
            auth.as_ref(),
        ) {
            Ok(outcome) => {
                sync::warn_outcome(&outcome);
                maybe_optimize(config, &settings);
            }
            Err(error) => eprintln!("ranki: sync: {error}"),
        }
    }

    if matches!(cli.command, Some(Command::Mcp)) {
        return mcp::serve(
            Some(&settings.collection),
            scheduler_for(&settings),
        );
    }
    let scheduler = scheduler_for(&settings);
    let collection_path = settings.collection.as_path();
    let media_dir = settings.media_dir.as_path();
    let command = cli.command.unwrap_or(Command::Gui);
    let result = match command {
        Command::Mcp => Ok(()),
        Command::Gui => open_gui(&settings),
        Command::Import { package } => {
            let result = import_package(collection_path, media_dir, &package);
            if result.is_ok() {
                maybe_optimize(config, &settings);
            }
            result
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
        Command::Sync {
            status,
            full_upload,
            full_download,
        } => {
            let mode = if status {
                sync::SyncMode::Status
            } else if full_upload {
                sync::SyncMode::FullUpload
            } else if full_download {
                sync::SyncMode::FullDownload
            } else {
                sync::SyncMode::Safe
            };
            let result = sync::manual_sync(
                sync::TransportRef(transport),
                &settings,
                auth.as_ref(),
                mode,
            );
            if result.is_ok() && mode != sync::SyncMode::Status {
                maybe_optimize(config, &settings);
            }
            result
                .map(|text| println!("{text}"))
                .map_err(|error| CliError::Message(format!("sync: {error}")))
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
    };
    if auto_sync {
        match sync::sync_once(
            sync::TransportRef(transport),
            &settings,
            auth.as_ref(),
        ) {
            Ok(outcome) => sync::warn_outcome(&outcome),
            Err(error) => eprintln!("ranki: sync: {error}"),
        }
    }
    result
}

fn scheduler_for(settings: &config::Settings) -> Arc<dyn Scheduler> {
    if settings.scheduler != ranki_model::SchedulerKind::Fsrs {
        return ranki_core::from_settings(settings);
    }
    let params = fsrs_params(settings);
    Arc::new(
        FsrsScheduler::new(FsrsPreset {
            desired_retention: settings.desired_retention,
            ..FsrsPreset::default()
        })
        .with_params(params),
    )
}

fn fsrs_params(settings: &config::Settings) -> FsrsParams {
    if settings.fsrs_params.is_empty() {
        return FsrsParams::default();
    }
    let mut values = settings.fsrs_params.clone();
    apply_decay(&mut values, settings.fsrs_decay);
    FsrsParams::from_slice(&values).unwrap_or_default()
}
fn apply_decay(values: &mut Vec<f32>, decay: f32) {
    match values.len() {
        19 => values.push(decay),
        21 => {
            if let Some(slot) = values.get_mut(20) {
                *slot = decay;
            }
        }
        _ => {}
    }
}

fn maybe_optimize(config: &mut config::Config, settings: &config::Settings) {
    let Ok(now) = now_secs() else {
        return;
    };
    if !settings.collection.exists() {
        return;
    }
    let Ok(collection) = Collection::open(&settings.collection) else {
        eprintln!("ranki: auto-optimize: cannot open the collection");
        return;
    };
    let before = config.clone();
    optimize::maybe_auto_optimize(&collection, config, now);
    if *config != before
        && let Err(error) = config::save(config)
    {
        eprintln!("ranki: config: cannot save: {error}");
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
            rects,
            ellipses,
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
            &shapes(&rects, &ellipses),
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
        | Command::Sync { .. }
        | Command::Mcp => Ok(()),
    }
}

fn open_gui(settings: &config::Settings) -> CliResult<()> {
    let collection = open_or_create(&settings.collection)?;
    let options = settings.clone();
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
        | Command::Sync { .. }
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
            let kind = StockModelKind::from_name(name).ok_or_else(|| {
                CliError::Message(format!("unknown note type: {name}"))
            })?;
            Ok(collection.ensure_model(kind)?.id)
        }
    }
}

/// `--rect`/`--ellipse` coordinates: normalized, comma separated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MaskRect {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

fn parse_rect(value: &str) -> Result<MaskRect, String> {
    let parts = value
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            format!("expected LEFT,TOP,WIDTH,HEIGHT in 0..1, got {value}")
        })?;
    let [left, top, width, height] = parts.as_slice() else {
        return Err(format!(
            "expected LEFT,TOP,WIDTH,HEIGHT in 0..1, got {value}"
        ));
    };
    let inside = |number: &f32| (0.0..=1.0).contains(number);
    if !inside(left)
        || !inside(top)
        || !inside(width)
        || !inside(height)
        || *width <= 0.0
        || *height <= 0.0
    {
        return Err(format!("mask coordinates must stay in 0..1: {value}"));
    }
    Ok(MaskRect {
        left: *left,
        top: *top,
        width: *width,
        height: *height,
    })
}

/// Rectangles first, then ellipses, in the order the flags were given.
fn shapes(rects: &[MaskRect], ellipses: &[MaskRect]) -> Vec<OcclusionMask> {
    let rect = |mask: &MaskRect, shape: MaskShape| OcclusionMask {
        shape,
        rect: NormalizedRect {
            left: mask.left,
            top: mask.top,
            width: mask.width,
            height: mask.height,
        },
    };
    rects
        .iter()
        .map(|mask| rect(mask, MaskShape::Rect))
        .chain(ellipses.iter().map(|mask| rect(mask, MaskShape::Ellipse)))
        .collect()
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
    masks: &[OcclusionMask],
    header: Option<&str>,
    back_extra: Option<&str>,
    comments: Option<&str>,
    tags: &[String],
) -> CliResult<()> {
    let built = occlusion_field(masks);
    let raw = occlusions.join("<br>");
    let occlusion = [built, raw]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("<br>");
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
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use clap::Parser;
    use ranki_sync::MockResponse;
    use ranki_sync::MockTransport;
    use ranki_sync::SyncMethod;

    use super::Cli;
    use super::Command;
    use super::default_collection_path;
    use super::fsrs_params;
    use super::media_dir_for;
    use super::parse_rating;
    use super::run_with_sync;
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
    fn add_image_occlusion_accepts_rects_and_ellipses() {
        let cli = parse(&[
            "ranki",
            "add-image-occlusion",
            "--deck",
            "1",
            "--image",
            "/tmp/diagram.png",
            "--rect",
            "0.1,0.2,0.3,0.4",
            "--ellipse",
            "0.5,0.5,0.2,0.2",
            "--rect",
            "0,0,1,1",
        ]);
        let Some(Command::AddImageOcclusion {
            rects,
            ellipses,
            occlusions,
            ..
        }) = cli.command
        else {
            panic!("wrong command");
        };
        assert_eq!(occlusions.len(), 0);
        assert_eq!(rects.len(), 2);
        assert_eq!(ellipses.len(), 1);
        let masks = super::shapes(&rects, &ellipses);
        assert_eq!(
            super::occlusion_field(&masks),
            "{{c1::image-occlusion:rect:left=0.1:top=0.2:width=0.3:height=0.4}}<br>\
             {{c2::image-occlusion:rect:left=0:top=0:width=1:height=1}}<br>\
             {{c3::image-occlusion:ellipse:left=0.5:top=0.5:width=0.2:height=0.2}}"
        );
        let parsed = ranki_model::parse_image_occlusion_field(
            &super::occlusion_field(&masks),
        );
        assert_eq!(parsed.shapes.len(), 3);
        assert_eq!(parsed.shapes[2].ordinals, [3]);
    }

    #[test]
    fn add_image_occlusion_rejects_out_of_range_rects() {
        assert!(super::parse_rect("0.1,0.2,0.3").is_err());
        assert!(super::parse_rect("0.1,0.2,0.3,1.4").is_err());
        assert!(super::parse_rect("0.1,0.2,0,0.3").is_err());
        assert!(super::parse_rect("left,top,width,height").is_err());
        assert_eq!(
            super::parse_rect(" 0.25 , 0.5, 0.125, 0.25 ").map(|mask| (
                mask.left,
                mask.top,
                mask.width,
                mask.height
            )),
            Ok((0.25, 0.5, 0.125, 0.25))
        );
    }

    #[test]
    fn add_image_occlusion_needs_markers_or_shapes() {
        assert_eq!(super::shapes(&[], &[]).len(), 0);
        assert_eq!(super::occlusion_field(&[]), "");
        assert!(
            Cli::try_parse_from([
                "ranki",
                "add-image-occlusion",
                "--deck",
                "1",
                "--image",
                "/tmp/diagram.png",
            ])
            .is_err()
        );
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

    #[test]
    fn sync_command_parses_escape_hatches() {
        assert!(matches!(
            parse(&["ranki", "sync"]).command,
            Some(Command::Sync {
                status: false,
                full_upload: false,
                full_download: false,
            })
        ));
        assert!(matches!(
            parse(&["ranki", "sync", "--status"]).command,
            Some(Command::Sync { status: true, .. })
        ));
        assert!(matches!(
            parse(&["ranki", "sync", "--full-upload"]).command,
            Some(Command::Sync {
                full_upload: true,
                ..
            })
        ));
        assert!(matches!(
            parse(&["ranki", "sync", "--full-download"]).command,
            Some(Command::Sync {
                full_download: true,
                ..
            })
        ));
    }

    #[test]
    fn cli_sync_command_uses_the_injected_transport() {
        let directory = tempfile::tempdir().unwrap();
        let key_file = directory.path().join("hkey");
        fs::write(&key_file, "test-hkey").unwrap();
        fs::set_permissions(&key_file, fs::Permissions::from_mode(0o600))
            .unwrap();
        let mut config = ranki_model::Config::default();
        config.sync.hkey_file = Some(key_file.to_string_lossy().into_owned());
        let collection = directory.path().join("collection.anki2");
        let cli = parse(&[
            "ranki",
            "--collection",
            collection.to_str().unwrap(),
            "sync",
            "--status",
        ]);
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        )
        .unwrap_or(0);
        let mut transport = MockTransport::new().with_response(
            SyncMethod::Meta,
            MockResponse::ok_json(&serde_json::json!({
                "mod": 1, "scm": 1, "usn": 1, "ts": now, "empty": true,
            }))
            .unwrap(),
        );
        run_with_sync(cli, &mut config, &mut transport).unwrap();
        assert_eq!(transport.request_count(), 1);
        assert_eq!(transport.requests()[0].method, SyncMethod::Meta);
    }

    #[test]
    fn scheduler_uses_configured_fsrs_params_and_decay() {
        let mut settings = ranki_model::Settings::default();
        let params = fsrs_params(&settings);
        assert_eq!(params.decay(), 0.1542);

        settings.fsrs_params = vec![0.2; 21];
        settings.fsrs_decay = 0.35;
        let params = fsrs_params(&settings);
        assert_eq!(params.decay(), 0.35);
        assert_eq!(params.weights().get(20).copied(), Some(0.35));
    }
}
