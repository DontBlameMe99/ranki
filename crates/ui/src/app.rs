//! Review window, deck sidebar, and store-backed card queue.

use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use egui::{Button, Key, Modifiers, ProgressBar, RichText};
use ranki_core::{
    FsrsPreset, FsrsScheduler, Scheduler, Sm2Preset, Sm2Scheduler,
};
use ranki_model::{
    Card, Deck, DeckStats, Id, Note, NoteModel, Rating, Store, StoreError,
};

use crate::{Options, card_view, theme};

const DUE_BATCH: usize = 20;
const REFILL_AT: usize = 5;
const APP_NAME: &str = "ranki";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueueState {
    Ready,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SchedulerKind {
    Fsrs,
    Sm2,
}

impl SchedulerKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Fsrs => "FSRS",
            Self::Sm2 => "SM-2",
        }
    }
}

struct ReviewApp {
    store: Box<dyn Store>,
    media_dir: Option<PathBuf>,
    decks: Vec<Deck>,
    models: Vec<NoteModel>,
    stats: Vec<DeckStats>,
    selected: Option<usize>,
    queue: VecDeque<Card>,
    seen: HashSet<Id>,
    queue_state: QueueState,
    current: Option<Card>,
    note: Option<Note>,
    templates: Option<(String, String)>,
    shown_at: Option<Instant>,
    revealed: bool,
    scheduler: Box<dyn Scheduler>,
    scheduler_kind: SchedulerKind,
    collection_start_secs: i64,
    answered: usize,
    session_total: usize,
    error: Option<String>,
}

/// Starts the native review window.
///
/// # Errors
/// Returns an error when the winit window or graphics context cannot start.
pub fn run(store: Box<dyn Store>, options: Options) -> eframe::Result<()> {
    let Options {
        window_title,
        media_dir,
        light_theme,
    } = options;
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(window_title)
            .with_inner_size(egui::vec2(1024.0, 768.0))
            .with_min_inner_size(egui::vec2(800.0, 600.0)),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        native_options,
        Box::new(move |creation| {
            theme::apply(&creation.egui_ctx, light_theme);
            Ok(Box::new(ReviewApp::new(store, media_dir)))
        }),
    )
}

impl ReviewApp {
    fn new(store: Box<dyn Store>, media_dir: Option<PathBuf>) -> Self {
        let (collection_start_secs, initial_error) = match store
            .collection_created()
        {
            Ok(value) => (value, None),
            Err(error) => {
                (0, Some(format!("Could not load collection start: {error}")))
            }
        };
        let mut app = Self {
            store,
            media_dir,
            decks: Vec::new(),
            models: Vec::new(),
            stats: Vec::new(),
            selected: None,
            queue: VecDeque::new(),
            seen: HashSet::new(),
            queue_state: QueueState::Ready,
            current: None,
            note: None,
            templates: None,
            shown_at: None,
            revealed: false,
            scheduler: Box::new(FsrsScheduler::new(FsrsPreset::default())),
            scheduler_kind: SchedulerKind::Fsrs,
            collection_start_secs,
            answered: 0,
            session_total: 0,
            error: initial_error,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        match self.store.decks() {
            Ok(decks) => self.decks = decks,
            Err(error) => self.set_error("Could not load decks", &error),
        }
        match self.store.models() {
            Ok(models) => self.models = models,
            Err(error) => self.set_error("Could not load models", &error),
        }
        self.refresh_stats();
        self.selected = (!self.decks.is_empty()).then_some(0);
        self.reset_queue();
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "deck counts are gathered in one short pass"
    )]
    fn refresh_stats(&mut self) {
        let now = now_secs();
        let mut stats = Vec::with_capacity(self.decks.len());
        let deck_ids: Vec<Id> = self.decks.iter().map(|deck| deck.id).collect();
        for deck_id in deck_ids {
            match self.store.stats(Some(deck_id), now) {
                Ok(counts) => stats.push(counts),
                Err(error) => {
                    self.set_error("Could not load deck stats", &error);
                    stats.push(DeckStats::default());
                }
            }
        }
        self.stats = stats;
    }

    fn set_error(&mut self, action: &str, error: &StoreError) {
        self.error = Some(format!("{action}: {error}"));
    }

