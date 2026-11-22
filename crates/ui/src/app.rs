//! Review window, deck sidebar, and store-backed card queue.

use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use egui::{Button, Key, Modal, Modifiers, ProgressBar, RichText, Theme};
use ranki_core::Scheduler;
use ranki_model::{
    Card, Deck, DeckStats, NewNote, Note, NoteModel, Rating, Store, StoreError,
};

use crate::{Options, SchedulerKind, card_view, theme};

const REFILL_AT: usize = 5;
const APP_NAME: &str = "ranki";
const DEBOUNCE: Duration = Duration::from_millis(120);
const TOAST_LIFETIME: Duration = Duration::from_secs(5);
const MAX_REVIEW_TIME: Duration = Duration::from_secs(60);
const STAGE_INNER_HEIGHT: f32 = 232.0;
const EXTERNAL_POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueueState {
    Ready,
    Exhausted,
}

#[derive(Clone, Debug)]
struct Toast {
    text: String,
    created: Instant,
}

impl Toast {
    fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.created) >= TOAST_LIFETIME
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Dialog {
    Scheduler(SchedulerKind),
    Refresh,
    Help,
    AddNote,
}

#[derive(Clone, Debug, Default)]
struct NoteDraft {
    fields: Vec<String>,
    tags: String,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EmptyKind {
    NoDecks,
    EmptyDeck,
    CaughtUp,
}

struct EmptyCopy {
    title: &'static str,
    detail: String,
    action: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Confirm,
    ToggleReveal,
    Rate(Rating),
    SchedulerChoice,
    Refresh,
    SelectNext,
    SelectPrevious,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "review focus and text focus are independent UI states"
)]
struct ReviewApp {
    store: Box<dyn Store>,
    media_dir: Option<PathBuf>,
    theme_choice: crate::ThemeChoice,
    theme: Theme,
    decks: Vec<Deck>,
    models: Vec<NoteModel>,
    stats: Vec<DeckStats>,
    selected: Option<usize>,
    queue: VecDeque<Card>,
    seen: HashSet<ranki_model::Id>,
    queue_state: QueueState,
    current: Option<Card>,
    note: Option<Note>,
    templates: Option<(String, String)>,
    images: card_view::ImageCache,
    revealed: bool,
    focused_elapsed: Duration,
    last_tick: Option<Instant>,
    last_grade_at: Option<Instant>,
    scheduler: Arc<dyn Scheduler>,
    scheduler_kind: SchedulerKind,
    desired_retention: f32,
    due_batch_size: usize,
    show_keyboard_hints: bool,
    font_size: f32,
    collection_start_secs: i64,
    collection_modified: Option<i64>,
    last_modified_check: Option<Instant>,
    external_change: bool,
    answered: usize,
    session_total: usize,
    toast: Option<Toast>,
    status: Option<String>,
    dialog: Option<Dialog>,
    note_draft: NoteDraft,
    review_focus: bool,
    text_field_focused: bool,
}

/// Starts the native review window.
///
/// # Errors
/// Returns an error when the winit window or graphics context cannot start.
pub fn run(store: Box<dyn Store>, options: Options) -> eframe::Result<()> {
    let theme_choice = options.theme;
    let font_size = options.font_size;
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ranki")
            .with_inner_size(egui::vec2(1024.0, 768.0))
            .with_min_inner_size(egui::vec2(800.0, 600.0)),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        native_options,
        Box::new(move |creation| {
            theme::apply(&creation.egui_ctx, theme_choice, font_size);
            Ok(Box::new(ReviewApp::new(store, &options)))
        }),
    )
}

impl ReviewApp {
    fn new(store: Box<dyn Store>, options: &Options) -> Self {
        let (collection_start_secs, initial_error) = match store
            .collection_created()
        {
            Ok(value) => (value, None),
            Err(error) => {
                (0, Some(format!("Could not load collection start: {error}")))
            }
        };
        let collection_modified = store.collection_modified().ok();
        let mut app = Self {
            store,
            media_dir: Some(options.media_dir.clone()),
            theme_choice: options.theme,
            theme: theme::resolve_theme(None, options.theme),
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
            images: card_view::ImageCache::default(),
            revealed: false,
            focused_elapsed: Duration::ZERO,
            last_tick: None,
            last_grade_at: None,
            scheduler: make_scheduler(
                options.scheduler,
                options.desired_retention,
            ),
            scheduler_kind: options.scheduler,
            desired_retention: options.desired_retention,
            due_batch_size: options.review_batch_size,
            show_keyboard_hints: options.show_keyboard_hints,
            font_size: options.font_size,
            collection_start_secs,
            collection_modified,
            last_modified_check: None,
            external_change: false,
            answered: 0,
            session_total: 0,
            toast: initial_error.map(|text| Toast {
                text,
                created: Instant::now(),
            }),
            status: None,
            dialog: None,
            note_draft: NoteDraft::default(),
            review_focus: true,
            text_field_focused: false,
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
        self.collection_modified = self.store.collection_modified().ok();
        self.external_change = false;
        self.dialog = None;
        self.reset_queue();
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "deck counts are gathered in one short pass"
    )]
    fn refresh_stats(&mut self) {
        let now = now_secs();
        let mut stats = Vec::with_capacity(self.decks.len());
        let deck_ids: Vec<ranki_model::Id> =
            self.decks.iter().map(|deck| deck.id).collect();
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
        self.toast = Some(Toast {
            text: format!("{action}: {error}"),
            created: Instant::now(),
        });
    }

    fn set_status(&mut self, text: String) {
        self.toast = None;
        self.status = Some(text);
    }

    fn selected_deck(&self) -> Option<&Deck> {
        self.selected.and_then(|index| self.decks.get(index))
    }

    fn selected_deck_id(&self) -> Option<ranki_model::Id> {
        self.selected_deck().map(|deck| deck.id)
    }

