//! Store-backed state for the touch-first mobile screens.

#![expect(
    clippy::excessive_nesting,
    clippy::too_many_arguments,
    reason = "store loading and synchronous sync keep their inputs explicit"
)]

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::env;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ranki_core::{FsrsScheduler, from_settings};
use ranki_model::{
    Card, CardQueue, Deck, DeckStats, Id, NewNote, Note, NoteModel, Rating,
    Settings, Store,
};
use ranki_storage::ReopeningCollection;
use ranki_sync::{
    Credentials, DEFAULT_ENDPOINT, HttpTransport, RetryPolicy, SyncClient,
    SyncOutcome, SyncState, TransportConfig, clear_host_key,
};

use crate::card::sides;
use crate::error::Result;

const BROWSE_ROW_LIMIT: usize = 200;

/// Full-screen destination shown on a phone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Decks,
    Review,
    Add,
    Browse,
    Settings,
}

/// What the review screen is showing for the current card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReviewPhase {
    #[default]
    Question,
    Answer,
    Waiting,
    Summary,
}

/// One row of the decks list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeckRow {
    pub id: Option<Id>,
    pub name: String,
    pub stats: DeckStats,
    pub due: usize,
}

/// One row of the flat browser list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowseRow {
    pub card_id: Id,
    pub front: String,
    pub back: String,
    pub deck: String,
    pub meta: String,
}

/// State of one review session.
#[derive(Clone, Debug, Default)]
pub struct ReviewState {
    pub queue: Vec<Card>,
    pub pending: Vec<Card>,
    pub current: Option<Card>,
    pub note: Option<Note>,
    pub model: Option<NoteModel>,
    pub phase: ReviewPhase,
    pub answered: usize,
    /// Snapshot and review-log id of the last persisted grade.
    pub last: Option<(Card, Id)>,
    pub intervals: [i32; 4],
    pub summary: String,
}

/// State of the add-note form.
#[derive(Clone, Debug, Default)]
pub struct AddState {
    pub decks: Vec<Deck>,
    pub deck_index: usize,
    pub models: Vec<NoteModel>,
    pub model_index: usize,
    pub fields: Vec<String>,
    pub tags: String,
    pub preview: String,
}

/// State of the flat browser screen.
#[derive(Clone, Debug, Default)]
pub struct BrowseState {
    pub query: String,
    pub rows: Vec<BrowseRow>,
}

/// Synchronous `AnkiWeb` state shown in settings.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the contract fixes these independent sync flags"
)]
#[derive(Clone, Debug, Default)]
pub struct SyncUiState {
    pub username: String,
    pub password: String,
    pub signed_in: bool,
    pub busy: bool,
    pub status: String,
    pub conflict: bool,
}

/// Tone of a short status message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastKind {
    #[default]
    Success,
    Error,
}

/// A short status message shown above the bottom tabs.
#[derive(Clone, Debug, Default)]
pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
}

/// Everything the mobile UI needs, including the reopening store.
#[derive(Debug)]
pub struct AppState {
    pub store: ReopeningCollection,
    pub settings: Settings,
    pub scheduler: FsrsScheduler,
    pub screen: Screen,
    pub decks: Vec<DeckRow>,
    pub deck_query: String,
    pub selected_deck: Option<Id>,
    pub review_scope: Option<Id>,
    pub review: ReviewState,
    pub add: AddState,
    pub browse: BrowseState,
    pub sync: SyncUiState,
    pub toast: Option<Toast>,
}

impl AppState {
    /// Builds the state from an opened collection and resolved settings.
    ///
    /// # Errors
    ///
    /// Returns an error when the persisted sync state cannot be read.
    pub fn new(store: ReopeningCollection, settings: Settings) -> Result<Self> {
        let scheduler = from_settings(&settings);
        let deck_query = settings.ui_state.deck_query.clone();
        let selected_deck = settings.ui_state.selected_deck;
        let browse_query = settings.ui_state.browse_query.clone();
        let sync_username = settings.sync.username.clone().unwrap_or_default();
        let mut state = Self {
            store,
            settings,
            scheduler,
            screen: dev_screen(),
            decks: Vec::new(),
            deck_query,
            selected_deck,
            review_scope: selected_deck,
            review: ReviewState::default(),
            add: AddState::default(),
            browse: BrowseState {
                query: browse_query,
                rows: Vec::new(),
            },
            sync: SyncUiState {
                username: sync_username,
                ..SyncUiState::default()
            },
            toast: None,
        };
        state.refresh()?;
        if state.screen == Screen::Review {
            state.start_review(state.selected_deck)?;
        }
        Ok(state)
    }