    fn selected_deck(&self) -> Option<&Deck> {
        self.selected.and_then(|index| self.decks.get(index))
    }

    fn selected_deck_id(&self) -> Option<Id> {
        self.selected_deck().map(|deck| deck.id)
    }

    fn selected_stats(&self) -> Option<&DeckStats> {
        self.selected.and_then(|index| self.stats.get(index))
    }

    fn reset_queue(&mut self) {
        self.queue.clear();
        self.seen.clear();
        self.queue_state = QueueState::Ready;
        self.current = None;
        self.note = None;
        self.templates = None;
        self.shown_at = None;
        self.revealed = false;
        self.answered = 0;
        self.session_total = self.selected_stats().map_or(0, due_count);
        self.fill_queue();
        self.ensure_current();
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the due queue is filled from one bounded store query"
    )]
    fn fill_queue(&mut self) {
        if self.queue_state == QueueState::Exhausted
            || self.queue.len() >= REFILL_AT
        {
            return;
        }
        let deck_id = self.selected_deck_id();
        match self.store.due_cards(deck_id, DUE_BATCH, now_secs()) {
            Ok(cards) => {
                let returned = cards.len();
                let mut added: usize = 0;
                for card in cards {
                    if self.seen.insert(card.id) {
                        self.queue.push_back(card);
                        added = added.saturating_add(1);
                    }
                }
                if added == 0 || returned < DUE_BATCH {
                    self.queue_state = QueueState::Exhausted;
                }
            }
            Err(error) => {
                self.set_error("Could not load due cards", &error);
                self.queue_state = QueueState::Exhausted;
            }
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the current note and its templates are loaded together"
    )]
    fn ensure_current(&mut self) {
        if self.current.is_some() {
            return;
        }
        self.fill_queue();
        let Some(card) = self.queue.pop_front() else {
            return;
        };
        self.shown_at = Some(Instant::now());
        self.revealed = false;
        match self.store.note(card.note_id) {
            Ok(note) => {
                let templates = note.as_ref().and_then(|loaded| {
                    render_templates(loaded, &card, &self.models)
                });
                self.note = note;
                self.templates = templates;
            }
            Err(error) => {
                self.set_error("Could not load note", &error);
                self.note = None;
                self.templates = None;
            }
        }
        self.current = Some(card);
    }

    fn select_deck(&mut self, index: usize) {
        if self.selected == Some(index) || index >= self.decks.len() {
            return;
        }
        self.selected = Some(index);
        self.reset_queue();
    }

    fn move_selection(&mut self, down: bool) {
        if self.decks.is_empty() {
            return;
        }
        let current = self.selected.unwrap_or(0);
        let next = if down {
            current
                .checked_add(1)
                .filter(|index| *index < self.decks.len())
                .unwrap_or(0)
        } else {
            current.saturating_sub(1)
        };
        self.select_deck(next);
    }

    const fn reveal(&mut self) {
        if self.current.is_some() {
            self.revealed = true;
        }
    }

    const fn toggle_reveal(&mut self) {
        if self.current.is_some() {
            self.revealed = !self.revealed;
        }
    }

    fn answer(&mut self, rating: Rating) {
        if !self.revealed {
            return;
        }
        let Some(card) = self.current.clone() else {
            return;
        };
        let elapsed_ms = self.shown_at.map_or(0, |start| {
            u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX)
        });
        let mut decision = self.scheduler.schedule(
            &card,
            rating,
            now_secs(),
            self.collection_start_secs,
        );
        decision.revlog.time_ms = elapsed_ms;
        if let Err(error) = self.store.apply_review(&decision) {
            self.set_error("Could not save review", &error);
            return;
        }
        self.answered = self.answered.saturating_add(1);
        self.current = None;
        self.note = None;
        self.templates = None;
        self.shown_at = None;
        self.revealed = false;
        self.queue_state = QueueState::Ready;
        self.refresh_stats();
        self.ensure_current();
    }

    fn toggle_scheduler(&mut self) {
        let (kind, scheduler): (SchedulerKind, Box<dyn Scheduler>) =
            match self.scheduler_kind {
                SchedulerKind::Fsrs => (
                    SchedulerKind::Sm2,
                    Box::new(Sm2Scheduler::new(Sm2Preset::default())),
                ),
                SchedulerKind::Sm2 => (
                    SchedulerKind::Fsrs,
                    Box::new(FsrsScheduler::new(FsrsPreset::default())),
                ),
            };
        self.scheduler_kind = kind;
        self.scheduler = scheduler;
    }

    fn handle_key(&mut self, key: Key) -> bool {
        if key == Key::Space || key == Key::Enter {
            self.reveal();
            true
        } else if key == Key::R {
            self.toggle_reveal();
            true
        } else if key == Key::ArrowDown {
            self.move_selection(true);
            true
        } else if key == Key::ArrowUp {
            self.move_selection(false);
            true
        } else if key == Key::T {
            self.toggle_scheduler();
            true
        } else if key == Key::F5 {
            self.reload();
            true
        } else if let Some(rating) = rating_for_key(key) {
            self.answer(rating);
            true
        } else {
            false
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "egui draws the deck list inside a scroll area"
    )]
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.heading("Ranki");
        ui.add_space(4.0);
        ui.label(RichText::new("Due today").weak());
        ui.separator();
        let mut choose = None;
        egui::ScrollArea::vertical().show(ui, |list_ui| {
            for index in 0..self.decks.len() {
                let Some(deck) = self.decks.get(index) else {
                    continue;
                };
                let counts = self.stats.get(index).copied().unwrap_or_default();
                let label = format!(
                    "{}\n{} new, {} learning, {} review",
                    deck.name,
                    counts.new_cards,
                    counts.learning_cards,
                    counts.review_cards
                );
                if list_ui
                    .selectable_label(self.selected == Some(index), label)
                    .clicked()
                {
                    choose = Some(index);
                }
            }
        });
        if let Some(index) = choose {
            self.select_deck(index);
        }
        ui.add_space(10.0);
        if ui.button("Refresh (F5)").clicked() {
            self.reload();
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "egui composes the review panel from several small closure calls"
    )]
    fn review(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        self.review_header(ui);
        self.progress(ui);
        if let Some(error) = &self.error {
            ui.add_space(6.0);
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.separator();
        if self.current.is_some() {
            self.current_card(ui);
        } else {
            ui.centered_and_justified(|center| {
                center.label(
                    "No cards due. Refresh the collection or add notes.",
                );
            });
        }
        ui.separator();
        Self::hint_bar(ui);
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "egui aligns the deck name and scheduler toggle"
    )]
    fn review_header(&mut self, ui: &mut egui::Ui) {
        let deck_name = self
            .selected_deck()
            .map_or("No deck selected", |deck| deck.name.as_str())
            .to_owned();
        ui.horizontal(|row| {
            row.heading(deck_name);
            row.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |end| {
                    let label = format!("{} (T)", self.scheduler_kind.label());
                    if end.add(Button::new(label)).clicked() {
                        self.toggle_scheduler();
                    }
                },
            );
        });
    }

    fn progress(&self, ui: &mut egui::Ui) {
        ui.add(
            ProgressBar::new(progress_fraction(
                self.answered,
                self.session_total,
            ))
            .text(format!(
                "{} of {} reviewed",
                self.answered, self.session_total
            )),
        );
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "egui renders the card and its actions in one panel"
    )]
    fn current_card(&mut self, ui: &mut egui::Ui) {
        let heading = if self.revealed { "Answer" } else { "Question" };
        ui.label(RichText::new(heading).weak());
        let markup = self.templates.as_ref().map_or(
            "This card has no template.",
            |(question, answer)| {
                if self.revealed {
                    answer.as_str()
                } else {
                    question.as_str()
                }
            },
        );
        card_view::render(ui, markup, self.media_dir.as_deref());
        if self.note.is_none() {
            ui.colored_label(ui.visuals().error_fg_color, "Note not found");
        }
        self.rating_buttons(ui);
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "egui lays out all rating actions in one row"
    )]
    fn rating_buttons(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal_wrapped(|row| {
            let label = if self.revealed {
                "Hide answer (R)"
            } else {
                "Reveal answer (Space)"
            };
            if row.button(label).clicked() {
                self.toggle_reveal();
            }
            for (number, rating) in rating_keys() {
                let button =
                    Button::new(format!("{} {}", number, rating_label(rating)))
                        .min_size(egui::vec2(74.0, 34.0));
                if row.add_enabled(self.revealed, button).clicked() {
                    self.answer(rating);
                }
            }
        });
    }

    fn hint_bar(ui: &mut egui::Ui) {
        ui.label(
            RichText::new(
                "Space/Enter reveal | 1-4 rate | R toggle | Up/Down deck | \
                 T scheduler | F5 refresh",
            )
            .weak(),
        );
    }
}