    fn selected_stats(&self) -> Option<&DeckStats> {
        self.selected.and_then(|index| self.stats.get(index))
    }

    fn reset_queue(&mut self) {
        let now = Instant::now();
        self.queue.clear();
        self.seen.clear();
        self.queue_state = QueueState::Ready;
        self.current = None;
        self.note = None;
        self.templates = None;
        self.revealed = false;
        self.focused_elapsed = Duration::ZERO;
        self.last_tick = Some(now);
        self.last_grade_at = Some(now);
        self.status = None;
        self.answered = 0;
        self.note_draft = NoteDraft::default();
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
        match self
            .store
            .due_cards(deck_id, self.due_batch_size, now_secs())
        {
            Ok(cards) => {
                let returned = cards.len();
                let mut added: usize = 0;
                for card in cards {
                    if self.seen.insert(card.id) {
                        self.queue.push_back(card);
                        added = added.saturating_add(1);
                    }
                }
                if added == 0 || returned < self.due_batch_size {
                    self.queue_state = QueueState::Exhausted;
                }
            }
            Err(error) => {
                self.set_error("Could not load due cards", &error);
                self.queue_state = QueueState::Exhausted;
            }
        }
    }

    fn ensure_current(&mut self) {
        if self.current.is_some() {
            return;
        }
        self.fill_queue();
        let Some(card) = self.queue.pop_front() else {
            return;
        };
        let now = Instant::now();
        self.revealed = false;
        self.focused_elapsed = Duration::ZERO;
        self.last_tick = Some(now);
        self.last_grade_at = Some(now);
        self.review_focus = true;
        match self.store.note(card.note_id) {
            Ok(note) => {
                let models = &self.models;
                let templates = note
                    .as_ref()
                    .and_then(|loaded| render_templates(loaded, &card, models));
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

    fn can_grade(&self, now: Instant) -> bool {
        self.last_grade_at
            .is_none_or(|last| now.duration_since(last) >= DEBOUNCE)
    }

    fn grade(&mut self, rating: Rating) {
        if !self.revealed || !self.can_grade(Instant::now()) {
            return;
        }
        let Some(card) = self.current.clone() else {
            return;
        };
        let mut decision = self.scheduler.schedule(
            &card,
            rating,
            now_secs(),
            self.collection_start_secs,
        );
        decision.revlog.time_ms = elapsed_millis(self.focused_elapsed);
        if let Err(error) = self.store.apply_review(&decision) {
            self.set_error("Could not save review", &error);
            return;
        }
        self.set_status(format!(
            "Rated {}, next in {}",
            rating_label(rating),
            interval_label(decision.revlog.interval)
        ));
        self.answered = self.answered.saturating_add(1);
        self.current = None;
        self.note = None;
        self.templates = None;
        self.revealed = false;
        self.focused_elapsed = Duration::ZERO;
        self.last_tick = Some(Instant::now());
        self.last_grade_at = self.last_tick;
        self.review_focus = true;
        self.queue_state = QueueState::Ready;
        self.refresh_stats();
        self.ensure_current();
    }

    fn tick_time(&mut self, window_focused: bool, now: Instant) {
        let Some(last) = self.last_tick else {
            self.last_tick = Some(now);
            return;
        };
        self.last_tick = Some(now);
        if self.current.is_none() {
            return;
        }
        let delta =
            focused_delta(last, now, window_focused, self.focused_elapsed);
        self.focused_elapsed = self.focused_elapsed.saturating_add(delta);
    }

    fn check_external_change(&mut self, now: Instant) -> bool {
        let due = self
            .last_modified_check
            .is_none_or(|last| now.duration_since(last) >= EXTERNAL_POLL);
        if !due {
            return false;
        }
        self.last_modified_check = Some(now);
        let Ok(value) = self.store.collection_modified() else {
            return false;
        };
        if self.collection_modified.is_some_and(|base| base != value) {
            self.external_change = true;
            return true;
        }
        false
    }

    fn request_scheduler(&mut self, requested: SchedulerKind) {
        if requested == self.scheduler_kind {
            return;
        }
        if self.answered > 0 || self.current.is_some() {
            self.dialog = Some(Dialog::Scheduler(requested));
        } else {
            self.apply_scheduler(requested);
        }
    }

    fn apply_scheduler(&mut self, kind: SchedulerKind) {
        self.scheduler_kind = kind;
        self.scheduler = make_scheduler(kind, self.desired_retention);
        self.dialog = None;
        self.set_status(format!("{} selected for this session", kind.label()));
    }

    fn request_refresh(&mut self) {
        if self.answered > 0 || self.current.is_some() {
            self.dialog = Some(Dialog::Refresh);
        } else {
            self.reload();
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "one dispatcher owns all review actions"
    )]
    fn dispatch(&mut self, action: Action) {
        match action {
            Action::Confirm => {
                if self.revealed {
                    self.grade(Rating::Good);
                } else {
                    self.reveal();
                }
            }
            Action::Rate(rating) => {
                if self.revealed {
                    self.grade(rating);
                } else {
                    self.reveal();
                }
            }
            Action::ToggleReveal => self.toggle_reveal(),
            Action::SchedulerChoice => {
                self.request_scheduler(self.scheduler_kind.other());
            }
            Action::Refresh => self.request_refresh(),
            Action::SelectNext => self.move_selection(true),
            Action::SelectPrevious => self.move_selection(false),
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "egui draws the deck list inside a scroll area"
    )]
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("Ranki").size(20.0).strong());
        ui.add_space(8.0);
        let mut choose = None;
        egui::ScrollArea::vertical().show(ui, |list_ui| {
            for index in 0..self.decks.len() {
                let Some(deck) = self.decks.get(index) else {
                    continue;
                };
                let counts = self.stats.get(index).copied().unwrap_or_default();
                let selected = self.selected == Some(index);
                list_ui.vertical(|row| {
                    let button = Button::new(
                        RichText::new(&deck.name).size(15.0).strong(),
                    )
                    .frame(selected)
                    .min_size(egui::vec2(150.0, 26.0));
                    if row.add(button).clicked() {
                        choose = Some(index);
                    }
                    row.label(
                        RichText::new(format!(
                            "{} new, {} learning, {} review",
                            counts.new_cards,
                            counts.learning_cards,
                            counts.review_cards
                        ))
                        .size(12.0)
                        .weak(),
                    );
                });
            }
        });
        if let Some(index) = choose {
            self.select_deck(index);
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Scheduler (session)").size(12.0).weak());
        ui.horizontal(|row| {
            if row
                .add(egui::RadioButton::new(
                    self.scheduler_kind == SchedulerKind::Fsrs,
                    "FSRS",
                ))
                .clicked()
            {
                self.request_scheduler(SchedulerKind::Fsrs);
            }
            if row
                .add(egui::RadioButton::new(
                    self.scheduler_kind == SchedulerKind::Sm2,
                    "SM-2",
                ))
                .clicked()
            {
                self.request_scheduler(SchedulerKind::Sm2);
            }
        });
        ui.label(
            RichText::new("Applies to this session only")
                .size(12.0)
                .weak(),
        );
        ui.add_space(8.0);
        if ui.button("Refresh").clicked() {
            self.request_refresh();
        }
    }

    fn review(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("grading").show(ui, |bar| self.grading(bar));
        self.toast(ui);
        self.external_banner(ui);
        ui.add_space(4.0);
        self.review_header(ui);
        self.progress(ui);
        ui.add_space(8.0);
        if self.current.is_some() {
            self.current_card(ui);
        } else {
            self.empty_state(ui);
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the header aligns two short labels"
    )]
    fn review_header(&self, ui: &mut egui::Ui) {
        let deck_name = self.selected_deck().map_or_else(
            || String::from("No deck selected"),
            |deck| deck.name.clone(),
        );
        ui.horizontal(|row| {
            row.label(RichText::new(deck_name).size(20.0).strong());
            row.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |end| {
                    end.label(
                        RichText::new(self.scheduler_kind.label())
                            .size(12.0)
                            .weak(),
                    );
                },
            );
        });
    }

    fn progress(&self, ui: &mut egui::Ui) {
        let remaining = self.session_total.saturating_sub(self.answered);
        let width = progress_width(ui.available_width());
        ui.horizontal(|row| {
            row.add(
                ProgressBar::new(progress_fraction(
                    self.answered,
                    self.session_total,
                ))
                .fill(theme::accent_fill(self.theme))
                .desired_height(6.0)
                .desired_width(width),
            );
            row.label(
                RichText::new(format!("{} / {}", self.answered, remaining))
                    .size(12.0)
                    .weak(),
            );
        });
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the toast is one small overlay"
    )]
    fn toast(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        if self.toast.as_ref().is_some_and(|toast| toast.expired(now)) {
            self.toast = None;
        }
        let Some(toast) = self.toast.as_ref() else {
            return;
        };
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        let text = toast.text.clone();
        let error_color = ui.visuals().error_fg_color;
        let mut dismiss = false;
        egui::Frame::NONE
            .fill(ui.visuals().widgets.noninteractive.bg_fill)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .corner_radius(8.0)
            .show(ui, |frame| {
                frame.horizontal(|row| {
                    row.label(RichText::new(text).color(error_color))
                        .on_hover_ui(|hover| {
                            hover.label("Error");
                        });
                    row.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |end| {
                            dismiss = end.button("Dismiss").clicked();
                        },
                    );
                });
            });
        if dismiss {
            self.toast = None;
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the banner is one small overlay"
    )]
    fn external_banner(&mut self, ui: &mut egui::Ui) {
        if !self.external_change {
            return;
        }
        let mut refresh = false;
        egui::Frame::NONE
            .fill(ui.visuals().faint_bg_color)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .corner_radius(8.0)
            .show(ui, |frame| {
                frame.horizontal(|row| {
                    row.label("Collection changed outside Ranki. Refresh to see new cards");
                    refresh = row.button("Refresh").clicked();
                });
            });
        if refresh {
            self.request_refresh();
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "egui renders the card stage and its media status together"
    )]
    fn current_card(&mut self, ui: &mut egui::Ui) {
        let (question, answer) = self.templates.clone().unwrap_or_else(|| {
            (
                String::from("This card has no template."),
                String::from("This card has no answer."),
            )
        });
        let media_dir = self.media_dir.clone();
        let question_size = theme::question_size(self.font_size);
        let answer_size = theme::answer_size(self.font_size);
        let revealed = self.revealed;
        let has_note = self.note.is_some();
        let fill = ui.visuals().faint_bg_color;
        let cache = &mut self.images;
        let mut notices = Vec::new();

        egui::Frame::NONE
            .fill(fill)
            .inner_margin(egui::Margin::same(24))
            .corner_radius(12.0)
            .show(ui, |stage| {
                stage.set_min_size(egui::vec2(0.0, STAGE_INNER_HEIGHT));
                stage.vertical_centered_justified(|content| {
                    notices.extend(card_view::render(
                        content,
                        &question,
                        media_dir.as_deref(),
                        question_size,
                        cache,
                    ));
                    if revealed {
                        content.add_space(12.0);
                        notices.extend(card_view::render(
                            content,
                            &answer,
                            media_dir.as_deref(),
                            answer_size,
                            cache,
                        ));
                    }
                });
            });
        if !has_note {
            notices.push(String::from("Note not found"));
        }
        notices.dedup();
        if !notices.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(notices.join(", ")).size(12.0).weak());
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the empty state is centered in one pass"
    )]
    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let kind = empty_kind(
            self.decks.len(),
            self.selected_stats().copied().unwrap_or_default(),
        );
        let copy = empty_copy(kind, self.answered);
        let mut action = false;
        ui.centered_and_justified(|center| {
            center.vertical_centered(|column| {
                column.label(RichText::new(copy.title).size(22.0).strong());
                column.add_space(6.0);
                column.label(RichText::new(copy.detail).size(14.0).weak());
                column.add_space(16.0);
                action = column
                    .add(
                        Button::new(copy.action)
                            .min_size(egui::vec2(180.0, 40.0)),
                    )
                    .clicked();
            });
        });
        if action {
            self.perform_empty_action(kind);
        }
    }

    fn perform_empty_action(&mut self, kind: EmptyKind) {
        match kind {
            EmptyKind::NoDecks => match self.store.create_deck("Default") {
                Ok(_) => self.reload(),
                Err(error) => self.set_error("Could not create deck", &error),
            },
            EmptyKind::EmptyDeck => {
                let field_count =
                    self.models.first().map_or(0, |model| model.fields.len());
                self.note_draft = NoteDraft {
                    fields: vec![String::new(); field_count],
                    tags: String::new(),
                    error: None,
                };
                self.dialog = Some(Dialog::AddNote);
            }
            EmptyKind::CaughtUp => {
                let next =
                    next_due_deck(&self.decks, &self.stats, self.selected);
                match next {
                    Some(index) => self.select_deck(index),
                    None => self.set_status(String::from(
                        "No other decks have cards due",
                    )),
                }
            }
        }
    }

    #[expect(
        clippy::if_not_else,
        reason = "the question state is the primary state"
    )]
    #[expect(
        clippy::excessive_nesting,
        reason = "the bottom panel has two states"
    )]
    fn grading(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 8.0;
        if self.current.is_none() {
            ui.label(RichText::new("No card in review").size(12.0).weak());
            return;
        }
        let focus = self.review_focus;
        if !self.revealed {
            let label = if self.show_keyboard_hints {
                "Reveal   Space"
            } else {
                "Reveal"
            };
            let (fill, ink) = (
                theme::accent_fill(self.theme),
                theme::accent_ink(self.theme),
            );
            ui.horizontal_centered(|row| {
                let response = row.add(
                    Button::new(RichText::new(label).color(ink).strong())
                        .fill(fill)
                        .min_size(egui::vec2(180.0, 40.0)),
                );
                if response.clicked() {
                    self.reveal();
                }
                if focus {
                    response.request_focus();
                }
            });
        } else {
            let previews = self.rating_previews();
            ui.columns_const::<4, _>(|columns| {
                for (column, (rating, interval)) in
                    columns.iter_mut().zip(previews)
                {
                    let (fill, ink) = rating_style(rating, self.theme);
                    let label = if self.show_keyboard_hints {
                        format!(
                            "{} {}   {interval}",
                            rating.code(),
                            rating_label(rating)
                        )
                    } else {
                        format!("{}   {interval}", rating_label(rating))
                    };
                    let response = column.add(
                        Button::new(RichText::new(label).color(ink).strong())
                            .fill(fill)
                            .min_size(egui::vec2(92.0, 40.0)),
                    );
                    if response.clicked() {
                        self.grade(rating);
                    }
                    if focus && rating == Rating::Good {
                        response.request_focus();
                    }
                }
            });
        }
        self.review_focus = false;
        let status = self.status.clone().unwrap_or_default();
        ui.horizontal(|row| {
            if status.is_empty() {
                row.label(RichText::new(" ").size(12.0));
            } else {
                row.label(RichText::new(status).size(12.0).weak());
            }
            row.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |end| {
                    if end
                        .button("?")
                        .on_hover_text("Keyboard shortcuts")
                        .clicked()
                    {
                        self.dialog = Some(Dialog::Help);
                    }
                },
            );
        });
    }

    fn rating_previews(&self) -> [(Rating, String); 4] {
        let Some(card) = self.current.as_ref() else {
            return rating_keys()
                .map(|(key, rating)| (rating, key.to_string()));
        };
        let now = now_secs();
        rating_keys().map(|(_, rating)| {
            let interval = self.scheduler.preview(
                card,
                rating,
                now,
                self.collection_start_secs,
            );
            (rating, interval_label(interval))
        })
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        match self.dialog {
            Some(Dialog::Scheduler(requested)) => {
                self.scheduler_dialog(ctx, requested);
            }
            Some(Dialog::Refresh) => self.refresh_dialog(ctx),
            Some(Dialog::Help) => self.help_dialog(ctx),
            Some(Dialog::AddNote) => self.add_note_dialog(ctx),
            None => {}
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the confirmation modal has two actions"
    )]
    fn scheduler_dialog(
        &mut self,
        ctx: &egui::Context,
        requested: SchedulerKind,
    ) {
        let mut confirm = false;
        let response = Modal::new(
            egui::Id::NULL.with_salt(egui::IdSalt::new("scheduler-dialog")),
        )
        .show(ctx, |ui| {
            ui.heading("Change scheduler?");
            ui.label(format!(
                "Switch to {} for this session? The current session will end.",
                requested.label()
            ));
            ui.horizontal(|row| {
                confirm = row.button("Switch").clicked();
                if row.button("Cancel").clicked() {
                    self.dialog = None;
                }
            });
        });
        if response.should_close() {
            self.dialog = None;
        } else if confirm {
            self.apply_scheduler(requested);
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the confirmation modal has two actions"
    )]
    fn refresh_dialog(&mut self, ctx: &egui::Context) {
        let mut confirm = false;
        let response = Modal::new(egui::Id::NULL.with_salt(egui::IdSalt::new("refresh-dialog"))).show(ctx, |ui| {
            ui.heading("Refresh collection?");
            ui.label("Refreshing ends this review session and discards session counts.");
            ui.horizontal(|row| {
                confirm = row.button("Refresh").clicked();
                if row.button("Cancel").clicked() {
                    self.dialog = None;
                }
            });
        });
        if response.should_close() {
            self.dialog = None;
        } else if confirm {
            self.reload();
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the help modal lists shortcuts"
    )]
    fn help_dialog(&mut self, ctx: &egui::Context) {
        let response = Modal::new(
            egui::Id::NULL.with_salt(egui::IdSalt::new("help-dialog")),
        )
        .show(ctx, |ui| {
            ui.heading("Keyboard shortcuts");
            ui.label("Space or Enter: reveal, then grade Good");
            ui.label("1 to 4: reveal on the question side, then grade");
            ui.label("R: hide or show the answer");
            ui.label("T: choose the session scheduler");
            ui.label("F5: refresh the collection");
            ui.label("Up or Down: change deck");
            if ui.button("Close").clicked() {
                self.dialog = None;
            }
        });
        if response.should_close() {
            self.dialog = None;
        }
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "the add-note modal maps one text field per model field"
    )]
    fn add_note_dialog(&mut self, ctx: &egui::Context) {
        let field_names: Vec<String> = self
            .models
            .first()
            .map_or_else(Vec::new, |model| model.fields.clone());
        let mut save = false;
        let mut focused = false;
        let response = Modal::new(
            egui::Id::NULL.with_salt(egui::IdSalt::new("add-note-dialog")),
        )
        .show(ctx, |ui| {
            ui.heading("Add note");
            for (field, value) in
                field_names.iter().zip(self.note_draft.fields.iter_mut())
            {
                ui.horizontal(|row| {
                    row.label(RichText::new(field).size(12.0).weak());
                    focused |= row
                        .add(
                            egui::TextEdit::singleline(value)
                                .desired_width(260.0),
                        )
                        .has_focus();
                });
            }
            ui.horizontal(|row| {
                row.label(RichText::new("Tags").size(12.0).weak());
                focused |= row
                    .add(
                        egui::TextEdit::singleline(&mut self.note_draft.tags)
                            .desired_width(260.0),
                    )
                    .has_focus();
            });
            if let Some(error) = &self.note_draft.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|row| {
                save = row.button("Save note").clicked();
                if row.button("Cancel").clicked() {
                    self.dialog = None;
                }
            });
        });
        self.text_field_focused = focused;
        if response.should_close() {
            self.dialog = None;
        } else if save {
            self.save_note();
        }
    }

    fn save_note(&mut self) {
        let Some(deck) = self.selected_deck() else {
            self.set_error(
                "Could not add note",
                &StoreError::NotFound(String::from("deck")),
            );
            return;
        };
        let Some(model) = self.models.first() else {
            self.set_error(
                "Could not add note",
                &StoreError::NotFound(String::from("model")),
            );
            return;
        };
        let fields = self.note_draft.fields.clone();
        if fields.iter().all(|field| field.trim().is_empty()) {
            self.note_draft.error =
                Some(String::from("Fill in at least one field"));
            return;
        }
        let tags = self
            .note_draft
            .tags
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let note = NewNote {
            model_id: model.id,
            deck_id: deck.id,
            fields,
            tags,
        };
        match self.store.add_note(&note) {
            Ok(ids) => {
                self.dialog = None;
                self.text_field_focused = false;
                self.refresh_stats();
                self.reset_queue();
                self.set_status(format!("Added {} cards", ids.len()));
            }
            Err(error) => self.set_error("Could not add note", &error),
        }
    }
}

