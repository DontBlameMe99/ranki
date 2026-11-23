//! Root application state, keyboard dispatcher, and store-backed actions.

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    env,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, KeyBinding,
    Render, Window, WindowAppearance, actions, div, prelude::*,
};
use ranki_core::{
    FsrsParams, FsrsPreset, FsrsScheduler, Scheduler, from_settings,
};
use ranki_model::{
    Card, Deck, DeckStats, Id, NewNote, Note, NoteModel, Rating,
    ReviewDecision, Store,
};

use crate::{
    Options, SchedulerKind, ThemeChoice,
    card_view::{self, RenderedCard, Side},
    text_field::{EditHook, TextField},
    theme::{self, Palette, Theme, interval_label},
};

const DEBOUNCE: Duration = Duration::from_millis(120);
const TOAST_LIFETIME: Duration = Duration::from_secs(5);
const MAX_REVIEW_TIME: Duration = Duration::from_secs(60);

actions!(
    ranki_ui,
    [
        Reveal,
        GradeAgain,
        GradeHard,
        GradeGood,
        GradeEasy,
        Retry,
        EndSession,
        OpenDecks,
        OpenAdd,
        CreateDeck,
        SubmitDeck,
        SaveNote,
        CancelAdd,
        ToggleHelp,
        ToggleScheduler,
        ConfirmDialog,
        CancelDialog,
        NextDeck,
        ClearSearch,
        IgnoreShortcut,
    ]
);

/// Which full-view screen is visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Decks,
    Review,
    AddCard,
}

/// Review content phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Question,
    Answer,
}

/// One explicit deck-table row, including virtual `A::B` parents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckRow {
    pub id: Option<Id>,
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub stats: DeckStats,
    pub virtual_group: bool,
    pub has_children: bool,
    pub expanded: bool,
}

/// Deck rename state.
#[derive(Clone, Debug)]
pub struct DeckEdit {
    pub id: Id,
}

/// Pending destructive action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmation {
    RemoveDeck { id: Id, name: String },
    SwitchScheduler { kind: SchedulerKind },
}

#[derive(Clone, Debug)]
pub struct Toast {
    text: String,
    created_at: Instant,
}

impl Toast {
    fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.created_at) >= TOAST_LIFETIME
    }
}

/// One note added during this session, shown under the live preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedNote {
    pub headline: String,
    pub detail: String,
    pub time: String,
}

/// Add-card form state.
pub struct AddState {
    pub deck_index: usize,
    pub model_index: usize,
    pub fields: Vec<Entity<TextField>>,
    pub tags: Entity<TextField>,
    pub tag_chips: Vec<String>,
    pub preview_side: Side,
    pub errors: BTreeMap<String, String>,
    pub added: Vec<AddedNote>,
}

/// Review-session state owned by the root entity.
#[derive(Default)]
pub struct ReviewState {
    pub queue: VecDeque<Card>,
    pub seen: HashSet<Id>,
    pub current: Option<Card>,
    pub note: Option<Note>,
    pub question: Option<RenderedCard>,
    pub answer: Option<RenderedCard>,
    pub phase: Phase,
    pub focused_elapsed: Duration,
    pub last_tick: Option<Instant>,
    pub last_grade_at: Option<Instant>,
    pub answered: usize,
    pub session_total: usize,
    pub again_count: usize,
    pub scope_all: bool,
    pub status: Option<String>,
}

/// Root entity. The store boundary stays identical to the previous UI.
pub struct Root {
    pub(crate) store: Box<dyn Store>,
    pub(crate) options: Options,
    pub(crate) focus: FocusHandle,
    pub(crate) system_dark: bool,
    pub(crate) view: View,
    pub(crate) previous_view: View,
    pub(crate) decks: Vec<Deck>,
    pub(crate) models: Vec<NoteModel>,
    pub(crate) stats: Vec<DeckStats>,
    pub(crate) deck_rows: Vec<DeckRow>,
    pub(crate) selected: Option<usize>,
    pub(crate) expanded: HashSet<String>,
    pub(crate) deck_search: Entity<TextField>,
    pub(crate) create_field: Entity<TextField>,
    pub(crate) rename_field: Entity<TextField>,
    pub(crate) deck_edit: Option<DeckEdit>,
    pub(crate) deck_menu: Option<Id>,
    pub(crate) confirmation: Option<Confirmation>,
    pub(crate) review: ReviewState,
    pub(crate) add: AddState,
    pub(crate) scheduler: Arc<dyn Scheduler>,
    pub(crate) toast: Option<Toast>,
    pub(crate) help_open: bool,
    pub(crate) dev_reveal: bool,
    pub(crate) external_change: bool,
    pub(crate) collection_modified: i64,
    pub(crate) images: Entity<card_view::LruImageCache>,
    pub(crate) font_family: &'static str,
}

/// Values the window needs before the root entity exists.
pub struct Launch {
    pub store: Box<dyn Store>,
    pub options: Options,
}

impl Root {
    pub(crate) const fn style(&self) -> crate::theme::Style {
        crate::theme::Style::new(self.colors(), self.theme())
    }
}

