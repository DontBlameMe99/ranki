//! Built-in MCP stdio server over the same store the CLI uses.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "rmcp's tool_handler macro generates async ServerHandler methods"
)]

use std::borrow::Cow;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::MutexGuard;

use ranki_core::FsrsScheduler;
use ranki_model::Rating;
use ranki_model::Store;
use ranki_model::{DataPaths, Id};
use ranki_storage::Collection;
use rmcp::ErrorData;
use rmcp::Json;
use rmcp::ServiceExt;
use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResponse;
use rmcp::model::CallToolResult;
use rmcp::model::ContentBlock;
use rmcp::schemars;
use rmcp::schemars::JsonSchema;
use rmcp::tool;
use rmcp::tool_router;
use rmcp::transport::stdio;
use serde::Deserialize;
use serde::Serialize;
use tokio::runtime::Builder;

use crate::CliError;
use crate::CliResult;
use crate::error::{ErrorCode, InterfaceError};

const MAX_DUE_LIMIT: usize = 200;

/// Protocol errors stay protocol errors; operational failures become tool errors.
enum ToolError {
    Protocol(ErrorData),
    Call(InterfaceError),
}

impl ToolError {
    fn call(message: impl Into<String>) -> Self {
        Self::Call(InterfaceError::message(message))
    }
}

impl From<ErrorData> for ToolError {
    fn from(error: ErrorData) -> Self {
        Self::Protocol(error)
    }
}

impl From<ranki_model::StoreError> for ToolError {
    fn from(error: ranki_model::StoreError) -> Self {
        Self::Call(error.into())
    }
}

impl From<CliError> for ToolError {
    fn from(error: CliError) -> Self {
        Self::Call(error.into())
    }
}

impl From<ranki_format::FormatError> for ToolError {
    fn from(error: ranki_format::FormatError) -> Self {
        Self::Call(error.into())
    }
}

impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        match self {
            Self::Protocol(error) => Err(error),
            Self::Call(error) => {
                let (code, message) = error.into_parts();
                let text = tool_message(code, message);
                Ok(CallToolResult::error(vec![ContentBlock::text(text)]).into())
            }
        }
    }
}

fn tool_message(code: ErrorCode, message: String) -> String {
    if code == ErrorCode::Message {
        message
    } else {
        format!("[{}] {message}", code.as_str())
    }
}

type ToolOutcome<T> = Result<Json<T>, ToolError>;

fn invalid(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(Cow::Owned(message.into()), None)
}

fn require_confirm(confirm: bool) -> Result<(), ToolError> {
    if !confirm {
        return Err(ToolError::call(
            "this tool requires confirm: true to make changes",
        ));
    }
    Ok(())
}

fn validate_limit(limit: usize) -> Result<usize, ErrorData> {
    if limit == 0 || limit > MAX_DUE_LIMIT {
        return Err(invalid("limit must be between 1 and 200"));
    }
    Ok(limit)
}

fn validate_query(query: &str) -> Result<(), ErrorData> {
    if query.trim().is_empty() {
        return Err(invalid("query must not be empty"));
    }
    Ok(())
}

fn validate_deck_name(name: &str) -> Result<&str, ErrorData> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.split("::").any(|part| part.trim().is_empty())
    {
        return Err(invalid("invalid deck name"));
    }
    Ok(trimmed)
}

fn validate_path(raw: &str) -> Result<PathBuf, ErrorData> {
    let path = Path::new(raw);
    if raw.is_empty() || !path.is_relative() {
        return Err(invalid(
            "path must be a relative path inside the collection directory",
        ));
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => result.push(part),
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => {
                return Err(invalid("path must not contain '.' or '..'"));
            }
        }
    }
    if result.file_name().is_none() {
        return Err(invalid("path must name a file"));
    }
    Ok(result)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListDecksInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateDeckInput {
    #[schemars(length(min = 1))]
    name: String,
    #[schemars(extend("const" = true))]
    confirm: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameDeckInput {
    id: Id,
    #[schemars(length(min = 1))]
    name: String,
    #[schemars(extend("const" = true))]
    confirm: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveDeckInput {
    id: Id,
    #[schemars(extend("const" = true))]
    confirm: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DueCardsInput {
    deck_id: Option<Id>,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 200))]
    limit: usize,
    #[schemars(description = "Unix seconds; defaults to server clock")]
    now_secs: Option<i64>,
}