    /// Reloads collection-backed lists and the local sign-in state.
    ///
    /// Store failures are reported as toasts so the UI stays usable.
    ///
    /// # Errors
    ///
    /// Returns an error when the sync state file cannot be read.
    pub fn refresh(&mut self) -> Result<()> {
        self.load_sync_state()?;
        self.load_decks();
        self.load_models();
        self.refresh_browse();
        Ok(())
    }

    /// Navigates to a screen. Entering review starts a fresh session.
    ///
    /// # Errors
    ///
    /// Returns an error when the sync state file cannot be read.
    pub fn set_screen(&mut self, screen: Screen) -> Result<()> {
        if screen == Screen::Review {
            return self.start_review(self.selected_deck);
        }
        self.screen = screen;
        self.refresh()
    }

    /// Replaces the decks filter text.
    pub fn set_deck_query(&mut self, value: String) {
        self.deck_query = value;
    }

    /// Starts a review session for one deck, or all decks when `deck` is none.
    ///
    /// # Errors
    ///
    /// Store failures are reported as toasts; this call still succeeds.
    pub fn start_review(&mut self, deck: Option<Id>) -> Result<()> {
        self.load_decks();
        let scope = if let Some(id) = deck
            && self.decks.iter().any(|row| row.id == Some(id))
        {
            self.selected_deck = Some(id);
            Some(id)
        } else {
            self.selected_deck = None;
            None
        };
        self.review_scope = scope;
        self.screen = Screen::Review;
        self.review = ReviewState::default();
        self.review.phase = if dev_reveal() {
            ReviewPhase::Answer
        } else {
            ReviewPhase::Question
        };

        let now = now_secs();
        let limit = self.settings.review_batch_size.max(1);
        match self.store.due_cards(scope, limit, now) {
            Ok(cards) => self.review.queue = cards,
            Err(error) => self.fail(format!("Could not load cards: {error}")),
        }
        self.take_next_card();
        if self.review.current.is_none()
            && self.review.phase != ReviewPhase::Summary
        {
            self.review.phase = ReviewPhase::Summary;
            self.review.summary = if self.review.answered == 0 {
                String::from("No cards are due right now.")
            } else {
                format!(
                    "Reviewed {} cards. Session complete.",
                    self.review.answered
                )
            };
        }
        Ok(())
    }

    /// Moves the current card from question to answer.
    pub fn reveal(&mut self) {
        if self.screen != Screen::Review || self.review.current.is_none() {
            return;
        }
        if self.review.phase == ReviewPhase::Question {
            self.review.phase = ReviewPhase::Answer;
        }
    }

    /// Schedules and persists a grade, then advances the session.
    ///
    /// # Errors
    ///
    /// Store failures are reported as toasts; this call still succeeds.
    pub fn rate(&mut self, rating: Rating) -> Result<()> {
        if self.screen != Screen::Review
            || self.review.phase != ReviewPhase::Answer
        {
            return Ok(());
        }
        let Some(mut before) = self.review.current.clone() else {
            return Ok(());
        };
        if let Ok(Some(fresh)) = self.store.card(before.id) {
            before = fresh;
        }
        let now = now_secs();
        let start = self.store.collection_created().unwrap_or(0);
        let decision = self.scheduler.schedule(&before, rating, now, start);
        let log_id = match self.store.apply_review(&decision) {
            Ok(id) => id,
            Err(error) => {
                self.fail(format!("Could not save review: {error}"));
                return Ok(());
            }
        };

        self.review.answered = self.review.answered.saturating_add(1);
        self.review.last = Some((before, log_id));
        if matches!(
            decision.card.queue,
            CardQueue::Learning | CardQueue::DayLearning
        ) {
            self.review.pending.push(decision.card);
        }
        self.take_next_card();
        if self.review.current.is_some()
            && self.review.phase != ReviewPhase::Summary
        {
            self.review.phase = ReviewPhase::Question;
        }
        self.load_decks();
        if self.review.current.is_none()
            && self.review.phase != ReviewPhase::Summary
        {
            if self.review.pending.is_empty() {
                self.review.phase = ReviewPhase::Summary;
                self.review.summary = format!(
                    "Reviewed {} cards. Session complete.",
                    self.review.answered
                );
            } else {
                self.review.phase = ReviewPhase::Waiting;
                self.review.summary =
                    String::from("Next card is waiting for its learning step.");
            }
        }
        Ok(())
    }