impl ReviewApp {
    #[expect(
        clippy::excessive_nesting,
        reason = "input handling and two eframe panels share one frame callback"
    )]
    fn ui_impl(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        let now = Instant::now();
        let focused = ctx.input(|input| input.raw.focused);
        let system_theme = ctx.input(|input| input.raw.system_theme);
        self.theme = theme::resolve_theme(system_theme, self.theme_choice);
        self.fill_queue();
        self.ensure_current();
        self.tick_time(focused, now);
        if self.check_external_change(now) {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(EXTERNAL_POLL);

        let in_text_field = self.text_field_focused;
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
        let mut consumed = Vec::new();
        for key in keys {
            if let Some(action) =
                action_for_key(key, Modifiers::NONE, in_text_field)
            {
                self.dispatch(action);
                consumed.push(key);
            }
        }
        for key in consumed {
            ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key));
        }

        egui::Panel::left("decks")
            .resizable(true)
            .default_size(200.0)
            .min_size(160.0)
            .max_size(260.0)
            .show(ui, |panel_ui| self.sidebar(panel_ui));
        egui::CentralPanel::default()
            .show(ui, |center_ui| self.review(center_ui));
        self.dialogs(ui.ctx());
    }
}

impl eframe::App for ReviewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui_impl(ui);
    }
}