impl Focusable for Root {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Builds the session scheduler, honoring the optional FSRS settings fields.
///
/// `Settings` is serialized so newer fields are read without changing the
/// shared model or CLI. Missing fields use the scheduler defaults.
#[must_use]
pub fn scheduler_from_options(options: &Options) -> Arc<dyn Scheduler> {
    let mut scheduler = from_settings(options);
    if options.scheduler != SchedulerKind::Fsrs {
        return scheduler;
    }
    let Ok(value) = serde_json::to_value(options) else {
        return scheduler;
    };
    let weights = value
        .get("fsrs_params")
        .and_then(|field| {
            serde_json::from_value::<Vec<f32>>(field.clone()).ok()
        })
        .and_then(|weights| FsrsParams::from_slice(&weights));
    let Some(mut params) = weights else {
        return scheduler;
    };
    if let Some(decay) = value
        .get("fsrs_decay")
        .and_then(|field| serde_json::from_value::<f32>(field.clone()).ok())
    {
        let mut values = *params.weights();
        if let Some(last) = values.last_mut() {
            *last = decay;
        }
        if let Some(updated) = FsrsParams::from_fsrs6(&values) {
            params = updated;
        }
    }
    let preset = FsrsPreset {
        desired_retention: options.desired_retention,
        ..FsrsPreset::default()
    };
    scheduler = Arc::new(FsrsScheduler::new(preset).with_params(params));
    scheduler
}

impl Root {
    #[must_use]
    pub fn new(
        launch: Launch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Launch { store, options } = launch;
        let focus = cx.focus_handle();
        window.focus(&focus);
        let system_dark = !matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        );
        let hook = Self::edit_hook(cx);
        let make_field = |cx: &mut Context<TextField>,
                          placeholder: &'static str| {
            let mut field = TextField::new(cx, placeholder);
            field.set_edit_hook(hook.clone());
            field
        };
        let deck_search = cx.new(|cx| make_field(cx, "Search decks"));
        let create_field = cx.new(|cx| make_field(cx, "Deck name"));
        let rename_field = cx.new(|cx| make_field(cx, "New name"));
        let tags = cx.new(|cx| make_field(cx, "Add a tag"));
        let tag_hook = Self::tag_hook(cx);
        tags.update(cx, |field, _| field.set_edit_hook(tag_hook));
        let mut root = Self {
            scheduler: scheduler_from_options(&options),
            store,
            options,
            focus,
            system_dark,
            view: View::Decks,
            previous_view: View::Decks,
            decks: Vec::new(),
            models: Vec::new(),
            stats: Vec::new(),
            deck_rows: Vec::new(),
            selected: None,
            expanded: HashSet::new(),
            deck_search,
            create_field,
            rename_field,
            deck_edit: None,
            deck_menu: None,
            confirmation: None,
            review: ReviewState::default(),
            add: AddState {
                deck_index: 0,
                model_index: 0,
                fields: Vec::new(),
                tags,
                tag_chips: Vec::new(),
                preview_side: Side::Question,
                errors: BTreeMap::new(),
                added: Vec::new(),
            },
            toast: None,
            help_open: false,
            dev_reveal: env::var("RANKI_UI_DEV_REVEAL")
                .is_ok_and(|value| value == "1"),
            external_change: false,
            collection_modified: 0,
            images: card_view::LruImageCache::new(cx),
            font_family: theme::ui_font_family(),
        };
        root.reload(cx);
        root.reset_fields(cx);
        root.refresh_field_colors(cx);
        if let Ok(prefill) = env::var("RANKI_UI_DEV_FIELDS") {
            for (field, value) in
                root.add.fields.iter().zip(prefill.split('\t'))
            {
                field.update(cx, |field, cx| {
                    field.seed(value.to_owned(), cx);
                });
            }
        }
        if let Ok(tags) = env::var("RANKI_UI_DEV_TAGS") {
            root.add.tag_chips =
                tags.split_whitespace().map(str::to_owned).collect();
        }
        match env::var("RANKI_UI_DEV_VIEW").as_deref() {
            Ok("add") => root.view = View::AddCard,
            Ok("question") => {
                root.study_all(cx);
                root.review.phase = Phase::Question;
            }
            Ok("answer") => {
                root.study_all(cx);
                root.review.phase = Phase::Answer;
            }
            _ => {}
        }
        root
    }
    /// Keeps the chip list in step with the tag field as words are typed.
    fn tag_hook(cx: &mut Context<Self>) -> EditHook {
        let weak = cx.entity().downgrade();
        Arc::new(move |app| {
            let _ = weak.update(app, |root, cx| {
                root.sync_tags(cx);
                cx.notify();
            });
        })
    }

    pub(crate) fn sync_tags(&mut self, cx: &mut Context<Self>) {
        let text = self.add.tags.read(cx).text();
        let remainder = collect_tags(&mut self.add.tag_chips, &text);
        if remainder != text {
            self.add
                .tags
                .update(cx, |field, cx| field.seed(remainder, cx));
        }
    }

    fn edit_hook(cx: &mut Context<Self>) -> EditHook {
        let weak = cx.entity().downgrade();
        Arc::new(move |app| {
            let _ = weak.update(app, |_, cx| cx.notify());
        })
    }

    pub(crate) fn observe_appearance(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.options.theme != ThemeChoice::System {
            return;
        }
        cx.observe_window_appearance(window, |this, window, cx| {
            this.system_dark = !matches!(
                window.appearance(),
                WindowAppearance::Light | WindowAppearance::VibrantLight
            );
            this.refresh_field_colors(cx);
            cx.notify();
        })
        .detach();
    }

    pub(crate) const fn theme(&self) -> Theme {
        theme::resolve_theme(self.system_dark, self.options.theme)
    }

    pub(crate) const fn colors(&self) -> Palette {
        theme::palette(self.theme())
    }

    pub(crate) fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .try_into()
            .unwrap_or(0)
    }

    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let now = Self::now();
        match self.store.decks() {
            Ok(decks) => self.decks = decks,
            Err(error) => {
                self.set_error(format!("Could not load decks: {error}"));
            }
        }
        match self.store.models() {
            Ok(models) => self.models = models_with_stock(models),
            Err(error) => {
                self.set_error(format!("Could not load note types: {error}"));
            }
        }
        self.stats = self
            .decks
            .iter()
            .map(|deck| {
                self.store.stats(Some(deck.id), now).unwrap_or_default()
            })
            .collect();
        self.collection_modified =
            self.store.collection_modified().unwrap_or(0);
        self.external_change = false;
        if self.selected.is_none_or(|index| index >= self.decks.len()) {
            self.selected = (!self.decks.is_empty()).then_some(0);
        }
        self.rebuild_deck_rows();
        self.reset_fields(cx);
        cx.notify();
    }

    pub(crate) fn set_error(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            created_at: Instant::now(),
        });
    }

    pub(crate) fn clear_toast(&mut self) {
        self.toast = None;
    }

    pub(crate) fn toast_text(&self) -> Option<&str> {
        self.toast
            .as_ref()
            .filter(|toast| !toast.expired(Instant::now()))
            .map(|toast| toast.text.as_str())
    }

    fn reset_fields(&mut self, cx: &mut Context<Self>) {
        let model = self.models.get(self.add.model_index);
        let names = model.map_or_else(Vec::new, |item| item.fields.clone());
        let hook = Self::edit_hook(cx);
        self.add.fields = names
            .into_iter()
            .map(|name| {
                cx.new(|cx| {
                    let mut field = TextField::new(cx, name);
                    field.set_edit_hook(hook.clone());
                    field
                })
            })
            .collect();
    }

    pub(crate) fn select_model(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index >= self.models.len() {
            return;
        }
        self.add.model_index = index;
        self.add.errors.clear();
        self.reset_fields(cx);
        cx.notify();
    }

    pub(crate) fn study_deck(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(deck) = self.decks.get(index) {
            self.selected = Some(index);
            self.review = ReviewState::default();
            let now = Self::now();
            match self.store.due_cards(
                Some(deck.id),
                self.options.review_batch_size,
                now,
            ) {
                Ok(cards) => self.review.queue.extend(cards),
                Err(error) => {
                    self.set_error(format!("Could not load cards: {error}"));
                }
            }
            self.review.session_total = self.review.queue.len();
            self.previous_view = View::Decks;
            self.view = View::Review;
            self.ensure_current(cx);
            cx.notify();
        }
    }

    pub(crate) fn study_all(&mut self, cx: &mut Context<Self>) {
        self.review = ReviewState::default();
        self.review.scope_all = true;
        let now = Self::now();
        match self
            .store
            .due_cards(None, self.options.review_batch_size, now)
        {
            Ok(cards) => self.review.queue.extend(cards),
            Err(error) => {
                self.set_error(format!("Could not load cards: {error}"));
            }
        }
        self.review.session_total = self.review.queue.len();
        self.previous_view = View::Decks;
        self.view = View::Review;
        self.ensure_current(cx);
        cx.notify();
    }

    pub(crate) fn end_session(&mut self, cx: &mut Context<Self>) {
        self.review.queue.clear();
        self.review.current = None;
        self.review.phase = Phase::Question;
        self.view = View::Decks;
        cx.notify();
    }

    /// Placeholder ink must follow the palette, including live appearance swaps.
    pub(crate) fn refresh_field_colors(&mut self, cx: &mut Context<Self>) {
        let color = gpui::Hsla::from(theme::placeholder_color(self.theme()));
        let fields = [
            &self.deck_search,
            &self.create_field,
            &self.rename_field,
            &self.add.tags,
        ];
        for field in fields {
            field.update(cx, |field, _| field.set_placeholder_color(color));
        }
        for field in &self.add.fields {
            field.update(cx, |field, _| field.set_placeholder_color(color));
        }
    }

    fn ensure_current(&mut self, cx: &mut Context<Self>) {
        while self.review.current.is_none()
            && let Some(card) = self.review.queue.pop_front()
        {
            if self.review.seen.contains(&card.id) {
                continue;
            }
            let _ = self.review.seen.insert(card.id);
            let note = self.store.note(card.note_id).ok().flatten();
            let model = note.as_ref().and_then(|note| {
                self.models.iter().find(|model| model.id == note.model_id)
            });
            let (question, answer) = match (note.as_ref(), model) {
                (Some(note), Some(model)) => (
                    Some(card_view::render_card(
                        model,
                        note,
                        card_view::CardRequest::question(card.ordinal),
                    )),
                    Some(card_view::render_card(
                        model,
                        note,
                        card_view::CardRequest::answer_only(card.ordinal),
                    )),
                ),
                _ => (None, None),
            };
            self.review.phase = if self.dev_reveal {
                Phase::Answer
            } else {
                Phase::Question
            };
            self.review.current = Some(card);
            self.review.note = note;
            self.review.question = question;
            self.review.answer = answer;
            self.review.last_tick = Some(Instant::now());
            self.review.last_grade_at = Some(Instant::now());
            break;
        }
        if self.review.current.is_none() {
            self.review.question = None;
            self.review.answer = None;
            self.review.note = None;
        }
        let _ = cx;
    }

    pub(crate) fn reveal(&mut self, cx: &mut Context<Self>) {
        if self.review.current.is_some() && self.review.phase == Phase::Question
        {
            self.review.phase = Phase::Answer;
            self.review.last_tick = Some(Instant::now());
            cx.notify();
        }
    }

    pub(crate) fn grade(&mut self, rating: Rating, cx: &mut Context<Self>) {
        let Some(card) = self.review.current.clone() else {
            return;
        };
        if self.review.phase != Phase::Answer {
            self.reveal(cx);
            return;
        }
        let now = Instant::now();
        if self
            .review
            .last_grade_at
            .is_some_and(|last| now.saturating_duration_since(last) < DEBOUNCE)
        {
            return;
        }
        let collection_start = self.store.collection_created().unwrap_or(0);
        let decision: ReviewDecision = self.scheduler.schedule(
            &card,
            rating,
            Self::now(),
            collection_start,
        );
        if let Err(error) = self.store.apply_review(&decision) {
            self.set_error(format!("Could not save review: {error}"));
            return;
        }
        self.review.answered = self.review.answered.saturating_add(1);
        if rating == Rating::Again {
            self.review.again_count = self.review.again_count.saturating_add(1);
        }
        self.review.status = Some(format!(
            "Rated {}, next in {}",
            rating_name(rating),
            interval_label(decision.revlog.interval)
        ));
        self.review.focused_elapsed = Duration::ZERO;
        self.review.current = None;
        self.review.note = None;
        self.review.question = None;
        self.review.answer = None;
        self.review.last_grade_at = Some(now);
        self.review.last_tick = None;
        self.ensure_current(cx);
        cx.notify();
    }

    pub(crate) fn toggle_scheduler(&mut self, cx: &mut Context<Self>) {
        let other = self.options.scheduler.other();
        self.confirmation = Some(Confirmation::SwitchScheduler { kind: other });
        cx.notify();
    }

    pub(crate) fn confirm(&mut self, cx: &mut Context<Self>) {
        match self.confirmation.take() {
            Some(Confirmation::RemoveDeck { id, .. }) => {
                match self.store.remove_deck(id) {
                    Ok(()) => {
                        self.set_error("Deck removed");
                        self.reload(cx);
                    }
                    Err(error) => self
                        .set_error(format!("Could not remove deck: {error}")),
                }
            }
            Some(Confirmation::SwitchScheduler { kind }) => {
                self.options.scheduler = kind;
                self.scheduler = scheduler_from_options(&self.options);
                self.clear_toast();
            }
            None => {}
        }
        cx.notify();
    }

    pub(crate) fn create_deck(&mut self, cx: &mut Context<Self>) {
        let name = self.create_field.read(cx).text().trim().to_owned();
        if name.is_empty() {
            self.set_error("Deck name cannot be empty");
            cx.notify();
            return;
        }
        match self.store.create_deck(&name) {
            Ok(_) => {
                self.create_field
                    .update(cx, super::text_field::TextField::clear);
                self.clear_toast();
                self.reload(cx);
            }
            Err(error) => {
                self.set_error(format!("Could not create deck: {error}"));
            }
        }
    }

    pub(crate) fn start_rename(&mut self, deck: &Deck, cx: &mut Context<Self>) {
        self.deck_edit = Some(DeckEdit { id: deck.id });
        self.rename_field
            .update(cx, |field, cx| field.set_text(deck.name.clone(), cx));
        cx.notify();
    }

    pub(crate) fn apply_rename(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.deck_edit.clone() else {
            return;
        };
        let name = self.rename_field.read(cx).text().trim().to_owned();
        if name.is_empty() {
            self.set_error("Deck name cannot be empty");
            cx.notify();
            return;
        }
        match self.store.rename_deck(edit.id, &name) {
            Ok(_) => {
                self.deck_edit = None;
                self.clear_toast();
                self.reload(cx);
            }
            Err(error) => {
                self.set_error(format!("Could not rename deck: {error}"));
            }
        }
    }

    pub(crate) fn save_note(&mut self, cx: &mut Context<Self>) {
        let Some(deck) = self.decks.get(self.add.deck_index).cloned() else {
            self.add
                .errors
                .insert("deck".into(), "Choose a deck first".into());
            cx.notify();
            return;
        };
        let Some(model) = self.models.get(self.add.model_index).cloned() else {
            self.add
                .errors
                .insert("model".into(), "Choose a note type first".into());
            cx.notify();
            return;
        };
        let fields = self
            .add
            .fields
            .iter()
            .map(|field| field.read(cx).text())
            .collect::<Vec<_>>();
        let mut tags = self.add.tag_chips.clone();
        for tag in self.add.tags.read(cx).text().split_whitespace() {
            if !tags.iter().any(|known| known == tag) {
                tags.push(tag.to_owned());
            }
        }
        let errors = validate_draft(&deck, &model, &fields);
        self.add.errors = errors;
        if !self.add.errors.is_empty() {
            cx.notify();
            return;
        }
        let headline = fields
            .iter()
            .map(|value| value.trim())
            .find(|value| !value.is_empty())
            .map(|value| value.chars().take(32).collect::<String>())
            .unwrap_or_default();
        let note = NewNote {
            model_id: model.id,
            deck_id: deck.id,
            fields,
            tags,
        };
        match self.store.add_note(&note) {
            Ok(ids) => {
                self.add.added.push(AddedNote {
                    headline,
                    detail: model.name,
                    time: clock_label(),
                });
                let count = ids.len();
                self.clear_toast();
                for field in &self.add.fields {
                    field.update(cx, super::text_field::TextField::clear);
                }
                self.add
                    .tags
                    .update(cx, super::text_field::TextField::clear);
                self.add.errors.clear();
                self.reload(cx);
                self.set_error(format!("Added {count} cards"));
            }
            Err(error) => {
                self.set_error(format!("Could not add note: {error}"));
            }
        }
        cx.notify();
    }

    pub(crate) fn rebuild_deck_rows(&mut self) {
        self.deck_rows =
            build_deck_rows(&self.decks, &self.stats, &self.expanded);
    }

    pub(crate) fn toggle_deck(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.expanded.remove(&path) {
            let _ = self.expanded.insert(path);
        }
        self.rebuild_deck_rows();
        cx.notify();
    }

    pub(crate) fn tick_focus(&mut self, window: &mut Window) {
        let now = Instant::now();
        let active = window.is_window_active()
            && self.view == View::Review
            && self.review.current.is_some();
        let Some(last) = self.review.last_tick else {
            self.review.last_tick = Some(now);
            return;
        };
        let elapsed = now.saturating_duration_since(last);
        self.review.focused_elapsed =
            focused_delta(elapsed, active, self.review.focused_elapsed);
        self.review.last_tick = Some(now);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one dispatcher entry point keeps every key and click handler on the same path"
    )]
    pub(crate) fn dispatch(
        &mut self,
        action: UiAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            UiAction::Reveal => self.reveal(cx),
            UiAction::Grade(Rating::Again) => self.grade(Rating::Again, cx),
            UiAction::Grade(Rating::Hard) => self.grade(Rating::Hard, cx),
            UiAction::Grade(Rating::Good) => self.grade(Rating::Good, cx),
            UiAction::Grade(Rating::Easy) => self.grade(Rating::Easy, cx),
            UiAction::Retry => self.reveal(cx),
            UiAction::EndSession => {
                if self.view == View::Review {
                    self.end_session(cx);
                } else if self.confirmation.is_some() || self.help_open {
                    self.confirmation = None;
                    self.help_open = false;
                    cx.notify();
                }
            }
            UiAction::OpenDecks => {
                self.view = next_view(self.view, View::Decks);
                cx.notify();
            }
            UiAction::OpenAdd => {
                self.previous_view = self.view;
                self.view = next_view(self.view, View::AddCard);
                self.add.errors.clear();
                cx.notify();
            }
            UiAction::CreateDeck => {
                self.view = View::Decks;
                window.focus(self.create_field.read(cx).focus_handle());
                cx.notify();
            }
            UiAction::SubmitDeck => self.create_deck(cx),
            UiAction::SaveNote => self.save_note(cx),
            UiAction::CancelAdd => {
                self.view = next_view(self.view, self.previous_view);
                self.add.errors.clear();
                cx.notify();
            }
            UiAction::ToggleHelp => {
                self.help_open = !self.help_open;
                cx.notify();
            }
            UiAction::ToggleScheduler => self.toggle_scheduler(cx),
            UiAction::Confirm => self.confirm(cx),
            UiAction::CancelDialog => {
                self.confirmation = None;
                cx.notify();
            }
            UiAction::NextDeck => {
                let next =
                    next_due_deck(&self.decks, &self.stats, self.selected);
                if let Some(index) = next {
                    self.study_deck(index, cx);
                }
            }
            UiAction::Ignore => {}
            UiAction::ClearSearch => {
                self.deck_search
                    .update(cx, super::text_field::TextField::clear);
                self.view = View::Decks;
                cx.notify();
            }
        }
    }
}

