//! Data model shared by storage, scheduling, import/export and the UI.
//!
//! Field names and numeric values mirror Anki's `collection.anki2` schema so
//! records can be read and written without conversion loss.

pub mod cloze;
pub mod config;
mod id;
pub mod image_occlusion;
pub mod raw;
pub mod stock;
pub use cloze::cloze_markers;
pub use cloze::cloze_ordinal;
pub use cloze::cloze_ordinals;
pub use config::{
    Config, GeneralConfig, SchedulerConfig, SchedulerKind, Settings,
    SyncConfig, SyncSettings, ThemeChoice, UiConfig, media_dir_for,
};
pub use id::next_id;
pub use image_occlusion::parse_image_occlusion_field;
pub use raw::decode_hex;
pub use raw::encode_hex;
use std::collections::BTreeSet;
pub use stock::StockModelKind;
pub use stock::stock_models;

use serde::{Deserialize, Serialize};

/// Anki identifiers are 64-bit integers.
pub type Id = i64;

/// Separator between note fields in `notes.flds`.
pub const FIELD_SEPARATOR: char = '\u{001f}';
pub const DEFAULT_DECK_ID: Id = 1;
pub const DEFAULT_MODEL_ID: Id = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: Id,
    pub guid: String,
    pub model_id: Id,
    /// Seconds since epoch.
    pub modified: i64,
    pub usn: i32,
    pub tags: Vec<String>,
    pub fields: Vec<String>,
    pub sort_field: String,
    pub checksum: i32,
    pub flags: i32,
    pub data: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardType {
    New,
    Learning,
    Review,
    Relearning,
}

impl CardType {
    #[must_use]
    pub const fn code(self) -> i64 {
        match self {
            Self::New => 0,
            Self::Learning => 1,
            Self::Review => 2,
            Self::Relearning => 3,
        }
    }

