//! Headless review frame for the performance gate.

use ranki_model::{Card, Note, NoteModel, Settings, Store};

use crate::card_view::{self, CardRequest};

/// Key the benchmark injects into a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchKey {
    Space,
}

/// A clock far enough ahead that every fixture card counts as due.
const BENCH_CLOCK: i64 = 4_102_444_800;

/// One review screen pass without a window.
///
/// gpui has no headless window, so this measures the render-model build for
/// one review frame, not GPU submission.
pub struct BenchApp {
    store: Box<dyn Store>,
    models: Vec<NoteModel>,
    batch: usize,
    current: Option<Card>,
    note: Option<Note>,
    revealed: bool,
}

impl BenchApp {
    /// Opens the store the window would use and caches its note models.
    #[must_use]
    pub fn new(store: Box<dyn Store>, options: &Settings) -> Self {
        let models = store.models().unwrap_or_default();
        Self {
            store,
            models,
            batch: options.review_batch_size.max(1),
            current: None,
            note: None,
            revealed: false,
        }
    }

    /// Runs the work of one review frame. A key is applied before the pass.
    pub fn frame(&mut self, key: Option<BenchKey>) {
        self.ensure_current();
        if matches!(key, Some(BenchKey::Space)) && self.current.is_some() {
            self.revealed = true;
        }
        let (Some(note), Some(card)) =
            (self.note.as_ref(), self.current.as_ref())
        else {
            return;
        };
        let Some(model) =
            self.models.iter().find(|model| model.id == note.model_id)
        else {
            return;
        };
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