/// UI action routed through the single dispatcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiAction {
    Reveal,
    Grade(Rating),
    Retry,
    EndSession,
    OpenDecks,
    OpenAdd,
    CreateDeck,
    SubmitDeck,
    SaveNote,
    CancelAdd,
    ToggleHelp,
    ToggleScheduler,
    Confirm,
    CancelDialog,
    NextDeck,
    ClearSearch,
    Ignore,
}

impl From<&Reveal> for UiAction {
    fn from(_: &Reveal) -> Self {
        Self::Reveal
    }
}
impl From<&GradeAgain> for UiAction {
    fn from(_: &GradeAgain) -> Self {
        Self::Grade(Rating::Again)
    }
}
impl From<&GradeHard> for UiAction {
    fn from(_: &GradeHard) -> Self {
        Self::Grade(Rating::Hard)
    }
}
impl From<&GradeGood> for UiAction {
    fn from(_: &GradeGood) -> Self {
        Self::Grade(Rating::Good)
    }
}
impl From<&GradeEasy> for UiAction {
    fn from(_: &GradeEasy) -> Self {
        Self::Grade(Rating::Easy)
    }
}
impl From<&Retry> for UiAction {
    fn from(_: &Retry) -> Self {
        Self::Retry
    }
}
impl From<&EndSession> for UiAction {
    fn from(_: &EndSession) -> Self {
        Self::EndSession
    }
}
impl From<&OpenDecks> for UiAction {
    fn from(_: &OpenDecks) -> Self {
        Self::OpenDecks
    }
}
impl From<&OpenAdd> for UiAction {
    fn from(_: &OpenAdd) -> Self {
        Self::OpenAdd
    }
}
impl From<&CreateDeck> for UiAction {
    fn from(_: &CreateDeck) -> Self {
        Self::CreateDeck
    }
}
impl From<&SubmitDeck> for UiAction {
    fn from(_: &SubmitDeck) -> Self {
        Self::SubmitDeck
    }
}
impl From<&SaveNote> for UiAction {
    fn from(_: &SaveNote) -> Self {
        Self::SaveNote
    }
}
impl From<&CancelAdd> for UiAction {
    fn from(_: &CancelAdd) -> Self {
        Self::CancelAdd
    }
}
impl From<&ToggleHelp> for UiAction {
    fn from(_: &ToggleHelp) -> Self {
        Self::ToggleHelp
    }
}
impl From<&ToggleScheduler> for UiAction {
    fn from(_: &ToggleScheduler) -> Self {
        Self::ToggleScheduler
    }
}
impl From<&ConfirmDialog> for UiAction {
    fn from(_: &ConfirmDialog) -> Self {
        Self::Confirm
    }
}
impl From<&CancelDialog> for UiAction {
    fn from(_: &CancelDialog) -> Self {
        Self::CancelDialog
    }
}
impl From<&NextDeck> for UiAction {
    fn from(_: &NextDeck) -> Self {
        Self::NextDeck
    }
}
impl From<&IgnoreShortcut> for UiAction {
    fn from(_: &IgnoreShortcut) -> Self {
        Self::Ignore
    }
}
impl From<&ClearSearch> for UiAction {
    fn from(_: &ClearSearch) -> Self {
        Self::ClearSearch
    }
}
/// Dispatches one of the app's GPUI actions from a pointer handler.
pub fn dispatch_app_action(cx: &mut App, action: UiAction) {
    match action {
        UiAction::Reveal => cx.dispatch_action(&Reveal),
        UiAction::Grade(Rating::Again) => cx.dispatch_action(&GradeAgain),
        UiAction::Grade(Rating::Hard) => cx.dispatch_action(&GradeHard),
        UiAction::Grade(Rating::Good) => cx.dispatch_action(&GradeGood),
        UiAction::Grade(Rating::Easy) => cx.dispatch_action(&GradeEasy),
        UiAction::Retry => cx.dispatch_action(&Retry),
        UiAction::EndSession => cx.dispatch_action(&EndSession),
        UiAction::OpenDecks => cx.dispatch_action(&OpenDecks),
        UiAction::OpenAdd => cx.dispatch_action(&OpenAdd),
        UiAction::CreateDeck => cx.dispatch_action(&CreateDeck),
        UiAction::SubmitDeck => cx.dispatch_action(&SubmitDeck),
        UiAction::SaveNote => cx.dispatch_action(&SaveNote),
        UiAction::CancelAdd => cx.dispatch_action(&CancelAdd),
        UiAction::ToggleHelp => cx.dispatch_action(&ToggleHelp),
        UiAction::ToggleScheduler => cx.dispatch_action(&ToggleScheduler),
        UiAction::Confirm => cx.dispatch_action(&ConfirmDialog),
        UiAction::CancelDialog => cx.dispatch_action(&CancelDialog),
        UiAction::NextDeck => cx.dispatch_action(&NextDeck),
        UiAction::ClearSearch => cx.dispatch_action(&ClearSearch),
        UiAction::Ignore => {}
    }
}