    /// Seconds until the next pending learning card is due, if any.
    #[must_use]
    pub fn pending_delay_secs(&self) -> Option<i64> {
        if self.screen != Screen::Review
            || self.review.phase != ReviewPhase::Waiting
        {
            return None;
        }
        let now = now_secs();
        self.review
            .pending
            .iter()
            .filter(|card| {
                matches!(
                    card.queue,
                    CardQueue::Learning | CardQueue::DayLearning
                )
            })
            .map(|card| card.due.saturating_sub(now).max(0))
            .min()
    }

    /// Advances when a pending learning card becomes due.
    pub fn poll_pending(&mut self) {
        if self.screen != Screen::Review
            || self.review.phase != ReviewPhase::Waiting
        {
            return;
        }
        self.take_next_card();
        if self.review.current.is_some() {
            self.review.phase = ReviewPhase::Question;
            self.review.summary.clear();
        }
    }

    /// Leaves review and returns to the decks list.
    pub fn end_review(&mut self) {
        self.review.queue.clear();
        self.review.pending.clear();
        self.review.current = None;
        self.review.note = None;
        self.review.model = None;
        self.review.last = None;
        self.review.phase = ReviewPhase::Question;
        self.review.summary.clear();
        self.review.intervals = [0; 4];
        self.screen = Screen::Decks;
        self.load_decks();
    }

    /// Reverses the last persisted grade and restores that card.
    ///
    /// # Errors
    ///
    /// Store failures are reported as toasts; this call still succeeds.
    pub fn undo(&mut self) -> Result<()> {
        if self.screen != Screen::Review {
            return Ok(());
        }
        let Some((before, log_id)) = self.review.last.take() else {
            self.fail(String::from("Nothing to undo"));
            return Ok(());
        };
        match self.store.undo_review(&before, log_id) {
            Ok(true) => {}
            Ok(false) => {
                self.fail(String::from("That review is already gone"));
                return Ok(());
            }
            Err(error) => {
                self.review.last = Some((before, log_id));
                self.fail(format!("Could not undo review: {error}"));
                return Ok(());
            }
        }

        self.review.answered = self.review.answered.saturating_sub(1);
        self.review.pending.retain(|card| card.id != before.id);
        if let Some(current) = self.review.current.take()
            && current.id != before.id
        {
            self.review.queue.insert(0, current);
        }
        self.review.current = Some(before);
        self.review.phase = if dev_reveal() {
            ReviewPhase::Answer
        } else {
            ReviewPhase::Question
        };
        self.review.summary.clear();
        self.load_current();
        self.load_decks();
        Ok(())
    }

    /// Replaces one note field and refreshes the add preview.
    pub fn set_add_field(&mut self, index: usize, value: String) {
        let updated = self.add.fields.get_mut(index).is_some_and(|slot| {
            *slot = value;
            true
        });
        if updated {
            self.update_preview();
        }
    }

    /// Selects the deck used by new notes.
    pub const fn set_add_deck(&mut self, index: usize) {
        if index < self.add.decks.len() {
            self.add.deck_index = index;
        }
    }

