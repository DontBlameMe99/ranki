//! Headless review and deck frames for the performance gate.

use std::collections::{HashMap, HashSet};

use ranki_core::{FsrsScheduler, from_settings};
use ranki_model::{
    Card, Deck, DeckStats, Note, NoteModel, Rating, Settings, Store,
};

use crate::{
    app::{build_deck_rows, due_total, filter_decks, leaf_deck_totals},
    card_view::{self, CardRequest},
};

/// Key the benchmark injects into a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchKey {
    Space,
}

/// A clock far enough ahead that every fixture card counts as due.
const BENCH_CLOCK: i64 = 4_102_444_800;

/// Deck tree the deck view work covers: roots times children.
const DECK_PARENTS: usize = 10;
const DECK_CHILDREN: usize = 10;

/// One review and deck screen pass without a window.
///
/// gpui has no headless window, so this measures the per-frame render-model
/// work of the review and decks views: the previews, the card text, the deck
/// rows, and the search filter. GPU submission is not included.
pub struct BenchApp {
    store: Box<dyn Store>,
    models: Vec<NoteModel>,
    scheduler: FsrsScheduler,
    batch: usize,
    current: Option<Card>,
    note: Option<Note>,
    revealed: bool,
    decks: Vec<Deck>,
    stats: Vec<DeckStats>,
    expanded: HashSet<String>,
    collection_start: i64,
}

impl BenchApp {
    /// Opens the store the window would use and seeds its deck set.
    #[must_use]
    pub fn new(store: Box<dyn Store>, options: &Settings) -> Self {
        let mut store = store;
        let roots = (0..DECK_PARENTS)
            .map(|index| format!("Bench {index}"))
            .collect::<Vec<_>>();
        for root in &roots {
            let _ = store.create_deck(root);
            for index in 0..DECK_CHILDREN {
                let _ = store.create_deck(&format!("{root}::Deck {index}"));
            }
        }
        let collection_start = store.collection_created().unwrap_or(0);
        let models = store.models().unwrap_or_default();
        let decks = store.decks().unwrap_or_default();
        let stats = deck_stats(&*store, &decks);
        Self {
            store,
            models,
            scheduler: from_settings(options),
            batch: options.review_batch_size.max(1),
            current: None,
            note: None,
            revealed: false,
            decks,
            stats,
            expanded: roots.into_iter().collect(),
            collection_start,
        }
    }

    /// Runs the work of one review and deck frame. A key is applied first.
    pub fn frame(&mut self, key: Option<BenchKey>) {
        self.ensure_current();
        if matches!(key, Some(BenchKey::Space)) && self.current.is_some() {
            self.revealed = true;
        }
        let collection_start = self.collection_start;
        if let Some(card) = self.current.clone() {
            let _ = [
                self.scheduler.preview(
                    &card,
                    Rating::Again,
                    BENCH_CLOCK,
                    collection_start,
                ),
                self.scheduler.preview(
                    &card,
                    Rating::Hard,
                    BENCH_CLOCK,
                    collection_start,
                ),
                self.scheduler.preview(
                    &card,
                    Rating::Good,
                    BENCH_CLOCK,
                    collection_start,
                ),
                self.scheduler.preview(
                    &card,
                    Rating::Easy,
                    BENCH_CLOCK,
                    collection_start,
                ),
            ];
            if let Some(note) = self.note.as_ref()
                && let Some(model) =
                    self.models.iter().find(|model| model.id == note.model_id)
            {
                let _ = card_view::render_card(
                    model,
                    note,
                    CardRequest::question(card.ordinal),
                );
                if self.revealed {
                    let _ = card_view::render_card(
                        model,
                        note,
                        CardRequest::answer_only(card.ordinal),
                    );
                }
            }
        }
        let _ = self.store.due_cards(None, self.batch, BENCH_CLOCK);
        let rows = build_deck_rows(&self.decks, &self.stats, &self.expanded);
        let _ = due_total(leaf_deck_totals(&self.decks, &self.stats));
        let _ = filter_decks(&rows, "bench");
    }

    /// Returns to the question side so the next frame reloads its card.
    pub fn reset(&mut self) {
        self.current = None;
        self.note = None;
        self.revealed = false;
    }

    fn ensure_current(&mut self) {
        if self.current.is_some() {
            return;
        }
        let cards = self
            .store
            .due_cards(None, self.batch, BENCH_CLOCK)
            .unwrap_or_default();
        let Some(card) = cards.into_iter().next() else {
            return;
        };
        self.note = self.store.note(card.note_id).unwrap_or_default();
        self.current = Some(card);
        self.revealed = false;
    }
}

/// Counts each deck's cards in one store call, aligned with `decks`.
fn deck_stats(store: &dyn Store, decks: &[Deck]) -> Vec<DeckStats> {
    let counts = store
        .deck_stats(BENCH_CLOCK)
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<_, _>>();
    decks
        .iter()
        .map(|deck| counts.get(&deck.id).copied().unwrap_or_default())
        .collect()
}