impl Render for Root {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        self.tick_focus(window);
        let colors = self.colors();
        let view = self.view;
        let main = match view {
            View::Decks => self.render_decks(window, cx).into_any_element(),
            View::Review => self.render_review(window, cx).into_any_element(),
            View::AddCard => {
                self.render_add_card(window, cx).into_any_element()
            }
        };
        div()
            .id("root")
            .size_full()
            .flex()
            .key_context("ranki")
            .bg(colors.base)
            .text_color(colors.text)
            .font_family(self.font_family)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, action: &Reveal, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &GradeAgain, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &GradeHard, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &GradeGood, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &GradeEasy, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &Retry, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &EndSession, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &OpenDecks, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &OpenAdd, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &CreateDeck, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &SubmitDeck, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &SaveNote, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &CancelAdd, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &ToggleHelp, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(
                |this, action: &ToggleScheduler, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .on_action(cx.listener(
                |this, action: &ConfirmDialog, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .on_action(cx.listener(
                |this, action: &CancelDialog, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .on_action(cx.listener(|this, action: &ClearSearch, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &NextDeck, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(
                |this, action: &IgnoreShortcut, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .when(view != View::Review, |this| this.child(self.render_nav(cx)))
            .child(div().flex_1().overflow_hidden().child(main))
            .children(self.render_overlay(cx))
    }
}

/// Builds deck rows with virtual parents and stable depth-first ordering.
/// Sums only decks that are not prefixes of another deck.
#[must_use]
pub fn leaf_deck_totals(decks: &[Deck], stats: &[DeckStats]) -> DeckStats {
    let mut total = DeckStats::default();
    for (index, deck) in decks.iter().enumerate() {
        let prefix = format!("{}::", deck.name);
        if decks.iter().any(|other| other.name.starts_with(&prefix)) {
            continue;
        }
        let Some(stat) = stats.get(index) else {
            continue;
        };
        total.new_cards = total.new_cards.saturating_add(stat.new_cards);
        total.learning_cards =
            total.learning_cards.saturating_add(stat.learning_cards);
        total.review_cards =
            total.review_cards.saturating_add(stat.review_cards);
        total.total_cards = total.total_cards.saturating_add(stat.total_cards);
    }
    total
}

#[must_use]
pub fn build_deck_rows(
    decks: &[Deck],
    stats: &[DeckStats],
    expanded: &HashSet<String>,
) -> Vec<DeckRow> {
    let mut nodes: Vec<(String, Option<&Deck>, DeckStats)> = Vec::new();
    for (deck, stat) in decks.iter().zip(stats.iter()) {
        let path = deck.name.trim_matches(':').to_owned();
        let segments = path.split("::").collect::<Vec<_>>();
        let mut prefix = String::new();
        for (index, segment) in segments.iter().enumerate() {
            if index > 0 {
                prefix.push_str("::");
            }
            prefix.push_str(segment);
            if index + 1 == segments.len() {
                nodes.push((prefix.clone(), Some(deck), *stat));
            } else if !nodes.iter().any(|(path, _, _)| path == &prefix) {
                nodes.push((prefix.clone(), None, DeckStats::default()));
            }
        }
    }
    nodes.sort_by(|left, right| left.0.cmp(&right.0));
    let mut children: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, (path, _, _)) in nodes.iter().enumerate() {
        let parent = path.rsplit_once("::").map_or("", |(parent, _)| parent);
        children.entry(parent.to_owned()).or_default().push(index);
    }
    fn subtree(
        index: usize,
        nodes: &[(String, Option<&Deck>, DeckStats)],
        children: &BTreeMap<String, Vec<usize>>,
    ) -> DeckStats {
        let (_, deck, stats) = nodes[index];
        let mut total = if deck.is_some() {
            stats
        } else {
            DeckStats::default()
        };
        if let Some(kids) = children.get(&nodes[index].0) {
            for kid in kids {
                let child = subtree(*kid, nodes, children);
                total.new_cards =
                    total.new_cards.saturating_add(child.new_cards);
                total.learning_cards =
                    total.learning_cards.saturating_add(child.learning_cards);
                total.review_cards =
                    total.review_cards.saturating_add(child.review_cards);
                total.total_cards =
                    total.total_cards.saturating_add(child.total_cards);
            }
        }
        total
    }
    struct Tree<'a> {
        nodes: &'a [(String, Option<&'a Deck>, DeckStats)],
        children: &'a BTreeMap<String, Vec<usize>>,
        expanded: &'a HashSet<String>,
        rows: Vec<DeckRow>,
    }

    impl Tree<'_> {
        fn visit(&mut self, path: &str, depth: usize) {
            let Some(indices) = self.children.get(path) else {
                return;
            };
            for index in indices.clone() {
                let child_path = self.nodes[index].0.clone();
                let deck = self.nodes[index].1;
                let stats = self.nodes[index].2;
                let has_children = self.children.contains_key(&child_path);
                let aggregate = if has_children {
                    subtree(index, self.nodes, self.children)
                } else {
                    stats
                };
                let name = child_path
                    .rsplit("::")
                    .next()
                    .unwrap_or(&child_path)
                    .to_owned();
                let expanded = self.expanded.contains(&child_path);
                self.rows.push(DeckRow {
                    id: deck.map(|item| item.id),
                    path: child_path.clone(),
                    name,
                    depth,
                    stats: aggregate,
                    virtual_group: deck.is_none(),
                    has_children,
                    expanded,
                });
                if expanded {
                    self.visit(&child_path, depth.saturating_add(1));
                }
            }
        }
    }

    let mut tree = Tree {
        nodes: &nodes,
        children: &children,
        expanded,
        rows: Vec::new(),
    };
    tree.visit("", 0);
    tree.rows
}

#[must_use]
pub const fn due_total(stats: DeckStats) -> usize {
    stats
        .new_cards
        .saturating_add(stats.learning_cards)
        .saturating_add(stats.review_cards)
}

/// Sums only real leaf decks so parents never double count.
#[must_use]
#[cfg(test)]
pub fn sum_leaf_counts(rows: &[DeckRow]) -> DeckStats {
    let mut total = DeckStats::default();
    for row in rows {
        if row.has_children || row.virtual_group {
            continue;
        }
        total.new_cards = total.new_cards.saturating_add(row.stats.new_cards);
        total.learning_cards = total
            .learning_cards
            .saturating_add(row.stats.learning_cards);
        total.review_cards =
            total.review_cards.saturating_add(row.stats.review_cards);
        total.total_cards =
            total.total_cards.saturating_add(row.stats.total_cards);
    }
    total
}

/// Keeps a matching row and every ancestor of a match.
#[must_use]
pub fn filter_decks(rows: &[DeckRow], query: &str) -> Vec<DeckRow> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return rows.to_vec();
    }
    let matched = rows
        .iter()
        .filter(|row| row.path.to_lowercase().contains(&needle))
        .map(|row| row.path.clone())
        .collect::<Vec<_>>();
    rows.iter()
        .filter(|row| {
            matched.iter().any(|path| {
                path == &row.path
                    || path.starts_with(&format!("{}::", row.path))
                    || row.path.starts_with(&format!("{path}::"))
            })
        })
        .cloned()
        .collect()
}