const fn default_limit() -> usize {
    20
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindCardsInput {
    #[schemars(length(min = 1))]
    query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetCardInput {
    id: Id,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddNoteInput {
    model_id: Id,
    deck_id: Id,
    #[schemars(length(min = 1))]
    fields: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[schemars(extend("const" = true))]
    confirm: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AnswerCardInput {
    card_id: Id,
    rating: RatingInput,
    #[serde(default)]
    #[schemars(default, range(min = 0))]
    elapsed_ms: u64,
    #[schemars(extend("const" = true))]
    confirm: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum RatingInput {
    Again,
    Hard,
    Good,
    Easy,
}

impl From<RatingInput> for Rating {
    fn from(value: RatingInput) -> Self {
        match value {
            RatingInput::Again => Self::Again,
            RatingInput::Hard => Self::Hard,
            RatingInput::Good => Self::Good,
            RatingInput::Easy => Self::Easy,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatsInput {
    deck_id: Option<Id>,
    now_secs: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PackageInput {
    path: String,
    #[schemars(extend("const" = true))]
    confirm: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DeckView {
    id: Id,
    name: String,
    description: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CardTypeView {
    New,
    Learning,
    Review,
    Relearning,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CardQueueView {
    New,
    Learning,
    Review,
    DayLearning,
    Suspended,
    Buried,
    Preview,
}

#[derive(Debug, Serialize, JsonSchema)]
struct CardView {
    id: Id,
    note_id: Id,
    deck_id: Id,
    ordinal: u16,
    modified: i64,
    usn: i32,
    card_type: CardTypeView,
    queue: CardQueueView,
    due: i64,
    interval: i32,
    factor: u32,
    reps: u32,
    lapses: u32,
    left: u32,
    original_due: i64,
    original_deck_id: Id,
    flags: i32,
    data: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct NoteView {
    id: Id,
    guid: String,
    model_id: Id,
    modified: i64,
    usn: i32,
    tags: Vec<String>,
    fields: Vec<String>,
    sort_field: String,
    checksum: i32,
    flags: i32,
    data: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum RatingView {
    Again,
    Hard,
    Good,
    Easy,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum RevLogKindView {
    Learn,
    Review,
    Relearn,
    Filtered,
    Manual,
}

#[derive(Debug, Serialize, JsonSchema)]
struct RevLogView {
    id: Id,
    card_id: Id,
    usn: i32,
    ease: RatingView,
    interval: i32,
    last_interval: i32,
    factor: u32,
    time_ms: u32,
    kind: RevLogKindView,
}

#[expect(
    clippy::struct_field_names,
    reason = "the MCP schema fixes these field names"
)]
#[derive(Debug, Serialize, JsonSchema)]
struct StatsView {
    new_cards: usize,
    learning_cards: usize,
    review_cards: usize,
    total_cards: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DeckList {
    decks: Vec<DeckView>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DueCards {
    cards: Vec<CardView>,
    now_secs: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct CardIds {
    card_ids: Vec<Id>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct CardAndNote {
    card: Option<CardView>,
    note: Option<NoteView>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct NoteCards {
    card_ids: Vec<Id>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ReviewResult {
    card: CardView,
    revlog: RevLogView,
}

#[derive(Debug, Serialize, JsonSchema)]
struct StatsResult {
    stats: StatsView,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ImportResult {
    path: String,
    notes_added: usize,
    cards_added: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ExportResult {
    path: String,
    bytes: u64,
}
fn deck_view(deck: &ranki_model::Deck) -> DeckView {
    DeckView {
        id: deck.id,
        name: deck.name.clone(),
        description: deck.description.clone(),
    }
}

fn card_view(card: &ranki_model::Card) -> CardView {
    let (card_type, initial_queue) = match (card.card_type, card.queue) {
        (ranki_model::CardType::New, _) => {
            (CardTypeView::New, CardQueueView::New)
        }
        (
            ranki_model::CardType::Learning,
            ranki_model::CardQueue::DayLearning,
        ) => (CardTypeView::Learning, CardQueueView::DayLearning),
        (ranki_model::CardType::Learning, _) => {
            (CardTypeView::Learning, CardQueueView::Learning)
        }
        (ranki_model::CardType::Review, _) => {
            (CardTypeView::Review, CardQueueView::Review)
        }
        (ranki_model::CardType::Relearning, _) => {
            (CardTypeView::Relearning, CardQueueView::DayLearning)
        }
    };
    let queue_view = match card.queue {
        ranki_model::CardQueue::Suspended => CardQueueView::Suspended,
        ranki_model::CardQueue::Buried
        | ranki_model::CardQueue::ManuallyBuried => CardQueueView::Buried,
        ranki_model::CardQueue::Preview => CardQueueView::Preview,
        ranki_model::CardQueue::New
        | ranki_model::CardQueue::Learning
        | ranki_model::CardQueue::Review
        | ranki_model::CardQueue::DayLearning => initial_queue,
    };
    CardView {
        id: card.id,
        note_id: card.note_id,
        deck_id: card.deck_id,
        ordinal: card.ordinal,
        modified: card.modified,
        usn: card.usn,
        card_type,
        queue: queue_view,
        due: card.due,
        interval: card.interval,
        factor: card.factor,
        reps: card.reps,
        lapses: card.lapses,
        left: card.left,
        original_due: card.original_due,
        original_deck_id: card.original_deck_id,
        flags: card.flags,
        data: card.data.clone(),
    }
}

fn note_view(note: &ranki_model::Note) -> NoteView {
    NoteView {
        id: note.id,
        guid: note.guid.clone(),
        model_id: note.model_id,
        modified: note.modified,
        usn: note.usn,
        tags: note.tags.clone(),
        fields: note.fields.clone(),
        sort_field: note.sort_field.clone(),
        checksum: note.checksum,
        flags: note.flags,
        data: note.data.clone(),
    }
}

const fn rating_view(rating: Rating) -> RatingView {
    match rating {
        Rating::Again => RatingView::Again,
        Rating::Hard => RatingView::Hard,
        Rating::Good => RatingView::Good,
        Rating::Easy => RatingView::Easy,
    }
}

const fn kind_view(kind: ranki_model::RevLogKind) -> RevLogKindView {
    match kind {
        ranki_model::RevLogKind::Learn => RevLogKindView::Learn,
        ranki_model::RevLogKind::Review => RevLogKindView::Review,
        ranki_model::RevLogKind::Relearn => RevLogKindView::Relearn,
        ranki_model::RevLogKind::Filtered => RevLogKindView::Filtered,
        ranki_model::RevLogKind::Manual => RevLogKindView::Manual,
    }
}

const fn revlog_view(log: &ranki_model::RevLog) -> RevLogView {
    RevLogView {
        id: log.id,
        card_id: log.card_id,
        usn: log.usn,
        ease: rating_view(log.ease),
        interval: log.interval,
        last_interval: log.last_interval,
        factor: log.factor,
        time_ms: log.time_ms,
        kind: kind_view(log.kind),
    }
}

struct RankiMcp {
    collection: Mutex<Collection>,
    paths: DataPaths,
    root: PathBuf,
    scheduler: FsrsScheduler,
}

impl RankiMcp {
    fn new(
        collection: Collection,
        paths: DataPaths,
        scheduler: FsrsScheduler,
    ) -> Self {
        let root = collection
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Self {
            collection: Mutex::new(collection),
            paths,
            root,
            scheduler,
        }
    }

    fn store(&self) -> Result<MutexGuard<'_, Collection>, ToolError> {
        self.collection
            .lock()
            .map_err(|_| ToolError::call("collection lock poisoned"))
    }

    fn file(&self, raw: &str) -> Result<PathBuf, ToolError> {
        let relative = validate_path(raw)?;
        let candidate = self.root.join(relative);
        let parent = candidate
            .parent()
            .ok_or_else(|| ToolError::call("path has no parent directory"))?;
        let canonical_parent = parent.canonicalize().map_err(|error| {
            ToolError::call(format!("cannot resolve path: {error}"))
        })?;
        let name = candidate
            .file_name()
            .ok_or_else(|| ToolError::call("path must name a file"))?;
        let resolved = canonical_parent.join(name);
        if !resolved.starts_with(&self.root) {
            return Err(ToolError::call(
                "path is outside the collection directory",
            ));
        }
        Ok(resolved)
    }

    fn media_dir(&self) -> PathBuf {
        self.paths.media_dir.clone()
    }
}

#[tool_router(server_handler)]
impl RankiMcp {
    #[tool(
        description = "List decks in the active collection",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn list_decks(
        &self,
        Parameters(_): Parameters<ListDecksInput>,
    ) -> ToolOutcome<DeckList> {
        let decks = self.store()?.decks()?;
        Ok(Json(DeckList {
            decks: decks.iter().map(deck_view).collect(),
        }))
    }

    #[tool(
        description = "Create a deck in the active collection",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    fn create_deck(
        &self,
        Parameters(input): Parameters<CreateDeckInput>,
    ) -> ToolOutcome<DeckView> {
        require_confirm(input.confirm)?;
        let name = validate_deck_name(&input.name)?;
        let deck = self.store()?.create_deck(name)?;
        Ok(Json(deck_view(&deck)))
    }

    #[tool(
        description = "Rename a deck and its subdecks",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    fn rename_deck(
        &self,
        Parameters(input): Parameters<RenameDeckInput>,
    ) -> ToolOutcome<DeckView> {
        require_confirm(input.confirm)?;
        let name = validate_deck_name(&input.name)?;
        let deck = self.store()?.rename_deck(input.id, name)?;
        Ok(Json(deck_view(&deck)))
    }

    #[tool(
        description = "Remove a deck, its subdecks, and their cards",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    fn remove_deck(
        &self,
        Parameters(input): Parameters<RemoveDeckInput>,
    ) -> ToolOutcome<DeckView> {
        require_confirm(input.confirm)?;
        let mut store = self.store()?;
        let deck = store
            .decks()?
            .into_iter()
            .find(|deck| deck.id == input.id)
            .ok_or_else(|| {
                ToolError::from(ranki_model::StoreError::NotFound(format!(
                    "deck {}",
                    input.id
                )))
            })?;
        store.remove_deck(input.id)?;
        Ok(Json(deck_view(&deck)))
    }

    #[tool(
        description = "List cards that are due",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn due_cards(
        &self,
        Parameters(input): Parameters<DueCardsInput>,
    ) -> ToolOutcome<DueCards> {
        let limit = validate_limit(input.limit)?;
        let now = match input.now_secs {
            Some(value) => value,
            None => crate::now_secs()?,
        };
        let cards = self.store()?.due_cards(input.deck_id, limit, now)?;
        Ok(Json(DueCards {
            cards: cards.iter().map(card_view).collect(),
            now_secs: now,
        }))
    }

    #[tool(
        description = "Find card ids with an Anki search query",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn find_cards(
        &self,
        Parameters(input): Parameters<FindCardsInput>,
    ) -> ToolOutcome<CardIds> {
        validate_query(&input.query)?;
        let card_ids = self.store()?.find_cards(&input.query)?;
        Ok(Json(CardIds { card_ids }))
    }

    #[tool(
        description = "Read one card and its note",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn get_card(
        &self,
        Parameters(input): Parameters<GetCardInput>,
    ) -> ToolOutcome<CardAndNote> {
        let store = self.store()?;
        let card = store.card(input.id)?;
        let note = match &card {
            Some(existing) => store.note(existing.note_id)?,
            None => None,
        };
        Ok(Json(CardAndNote {
            card: card.as_ref().map(card_view),
            note: note.as_ref().map(note_view),
        }))
    }

    #[tool(
        description = "Add a note and its generated cards",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    fn add_note(
        &self,
        Parameters(input): Parameters<AddNoteInput>,
    ) -> ToolOutcome<NoteCards> {
        require_confirm(input.confirm)?;
        if input.fields.is_empty() || input.fields.iter().any(String::is_empty)
        {
            return Err(invalid(
                "fields must contain at least one non-empty value",
            )
            .into());
        }
        let note = ranki_model::NewNote {
            model_id: input.model_id,
            deck_id: input.deck_id,
            fields: input.fields,
            tags: input.tags,
        };
        let card_ids = self.store()?.add_note(&note)?;
        Ok(Json(NoteCards { card_ids }))
    }

    #[tool(
        description = "Answer a card and record a review",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    fn answer_card(
        &self,
        Parameters(input): Parameters<AnswerCardInput>,
    ) -> ToolOutcome<ReviewResult> {
        require_confirm(input.confirm)?;
        let now = crate::now_secs()?;
        let mut store = self.store()?;
        let card = store.card(input.card_id)?.ok_or_else(|| {
            ToolError::call(format!("card not found: {}", input.card_id))
        })?;
        let collection_start_secs = store.collection_created()?;
        let mut decision = self.scheduler.schedule(
            &card,
            Rating::from(input.rating),
            now,
            collection_start_secs,
        );
        decision.revlog.time_ms =
            u32::try_from(input.elapsed_ms).unwrap_or(u32::MAX);
        store.apply_review(&decision)?;
        Ok(Json(ReviewResult {
            card: card_view(&decision.card),
            revlog: revlog_view(&decision.revlog),
        }))
    }

    #[tool(
        description = "Return card counts for a deck",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn stats(
        &self,
        Parameters(input): Parameters<StatsInput>,
    ) -> ToolOutcome<StatsResult> {
        let now = match input.now_secs {
            Some(value) => value,
            None => crate::now_secs()?,
        };
        let stats = self.store()?.stats(input.deck_id, now)?;
        Ok(Json(StatsResult {
            stats: StatsView {
                new_cards: stats.new_cards,
                learning_cards: stats.learning_cards,
                review_cards: stats.review_cards,
                total_cards: stats.total_cards,
            },
        }))
    }

    #[tool(
        description = "Import an Anki package from the collection directory",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    fn import_apkg(
        &self,
        Parameters(input): Parameters<PackageInput>,
    ) -> ToolOutcome<ImportResult> {
        require_confirm(input.confirm)?;
        let path = self.file(&input.path)?;
        if !path.is_file() {
            return Err(ToolError::call(format!(
                "package not found: {}",
                input.path
            )));
        }
        self.store()?.checkpoint()?;
        let report = ranki_format::import_apkg(
            &path,
            &self.paths.collection,
            &self.media_dir(),
        )?;
        Ok(Json(ImportResult {
            path: input.path,
            notes_added: report.notes,
            cards_added: report.cards,
        }))
    }

    #[tool(
        description = "Export the collection to a new Anki package",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    fn export_apkg(
        &self,
        Parameters(input): Parameters<PackageInput>,
    ) -> ToolOutcome<ExportResult> {
        require_confirm(input.confirm)?;
        let path = self.file(&input.path)?;
        if path.exists() {
            return Err(ToolError::call(format!(
                "export target already exists: {}",
                input.path
            )));
        }
        {
            let store = self.store()?;
            store.checkpoint()?;
        }
        ranki_format::export_apkg(
            &self.paths.collection,
            &self.media_dir(),
            &path,
        )?;
        let bytes = fs::metadata(&path)
            .map_err(|error| ToolError::call(error.to_string()))?
            .len();
        Ok(Json(ExportResult {
            path: input.path,
            bytes,
        }))
    }
}

pub fn collection_path(paths: &DataPaths) -> CliResult<PathBuf> {
    let path = paths.collection.clone();
    fs::canonicalize(&path).map_err(|error| {
        CliError::Message(format!(
            "collection not found: {}: {error}",
            path.display()
        ))
    })
}

pub fn serve(paths: &DataPaths, scheduler: FsrsScheduler) -> CliResult<()> {
    let path = collection_path(paths)?;
    let resolved_paths = DataPaths {
        collection: path.clone(),
        ..paths.clone()
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| CliError::Message(error.to_string()))?;
    runtime.block_on(async move {
        let collection = Collection::open(&path)?;
        let server = RankiMcp::new(collection, resolved_paths, scheduler);
        let service = server.serve(stdio()).await.map_err(|error| {
            CliError::Message(format!("mcp server: {error}"))
        })?;
        service.waiting().await.map_err(|error| {
            CliError::Message(format!("mcp server: {error}"))
        })?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::IntoCallToolResult;
    use rmcp::schemars::schema_for;
    use serde_json::json;
    use std::path::PathBuf;

    use super::AddNoteInput;
    use super::AnswerCardInput;
    use super::CreateDeckInput;
    use super::DueCardsInput;
    use super::FindCardsInput;
    use super::GetCardInput;
    use super::ListDecksInput;
    use super::PackageInput;
    use super::RankiMcp;
    use super::RatingInput;
    use super::RemoveDeckInput;
    use super::RenameDeckInput;
    use super::StatsInput;
    use super::ToolError;
    use super::ToolOutcome;
    use super::validate_deck_name;
    use super::validate_limit;
    use super::validate_path;
    use super::validate_query;
    use crate::error::ErrorCode;
    use ranki_storage::Collection;
    use rmcp::handler::server::wrapper::Parameters;
    use tempfile::TempDir;

    #[test]
    fn tool_errors_keep_the_source_error_code() {
        let error = ToolError::from(ranki_model::StoreError::NotFound(
            String::from("card"),
        ));
        let ToolError::Call(mapped) = error else {
            panic!("store failure must be a tool call error");
        };
        assert_eq!(mapped.code(), ErrorCode::NotFound);
    }

    #[test]
    fn input_schemas_close_unknown_fields() {
        for schema in [
            schema_for!(ListDecksInput),
            schema_for!(DueCardsInput),
            schema_for!(FindCardsInput),
            schema_for!(AddNoteInput),
            schema_for!(CreateDeckInput),
            schema_for!(RenameDeckInput),
            schema_for!(RemoveDeckInput),
            schema_for!(AnswerCardInput),
            schema_for!(PackageInput),
        ] {
            let value = serde_json::to_value(schema).unwrap();
            assert_eq!(value["additionalProperties"], json!(false));
            assert_eq!(value["type"], json!("object"));
        }
    }

    #[test]
    fn write_tools_require_const_true() {
        for schema in [
            schema_for!(AddNoteInput),
            schema_for!(CreateDeckInput),
            schema_for!(RenameDeckInput),
            schema_for!(RemoveDeckInput),
            schema_for!(AnswerCardInput),
            schema_for!(PackageInput),
        ] {
            let value = serde_json::to_value(schema).unwrap();
            assert_eq!(value["properties"]["confirm"]["const"], json!(true));
            assert!(
                value["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("confirm"))
            );
        }
    }

    #[test]
    fn query_and_limit_schemas_match_the_design() {
        let value = serde_json::to_value(schema_for!(FindCardsInput)).unwrap();
        assert_eq!(value["properties"]["query"]["minLength"], json!(1));
        let value = serde_json::to_value(schema_for!(DueCardsInput)).unwrap();
        assert_eq!(value["properties"]["limit"]["minimum"], json!(1));
        assert_eq!(value["properties"]["limit"]["maximum"], json!(200));
        assert_eq!(value["properties"]["limit"]["default"], json!(20));
    }

    #[test]
    fn limit_validation_rejects_zero_and_oversized_values() {
        assert_eq!(validate_limit(20).unwrap(), 20);
        assert!(validate_limit(0).is_err());
        assert!(validate_limit(201).is_err());
    }

    #[test]
    fn query_validation_rejects_blank_strings() {
        assert!(validate_query("deck:Default").is_ok());
        assert!(validate_query("  ").is_err());
    }

    #[test]
    fn unknown_fields_fail_deserialization() {
        let result = serde_json::from_value::<AddNoteInput>(json!({
            "model_id": 1, "deck_id": 1, "fields": ["a"], "confirm": true, "extra": 1,
        }));
        assert!(result.is_err());
    }

    #[test]
    fn false_confirmation_deserializes_but_is_rejected_by_the_tool() {
        let parsed: AddNoteInput = serde_json::from_value(json!({
            "model_id": 1, "deck_id": 1, "fields": ["a"], "confirm": false,
        }))
        .unwrap();
        assert!(!parsed.confirm);
    }

    #[test]
    fn rating_enum_is_lowercase() {
        let parsed: AnswerCardInput = serde_json::from_value(json!({
            "card_id": 1, "rating": "good", "confirm": true,
        }))
        .unwrap();
        assert_eq!(serde_json::to_value(parsed.rating).unwrap(), json!("good"));
        assert!(
            serde_json::from_value::<AnswerCardInput>(json!({
                "card_id": 1, "rating": "very-good", "confirm": true,
            }))
            .is_err()
        );
    }

    #[test]
    fn path_validation_stays_inside_the_collection_directory() {
        assert_eq!(
            validate_path("backup.apkg").unwrap(),
            PathBuf::from("backup.apkg")
        );
        assert!(validate_path("../other.apkg").is_err());
        assert!(validate_path("/tmp/other.apkg").is_err());
        assert!(validate_path("").is_err());
        assert!(validate_path(".").is_err());
    }
    fn mcp_server() -> (TempDir, RankiMcp) {
        let directory = tempfile::tempdir().unwrap();
        let paths = ranki_model::DataPaths::new(directory.path());
        let collection = Collection::create(&paths.collection).unwrap();
        let scheduler =
            ranki_core::from_settings(&ranki_model::Settings::default());
        let server = RankiMcp::new(collection, paths, scheduler);
        (directory, server)
    }

    fn ok<T>(result: ToolOutcome<T>) -> T {
        result
            .map(|value| value.0)
            .ok()
            .unwrap_or_else(|| panic!("deck tool failed"))
    }

    #[test]
    fn deck_name_validation_rejects_blank_and_broken_hierarchies() {
        assert_eq!(validate_deck_name(" A::B ").unwrap(), "A::B");
        assert!(validate_deck_name("").is_err());
        assert!(validate_deck_name("A::").is_err());
        assert!(validate_deck_name("::B").is_err());
    }

    #[test]
    fn deck_tools_create_rename_and_remove() {
        let (_directory, server) = mcp_server();
        let created = ok(server.create_deck(Parameters(CreateDeckInput {
            name: "Study::Languages".into(),
            confirm: true,
        })));
        assert_eq!(created.name, "Study::Languages");

        let renamed = ok(server.rename_deck(Parameters(RenameDeckInput {
            id: created.id,
            name: "Learning".into(),
            confirm: true,
        })));
        assert_eq!(renamed.id, created.id);
        assert_eq!(renamed.name, "Learning");

        let rejected = server
            .remove_deck(Parameters(RemoveDeckInput {
                id: created.id,
                confirm: false,
            }))
            .err()
            .unwrap_or_else(|| panic!("unconfirmed removal must fail"));
        let ToolError::Call(error) = rejected else {
            panic!("unconfirmed removal must be a tool error");
        };
        assert_eq!(error.code(), ErrorCode::Message);

        let removed = ok(server.remove_deck(Parameters(RemoveDeckInput {
            id: created.id,
            confirm: true,
        })));
        assert_eq!(removed.id, created.id);
        let decks = ok(server.list_decks(Parameters(ListDecksInput {}))).decks;
        assert!(!decks.iter().any(|deck| deck.id == created.id));
    }

    #[test]
    fn note_tools_create_read_answer_and_report_cards() {
        let (_directory, server) = mcp_server();
        let added = ok(server.add_note(Parameters(AddNoteInput {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Front"), String::from("Back")],
            tags: vec![String::from("reviewed")],
            confirm: true,
        })));
        assert_eq!(added.card_ids.len(), 1);
        let card_id = added.card_ids[0];

        let found = ok(server.find_cards(Parameters(FindCardsInput {
            query: String::from("Front"),
        })));
        assert_eq!(found.card_ids, vec![card_id]);

        let due = ok(server.due_cards(Parameters(DueCardsInput {
            deck_id: None,
            limit: 20,
            now_secs: Some(1_000_000),
        })));
        assert_eq!(due.now_secs, 1_000_000);
        assert_eq!(due.cards[0].id, card_id);

        let read =
            ok(server.get_card(Parameters(GetCardInput { id: card_id })));
        let note = read.note.unwrap();
        assert_eq!(note.fields, vec!["Front", "Back"]);
        assert_eq!(note.tags, vec!["reviewed"]);
        assert_eq!(read.card.unwrap().id, card_id);

        let reviewed = ok(server.answer_card(Parameters(AnswerCardInput {
            card_id,
            rating: RatingInput::Good,
            elapsed_ms: 1234,
            confirm: true,
        })));
        assert_eq!(reviewed.card.id, card_id);
        assert_eq!(
            serde_json::to_value(reviewed.revlog.ease).unwrap(),
            json!("good")
        );
        assert_eq!(reviewed.revlog.time_ms, 1234);

        let stats = ok(server.stats(Parameters(StatsInput {
            deck_id: None,
            now_secs: Some(1_000_000),
        })));
        assert_eq!(stats.stats.total_cards, 1);
        assert_eq!(stats.stats.new_cards, 0);
    }

    #[test]
    fn write_tools_reject_invalid_arguments_without_writing() {
        let (_directory, server) = mcp_server();
        let refused = server
            .add_note(Parameters(AddNoteInput {
                model_id: 1,
                deck_id: 1,
                fields: vec![String::from("Front")],
                tags: vec![],
                confirm: false,
            }))
            .err()
            .unwrap();
        let ToolError::Call(error) = refused else {
            panic!("unconfirmed writes must be tool errors");
        };
        assert_eq!(error.code(), ErrorCode::Message);

        assert!(
            server
                .add_note(Parameters(AddNoteInput {
                    model_id: 1,
                    deck_id: 1,
                    fields: vec![String::new()],
                    tags: vec![],
                    confirm: true,
                }))
                .is_err()
        );
        assert!(
            server
                .find_cards(Parameters(FindCardsInput {
                    query: String::new(),
                }))
                .is_err()
        );
        assert!(
            server
                .due_cards(Parameters(DueCardsInput {
                    deck_id: None,
                    limit: 0,
                    now_secs: None,
                }))
                .is_err()
        );

        let missing = server
            .answer_card(Parameters(AnswerCardInput {
                card_id: 999,
                rating: RatingInput::Good,
                elapsed_ms: 0,
                confirm: true,
            }))
            .err()
            .unwrap();
        let ToolError::Call(error) = missing else {
            panic!("missing cards must be tool errors");
        };
        assert!(error.to_string().contains("card not found"));
    }

    #[test]
    fn package_tools_export_and_import_inside_the_collection_directory() {
        let (directory, server) = mcp_server();
        ok(server.add_note(Parameters(AddNoteInput {
            model_id: 1,
            deck_id: 1,
            fields: vec![String::from("Front"), String::from("Back")],
            tags: vec![],
            confirm: true,
        })));
        let export = PackageInput {
            path: String::from("backup.apkg"),
            confirm: true,
        };
        let exported = ok(server.export_apkg(Parameters(PackageInput {
            path: export.path.clone(),
            confirm: true,
        })));
        assert!(exported.bytes > 0);
        assert!(directory.path().join("backup.apkg").is_file());
        assert!(server.export_apkg(Parameters(export)).err().is_some());

        let path = directory.path().join("other.anki2");
        let collection = Collection::create(&path).unwrap();
        let paths = ranki_model::DataPaths {
            collection: path,
            media_dir: directory.path().join("collection.media"),
            sync_state: directory.path().join("ranki-sync.json"),
        };
        let scheduler =
            ranki_core::from_settings(&ranki_model::Settings::default());
        let other = RankiMcp::new(collection, paths, scheduler);
        let imported = ok(other.import_apkg(Parameters(PackageInput {
            path: String::from("backup.apkg"),
            confirm: true,
        })));
        assert_eq!(imported.notes_added, 1);
        assert_eq!(imported.cards_added, 1);
        let found = ok(other.find_cards(Parameters(FindCardsInput {
            query: String::from("Front"),
        })));
        assert_eq!(found.card_ids.len(), 1);

        let missing = other
            .import_apkg(Parameters(PackageInput {
                path: String::from("missing.apkg"),
                confirm: true,
            }))
            .err()
            .unwrap();
        let ToolError::Call(error) = missing else {
            panic!("missing packages must be tool errors");
        };
        assert!(error.to_string().contains("package not found"));
    }
    #[test]
    fn protocol_errors_and_call_errors_stay_distinct() {
        let protocol = ToolError::from(super::invalid("bad input"));
        assert!(protocol.into_call_tool_result().is_err());

        let call = ToolError::from(crate::CliError::Store(
            ranki_model::StoreError::NotFound(String::from("card")),
        ));
        let response = call.into_call_tool_result().unwrap();
        let super::CallToolResponse::Complete(result) = response else {
            panic!("call errors must be complete tool responses");
        };
        let response = serde_json::to_value(result).unwrap();
        assert_eq!(response["isError"], json!(true));
        assert!(
            response["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("[not_found]")
        );

        let format = ToolError::from(ranki_format::FormatError::new("bad"));
        let ToolError::Call(mapped) = format else {
            panic!("format failures must be tool errors");
        };
        assert_eq!(mapped.code(), ErrorCode::Format);

        let plain = ToolError::call("plain message");
        let response = plain.into_call_tool_result().unwrap();
        let super::CallToolResponse::Complete(result) = response else {
            panic!("plain errors must be complete tool responses");
        };
        let response = serde_json::to_value(result).unwrap();
        assert_eq!(response["content"][0]["text"], json!("plain message"));
    }
    #[test]
    fn collection_path_resolves_existing_files_and_reports_missing_ones() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("collection.anki2");
        fs::write(&path, b"").unwrap();
        let paths = ranki_model::DataPaths {
            collection: path.clone(),
            media_dir: directory.path().join("collection.media"),
            sync_state: directory.path().join("ranki-sync.json"),
        };
        assert_eq!(
            super::collection_path(&paths).unwrap(),
            fs::canonicalize(&path).unwrap()
        );

        let missing = ranki_model::DataPaths {
            collection: directory.path().join("missing.anki2"),
            ..paths
        };
        assert!(super::collection_path(&missing).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn package_paths_work_through_a_symlinked_app_directory() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real");
        fs::create_dir_all(&real).unwrap();
        let collection_path = real.join("collection.anki2");
        let collection = Collection::create(&collection_path).unwrap();
        let link = directory.path().join("link");
        symlink(&real, &link).unwrap();
        let paths = ranki_model::DataPaths {
            collection: link.join("collection.anki2"),
            media_dir: link.join("collection.media"),
            sync_state: link.join("ranki-sync.json"),
        };
        let scheduler =
            ranki_core::from_settings(&ranki_model::Settings::default());
        let server = RankiMcp::new(collection, paths, scheduler);

        assert!(
            server.file("backup.apkg").is_ok(),
            "package path was rejected through the symlinked app directory"
        );
    }
}