    /// Unknown values are treated as new, matching Anki's tolerant readers.
    #[must_use]
    pub const fn from_code(code: i64) -> Self {
        match code {
            1 => Self::Learning,
            2 => Self::Review,
            3 => Self::Relearning,
            _ => Self::New,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardQueue {
    New,
    Learning,
    Review,
    DayLearning,
    Suspended,
    Buried,
    Preview,
}

impl CardQueue {
    #[must_use]
    pub const fn code(self) -> i64 {
        match self {
            Self::New => 0,
            Self::Learning => 1,
            Self::Review => 2,
            Self::DayLearning => 3,
            Self::Suspended => -1,
            Self::Buried => -2,
            Self::Preview => -3,
        }
    }

    /// Unknown values are treated as suspended so they are never shown by accident.
    #[must_use]
    pub const fn from_code(code: i64) -> Self {
        match code {
            0 => Self::New,
            1 => Self::Learning,
            2 => Self::Review,
            3 => Self::DayLearning,
            -2 => Self::Buried,
            -3 => Self::Preview,
            _ => Self::Suspended,
        }
    }

    #[must_use]
    pub const fn is_due(self) -> bool {
        matches!(
            self,
            Self::New | Self::Learning | Self::Review | Self::DayLearning
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: Id,
    pub note_id: Id,
    pub deck_id: Id,
    pub ordinal: u16,
    /// Seconds since epoch.
    pub modified: i64,
    pub usn: i32,
    pub card_type: CardType,
    pub queue: CardQueue,
    /// New cards: position. Learning cards: milliseconds. Review cards: day number.
    pub due: i64,
    pub interval: i32,
    pub factor: u32,
    pub reps: u32,
    pub lapses: u32,
    pub left: u32,
    pub original_due: i64,
    pub original_deck_id: Id,
    pub flags: i32,
    pub data: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

impl Rating {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Again => 1,
            Self::Hard => 2,
            Self::Good => 3,
            Self::Easy => 4,
        }
    }

    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Again),
            2 => Some(Self::Hard),
            3 => Some(Self::Good),
            4 => Some(Self::Easy),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevLogKind {
    Learn,
    Review,
    Relearn,
    Filtered,
    Manual,
}

impl RevLogKind {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Learn => 0,
            Self::Review => 1,
            Self::Relearn => 2,
            Self::Filtered => 3,
            Self::Manual => 4,
        }
    }

    #[must_use]
    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Review,
            2 => Self::Relearn,
            3 => Self::Filtered,
            4 => Self::Manual,
            _ => Self::Learn,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevLog {
    pub id: Id,
    pub card_id: Id,
    pub usn: i32,
    pub ease: Rating,
    pub interval: i32,
    pub last_interval: i32,
    pub factor: u32,
    /// Milliseconds spent on the review.
    pub time_ms: u32,
    pub kind: RevLogKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deck {
    pub id: Id,
    pub name: String,
    pub description: String,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize,
)]
pub enum NoteKind {
    #[default]
    Normal,
    Cloze,
}

impl NoteKind {
    /// Anki's `type` value for the note kind.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Normal => 0,
            Self::Cloze => 1,
        }
    }

    /// Unknown values are treated as normal, matching Anki's tolerant readers.
    #[must_use]
    pub const fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Cloze,
            _ => Self::Normal,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardTemplate {
    pub name: String,
    pub question_format: String,
    pub answer_format: String,
    #[serde(default)]
    pub ord: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteModel {
    pub id: Id,
    pub name: String,
    pub fields: Vec<String>,
    pub templates: Vec<CardTemplate>,
    pub css: String,
    #[serde(default)]
    pub kind: NoteKind,
    #[serde(default)]
    pub original_stock_kind: Option<i32>,
    #[serde(default)]
    pub field_tags: Vec<Option<u32>>,
    /// Ordinals parallel to `templates`; missing `ord` values use the position.
    #[serde(default)]
    pub template_ordinals: Vec<u16>,
    /// Raw schema-18 protobuf config kept for export round trips.
    #[serde(default)]
    pub raw_config: Option<Vec<u8>>,
}

impl NoteModel {
    /// True for cloze note types, including Image Occlusion.
    #[must_use]
    pub const fn is_cloze(&self) -> bool {
        matches!(self.kind, NoteKind::Cloze)
    }

    /// True when Anki's original stock kind identifies Image Occlusion.
    #[must_use]
    pub fn is_image_occlusion(&self) -> bool {
        self.original_stock_kind
            == Some(stock::ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION)
    }

    /// Finds the template for a card ordinal.
    ///
    /// Cloze note types use their first template for every ordinal. Older
    /// models without ordinals fall back to the template position.
    #[must_use]
    pub fn template_for(&self, ordinal: u16) -> Option<&CardTemplate> {
        if self.is_cloze() {
            return self.templates.first();
        }
        self.templates
            .iter()
            .find(|template| template.ord == ordinal)
            .or_else(|| self.templates.get(usize::from(ordinal)))
    }

    /// Ordinals of the cards Anki generates for a note with these fields.
    #[must_use]
    pub fn card_ordinals(&self, fields: &[String]) -> Vec<u16> {
        if self.is_cloze() {
            return cloze_ordinals(fields).into_iter().collect();
        }
        let mut used = BTreeSet::new();
        let mut ordinals = self
            .templates
            .iter()
            .enumerate()
            .map(|(position, template)| {
                let declared = self
                    .template_ordinals
                    .get(position)
                    .copied()
                    .unwrap_or(template.ord);
                unique_ordinal(declared, position, &mut used)
            })
            .collect::<Vec<_>>();
        ordinals.sort_unstable();
        ordinals
    }
}
fn unique_ordinal(
    declared: u16,
    position: usize,
    used: &mut BTreeSet<u16>,
) -> u16 {
    if used.insert(declared) {
        return declared;
    }
    let fallback = u16::try_from(position)
        .unwrap_or(u16::MAX)
        .min(u16::MAX.saturating_sub(1));
    let mut ordinal = fallback;
    while used.contains(&ordinal) {
        ordinal = ordinal.saturating_add(1);
    }
    let _ = used.insert(ordinal);
    ordinal
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewNote {
    pub model_id: Id,
    pub deck_id: Id,
    pub fields: Vec<String>,
    pub tags: Vec<String>,
}

/// A card after scheduling, ready to be persisted together with its log entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewDecision {
    pub card: Card,
    pub revlog: RevLog,
}

/// Card counts for every deck in a collection, in deck order.
pub type DeckTotals = Vec<(Id, DeckStats)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeckStats {
    pub new_cards: usize,
    pub learning_cards: usize,
    pub review_cards: usize,
    pub total_cards: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid data: {0}")]
    InvalidData(String),
    #[error("storage failure: {0}")]
    Backend(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

/// Synchronous collection access. Implementations must be atomic per call.
pub trait Store {
    /// Lists all decks in the collection.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn decks(&self) -> Result<Vec<Deck>, StoreError>;

    /// Creates a deck and returns it.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the name is invalid or the write fails.
    fn create_deck(&mut self, name: &str) -> Result<Deck, StoreError>;

    /// Deletes a deck and every deck below it in the `A::B` hierarchy.
    ///
    /// Cards in those decks are deleted with their review logs. Notes are
    /// deleted when no cards remain, and grave rows are written for every
    /// deleted card, note, and deck. Media files are left in place because
    /// Anki's collection schema has no media ownership table and a file can
    /// be shared by notes outside the deleted subtree. The Default deck
    /// (id 1) cannot be deleted.
    ///
    /// Implementations that cannot delete decks return
    /// [`StoreError::Unsupported`].
    ///
    /// # Errors
    /// Returns [`StoreError`] if the deck is missing, is the Default deck,
    /// or the write fails.
    fn remove_deck(&mut self, id: Id) -> Result<(), StoreError> {
        Err(StoreError::Unsupported(format!(
            "remove_deck is not supported for deck {id}"
        )))
    }

    /// Renames a deck and every subdeck, preserving all ids.
    ///
    /// Names use Anki's `A::B` hierarchy. An empty name, an empty name
    /// segment, or a name that collides with another deck is rejected. The
    /// collection's `col.decks` JSON is updated in the same write.
    ///
    /// Implementations that cannot rename decks return
    /// [`StoreError::Unsupported`].
    ///
    /// # Errors
    /// Returns [`StoreError`] if the deck is missing, the name is invalid,
    /// or the write fails.
    fn rename_deck(&mut self, id: Id, name: &str) -> Result<Deck, StoreError> {
        Err(StoreError::Unsupported(format!(
            "rename_deck is not supported for deck {id} to {name}"
        )))
    }

    /// Lists note models with their templates.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn models(&self) -> Result<Vec<NoteModel>, StoreError>;
    /// Installs a stock note type and returns the stored model.
    ///
    /// Returns the collection's existing model when it already has that stock
    /// type, so the call is safe to repeat.
    ///
    /// Implementations that cannot install models return
    /// [`StoreError::Unsupported`].
    ///
    /// # Errors
    /// Returns [`StoreError`] if the write fails.
    fn ensure_model(
        &mut self,
        kind: StockModelKind,
    ) -> Result<NoteModel, StoreError> {
        Err(StoreError::Unsupported(format!(
            "ensure_model is not supported for {}",
            kind.name()
        )))
    }

    /// Finds card ids matching an Anki-style search query.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the query is invalid or the search fails.
    fn find_cards(&self, query: &str) -> Result<Vec<Id>, StoreError>;

    /// Returns cards due in a deck, including subdecks.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    #[expect(
        clippy::too_many_arguments,
        reason = "the store contract fixes these arguments"
    )]
    fn due_cards(
        &self,
        deck_id: Option<Id>,
        limit: usize,
        now_secs: i64,
    ) -> Result<Vec<Card>, StoreError>;

    /// Loads one card by id.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn card(&self, id: Id) -> Result<Option<Card>, StoreError>;

    /// Loads one note by id.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn note(&self, id: Id) -> Result<Option<Note>, StoreError>;

    /// Adds a note and its generated cards.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the note is invalid or the write fails.
    fn add_note(&mut self, note: &NewNote) -> Result<Vec<Id>, StoreError>;

    /// Persists a scheduled review and its log entry atomically.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the card does not exist or the write fails.
    fn apply_review(
        &mut self,
        decision: &ReviewDecision,
    ) -> Result<(), StoreError>;

    /// Reverses one persisted review: deletes the review log and restores the
    /// card row to the snapshot taken before that review.
    ///
    /// Returns `Ok(false)` when the log is already gone, so a retried undo is
    /// not an error. Implementations that cannot reverse writes return
    /// [`StoreError::Unsupported`].
    ///
    /// # Errors
    /// Returns [`StoreError`] if the card is missing or the write fails.
    fn undo_review(
        &mut self,
        before: &Card,
        _revlog_id: Id,
    ) -> Result<bool, StoreError> {
        Err(StoreError::Unsupported(format!(
            "undo_review is not supported for card {}",
            before.id
        )))
    }

    /// Returns every review log, ordered by card and then by log id.
    ///
    /// Implementations without a review history return
    /// [`StoreError::Unsupported`].
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn review_history(&self) -> Result<Vec<RevLog>, StoreError> {
        Err(StoreError::Unsupported(
            "review_history is not supported by this store".to_owned(),
        ))
    }
    /// Counts scheduled cards for each of the next `days` Anki days.
    ///
    /// The first entry is today. It includes overdue cards and learning cards
    /// due before the next day rollover. New cards have no scheduled day and
    /// are excluded.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn due_forecast(
        &self,
        _days: usize,
        _now_secs: i64,
    ) -> Result<Vec<usize>, StoreError> {
        Err(StoreError::Unsupported(
            "due_forecast is not supported by this store".to_owned(),
        ))
    }

    /// Returns card counts for a deck.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn stats(
        &self,
        deck_id: Option<Id>,
        now_secs: i64,
    ) -> Result<DeckStats, StoreError>;

    /// Returns card counts for every deck, each including its subdecks.
    ///
    /// Implementations without a batch query return
    /// [`StoreError::Unsupported`].
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn deck_stats(&self) -> Result<DeckTotals, StoreError> {
        Err(StoreError::Unsupported(
            "deck_stats is not supported by this store".to_owned(),
        ))
    }

    /// Returns the collection modification timestamp in seconds.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn collection_modified(&self) -> Result<i64, StoreError>;

    /// Returns the collection creation time in Unix seconds.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the collection cannot be read.
    fn collection_created(&self) -> Result<i64, StoreError>;
}