impl eframe::App for ReviewApp {
    #[expect(
        clippy::excessive_nesting,
        reason = "input handling and two eframe panels share one frame callback"
    )]
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.fill_queue();
        self.ensure_current();
        let ctx = ui.ctx();
        let keys: Vec<Key> = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } = event
                        && *modifiers == Modifiers::NONE
                    {
                        Some(*key)
                    } else {
                        None
                    }
                })
                .collect()
        });
        for key in keys {
            if self.handle_key(key) {
                ctx.input_mut(|input| {
                    input.consume_key(Modifiers::NONE, key);
                });
            }
        }
        egui::Panel::left("decks")
            .resizable(true)
            .default_size(210.0)
            .min_size(150.0)
            .max_size(300.0)
            .show(ui, |panel_ui| self.sidebar(panel_ui));
        egui::CentralPanel::default()
            .show(ui, |center_ui| self.review(center_ui));
    }
}

const fn due_count(stats: &DeckStats) -> usize {
    stats
        .new_cards
        .saturating_add(stats.learning_cards)
        .saturating_add(stats.review_cards)
}

fn progress_fraction(answered: usize, total: usize) -> f32 {
    if total == 0 {
        return 1.0;
    }
    let answered_value = u16::try_from(answered).unwrap_or(u16::MAX);
    let total_value = u16::try_from(total).unwrap_or(u16::MAX);
    (f32::from(answered_value) / f32::from(total_value)).clamp(0.0, 1.0)
}