/// Review window driven without a native window, for headless frame benchmarks.
pub struct BenchApp {
    ctx: egui::Context,
    app: ReviewApp,
}

impl BenchApp {
    /// Builds the same review state the native window uses.
    #[must_use]
    pub fn new(store: Box<dyn Store>, options: &Options) -> Self {
        let ctx = egui::Context::default();
        theme::apply(&ctx, options.theme, options.font_size);
        let app = ReviewApp::new(store, options);
        Self { ctx, app }
    }

    /// Runs one full UI pass. `key` injects a key press for that frame.
    pub fn frame(&mut self, key: Option<egui::Key>) {
        let events = key
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key: pressed,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            })
            .collect();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1024.0, 768.0),
            )),
            events,
            ..Default::default()
        };
        let output = self.ctx.run_ui(input, |ui| self.app.ui_impl(ui));
        output.drop_without_applying_deltas();
    }
}

fn make_scheduler(
    kind: SchedulerKind,
    desired_retention: f32,
) -> Arc<dyn Scheduler> {
    ranki_core::from_settings(&Options {
        scheduler: kind,
        desired_retention,
        ..Options::default()
    })
}

const fn due_count(stats: &DeckStats) -> usize {
    stats
        .new_cards
        .saturating_add(stats.learning_cards)
        .saturating_add(stats.review_cards)
}