/// Resolves a note-type tab to a model from the store.
#[must_use]
pub fn resolve_model(models: &[NoteModel], index: usize) -> Option<&NoteModel> {
    models.get(index)
}

/// Stock names surfaced when a collection has no note types yet.
#[must_use]
pub fn note_type_name(model: &NoteModel) -> &'static str {
    let name = model.name.to_lowercase();
    if model.is_image_occlusion() || name.contains("occlusion") {
        "Image Occlusion"
    } else if name.contains("cloze") {
        "Cloze"
    } else if name.contains("reversed") {
        "Reversed"
    } else {
        "Basic"
    }
}

/// Inline add-card validation.
#[must_use]
pub fn validate_draft(
    deck: &Deck,
    model: &NoteModel,
    fields: &[String],
) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    if deck.name.trim().is_empty() || deck.id <= 0 {
        let _ =
            errors.insert("deck".to_owned(), "Choose a deck first".to_owned());
    }
    if model.id <= 0 {
        let _ = errors.insert(
            "model".to_owned(),
            "This note type is not in the collection".to_owned(),
        );
    }
    if fields.is_empty() || fields.iter().all(|value| value.trim().is_empty()) {
        let _ = errors.insert(
            "fields".to_owned(),
            "Enter text in at least one field".to_owned(),
        );
    }
    if model.is_cloze()
        && fields.iter().any(|value| !value.trim().is_empty())
        && !fields.iter().any(|value| {
            ranki_model::cloze_ordinals(std::slice::from_ref(value))
                .iter()
                .any(|ordinal| *ordinal < 500)
        })
    {
        let _ = errors.insert(
            "fields".to_owned(),
            "Cloze notes need a {{c1::answer}} marker".to_owned(),
        );
    }
    errors
}