    /// Selects the note type used by new notes.
    pub fn set_add_model(&mut self, index: usize) {
        if index >= self.add.models.len() || index == self.add.model_index {
            return;
        }
        let old_fields = self.add.fields.clone();
        let old_names: Vec<String> = self
            .add
            .models
            .get(self.add.model_index)
            .map_or_else(Vec::new, |model| model.fields.clone());
        let Some(new_names) =
            self.add.models.get(index).map(|model| model.fields.clone())
        else {
            return;
        };
        self.add.fields = new_names
            .iter()
            .map(|name| {
                old_names
                    .iter()
                    .position(|old| old.eq_ignore_ascii_case(name))
                    .and_then(|position| old_fields.get(position))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
        self.add.model_index = index;
        self.update_preview();
    }

    /// Saves the current add-note draft.
    ///
    /// # Errors
    ///
    /// Store failures are reported as toasts; this call still succeeds.
    pub fn add_note(&mut self) -> Result<()> {
        let Some(deck) = self.add.decks.get(self.add.deck_index).cloned()
        else {
            self.fail(String::from("Choose a deck first"));
            return Ok(());
        };
        let Some(model) = self.add.models.get(self.add.model_index).cloned()
        else {
            self.fail(String::from("Choose a note type first"));
            return Ok(());
        };
        if self.add.fields.len() != model.fields.len() {
            self.add.fields.resize(model.fields.len(), String::new());
        }
        let mut tags = Vec::new();
        for tag in self.add.tags.split_whitespace() {
            if !tags.iter().any(|known| known == tag) {
                tags.push(tag.to_owned());
            }
        }
        let note = NewNote {
            model_id: model.id,
            deck_id: deck.id,
            fields: self.add.fields.clone(),
            tags,
        };
        match self.store.add_note(&note) {
            Ok(ids) => {
                let count = ids.len();
                self.add.fields.iter_mut().for_each(String::clear);
                self.add.tags.clear();
                self.refresh()?;
                self.say(format!("Added {count} cards"));
            }
            Err(error) => self.fail(format!("Could not add note: {error}")),
        }
        Ok(())
    }

    /// Runs a store search and rebuilds the flat browser list.
    ///
    /// # Errors
    ///
    /// Store failures are reported as toasts; this call still succeeds.
    pub fn set_browse_query(&mut self, value: String) -> Result<()> {
        self.browse.query = value;
        self.refresh_browse();
        Ok(())
    }

    /// Signs in to `AnkiWeb` and runs one synchronous sync.
    ///
    /// # Errors
    ///
    /// Network and state failures are reported as toasts; this call succeeds.
    pub fn sign_in(&mut self) -> Result<()> {
        if self.sync.busy {
            return Ok(());
        }
        let username = self.sync.username.trim().to_owned();
        let password = self.sync.password.clone();
        if username.is_empty() || password.is_empty() {
            self.fail(String::from("Enter your AnkiWeb username and password"));
            return Ok(());
        }

        self.sync.busy = true;
        self.sync.status = String::from("Signing in to AnkiWeb");
        let result = Self::login_and_sync(
            &self.settings.paths.collection,
            &self.settings.paths.media_dir,
            &self.settings.paths.sync_state,
            self.settings.sync.retries,
            self.settings.sync.backoff_ms,
            &username,
            &password,
        );
        match result {
            Ok(outcome) => {
                self.sync.signed_in = true;
                self.settings.sync.enabled = true;
                self.settings.sync.username = Some(username);
                self.sync.password.clear();
                self.finish_sync(&outcome);
                self.load_sync_state()?;
            }
            Err(error) => {
                self.sync.busy = false;
                let message = format!("AnkiWeb sign-in failed: {error}");
                self.sync.status.clone_from(&message);
                self.fail(message);
                self.load_sync_state()?;
            }
        }
        Ok(())
    }

    /// Runs one synchronous automatic sync.
    ///
    /// # Errors
    ///
    /// Never propagates failures; they are reported as toasts.
    pub fn sync_now(&mut self) -> Result<()> {
        if self.sync.busy {
            return Ok(());
        }
        if !self.sync.signed_in {
            self.fail(String::from("Sign in before syncing"));
            return Ok(());
        }
        self.sync.busy = true;
        self.sync.status = String::from("Syncing with AnkiWeb");
        let result = Self::run_sync(
            &self.settings.paths.collection,
            &self.settings.paths.media_dir,
            &self.settings.paths.sync_state,
            self.settings.sync.retries,
            self.settings.sync.backoff_ms,
        );
        match result {
            Ok(outcome) => {
                self.finish_sync(&outcome);
                self.refresh()?;
            }
            Err(error) => {
                self.sync.busy = false;
                let message = format!("Sync failed: {error}");
                self.sync.status.clone_from(&message);
                self.fail(message);
            }
        }
        Ok(())
    }

    /// Removes the stored host key.
    ///
    /// # Errors
    ///
    /// State write failures are reported as toasts; this call succeeds.
    pub fn sign_out(&mut self) -> Result<()> {
        if self.sync.busy {
            return Ok(());
        }
        if let Err(error) = clear_host_key(&self.settings.paths.sync_state) {
            self.fail(format!("Could not sign out: {error}"));
            return Ok(());
        }
        self.sync.signed_in = false;
        self.sync.password.clear();
        self.sync.conflict = false;
        self.sync.status = String::from("Signed out of AnkiWeb");
        self.settings.sync.enabled = false;
        self.settings.sync.username = None;
        self.say("Signed out of AnkiWeb");
        Ok(())
    }

    /// Resolves a sync conflict with an explicit one-way choice.
    ///
    /// # Errors
    ///
    /// Network failures are reported as toasts; this call still succeeds.
    pub fn resolve_conflict(&mut self, keep_remote: bool) -> Result<()> {
        if self.sync.busy || !self.sync.conflict {
            return Ok(());
        }
        self.sync.busy = true;
        self.sync.status = if keep_remote {
            String::from("Downloading the remote collection")
        } else {
            String::from("Uploading the local collection")
        };
        let result = Self::run_conflict(
            &self.settings.paths.collection,
            &self.settings.paths.media_dir,
            &self.settings.paths.sync_state,
            self.settings.sync.retries,
            self.settings.sync.backoff_ms,
            keep_remote,
        );
        match result {
            Ok(outcome) => {
                self.finish_sync(&outcome);
                self.refresh()?;
            }
            Err(error) => {
                self.sync.busy = false;
                let message =
                    format!("Could not resolve sync conflict: {error}");
                self.sync.status.clone_from(&message);
                self.fail(message);
            }
        }
        Ok(())
    }

    /// Shows a success toast.
    pub fn say(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            kind: ToastKind::Success,
            text: text.into(),
        });
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            kind: ToastKind::Error,
            text: text.into(),
        });
    }

    fn load_sync_state(&mut self) -> Result<()> {
        let state = SyncState::load(&self.settings.paths.sync_state)?;
        self.sync.signed_in = !state.host_key.is_empty();
        Ok(())
    }

    fn load_decks(&mut self) {
        let now = now_secs();
        let decks = match self.store.decks() {
            Ok(decks) => decks,
            Err(error) => {
                self.fail(format!("Could not load decks: {error}"));
                return;
            }
        };
        let totals = self.store.deck_stats(now).map_or_else(
            |_| {
                decks
                    .iter()
                    .map(|deck| {
                        let stats = self
                            .store
                            .stats(Some(deck.id), now)
                            .unwrap_or_default();
                        (deck.id, stats)
                    })
                    .collect::<HashMap<_, _>>()
            },
            |totals| totals.into_iter().collect::<HashMap<_, _>>(),
        );
        let selected = self
            .selected_deck
            .filter(|id| decks.iter().any(|deck| deck.id == *id))
            .or_else(|| decks.first().map(|deck| deck.id));
        let add_deck_id = self
            .add
            .decks
            .get(self.add.deck_index)
            .map(|deck| deck.id)
            .or(selected);
        self.decks = decks
            .iter()
            .map(|deck| {
                let stats = totals.get(&deck.id).copied().unwrap_or_default();
                DeckRow {
                    id: Some(deck.id),
                    name: deck.name.clone(),
                    stats,
                    due: stats
                        .new_cards
                        .saturating_add(stats.learning_cards)
                        .saturating_add(stats.review_cards),
                }
            })
            .collect();
        self.selected_deck = selected;
        self.add.decks = decks;
        self.add.deck_index = add_deck_id
            .and_then(|id| self.add.decks.iter().position(|deck| deck.id == id))
            .unwrap_or(0);
    }

    fn load_models(&mut self) {
        let models = match self.store.models() {
            Ok(models) => models,
            Err(error) => {
                self.fail(format!("Could not load note types: {error}"));
                return;
            }
        };
        let current_id = self
            .add
            .models
            .get(self.add.model_index)
            .map(|model| model.id);
        self.add.models = models;
        self.add.model_index = current_id
            .and_then(|id| {
                self.add.models.iter().position(|model| model.id == id)
            })
            .unwrap_or(0);
        let len = self
            .add
            .models
            .get(self.add.model_index)
            .map_or(0, |model| model.fields.len());
        if self.add.fields.len() != len {
            self.add.fields.resize(len, String::new());
        }
        self.update_preview();
    }

    fn refresh_browse(&mut self) {
        let ids = match self.store.find_cards(&self.browse.query) {
            Ok(ids) => ids,
            Err(error) => {
                self.fail(format!("Could not search cards: {error}"));
                self.browse.rows.clear();
                return;
            }
        };
        let decks = self.decks.clone();
        let models = self.add.models.clone();
        let mut notes = HashMap::new();
        let mut rows = Vec::with_capacity(ids.len().min(BROWSE_ROW_LIMIT));
        for id in ids.into_iter().take(BROWSE_ROW_LIMIT) {
            let card = match self.store.card(id) {
                Ok(Some(card)) => card,
                Ok(None) => continue,
                Err(error) => {
                    self.fail(format!("Could not load card: {error}"));
                    break;
                }
            };
            let cached_note = match notes.entry(card.note_id) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => match self.store.note(card.note_id) {
                    Ok(Some(note)) => entry.insert(Some(note)),
                    Ok(None) => entry.insert(None),
                    Err(error) => {
                        self.fail(format!("Could not load note: {error}"));
                        break;
                    }
                },
            };
            let Some(note) = cached_note else {
                continue;
            };
            let Some(model) =
                models.iter().find(|model| model.id == note.model_id)
            else {
                continue;
            };
            let (question, answer) = sides(model, note, card.ordinal);
            let deck = decks
                .iter()
                .find(|row| row.id == Some(card.deck_id))
                .map_or_else(
                    || String::from("Unknown deck"),
                    |row| row.name.clone(),
                );
            rows.push(BrowseRow {
                card_id: card.id,
                front: question,
                back: answer,
                deck,
                meta: format!("{} / {}", model.name, queue_name(card.queue)),
            });
        }
        self.browse.rows = rows;
    }

    pub(crate) fn take_next_card(&mut self) {
        self.review.current = None;
        self.review.note = None;
        self.review.model = None;
        self.review.intervals = [0; 4];
        let now = now_secs();
        let due = self.review.pending.iter().position(|card| {
            matches!(card.queue, CardQueue::Learning | CardQueue::DayLearning)
                && card.due <= now
        });
        let next = if let Some(index) = due {
            Some(self.review.pending.remove(index))
        } else if self.review.queue.is_empty() {
            None
        } else {
            Some(self.review.queue.remove(0))
        };
        if let Some(card) = next {
            self.review.current = Some(card);
            self.review.summary.clear();
            self.load_current();
        }
    }

    fn load_current(&mut self) {
        let Some(card) = self.review.current.clone() else {
            return;
        };
        let note = match self.store.note(card.note_id) {
            Ok(note) => note,
            Err(error) => {
                self.fail(format!("Could not load note: {error}"));
                None
            }
        };
        let model = note.as_ref().and_then(|loaded| {
            self.store.models().ok().and_then(|models| {
                models.into_iter().find(|model| model.id == loaded.model_id)
            })
        });
        if note.is_none() || model.is_none() {
            self.review.summary = String::from("A card could not be loaded.");
            self.review.phase = ReviewPhase::Summary;
        }
        let now = now_secs();
        let start = self.store.collection_created().unwrap_or(0);
        self.review.intervals = self.scheduler.preview_all(&card, now, start);
        self.review.note = note;
        self.review.model = model;
    }

    fn update_preview(&mut self) {
        let Some(model) = self.add.models.get(self.add.model_index).cloned()
        else {
            self.add.preview.clear();
            return;
        };
        let note = Note {
            id: 0,
            guid: String::new(),
            model_id: model.id,
            modified: 0,
            usn: 0,
            tags: Vec::new(),
            fields: self.add.fields.clone(),
            sort_field: self.add.fields.first().cloned().unwrap_or_default(),
            checksum: 0,
            flags: 0,
            data: String::new(),
        };
        let (question, _) = sides(&model, &note, 0);
        self.add.preview = question;
    }

    fn sync_client(
        collection: &Path,
        media_dir: &Path,
        state_path: &Path,
        retries: u8,
        backoff_ms: u64,
    ) -> Result<SyncClient<HttpTransport>> {
        let transport = HttpTransport::with_config(
            DEFAULT_ENDPOINT,
            TransportConfig {
                retry_policy: RetryPolicy {
                    retries,
                    backoff: Duration::from_millis(backoff_ms),
                },
                ..TransportConfig::default()
            },
        );
        Ok(SyncClient::new(
            transport, collection, media_dir, state_path,
        )?)
    }

    fn login_and_sync(
        collection: &Path,
        media_dir: &Path,
        state_path: &Path,
        retries: u8,
        backoff_ms: u64,
        username: &str,
        password: &str,
    ) -> Result<SyncOutcome> {
        let mut client = Self::sync_client(
            collection, media_dir, state_path, retries, backoff_ms,
        )?;
        client.login(&Credentials::new(username, password))?;
        Ok(client.sync(None)?)
    }

    fn run_sync(
        collection: &Path,
        media_dir: &Path,
        state_path: &Path,
        retries: u8,
        backoff_ms: u64,
    ) -> Result<SyncOutcome> {
        let mut client = Self::sync_client(
            collection, media_dir, state_path, retries, backoff_ms,
        )?;
        Ok(client.sync(None)?)
    }

    fn run_conflict(
        collection: &Path,
        media_dir: &Path,
        state_path: &Path,
        retries: u8,
        backoff_ms: u64,
        keep_remote: bool,
    ) -> Result<SyncOutcome> {
        let mut client = Self::sync_client(
            collection, media_dir, state_path, retries, backoff_ms,
        )?;
        if keep_remote {
            client.full_download()?;
            Ok(SyncOutcome::Downloaded)
        } else {
            client.full_upload()?;
            Ok(SyncOutcome::Uploaded)
        }
    }

    fn finish_sync(&mut self, outcome: &SyncOutcome) {
        self.sync.busy = false;
        self.sync.conflict = false;
        let (status, error, conflict) = match outcome {
            SyncOutcome::Disabled => {
                (String::from("Sync did not start"), true, false)
            }
            SyncOutcome::NoChanges => {
                (String::from("Already up to date"), false, false)
            }
            SyncOutcome::Uploaded => {
                (String::from("Collection uploaded"), false, false)
            }
            SyncOutcome::Downloaded => {
                (String::from("Collection downloaded"), false, false)
            }
            SyncOutcome::Conflict { reason } => {
                (format!("Sync conflict: {reason}"), true, true)
            }
            SyncOutcome::RetryLater { reason } => {
                (format!("Sync failed: {reason}"), true, false)
            }
        };
        self.sync.status = status;
        self.sync.conflict = conflict;
        if error {
            self.fail(self.sync.status.clone());
        } else {
            self.say(self.sync.status.clone());
        }
    }
}

fn dev_screen() -> Screen {
    match env::var("RANKI_MOBILE_DEV_VIEW")
        .unwrap_or_default()
        .as_str()
    {
        "review" => Screen::Review,
        "add" => Screen::Add,
        "browse" => Screen::Browse,
        "settings" => Screen::Settings,
        _ => Screen::Decks,
    }
}

fn dev_reveal() -> bool {
    env::var("RANKI_MOBILE_DEV_REVEAL").is_ok_and(|value| value == "1")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0)
}

const fn queue_name(queue: CardQueue) -> &'static str {
    match queue {
        CardQueue::New => "New",
        CardQueue::Learning | CardQueue::DayLearning => "Learning",
        CardQueue::Review => "Review",
        CardQueue::Suspended => "Suspended",
        CardQueue::Buried | CardQueue::ManuallyBuried => "Buried",
        CardQueue::Preview => "Preview",
    }
}