const fn progress_width(available: f32) -> f32 {
    (available - 64.0).max(80.0)
}

fn progress_fraction(answered: usize, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    let answered_value = u16::try_from(answered).unwrap_or(u16::MAX);
    let total_value = u16::try_from(total).unwrap_or(u16::MAX);
    (f32::from(answered_value) / f32::from(total_value)).clamp(0.0, 1.0)
}

#[expect(
    clippy::too_many_arguments,
    reason = "time sampling needs four inputs"
)]
fn focused_delta(
    from: Instant,
    to: Instant,
    focused: bool,
    already: Duration,
) -> Duration {
    if !focused {
        return Duration::ZERO;
    }
    let elapsed = to.duration_since(from);
    MAX_REVIEW_TIME.saturating_sub(already).min(elapsed)
}

fn elapsed_millis(elapsed: Duration) -> u32 {
    u32::try_from(elapsed.min(MAX_REVIEW_TIME).as_millis()).unwrap_or(u32::MAX)
}

fn action_for_key(
    key: Key,
    modifiers: Modifiers,
    in_text_field: bool,
) -> Option<Action> {
    if in_text_field || modifiers != Modifiers::NONE {
        return None;
    }
    if key == Key::Space || key == Key::Enter {
        Some(Action::Confirm)
    } else if key == Key::R {
        Some(Action::ToggleReveal)
    } else if key == Key::T {
        Some(Action::SchedulerChoice)
    } else if key == Key::F5 {
        Some(Action::Refresh)
    } else if key == Key::ArrowDown {
        Some(Action::SelectNext)
    } else if key == Key::ArrowUp {
        Some(Action::SelectPrevious)
    } else {
        rating_for_key(key).map(Action::Rate)
    }
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

const fn rating_style(
    rating: Rating,
    theme: Theme,
) -> (egui::Color32, egui::Color32) {
    let code = rating.code();
    (
        theme::rating_fill(code, theme),
        theme::rating_ink(code, theme),
    )
}

fn interval_label(raw_interval: i32) -> String {
    let interval = i64::from(raw_interval);
    if interval < 0 {
        let seconds = interval.saturating_neg();
        if seconds < 60 {
            format!("{seconds}s")
        } else {
            format!("{}m", seconds.div_euclid(60))
        }
    } else if interval == 0 {
        String::from("<1m")
    } else if interval < 30 {
        format!("{interval}d")
    } else if interval < 365 {
        format!("{}mo", interval.div_euclid(30))
    } else {
        format!("{}y", interval.div_euclid(365))
    }
}

const fn empty_kind(decks: usize, stats: DeckStats) -> EmptyKind {
    if decks == 0 {
        EmptyKind::NoDecks
    } else if stats.total_cards == 0 {
        EmptyKind::EmptyDeck
    } else if due_count(&stats) == 0 {
        EmptyKind::CaughtUp
    } else {
        EmptyKind::EmptyDeck
    }
}

fn empty_copy(kind: EmptyKind, answered: usize) -> EmptyCopy {
    match kind {
        EmptyKind::NoDecks => EmptyCopy {
            title: "No decks yet",
            detail: String::from("Create a deck to start studying."),
            action: "Create deck",
        },
        EmptyKind::EmptyDeck => EmptyCopy {
            title: "This deck is empty",
            detail: String::from("Add a note to create your first card."),
            action: "Add note",
        },
        EmptyKind::CaughtUp => EmptyCopy {
            title: "All caught up",
            detail: format!("You reviewed {answered} cards this session."),
            action: "Study another deck",
        },
    }
}

fn next_due_deck(
    decks: &[Deck],
    stats: &[DeckStats],
    current: Option<usize>,
) -> Option<usize> {
    if decks.len() < 2 {
        return None;
    }
    let start = current.unwrap_or(0);
    (1..decks.len()).find_map(|offset| {
        let index = (start.saturating_add(offset)).rem_euclid(decks.len());
        stats
            .get(index)
            .is_some_and(|counts| due_count(counts) > 0)
            .then_some(index)
    })
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
        cell::{Cell, RefCell},
        collections::HashMap,
        rc::Rc,
        time::{Duration, Instant},
    };

    use egui::{Context, Key, Modifiers};
    use ranki_model::{
        CardQueue, CardTemplate, CardType, ReviewDecision, Store,
    };

    use super::{
        Action, BenchApp, DEBOUNCE, Dialog, EmptyKind, MAX_REVIEW_TIME,
        Options, Rating, ReviewApp, SchedulerKind, action_for_key,
        elapsed_millis, empty_copy, empty_kind, focused_delta, interval_label,
        next_due_deck, progress_fraction,
    };

    #[derive(Clone)]
    struct MemoryStore {
        decks: Vec<ranki_model::Deck>,
        models: Vec<ranki_model::NoteModel>,
        cards: Vec<ranki_model::Card>,
        notes: HashMap<ranki_model::Id, ranki_model::Note>,
        reviews: Rc<RefCell<Vec<ReviewDecision>>>,
        modified: Rc<Cell<i64>>,
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
        fn decks(
            &self,
        ) -> Result<Vec<ranki_model::Deck>, ranki_model::StoreError> {
            Ok(self.decks.clone())
        }

        fn create_deck(
            &mut self,
            name: &str,
        ) -> Result<ranki_model::Deck, ranki_model::StoreError> {
            let id = i64::try_from(self.decks.len()).unwrap_or(i64::MAX);
            let deck = ranki_model::Deck {
                id,
                name: name.to_owned(),
                description: String::new(),
            };
            self.decks.push(deck.clone());
            Ok(deck)
        }

        fn models(
            &self,
        ) -> Result<Vec<ranki_model::NoteModel>, ranki_model::StoreError>
        {
            Ok(self.models.clone())
        }

        fn find_cards(
            &self,
            _query: &str,
        ) -> Result<Vec<ranki_model::Id>, ranki_model::StoreError> {
            Ok(self.cards.iter().map(|card| card.id).collect())
        }

        fn due_cards(
            &self,
            deck_id: Option<ranki_model::Id>,
            limit: usize,
            now_secs: i64,
        ) -> Result<Vec<ranki_model::Card>, ranki_model::StoreError> {
            Ok(self.due(deck_id, limit, now_secs))
        }

        fn card(
            &self,
            id: ranki_model::Id,
        ) -> Result<Option<ranki_model::Card>, ranki_model::StoreError>
        {
            Ok(self.cards.iter().find(|card| card.id == id).cloned())
        }

        fn note(
            &self,
            id: ranki_model::Id,
        ) -> Result<Option<ranki_model::Note>, ranki_model::StoreError>
        {
            Ok(self.notes.get(&id).cloned())
        }

        fn add_note(
            &mut self,
            _note: &ranki_model::NewNote,
        ) -> Result<Vec<ranki_model::Id>, ranki_model::StoreError> {
            Ok(Vec::new())
        }

        #[expect(
            clippy::excessive_nesting,
            reason = "the mock updates one stored card"
        )]
        fn apply_review(
            &mut self,
            decision: &ReviewDecision,
        ) -> Result<(), ranki_model::StoreError> {
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
        ) -> Result<ranki_model::DeckStats, ranki_model::StoreError> {
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

        fn collection_created(&self) -> Result<i64, ranki_model::StoreError> {
            Ok(0)
        }

        fn collection_modified(&self) -> Result<i64, ranki_model::StoreError> {
            Ok(self.modified.get())
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
            guid: String::from("guid"),
            model_id: 1,
            modified: 0,
            usn: 0,
            tags: Vec::new(),
            fields: vec![String::from("Question?"), String::from("Answer!")],
            sort_field: String::from("Question?"),
            checksum: 0,
            flags: 0,
            data: String::new(),
        };
        let model = ranki_model::NoteModel {
            id: 1,
            name: String::from("Basic"),
            fields: vec![String::from("Front"), String::from("Back")],
            templates: vec![CardTemplate {
                name: String::from("Card 1"),
                question_format: String::from("{{Front}}"),
                answer_format: String::from("{{Front}}<br>{{Back}}"),
            }],
            css: String::new(),
        };
        let mut notes = HashMap::new();
        notes.insert(10, note.clone());
        notes.insert(
            11,
            ranki_model::Note {
                id: 11,
                fields: vec![
                    String::from("Second?"),
                    String::from("Second answer"),
                ],
                sort_field: String::from("Second?"),
                ..note
            },
        );
        MemoryStore {
            decks: vec![ranki_model::Deck {
                id: 1,
                name: String::from("Default"),
                description: String::new(),
            }],
            models: vec![model],
            cards: vec![card(1, 10), card(2, 11)],
            notes,
            reviews: Rc::new(RefCell::new(Vec::new())),
            modified: Rc::new(Cell::new(0)),
        }
    }

    fn app(store: MemoryStore) -> ReviewApp {
        ReviewApp::new(Box::new(store), &Options::default())
    }

    fn advance_past_debounce(target: &mut ReviewApp) {
        target.last_grade_at = Some(
            Instant::now()
                .checked_sub(DEBOUNCE.saturating_add(Duration::from_millis(10)))
                .unwrap_or_else(Instant::now),
        );
    }

    #[test]
    fn rating_keys_map_to_anki_eases() {
        assert_eq!(
            action_for_key(Key::Num1, Modifiers::NONE, false),
            Some(Action::Rate(Rating::Again))
        );
        assert_eq!(
            action_for_key(Key::Num2, Modifiers::NONE, false),
            Some(Action::Rate(Rating::Hard))
        );
        assert_eq!(
            action_for_key(Key::Num3, Modifiers::NONE, false),
            Some(Action::Rate(Rating::Good))
        );
        assert_eq!(
            action_for_key(Key::Num4, Modifiers::NONE, false),
            Some(Action::Rate(Rating::Easy))
        );
        assert_eq!(action_for_key(Key::Num5, Modifiers::NONE, false), None);
    }

    #[test]
    fn due_queue_advances_after_review() {
        let store = fixture();
        let reviews = Rc::clone(&store.reviews);
        let mut app = app(store);
        assert_eq!(app.current.as_ref().map(|card| card.id), Some(1));
        advance_past_debounce(&mut app);
        app.reveal();
        app.grade(Rating::Good);
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
            name: String::from("Other"),
            description: String::new(),
        });
        let mut app = app(store);
        app.select_deck(1);
        assert_eq!(app.selected, Some(1));
        assert!(app.current.is_none());
        assert!(app.queue.is_empty());
        assert!(app.seen.is_empty());
    }

    #[test]
    fn reviews_record_focused_time() {
        let store = fixture();
        let reviews = Rc::clone(&store.reviews);
        let mut app = app(store);
        advance_past_debounce(&mut app);
        app.focused_elapsed = Duration::from_millis(25);
        app.reveal();
        app.grade(Rating::Easy);
        assert_eq!(reviews.borrow()[0].revlog.time_ms, 25);
    }

    #[test]
    fn first_digit_reveals_and_second_grades() {
        let store = fixture();
        let reviews = Rc::clone(&store.reviews);
        let mut app = app(store);
        app.dispatch(Action::Rate(Rating::Easy));
        assert!(app.revealed);
        assert!(reviews.borrow().is_empty());
        advance_past_debounce(&mut app);
        app.dispatch(Action::Rate(Rating::Easy));
        assert_eq!(reviews.borrow().len(), 1);
    }

    #[test]
    #[expect(
        clippy::excessive_nesting,
        reason = "the test loops over suppressed keys"
    )]
    fn text_fields_do_not_receive_review_shortcuts() {
        assert_eq!(action_for_key(Key::Num1, Modifiers::NONE, true), None);
        assert_eq!(action_for_key(Key::Space, Modifiers::NONE, true), None);
        let store = fixture();
        let reviews = Rc::clone(&store.reviews);
        let mut app = app(store);
        app.text_field_focused = true;
        for key in [Key::Num1, Key::Num2] {
            if let Some(action) = action_for_key(key, Modifiers::NONE, true) {
                app.dispatch(action);
            }
        }
        assert!(reviews.borrow().is_empty());
        assert!(!app.revealed);
    }

    #[test]
    fn grading_is_debounced() {
        let store = fixture();
        let reviews = Rc::clone(&store.reviews);
        let mut app = app(store);
        app.reveal();
        app.grade(Rating::Good);
        assert!(reviews.borrow().is_empty());
        advance_past_debounce(&mut app);
        app.grade(Rating::Good);
        assert_eq!(reviews.borrow().len(), 1);
    }

    #[test]
    fn focused_time_ignores_unfocused_frames() {
        let now = Instant::now();
        let from = now.checked_sub(Duration::from_secs(3)).unwrap_or(now);
        assert_eq!(
            focused_delta(from, now, false, Duration::ZERO),
            Duration::ZERO
        );
        assert_eq!(
            focused_delta(from, now, true, Duration::ZERO),
            Duration::from_secs(3)
        );
    }

    #[test]
    fn focused_time_is_capped_at_one_minute() {
        let now = Instant::now();
        let from = now.checked_sub(Duration::from_secs(61)).unwrap_or(now);
        let delta = focused_delta(from, now, true, Duration::ZERO);
        assert_eq!(elapsed_millis(delta), 60_000);
        assert_eq!(elapsed_millis(MAX_REVIEW_TIME.saturating_mul(4)), 60_000);
    }

    #[test]
    fn scheduler_change_asks_for_confirmation() {
        let mut app = app(fixture());
        app.answered = 1;
        app.request_scheduler(SchedulerKind::Sm2);
        assert_eq!(app.dialog, Some(Dialog::Scheduler(SchedulerKind::Sm2)));
        assert_eq!(app.scheduler_kind, SchedulerKind::Fsrs);
        app.apply_scheduler(SchedulerKind::Sm2);
        assert_eq!(app.scheduler_kind, SchedulerKind::Sm2);
        assert_eq!(app.dialog, None);
    }

    #[test]
    fn external_change_raises_banner_and_reload_clears_it() {
        let store = fixture();
        let modified = Rc::clone(&store.modified);
        let mut app = app(store);
        assert!(!app.check_external_change(Instant::now()));
        modified.set(42);
        let later = Instant::now()
            .checked_add(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        assert!(app.check_external_change(later));
        assert!(app.external_change);
        app.reload();
        assert!(!app.external_change);
    }

    #[test]
    fn grade_status_is_a_single_quiet_line() {
        let mut app = app(fixture());
        advance_past_debounce(&mut app);
        app.reveal();
        app.grade(Rating::Good);
        let status = app.status.as_deref().unwrap_or_default();
        assert!(status.starts_with("Rated Good, next in "));
        assert!(!status.contains('\n'));
    }

    #[test]
    fn interval_labels_cover_seconds_days_and_years() {
        assert_eq!(interval_label(-60), "1m");
        assert_eq!(interval_label(4), "4d");
        assert_eq!(interval_label(365), "1y");
    }

    #[test]
    fn empty_states_are_distinct_with_one_action() {
        assert_eq!(
            empty_kind(0, ranki_model::DeckStats::default()),
            EmptyKind::NoDecks
        );
        let empty = ranki_model::DeckStats::default();
        assert_eq!(empty_kind(1, empty), EmptyKind::EmptyDeck);
        let caught = ranki_model::DeckStats {
            total_cards: 5,
            ..ranki_model::DeckStats::default()
        };
        assert_eq!(empty_kind(1, caught), EmptyKind::CaughtUp);
        assert_eq!(empty_copy(EmptyKind::NoDecks, 0).action, "Create deck");
        assert_eq!(empty_copy(EmptyKind::EmptyDeck, 0).action, "Add note");
        assert_eq!(
            empty_copy(EmptyKind::CaughtUp, 3).action,
            "Study another deck"
        );
    }

    #[test]
    fn progress_is_empty_at_zero_due() {
        assert_eq!(progress_fraction(0, 0), 0.0);
        assert_eq!(progress_fraction(1, 4), 0.25);
    }

    #[test]
    fn next_due_deck_skips_the_current_deck() {
        let store = fixture();
        let mut decks = store.decks;
        decks.push(ranki_model::Deck {
            id: 2,
            name: String::from("Other"),
            description: String::new(),
        });
        let mut stats = vec![ranki_model::DeckStats::default(); 2];
        stats[1].review_cards = 3;
        assert_eq!(next_due_deck(&decks, &stats, Some(0)), Some(1));
        assert_eq!(next_due_deck(&decks, &stats, Some(1)), None);
    }

    #[test]
    fn toast_expires_after_five_seconds() {
        let created = Instant::now()
            .checked_sub(Duration::from_secs(6))
            .unwrap_or_else(Instant::now);
        let toast = super::Toast {
            text: String::from("boom"),
            created,
        };
        assert!(toast.expired(Instant::now()));
    }

    #[test]
    fn scheduler_kind_helpers_are_stable() {
        assert_eq!(SchedulerKind::Fsrs.label(), "FSRS");
        assert_eq!(SchedulerKind::Sm2.label(), "SM-2");
        assert_eq!(SchedulerKind::Fsrs.other(), SchedulerKind::Sm2);
        let _ = Context::default();
    }
    #[test]
    fn bench_app_renders_frames_without_a_window() {
        let mut app = BenchApp::new(Box::new(fixture()), &Options::default());
        app.frame(None);
        app.frame(Some(Key::Space));
    }
}