#[must_use]
pub fn next_due_deck(
    decks: &[Deck],
    stats: &[DeckStats],
    selected: Option<usize>,
) -> Option<usize> {
    let current = selected.unwrap_or(usize::MAX);
    (0..decks.len()).find(|index| {
        *index != current
            && stats.get(*index).is_some_and(|value| due_total(*value) > 0)
    })
}

/// Human age for the footer badge.
#[must_use]
pub fn relative_age(modified: i64, now: i64) -> String {
    let seconds = now.saturating_sub(modified).max(0);
    match seconds {
        0..=59 => String::from("just now"),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// Last successful sync in Unix seconds. The store exposes no sync stamp yet,
/// so the footer truthfully reports a local, unsynced collection.
pub const SYNC_STATE: Option<i64> = None;

/// Footer sync badge: local state, or the age of the last sync.
#[must_use]
pub fn status_label(last_sync: Option<i64>, now: i64) -> String {
    last_sync.map_or_else(
        || String::from("Local \u{b7} not synced"),
        |stamp| format!("Synced \u{b7} {}", relative_age(stamp, now)),
    )
}

/// Installed note types plus every stock type the collection is missing.
///
/// Missing stock types keep placeholder id 0, so the add-card screen can show
/// their fields before the collection has the model.
#[must_use]
pub fn models_with_stock(installed: Vec<NoteModel>) -> Vec<NoteModel> {
    let mut models = installed;
    for stock in ranki_model::stock_models() {
        let known = models
            .iter()
            .any(|model| model.name.eq_ignore_ascii_case(&stock.name));
        if !known {
            models.push(stock);
        }
    }
    models
}

/// Moves completed words from the tag field into the chip list.
///
/// The trailing word stays in the field until a space commits it, so the field
/// only ever holds the word being typed.
#[must_use]
pub fn collect_tags(chips: &mut Vec<String>, text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>();
    let pending = !text.ends_with(char::is_whitespace);
    let (complete, remainder) = match words.split_last() {
        Some((last, rest)) if pending => (rest, *last),
        Some(_) => (&words[..], ""),
        None => (&[][..], ""),
    };
    for word in complete {
        if !chips.iter().any(|known| known == word) {
            chips.push((*word).to_owned());
        }
    }
    remainder.to_owned()
}

/// Deck row metadata: child count for parents and total cards for every row.
#[must_use]
pub fn deck_meta(rows: &[DeckRow], row: &DeckRow) -> String {
    let prefix = format!("{}::", row.path);
    let children = rows
        .iter()
        .filter(|other| {
            other.depth == row.depth.saturating_add(1)
                && other.path.starts_with(&prefix)
        })
        .count();
    let cards = row.stats.total_cards;
    let cards_label = if cards == 1 {
        String::from("1 card")
    } else {
        format!("{cards} cards")
    };
    if children == 0 {
        return cards_label;
    }
    let subdecks = if children == 1 {
        String::from("1 subdeck")
    } else {
        format!("{children} subdecks")
    };
    format!("{subdecks} · {cards_label}")
}

/// Calendar label for the removed date line, kept for its unit test.
#[cfg(test)]
#[must_use]
pub fn date_label(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let (_, month, day) = civil_from_days(days);
    let weekday = (days + 4).rem_euclid(7);
    let names = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let weekday_name = names[usize::try_from(weekday).unwrap_or(0)];
    let month_name = months[usize::try_from(month - 1).unwrap_or(0)];
    format!("{weekday_name}, {day} {month_name}")
}

/// Howard Hinnant's civil-from-days, valid for the Unix epoch range.
#[cfg(test)]
fn civil_from_days(days: i64) -> (i64, i32, i32) {
    let shifted = days.saturating_add(719_468);
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted.saturating_sub(146_096)
    } / 146_097;
    let day_of_era = shifted.saturating_sub(era.saturating_mul(146_097));
    let year_of_era = (day_of_era
        .saturating_sub(day_of_era / 1460)
        .saturating_add(day_of_era / 36_524)
        .saturating_sub(day_of_era / 146_096))
        / 365;
    let year = year_of_era.saturating_add(era.saturating_mul(400));
    let day_of_year = day_of_era
        .saturating_sub(year_of_era.saturating_mul(365))
        .saturating_sub(year_of_era / 4)
        .saturating_add(year_of_era / 100);
    let month_index =
        (5_i64.saturating_mul(day_of_year).saturating_add(2)) / 153;
    let day = day_of_year
        .saturating_sub(
            (153_i64.saturating_mul(month_index).saturating_add(2)) / 5,
        )
        .saturating_add(1);
    let month = if month_index < 10 {
        month_index.saturating_add(3)
    } else {
        month_index.saturating_sub(9)
    };
    let year = if month <= 2 {
        year.saturating_add(1)
    } else {
        year
    };
    (
        year,
        i32::try_from(month).unwrap_or(1),
        i32::try_from(day).unwrap_or(1),
    )
}

#[must_use]
pub fn focused_delta(
    elapsed: Duration,
    focused: bool,
    total: Duration,
) -> Duration {
    if !focused {
        return total;
    }
    total.saturating_add(elapsed).min(MAX_REVIEW_TIME)
}

#[must_use]
pub const fn rating_name(rating: Rating) -> &'static str {
    match rating {
        Rating::Again => "Again",
        Rating::Hard => "Hard",
        Rating::Good => "Good",
        Rating::Easy => "Easy",
    }
}

