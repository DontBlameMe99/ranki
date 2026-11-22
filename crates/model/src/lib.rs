//! Data model shared by storage, scheduling, import/export and the UI.
//!
//! Field names and numeric values mirror Anki's `collection.anki2` schema so
//! records can be read and written without conversion loss.

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardTemplate {
    pub name: String,
    pub question_format: String,
    pub answer_format: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteModel {
    pub id: Id,
    pub name: String,
    pub fields: Vec<String>,
    pub templates: Vec<CardTemplate>,
    pub css: String,
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

    /// Lists note models with their templates.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn models(&self) -> Result<Vec<NoteModel>, StoreError>;

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

    /// Returns card counts for a deck.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn stats(
        &self,
        deck_id: Option<Id>,
        now_secs: i64,
    ) -> Result<DeckStats, StoreError>;

    /// Returns the collection modification timestamp in seconds.
    ///
    /// # Errors
    /// Returns [`StoreError`] if the collection cannot be read.
    fn collection_modified(&self) -> Result<i64, StoreError>;
}