fn rating_for_key(key: Key) -> Option<Rating> {
    if key == Key::Num1 {
        Some(Rating::Again)
    } else if key == Key::Num2 {
        Some(Rating::Hard)
    } else if key == Key::Num3 {
        Some(Rating::Good)
    } else if key == Key::Num4 {
        Some(Rating::Easy)
    } else {
        None
    }
}

const fn rating_keys() -> [(u8, Rating); 4] {
    [
        (1, Rating::Again),
        (2, Rating::Hard),
        (3, Rating::Good),
        (4, Rating::Easy),
    ]
}

const fn rating_label(rating: Rating) -> &'static str {
    match rating {
        Rating::Again => "Again",
        Rating::Hard => "Hard",
        Rating::Good => "Good",
        Rating::Easy => "Easy",
    }
}

fn render_templates(
    note: &Note,
    card: &Card,
    models: &[NoteModel],
) -> Option<(String, String)> {
    let model = models.iter().find(|model| model.id == note.model_id)?;
    let template = model.templates.get(usize::from(card.ordinal))?;
    let question = card_view::substitute_fields(
        &template.question_format,
        &model.fields,
        &note.fields,
    );
    let answer = card_view::render_answer(
        &template.answer_format,
        &question,
        &model.fields,
        &note.fields,
    );
    Some((question, answer))
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::HashMap,
        rc::Rc,
        time::{Duration, Instant},
    };

    use ranki_model::{
        CardQueue, CardTemplate, CardType, NewNote, ReviewDecision, Store,
        StoreError,
    };

    use super::{Rating, ReviewApp};
    use crate::app::Key;

    #[derive(Clone)]
    struct MemoryStore {
        decks: Vec<ranki_model::Deck>,
        models: Vec<ranki_model::NoteModel>,
        cards: Vec<ranki_model::Card>,
        notes: HashMap<ranki_model::Id, ranki_model::Note>,
        reviews: Rc<RefCell<Vec<ReviewDecision>>>,
    }

    impl MemoryStore {
        #[expect(
            clippy::too_many_arguments,
            reason = "the mock mirrors the store query"
        )]
        #[expect(
            clippy::excessive_nesting,
            reason = "the mock filters the due queue in one pass"
        )]
        fn due(
            &self,
            deck_id: Option<ranki_model::Id>,
            limit: usize,
            now: i64,
        ) -> Vec<ranki_model::Card> {
            self.cards
                .iter()
                .filter(|card| {
                    let in_deck = deck_id.is_none_or(|id| card.deck_id == id);
                    let due = card.queue == CardQueue::New || card.due <= now;
                    in_deck && card.queue.is_due() && due
                })
                .take(limit)
                .cloned()
                .collect()
        }
    }

    impl Store for MemoryStore {
        fn decks(&self) -> Result<Vec<ranki_model::Deck>, StoreError> {
            Ok(self.decks.clone())
        }

        fn create_deck(
            &mut self,
            name: &str,
        ) -> Result<ranki_model::Deck, StoreError> {
            let id = i64::try_from(self.decks.len()).unwrap_or(i64::MAX);
            let deck = ranki_model::Deck {
                id,
                name: name.to_owned(),
                description: String::new(),
            };
            self.decks.push(deck.clone());
            Ok(deck)
        }

        fn models(&self) -> Result<Vec<ranki_model::NoteModel>, StoreError> {
            Ok(self.models.clone())
        }

        fn find_cards(
            &self,
            _query: &str,
        ) -> Result<Vec<ranki_model::Id>, StoreError> {
            Ok(self.cards.iter().map(|card| card.id).collect())
        }

        fn due_cards(
            &self,
            deck_id: Option<ranki_model::Id>,
            limit: usize,
            now_secs: i64,
        ) -> Result<Vec<ranki_model::Card>, StoreError> {
            Ok(self.due(deck_id, limit, now_secs))
        }

        fn card(
            &self,
            id: ranki_model::Id,
        ) -> Result<Option<ranki_model::Card>, StoreError> {
            Ok(self.cards.iter().find(|card| card.id == id).cloned())
        }

        fn note(
            &self,
            id: ranki_model::Id,
        ) -> Result<Option<ranki_model::Note>, StoreError> {
            Ok(self.notes.get(&id).cloned())
        }

        fn add_note(
            &mut self,
            _note: &NewNote,
        ) -> Result<Vec<ranki_model::Id>, StoreError> {
            Ok(Vec::new())
        }

        #[expect(
            clippy::excessive_nesting,
            reason = "the mock updates one stored card"
        )]
        fn apply_review(
            &mut self,
            decision: &ReviewDecision,
        ) -> Result<(), StoreError> {
            self.reviews.borrow_mut().push(decision.clone());
            if let Some(card) = self
                .cards
                .iter_mut()
                .find(|card| card.id == decision.card.id)
            {
                *card = decision.card.clone();
            }
            Ok(())
        }

        #[expect(
            clippy::excessive_nesting,
            reason = "the mock counts card queues in one pass"
        )]
        fn stats(
            &self,
            deck_id: Option<ranki_model::Id>,
            _now_secs: i64,
        ) -> Result<ranki_model::DeckStats, StoreError> {
            let mut counts = ranki_model::DeckStats::default();
            for card in &self.due(deck_id, usize::MAX, i64::MAX) {
                match card.queue {
                    CardQueue::New => {
                        counts.new_cards = counts.new_cards.saturating_add(1);
                    }
                    CardQueue::Learning | CardQueue::DayLearning => {
                        counts.learning_cards =
                            counts.learning_cards.saturating_add(1);
                    }
                    CardQueue::Review => {
                        counts.review_cards =
                            counts.review_cards.saturating_add(1);
                    }
                    CardQueue::Suspended
                    | CardQueue::Buried
                    | CardQueue::Preview => {}
                }
            }
            counts.total_cards = self.cards.len();
            Ok(counts)
        }

        fn collection_created(&self) -> Result<i64, StoreError> {
            Ok(0)
        }

        fn collection_modified(&self) -> Result<i64, StoreError> {
            Ok(0)
        }
    }

    fn card(
        id: ranki_model::Id,
        note_id: ranki_model::Id,
    ) -> ranki_model::Card {
        ranki_model::Card {
            id,
            note_id,
            deck_id: 1,
            ordinal: 0,
            modified: 0,
            usn: 0,
            card_type: CardType::New,
            queue: CardQueue::New,
            due: 0,
            interval: 0,
            factor: 2500,
            reps: 0,
            lapses: 0,
            left: 0,
            original_due: 0,
            original_deck_id: 0,
            flags: 0,
            data: String::new(),
        }
    }

    fn fixture() -> MemoryStore {
        let note = ranki_model::Note {
            id: 10,
            guid: "guid".to_owned(),
            model_id: 1,
            modified: 0,
            usn: 0,
            tags: Vec::new(),
            fields: vec!["Question?".to_owned(), "Answer!".to_owned()],
            sort_field: "Question?".to_owned(),
            checksum: 0,
            flags: 0,
            data: String::new(),
        };
        let model = ranki_model::NoteModel {
            id: 1,
            name: "Basic".to_owned(),
            fields: vec!["Front".to_owned(), "Back".to_owned()],
            templates: vec![CardTemplate {
                name: "Card 1".to_owned(),
                question_format: "{{Front}}".to_owned(),
                answer_format: "{{Front}}<br>{{Back}}".to_owned(),
            }],
            css: String::new(),
        };
        let mut notes = HashMap::new();
        notes.insert(10, note.clone());
        notes.insert(
            11,
            ranki_model::Note {
                id: 11,
                fields: vec!["Second?".to_owned(), "Second answer".to_owned()],
                sort_field: "Second?".to_owned(),
                ..note
            },
        );
        MemoryStore {
            decks: vec![ranki_model::Deck {
                id: 1,
                name: "Default".to_owned(),
                description: String::new(),
            }],
            models: vec![model],
            cards: vec![card(1, 10), card(2, 11)],
            notes,
            reviews: Rc::new(RefCell::new(Vec::new())),
        }
    }

    #[test]
    fn rating_keys_map_to_anki_eases() {
        assert_eq!(rating_key(Key::Num1), Some(Rating::Again));
        assert_eq!(rating_key(Key::Num2), Some(Rating::Hard));
        assert_eq!(rating_key(Key::Num3), Some(Rating::Good));
        assert_eq!(rating_key(Key::Num4), Some(Rating::Easy));
        assert_eq!(rating_key(Key::Num5), None);
    }

    #[test]
    fn due_queue_advances_after_review() {
        let store = fixture();
        let reviews = Rc::clone(&store.reviews);
        let mut app = ReviewApp::new(Box::new(store), None);
        assert_eq!(app.current.as_ref().map(|card| card.id), Some(1));

        app.reveal();
        app.answer(Rating::Good);

        assert_eq!(app.current.as_ref().map(|card| card.id), Some(2));
        assert_eq!(app.answered, 1);
        assert_eq!(reviews.borrow().len(), 1);
        assert_eq!(reviews.borrow()[0].revlog.ease, Rating::Good);
    }

    #[test]
    fn selecting_a_deck_resets_the_queue() {
        let mut store = fixture();
        store.decks.push(ranki_model::Deck {
            id: 2,
            name: "Other".to_owned(),
            description: String::new(),
        });
        let mut app = ReviewApp::new(Box::new(store), None);
        app.select_deck(1);

        assert_eq!(app.selected, Some(1));
        assert!(app.current.is_none());
        assert!(app.queue.is_empty());
        assert!(app.seen.is_empty());
    }

    #[test]
    fn reviews_record_elapsed_time() {
        let store = fixture();
        let reviews = Rc::clone(&store.reviews);
        let mut app = ReviewApp::new(Box::new(store), None);
        app.shown_at = Some(
            Instant::now()
                .checked_sub(Duration::from_millis(2))
                .unwrap_or_else(Instant::now),
        );
        app.reveal();
        app.answer(Rating::Easy);

        assert!(reviews.borrow()[0].revlog.time_ms >= 2);
    }

    #[test]
    fn scheduler_toggle_switches_kind() {
        let mut app = ReviewApp::new(Box::new(fixture()), None);
        assert_eq!(app.scheduler.name(), "fsrs");
        app.toggle_scheduler();
        assert_eq!(app.scheduler.name(), "sm2");
    }

    fn rating_key(key: Key) -> Option<Rating> {
        super::rating_for_key(key)
    }
}