#[must_use]
pub const fn empty_kind(
    deck_count: usize,
    total: usize,
    searching: bool,
) -> EmptyKind {
    if searching {
        EmptyKind::NoMatch
    } else if deck_count == 0 {
        EmptyKind::NoDecks
    } else if total == 0 {
        EmptyKind::CaughtUp
    } else {
        EmptyKind::None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyKind {
    None,
    NoDecks,
    NoMatch,
    CaughtUp,
}

#[must_use]
pub const fn empty_copy(kind: EmptyKind) -> (&'static str, &'static str) {
    match kind {
        EmptyKind::NoDecks => {
            ("No decks yet", "Create a deck to start studying")
        }
        EmptyKind::NoMatch => ("No decks match", "Try a different search"),
        EmptyKind::CaughtUp => ("All caught up", "No cards are due right now"),
        EmptyKind::None => ("", ""),
    }
}

#[must_use]
pub const fn next_view(current: View, target: View) -> View {
    match (current, target) {
        (View::Review, View::Decks) => View::Decks,
        (_, target) => target,
    }
}

fn clock_label() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        % 86_400;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    format!("{hours:02}:{minutes:02}")
}

#[must_use]
pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("space", Reveal, Some("ranki")),
        KeyBinding::new("1", GradeAgain, Some("ranki")),
        KeyBinding::new("2", GradeHard, Some("ranki")),
        KeyBinding::new("3", GradeGood, Some("ranki")),
        KeyBinding::new("4", GradeEasy, Some("ranki")),
        KeyBinding::new("r", Retry, Some("ranki")),
        KeyBinding::new("esc", EndSession, Some("ranki")),
        KeyBinding::new("d", OpenDecks, Some("ranki")),
        KeyBinding::new("a", OpenAdd, Some("ranki")),
        KeyBinding::new("?", ToggleHelp, Some("ranki")),
        KeyBinding::new("t", ToggleScheduler, Some("ranki")),
        KeyBinding::new("n", NextDeck, Some("ranki")),
        KeyBinding::new("cmd-enter", SaveNote, Some("ranki")),
        KeyBinding::new("ctrl-enter", SaveNote, Some("ranki")),
        KeyBinding::new("enter", ConfirmDialog, Some("ranki_dialog")),
        KeyBinding::new("esc", CancelDialog, Some("ranki_dialog")),
        KeyBinding::new("space", IgnoreShortcut, Some("ranki_input")),
        KeyBinding::new("1", IgnoreShortcut, Some("ranki_input")),
        KeyBinding::new("2", IgnoreShortcut, Some("ranki_input")),
        KeyBinding::new("3", IgnoreShortcut, Some("ranki_input")),
        KeyBinding::new("4", IgnoreShortcut, Some("ranki_input")),
        KeyBinding::new("r", IgnoreShortcut, Some("ranki_input")),
        KeyBinding::new("a", IgnoreShortcut, Some("ranki_input")),
        KeyBinding::new("d", IgnoreShortcut, Some("ranki_input")),
        KeyBinding::new("enter", SubmitDeck, Some("ranki_input")),
        KeyBinding::new("ctrl-enter", SaveNote, Some("ranki_input")),
        KeyBinding::new("cmd-enter", SaveNote, Some("ranki_input")),
    ];
    bindings.extend(crate::text_field::key_bindings());
    bindings
}

#[cfg(test)]
mod tests {
    use ranki_model::{CardTemplate, Deck, NoteKind, NoteModel, SchedulerKind};

    use crate::Options;

    use super::{
        DeckRow, EmptyKind, Phase, SYNC_STATE, UiAction, View, build_deck_rows,
        collect_tags, date_label, deck_meta, due_total, empty_copy, empty_kind,
        filter_decks, focused_delta, models_with_stock, next_due_deck,
        next_view, note_type_name, relative_age, status_label, sum_leaf_counts,
        validate_draft,
    };
    #[test]
    fn scheduler_follows_the_configured_algorithm() {
        let mut options = Options {
            scheduler: SchedulerKind::Sm2,
            ..Options::default()
        };
        assert_eq!(super::scheduler_from_options(&options).name(), "sm2");
        options.scheduler = SchedulerKind::Fsrs;
        assert_eq!(super::scheduler_from_options(&options).name(), "fsrs");
    }

    #[test]
    fn config_options_keep_their_defaults() {
        let options = Options::default();
        assert_eq!(options.font_size, 16.0);
        assert_eq!(options.desired_retention, 0.9);
        assert_eq!(options.review_batch_size, 20);
        assert!(options.show_keyboard_hints);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture with one argument per queue"
    )]
    fn deck(
        id: i64,
        name: &str,
        new: usize,
        learning: usize,
        review: usize,
    ) -> (Deck, ranki_model::DeckStats) {
        (
            Deck {
                id,
                name: name.to_owned(),
                description: String::new(),
            },
            ranki_model::DeckStats {
                new_cards: new,
                learning_cards: learning,
                review_cards: review,
                total_cards: new + learning + review,
            },
        )
    }

    #[test]
    fn deck_tree_builds_stable_indented_rows() {
        let decks = [
            deck(1, "A", 1, 1, 1),
            deck(2, "A::B", 2, 0, 3),
            deck(3, "Z", 0, 0, 0),
        ];
        let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
        let rows = build_deck_rows(
            &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
            &stats,
            &std::collections::HashSet::default(),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "A");
        assert!(!rows[0].virtual_group);
        assert_eq!(rows[0].stats.review_cards, 4);
        assert_eq!(rows[1].path, "Z");
    }

    #[test]
    fn expanded_tree_shows_children() {
        let decks = [deck(1, "A", 0, 0, 0), deck(2, "A::B", 1, 2, 3)];
        let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
        let mut expanded = std::collections::HashSet::new();
        let _ = expanded.insert(String::from("A"));
        let rows = build_deck_rows(
            &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
            &stats,
            &expanded,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].name, "B");
    }

    #[test]
    fn leaf_sum_does_not_double_count_parents() {
        let decks = [deck(1, "A", 1, 1, 1), deck(2, "A::B", 2, 0, 3)];
        let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
        let mut expanded = std::collections::HashSet::new();
        let _ = expanded.insert(String::from("A"));
        let rows = build_deck_rows(
            &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
            &stats,
            &expanded,
        );
        let total = sum_leaf_counts(&rows);
        assert_eq!(due_total(total), 5);
    }

    #[test]
    fn search_keeps_ancestors_and_sorts_stable() {
        let rows = vec![
            DeckRow {
                id: Some(1),
                path: "A".into(),
                name: "A".into(),
                depth: 0,
                stats: ranki_model::DeckStats::default(),
                virtual_group: false,
                has_children: false,
                expanded: false,
            },
            DeckRow {
                id: Some(2),
                path: "A::B".into(),
                name: "B".into(),
                depth: 1,
                stats: ranki_model::DeckStats::default(),
                virtual_group: false,
                has_children: false,
                expanded: true,
            },
            DeckRow {
                id: Some(3),
                path: "Z".into(),
                name: "Z".into(),
                depth: 0,
                stats: ranki_model::DeckStats::default(),
                virtual_group: false,
                has_children: false,
                expanded: false,
            },
        ];
        let filtered = filter_decks(&rows, "b");
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].path, "A");
        assert_eq!(filter_decks(&rows, "nothing").len(), 0);
    }

    #[test]
    fn note_type_names_cover_stock_types() {
        let model = NoteModel {
            id: 1,
            name: "Basic (and reversed card)".into(),
            fields: vec![],
            templates: vec![CardTemplate::default()],
            css: String::new(),
            kind: NoteKind::Normal,
            original_stock_kind: None,
            field_tags: vec![],
            template_ordinals: vec![0],
            raw_config: None,
        };
        assert_eq!(note_type_name(&model), "Reversed");
        let cloze = NoteModel {
            name: "Cloze".into(),
            kind: NoteKind::Cloze,
            ..model
        };
        assert_eq!(note_type_name(&cloze), "Cloze");
    }

    #[test]
    fn draft_validation_blocks_empty_and_cloze_without_marker() {
        let (deck, _) = deck(1, "Default", 0, 0, 0);
        let model = NoteModel {
            id: 1,
            name: "Cloze".into(),
            fields: vec!["Text".into()],
            templates: vec![CardTemplate::default()],
            css: String::new(),
            kind: NoteKind::Cloze,
            original_stock_kind: None,
            field_tags: vec![],
            template_ordinals: vec![0],
            raw_config: None,
        };
        assert!(
            validate_draft(&deck, &model, &[String::new()])
                .contains_key("fields")
        );
        assert!(
            validate_draft(&deck, &model, &[String::from("plain text")])
                .contains_key("fields")
        );
        assert!(
            validate_draft(&deck, &model, &[String::from("{{c1::answer}}")])
                .is_empty()
        );
    }

    #[test]
    fn empty_states_are_distinct() {
        assert_eq!(empty_kind(0, 0, false), EmptyKind::NoDecks);
        assert_eq!(empty_kind(1, 0, false), EmptyKind::CaughtUp);
        assert_eq!(empty_kind(1, 0, true), EmptyKind::NoMatch);
        assert_eq!(empty_kind(1, 3, false), EmptyKind::None);
        assert_eq!(empty_copy(EmptyKind::NoDecks).0, "No decks yet");
    }

    #[test]
    fn view_transitions_return_to_decks() {
        assert_eq!(next_view(View::Review, View::Decks), View::Decks);
        assert_eq!(next_view(View::Decks, View::AddCard), View::AddCard);
    }

    #[test]
    fn phase_and_action_are_copy_comparable() {
        assert_ne!(Phase::Question, Phase::Answer);
        assert_ne!(UiAction::OpenAdd, UiAction::OpenDecks);
    }

    #[test]
    fn next_due_deck_skips_selection() {
        let decks = [deck(1, "A", 0, 0, 0), deck(2, "B", 1, 0, 0)];
        let stats = decks.iter().map(|(_, s)| *s).collect::<Vec<_>>();
        let list = decks
            .iter()
            .map(|(deck, _)| deck.clone())
            .collect::<Vec<_>>();
        assert_eq!(next_due_deck(&list, &stats, Some(1)), None);
    }

    #[test]
    fn focused_time_is_capped_and_ignores_blur() {
        let elapsed = std::time::Duration::from_secs(61);
        assert_eq!(
            focused_delta(elapsed, false, std::time::Duration::ZERO),
            std::time::Duration::ZERO
        );
        assert_eq!(
            focused_delta(elapsed, true, std::time::Duration::ZERO),
            std::time::Duration::from_secs(60)
        );
    }

    #[test]
    fn footer_age_is_human_and_truthful() {
        assert_eq!(relative_age(100, 130), "just now");
        assert_eq!(relative_age(100, 220), "2 min ago");
        assert_eq!(relative_age(100, 7_300), "2 h ago");
        assert_eq!(relative_age(100, 172_900), "2 days ago");
    }

    #[test]
    fn date_label_uses_the_real_calendar() {
        assert_eq!(date_label(0), "Thursday, 1 January");
        assert_eq!(date_label(1_800_000_000), "Friday, 15 January");
    }

    #[test]
    fn deck_meta_counts_children_and_cards() {
        let row = DeckRow {
            id: None,
            path: "Japanese".into(),
            name: "Japanese".into(),
            depth: 0,
            stats: ranki_model::DeckStats {
                total_cards: 1284,
                ..Default::default()
            },
            virtual_group: true,
            has_children: true,
            expanded: false,
        };
        let child = DeckRow {
            id: Some(2),
            path: "Japanese::Vocabulary N5".into(),
            name: "Vocabulary N5".into(),
            depth: 1,
            stats: ranki_model::DeckStats::default(),
            virtual_group: false,
            has_children: false,
            expanded: false,
        };
        assert_eq!(
            deck_meta(&[row.clone(), child], &row),
            "1 subdeck · 1284 cards"
        );
        let solo = DeckRow {
            path: String::from("Solo"),
            stats: ranki_model::DeckStats {
                total_cards: 1,
                ..Default::default()
            },
            ..row
        };
        assert_eq!(deck_meta(std::slice::from_ref(&solo), &solo), "1 card");
    }

    #[test]
    fn due_totals_add_all_queues() {
        let (_, stats) = deck(1, "A", 2, 3, 4);
        assert_eq!(due_total(stats), 9);
    }

    #[test]
    fn footer_status_reports_local_until_a_sync_stamp_exists() {
        assert_eq!(SYNC_STATE, None);
        assert_eq!(status_label(SYNC_STATE, 1_000), "Local \u{b7} not synced");
        assert_eq!(status_label(Some(1_000), 1_120), "Synced \u{b7} 2 min ago");
    }

    #[test]
    fn stock_types_fill_in_missing_note_models() {
        let models = models_with_stock(Vec::new());
        let names = models
            .iter()
            .map(|model| model.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "Basic",
                "Basic (and reversed card)",
                "Cloze",
                "Image Occlusion"
            ]
        );
        assert!(models.iter().all(|model| model.id == 0));
        let installed = ranki_model::stock_models()
            .into_iter()
            .next()
            .map(|mut model| {
                model.id = 7;
                model
            })
            .expect("stock basic");
        let merged = models_with_stock(vec![installed]);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].id, 7);
    }

    #[test]
    fn tag_field_commits_words_into_chips() {
        let mut chips = Vec::new();
        assert_eq!(collect_tags(&mut chips, "japan"), "japan");
        assert_eq!(chips.len(), 0);
        assert_eq!(collect_tags(&mut chips, "japan "), "");
        assert_eq!(chips, ["japan"]);
        assert_eq!(collect_tags(&mut chips, "japan verb"), "verb");
        assert_eq!(chips, ["japan"]);
        assert_eq!(collect_tags(&mut chips, "verb "), "");
        assert_eq!(chips, ["japan", "verb"]);
        assert_eq!(collect_tags(&mut chips, " 	 "), "");
        assert_eq!(chips, ["japan", "verb"]);
    }
}
