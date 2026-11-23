//! Root application state, keyboard dispatcher, and store-backed actions.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    env,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    KeyBinding, Pixels, Render, Window, WindowAppearance, actions, div,
    prelude::*,
};
use ranki_core::{
    FsrsParams, FsrsPreset, FsrsScheduler, Scheduler, from_settings,
};
use ranki_model::{
    Card, CardType, Deck, DeckStats, Id, NewNote, Note, NoteModel, Rating,
    ReviewDecision, StockModelKind, Store,
};

use crate::{
    Options, SchedulerKind, ThemeChoice,
    card_view::{self, RenderedCard, Side},
    occlusion::{
        self, MaskShape, NormalizedRect, OcclusionMask, default_mask,
        image_occlusion_fields, media_images,
    },
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
        Undo,
        StudyAgain,
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

/// Tone of a toast. Errors keep the red role, everything else reports a
/// completed action and must not look like a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
    pub created_at: Instant,
    /// Monotonic id so the toast's fade replays for each new message.
    pub seq: u64,
}

impl Toast {
    /// A toast outlives its text only by the timer that clears it.
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

/// Image-occlusion editor state.
#[derive(Default)]
pub struct OcclusionState {
    /// Media file name chosen for the `Image` field.
    pub image: Option<String>,
    /// Image files found in the media directory.
    pub media: Vec<String>,
    pub masks: Vec<OcclusionMask>,
    pub selected: Option<usize>,
    /// Drag in progress, in image-relative coordinates.
    pub drag: Option<(f32, f32, f32, f32)>,
    /// Screen rectangle of the drawing area, from the last prepaint.
    pub bounds: Option<Bounds<Pixels>>,
}

impl OcclusionState {
    fn pointer(&self, position: (f32, f32)) -> Option<(f32, f32)> {
        let bounds = self.bounds?;
        Some(occlusion::relative_position(
            position,
            (f32::from(bounds.origin.x), f32::from(bounds.origin.y)),
            (f32::from(bounds.size.width), f32::from(bounds.size.height)),
        ))
    }

    /// Current drag as a mask, for the dashed preview outline.
    #[must_use]
    pub fn drag_rect(&self) -> Option<NormalizedRect> {
        let (left, top, right, bottom) = self.drag?;
        NormalizedRect::from_points((left, top), (right, bottom))
    }

    fn finish_drag(&mut self) {
        if let Some(rect) = self.drag_rect() {
            self.masks.push(OcclusionMask {
                shape: MaskShape::Rect,
                rect,
            });
            self.selected = Some(self.masks.len().saturating_sub(1));
        }
        self.drag = None;
    }
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
    pub media_filter: Entity<TextField>,
    pub occlusion: OcclusionState,
}

/// What one grade can be reversed into: the card before the write, the review
/// log that recorded it, and the rating the user picked.
#[derive(Clone, Debug)]
pub struct UndoEntry {
    pub card: Card,
    pub revlog_id: Id,
    pub rating: Rating,
    /// Time the undone card contributed to the session total.
    pub spent: Duration,
}

/// Rendered halves of one card, prepared before a session transition.
struct LoadedCard {
    note: Option<Note>,
    question: Option<RenderedCard>,
    answer: Option<RenderedCard>,
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
    /// Visible time spent on graded cards, accumulated per answer.
    pub studied: Duration,
    /// Fraction the progress bar had before the last grade, for its transition.
    pub progress_from: f32,
    /// Last grade this session, cleared once it is undone or replaced.
    pub last_grade: Option<UndoEntry>,
}

/// Root entity. The store boundary stays identical to the previous UI.
pub struct Root {
    pub(crate) store: Box<dyn Store>,
    pub(crate) options: Options,
    pub(crate) focus: FocusHandle,
    pub(crate) dialog_focus: FocusHandle,
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
    pub(crate) collection_start: i64,
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
        let dialog_focus = cx.focus_handle();
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
        let media_filter = cx.new(|cx| make_field(cx, "Filter images"));
        let mut root = Self {
            scheduler: scheduler_from_options(&options),
            store,
            options,
            focus,
            dialog_focus,
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
                media_filter,
                occlusion: OcclusionState::default(),
            },
            toast: None,
            help_open: false,
            dev_reveal: env::var("RANKI_UI_DEV_REVEAL")
                .is_ok_and(|value| value == "1"),
            external_change: false,
            collection_modified: 0,
            collection_start: 0,
            images: card_view::LruImageCache::new(cx),
            font_family: theme::ui_font_family(),
        };
        root.reload(cx);
        root.reset_fields(cx);
        root.refresh_field_colors(cx);
        if let Ok(prefill) = env::var("RANKI_UI_DEV_FIELDS") {
            root.seed_fields(&prefill, cx);
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
        if let Ok(seed) = env::var("RANKI_UI_DEV_OCCLUSION") {
            root.seed_occlusion(&seed, cx);
            if let Ok(prefill) = env::var("RANKI_UI_DEV_FIELDS") {
                root.seed_fields(&prefill, cx);
            }
        }
        if let Some(count) = env::var("RANKI_UI_DEV_GRADE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
        {
            let weak = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = weak.update(cx, |root, cx| {
                    for _ in 0..count {
                        if root.review.current.is_none() {
                            break;
                        }
                        root.review.phase = Phase::Answer;
                        root.review.last_grade_at = None;
                        root.grade(Rating::Good, cx);
                    }
                });
            });
        }
        if let Ok(spec) = env::var("RANKI_UI_DEV_TOAST") {
            let (kind, text) = spec.split_once(':').map_or(
                (ToastKind::Error, spec.as_str()),
                |(kind, text)| {
                    let kind = if kind.eq_ignore_ascii_case("success") {
                        ToastKind::Success
                    } else {
                        ToastKind::Error
                    };
                    (kind, text)
                },
            );
            root.show_toast(kind, text, cx);
        }
        if env::var("RANKI_UI_DEV_SAVE").is_ok_and(|value| value == "1") {
            root.view = View::AddCard;
            let weak = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = weak.update(cx, |root, cx| {
                    root.view = View::AddCard;
                    root.save_note(cx);
                });
            });
        }
        root
    }

    /// Fills the add-card fields from tab separated text.
    fn seed_fields(&mut self, prefill: &str, cx: &mut Context<Self>) {
        for (field, value) in self.add.fields.iter().zip(prefill.split('\t')) {
            field.update(cx, |field, cx| {
                field.seed(value.to_owned(), cx);
            });
        }
    }

    /// Seeds the occlusion editor from `image;l,t,w,h;l,t,w,h`.
    ///
    /// Masks go through the same path as drawn ones, so a scripted run saves
    /// exactly what a pointer run would.
    fn seed_occlusion(&mut self, seed: &str, cx: &mut Context<Self>) {
        let Some(model) =
            self.models.iter().position(NoteModel::is_image_occlusion)
        else {
            return;
        };
        self.select_model(model, cx);
        let mut parts = seed.split(';');
        if let Some(image) = parts.next().map(str::trim)
            && !image.is_empty()
        {
            self.choose_occlusion_image(image.to_owned(), cx);
        }
        for part in parts {
            let values = part
                .split(',')
                .filter_map(|value| value.trim().parse::<f32>().ok())
                .collect::<Vec<_>>();
            let Some(rect) = NormalizedRect::from_points(
                (
                    values.first().copied().unwrap_or(0.0),
                    values.get(1).copied().unwrap_or(0.0),
                ),
                (
                    values.first().copied().unwrap_or(0.0)
                        + values.get(2).copied().unwrap_or(0.0),
                    values.get(1).copied().unwrap_or(0.0)
                        + values.get(3).copied().unwrap_or(0.0),
                ),
            ) else {
                continue;
            };
            self.add_occlusion_mask(
                OcclusionMask {
                    shape: MaskShape::Rect,
                    rect,
                },
                cx,
            );
        }
    }
    /// Keeps the chip list in step with the tag field as words are typed.
    fn tag_hook(cx: &mut Context<Self>) -> EditHook {
        let weak = cx.entity().downgrade();
        Arc::new(move |app| {
            let weak = weak.clone();
            app.defer(move |app| {
                let _ = weak.update(app, |root, cx| {
                    root.sync_tags(cx);
                    cx.notify();
                });
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
            let weak = weak.clone();
            app.defer(move |app| {
                let _ = weak.update(app, |_, cx| cx.notify());
            });
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
        match self.store.decks() {
            Ok(decks) => self.decks = decks,
            Err(error) => {
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not load decks: {error}"),
                    cx,
                );
            }
        }
        let chosen = self
            .models
            .get(self.add.model_index)
            .map(|model| model.name.clone());
        match self.store.models() {
            Ok(models) => self.models = models_with_stock(models),
            Err(error) => {
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not load note types: {error}"),
                    cx,
                );
            }
        }
        if let Some(name) = chosen
            && let Some(index) = model_index(&self.models, &name)
        {
            self.add.model_index = index;
        }
        if self.add.model_index >= self.models.len() {
            self.add.model_index = 0;
        }
        self.refresh_stats();
        self.collection_start = self.store.collection_created().unwrap_or(0);
        self.collection_modified =
            self.store.collection_modified().unwrap_or(0);
        self.external_change = false;
        if self.selected.is_none_or(|index| index >= self.decks.len()) {
            self.selected = (!self.decks.is_empty()).then_some(0);
        }
        self.reset_fields(cx);
        cx.notify();
    }

    /// Recomputes per-deck counts without rebuilding the add-card fields.
    fn refresh_stats(&mut self) {
        let now = Self::now();
        self.stats = match self.store.deck_stats() {
            Ok(stats) => {
                let lookup = stats.into_iter().collect::<HashMap<_, _>>();
                self.decks
                    .iter()
                    .map(|deck| {
                        lookup.get(&deck.id).copied().unwrap_or_default()
                    })
                    .collect()
            }
            Err(_) => self
                .decks
                .iter()
                .map(|deck| {
                    self.store.stats(Some(deck.id), now).unwrap_or_default()
                })
                .collect(),
        };
        self.rebuild_deck_rows();
    }

    /// Reports a completed action. The toast retires on its own after
    /// [`TOAST_LIFETIME`], or sooner when dismissed.
    pub(crate) fn set_toast(
        &mut self,
        kind: ToastKind,
        text: impl Into<String>,
    ) {
        static NEXT: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        self.toast = Some(Toast {
            kind,
            text: text.into(),
            created_at: Instant::now(),
            seq: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        });
    }

    /// Clears the toast and arms its dismissal, so the window redraws even
    /// when the pointer and keyboard stay idle.
    pub(crate) fn show_toast(
        &mut self,
        kind: ToastKind,
        text: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.set_toast(kind, text);
        let created_at = self.toast.as_ref().map(|toast| toast.created_at);
        let Some(created_at) = created_at else {
            return;
        };
        let timer = cx.background_executor().timer(TOAST_LIFETIME);
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |root, cx| {
                let same = root
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.created_at == created_at);
                if same {
                    root.clear_toast();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn clear_toast(&mut self) {
        self.toast = None;
    }

    pub(crate) fn live_toast(&self) -> Option<&Toast> {
        self.toast
            .as_ref()
            .filter(|toast| !toast.expired(Instant::now()))
    }

    /// Motion is on unless the setting or the desktop preference disables it.
    pub(crate) fn motion_ms(&self, milliseconds: u64) -> u64 {
        if theme::reduced_motion(self.options.reduce_motion) {
            0
        } else {
            milliseconds
        }
    }

    /// How long the current card has been on screen, for the session summary.
    pub(crate) fn session_elapsed(&self) -> Duration {
        let live = if self.review.current.is_some() {
            self.review.focused_elapsed
        } else {
            Duration::ZERO
        };
        self.review.studied.saturating_add(live)
    }

    fn reset_fields(&mut self, cx: &mut Context<Self>) {
        let model = self.models.get(self.add.model_index);
        let names = model.map_or_else(Vec::new, |item| item.fields.clone());
        if model.is_some_and(NoteModel::is_image_occlusion) {
            self.refresh_media();
        }
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
        self.refresh_field_colors(cx);
    }

    pub(crate) fn select_model(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.models.get(index).cloned() else {
            return;
        };
        self.add.model_index = index;
        self.add.errors.clear();
        if model.id > 0 {
            self.reset_fields(cx);
            cx.notify();
            return;
        }
        match StockModelKind::of(&model) {
            Some(kind) => match self.store.ensure_model(kind) {
                Ok(installed) => {
                    self.reload(cx);
                    if let Some(found) =
                        model_index(&self.models, &installed.name)
                    {
                        self.add.model_index = found;
                    }
                    self.reset_fields(cx);
                    self.clear_toast();
                }
                Err(error) => {
                    self.add.errors.insert(
                        "model".into(),
                        format!("Could not install {}: {error}", model.name),
                    );
                }
            },
            None => {
                self.add.errors.insert(
                    "model".into(),
                    "This note type is not in the collection".into(),
                );
            }
        }
        cx.notify();
    }

    fn refresh_media(&mut self) {
        self.add.occlusion.media = media_images(&self.options.media_dir);
    }

    pub(crate) fn choose_occlusion_image(
        &mut self,
        name: String,
        cx: &mut Context<Self>,
    ) {
        if self.add.occlusion.image.as_deref() != Some(name.as_str()) {
            self.add.occlusion.masks.clear();
            self.add.occlusion.selected = None;
        }
        self.add.occlusion.image = Some(name);
        let _ = self.add.errors.remove("occlusion");
        cx.notify();
    }

    pub(crate) fn add_occlusion_mask(
        &mut self,
        mask: OcclusionMask,
        cx: &mut Context<Self>,
    ) {
        self.add.occlusion.masks.push(mask);
        self.add.occlusion.selected =
            Some(self.add.occlusion.masks.len().saturating_sub(1));
        let _ = self.add.errors.remove("masks");
        cx.notify();
    }

    pub(crate) fn add_default_mask(&mut self, cx: &mut Context<Self>) {
        let mask = default_mask(self.add.occlusion.masks.len());
        self.add_occlusion_mask(mask, cx);
    }

    pub(crate) fn select_occlusion_mask(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index < self.add.occlusion.masks.len() {
            self.add.occlusion.selected = Some(index);
            cx.notify();
        }
    }

    pub(crate) fn remove_selected_mask(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.add.occlusion.selected else {
            return;
        };
        if index >= self.add.occlusion.masks.len() {
            return;
        }
        let _ = self.add.occlusion.masks.remove(index);
        self.add.occlusion.selected =
            self.add.occlusion.masks.len().checked_sub(1);
        cx.notify();
    }

    pub(crate) fn clear_occlusion_masks(&mut self, cx: &mut Context<Self>) {
        self.add.occlusion.masks.clear();
        self.add.occlusion.selected = None;
        self.add.occlusion.drag = None;
        cx.notify();
    }

    /// Records the drawing area so pointer positions can be normalized.
    pub(crate) fn set_occlusion_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.add.occlusion.bounds = Some(bounds);
    }

    pub(crate) fn occlusion_press(
        &mut self,
        position: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        if let Some(pointer) = self.add.occlusion.pointer(position) {
            let corner = (pointer.0, pointer.1, pointer.0, pointer.1);
            self.add.occlusion.drag = Some(corner);
            cx.notify();
        }
    }

    pub(crate) fn occlusion_drag_to(
        &mut self,
        position: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let Some((left, top, _, _)) = self.add.occlusion.drag else {
            return;
        };
        if let Some(pointer) = self.add.occlusion.pointer(position) {
            self.add.occlusion.drag = Some((left, top, pointer.0, pointer.1));
            cx.notify();
        }
    }

    pub(crate) fn occlusion_release(&mut self, cx: &mut Context<Self>) {
        if self.add.occlusion.drag.is_some() {
            self.add.occlusion.finish_drag();
            let _ = self.add.errors.remove("masks");
            cx.notify();
        }
    }

    pub(crate) fn cancel_occlusion_drag(&mut self, cx: &mut Context<Self>) {
        if self.add.occlusion.drag.take().is_some() {
            cx.notify();
        }
    }

    fn occlusion_headline(&self, cx: &mut Context<Self>) -> String {
        let header = self
            .add
            .fields
            .get(2)
            .map(|field| field.read(cx).text())
            .unwrap_or_default();
        let text = if header.trim().is_empty() {
            self.add
                .occlusion
                .image
                .clone()
                .unwrap_or_else(|| String::from("Image occlusion"))
        } else {
            header.trim().to_owned()
        };
        text.chars().take(32).collect()
    }

    /// Image Occlusion field values in the stock model order.
    fn occlusion_fields(&self, cx: &mut Context<Self>) -> Vec<String> {
        let field = |index: usize, cx: &mut Context<Self>| {
            self.add
                .fields
                .get(index)
                .map_or_else(String::new, |entry| entry.read(cx).text())
        };
        let header = field(2, cx);
        let back_extra = field(3, cx);
        let comments = field(4, cx);
        let image = self.add.occlusion.image.clone().unwrap_or_default();
        image_occlusion_fields(
            &header,
            &back_extra,
            &comments,
            &image,
            &self.add.occlusion.masks,
        )
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
                    self.show_toast(
                        ToastKind::Error,
                        format!("Could not load cards: {error}"),
                        cx,
                    );
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
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not load cards: {error}"),
                    cx,
                );
            }
        }
        self.review.session_total = self.review.queue.len();
        self.previous_view = View::Decks;
        self.view = View::Review;
        self.ensure_current(cx);
        cx.notify();
    }

    /// Restarts the session the summary just ended, with the same scope.
    pub(crate) fn study_again(&mut self, cx: &mut Context<Self>) {
        match self.selected {
            Some(index) if !self.review.scope_all => self.study_deck(index, cx),
            _ => self.study_all(cx),
        }
    }

    pub(crate) fn end_session(&mut self, cx: &mut Context<Self>) {
        self.review.queue.clear();
        self.review.current = None;
        self.review.last_grade = None;
        self.review.phase = Phase::Question;
        self.view = View::Decks;
        self.refresh_stats();
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
            &self.add.media_filter,
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
            if !self.load_card(&card, cx) {
                continue;
            }
            break;
        }
        if self.review.current.is_none() {
            self.review.question = None;
            self.review.answer = None;
            self.review.note = None;
            self.review.progress_from = self.progress_fraction();
            self.refresh_stats();
        }
    }

    /// Renders one card into the review session. Returns false when its note
    /// cannot be loaded, so the caller can try the next queued card.
    fn load_card(&mut self, card: &Card, cx: &mut Context<Self>) -> bool {
        let Some(loaded) = self.render_state(card, cx) else {
            return false;
        };
        self.apply_loaded(card, loaded, cx);
        true
    }

    /// Reads and renders a card without touching the session. Callers prepare
    /// the next stage before a write, so a failed read cannot leave storage
    /// and the visible card disagreeing.
    fn render_state(
        &mut self,
        card: &Card,
        cx: &mut Context<Self>,
    ) -> Option<LoadedCard> {
        let note = match self.store.note(card.note_id) {
            Ok(note) => note,
            Err(error) => {
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not load card: {error}"),
                    cx,
                );
                return None;
            }
        };
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
        let _ = cx;
        Some(LoadedCard {
            note,
            question,
            answer,
        })
    }

    /// Puts a prepared card on the stage and restarts its timers.
    fn apply_loaded(
        &mut self,
        card: &Card,
        loaded: LoadedCard,
        cx: &mut Context<Self>,
    ) {
        self.review.phase = if self.dev_reveal {
            Phase::Answer
        } else {
            Phase::Question
        };
        self.review.current = Some(card.clone());
        self.review.note = loaded.note;
        self.review.question = loaded.question;
        self.review.answer = loaded.answer;
        self.review.last_tick = Some(Instant::now());
        self.review.last_grade_at = Some(Instant::now());
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
        if self.view != View::Review {
            return;
        }
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
        let decision: ReviewDecision = self.scheduler.schedule(
            &card,
            rating,
            Self::now(),
            self.collection_start,
        );
        let before_progress = self.progress_fraction();
        if let Err(error) = self.store.apply_review(&decision) {
            self.show_toast(
                ToastKind::Error,
                format!("Could not save review: {error}"),
                cx,
            );
            cx.notify();
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
        self.review.progress_from = before_progress;
        self.review.studied = self
            .review
            .studied
            .saturating_add(self.review.focused_elapsed);
        self.review.last_grade = Some(UndoEntry {
            card,
            revlog_id: decision.revlog.id,
            rating,
            spent: self.review.focused_elapsed,
        });
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

    /// Fraction of the session that is graded, clamped for both ends of the
    /// queue. Undo lowers it again because the answered count drops.
    pub(crate) fn progress_fraction(&self) -> f32 {
        session_progress(self.review.answered, self.review.session_total)
    }

    /// Reverses the last grade in this session: the review log is deleted, the
    /// card returns to the question it was graded from, and the session counts
    /// step back. Undo is one step deep, matching the single footer action.
    pub(crate) fn undo_grade(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Review {
            return;
        }
        let Some(entry) = self.review.last_grade.clone() else {
            return;
        };
        let Some(loaded) = self.render_state(&entry.card, cx) else {
            return;
        };
        match self.store.undo_review(&entry.card, entry.revlog_id) {
            Ok(false) => {
                self.review.last_grade = None;
                self.show_toast(
                    ToastKind::Error,
                    "That review is already gone",
                    cx,
                );
                cx.notify();
            }
            Ok(true) => {
                let before = self.progress_fraction();
                self.review.last_grade = None;
                self.review.answered = self.review.answered.saturating_sub(1);
                if entry.rating == Rating::Again {
                    self.review.again_count =
                        self.review.again_count.saturating_sub(1);
                }
                self.review.progress_from = before;
                self.review.status = Some(format!(
                    "Undone {}. Grade again.",
                    rating_name(entry.rating)
                ));
                if let Some(current) = self.review.current.clone() {
                    let _ = self.review.seen.remove(&current.id);
                    self.review.queue.push_front(current);
                }
                let _ = self.review.seen.remove(&entry.card.id);
                self.apply_loaded(&entry.card, loaded, cx);
                self.review.focused_elapsed = entry.spent;
                self.review.studied =
                    self.review.studied.saturating_sub(entry.spent);
                self.refresh_stats();
                cx.notify();
            }
            Err(error) => {
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not undo: {error}"),
                    cx,
                );
                cx.notify();
            }
        }
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
                        self.show_toast(ToastKind::Success, "Deck removed", cx);
                        self.reload(cx);
                    }
                    Err(error) => self.show_toast(
                        ToastKind::Error,
                        format!("Could not remove deck: {error}"),
                        cx,
                    ),
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
            self.show_toast(ToastKind::Error, "Deck name cannot be empty", cx);
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
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not create deck: {error}"),
                    cx,
                );
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
            self.show_toast(ToastKind::Error, "Deck name cannot be empty", cx);
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
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not rename deck: {error}"),
                    cx,
                );
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
        let fields = if model.is_image_occlusion() {
            self.occlusion_fields(cx)
        } else {
            self.add
                .fields
                .iter()
                .map(|field| field.read(cx).text())
                .collect::<Vec<_>>()
        };
        let mut tags = self.add.tag_chips.clone();
        for tag in self.add.tags.read(cx).text().split_whitespace() {
            if !tags.iter().any(|known| known == tag) {
                tags.push(tag.to_owned());
            }
        }
        let errors = if model.is_image_occlusion() {
            validate_occlusion(
                &deck,
                &model,
                self.add.occlusion.image.as_deref(),
                &self.add.occlusion.masks,
            )
        } else {
            validate_draft(&deck, &model, &fields)
        };
        self.add.errors = errors;
        if !self.add.errors.is_empty() {
            cx.notify();
            return;
        }
        let headline = if model.is_image_occlusion() {
            self.occlusion_headline(cx)
        } else {
            fields
                .iter()
                .map(|value| value.trim())
                .find(|value| !value.is_empty())
                .map(|value| value.chars().take(32).collect::<String>())
                .unwrap_or_default()
        };
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
                    field.update(cx, |field, cx| {
                        field.seed("", cx);
                    });
                }
                self.add.tags.update(cx, |field, cx| {
                    field.seed("", cx);
                });
                self.add.errors.clear();
                self.add.occlusion.masks.clear();
                self.add.occlusion.selected = None;
                self.add.occlusion.image = None;
                self.reload(cx);
                self.show_toast(
                    ToastKind::Success,
                    format!("Added {count} cards"),
                    cx,
                );
            }
            Err(error) => {
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not add note: {error}"),
                    cx,
                );
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

    pub(crate) fn dispatch(
        &mut self,
        action: UiAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tick_focus(window);
        let had_dialog = self.confirmation.is_some();
        match action {
            UiAction::Reveal => self.reveal(cx),
            UiAction::Grade(Rating::Again) => self.grade(Rating::Again, cx),
            UiAction::Grade(Rating::Hard) => self.grade(Rating::Hard, cx),
            UiAction::Grade(Rating::Good) => self.grade(Rating::Good, cx),
            UiAction::Grade(Rating::Easy) => self.grade(Rating::Easy, cx),
            UiAction::Retry => self.reveal(cx),
            UiAction::Undo => self.undo_grade(cx),
            UiAction::StudyAgain => {
                if self.view == View::Review
                    && self.review.current.is_none()
                    && self.review.answered > 0
                {
                    self.study_again(cx);
                }
            }
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
                self.refresh_stats();
                cx.notify();
            }
            UiAction::OpenAdd => {
                if self.view != View::AddCard {
                    self.previous_view = self.view;
                }
                self.view = next_view(self.view, View::AddCard);
                self.add.errors.clear();
                cx.notify();
            }
            UiAction::CreateDeck => {
                self.view = View::Decks;
                window.focus(self.create_field.read(cx).focus_handle());
                cx.notify();
            }
            UiAction::SubmitDeck => {
                if self.create_field.read(cx).focus_handle().is_focused(window)
                {
                    self.create_deck(cx);
                }
            }
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
            UiAction::ClearSearch => {
                self.deck_search
                    .update(cx, super::text_field::TextField::clear);
                self.view = View::Decks;
                cx.notify();
            }
        }
        if self.confirmation.is_some() != had_dialog {
            if self.confirmation.is_some() {
                window.focus(&self.dialog_focus);
            } else {
                window.focus(&self.focus);
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
    Undo,
    StudyAgain,
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
impl From<&Undo> for UiAction {
    fn from(_: &Undo) -> Self {
        Self::Undo
    }
}
impl From<&StudyAgain> for UiAction {
    fn from(_: &StudyAgain) -> Self {
        Self::StudyAgain
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
impl From<&ClearSearch> for UiAction {
    fn from(_: &ClearSearch) -> Self {
        Self::ClearSearch
    }
}
/// Dispatches one of the app's GPUI actions from a pointer handler.
pub fn dispatch_app_action(
    window: &mut Window,
    cx: &mut App,
    action: UiAction,
) {
    match action {
        UiAction::Reveal => window.dispatch_action(Box::new(Reveal), cx),
        UiAction::Grade(Rating::Again) => {
            window.dispatch_action(Box::new(GradeAgain), cx);
        }
        UiAction::Grade(Rating::Hard) => {
            window.dispatch_action(Box::new(GradeHard), cx);
        }
        UiAction::Grade(Rating::Good) => {
            window.dispatch_action(Box::new(GradeGood), cx);
        }
        UiAction::Grade(Rating::Easy) => {
            window.dispatch_action(Box::new(GradeEasy), cx);
        }
        UiAction::Retry => window.dispatch_action(Box::new(Retry), cx),
        UiAction::Undo => window.dispatch_action(Box::new(Undo), cx),
        UiAction::StudyAgain => {
            window.dispatch_action(Box::new(StudyAgain), cx);
        }
        UiAction::EndSession => {
            window.dispatch_action(Box::new(EndSession), cx);
        }
        UiAction::OpenDecks => window.dispatch_action(Box::new(OpenDecks), cx),
        UiAction::OpenAdd => window.dispatch_action(Box::new(OpenAdd), cx),
        UiAction::CreateDeck => {
            window.dispatch_action(Box::new(CreateDeck), cx);
        }
        UiAction::SubmitDeck => {
            window.dispatch_action(Box::new(SubmitDeck), cx);
        }
        UiAction::SaveNote => window.dispatch_action(Box::new(SaveNote), cx),
        UiAction::CancelAdd => window.dispatch_action(Box::new(CancelAdd), cx),
        UiAction::ToggleHelp => {
            window.dispatch_action(Box::new(ToggleHelp), cx);
        }
        UiAction::ToggleScheduler => {
            window.dispatch_action(Box::new(ToggleScheduler), cx);
        }
        UiAction::Confirm => {
            window.dispatch_action(Box::new(ConfirmDialog), cx);
        }
        UiAction::CancelDialog => {
            window.dispatch_action(Box::new(CancelDialog), cx);
        }
        UiAction::NextDeck => window.dispatch_action(Box::new(NextDeck), cx),
        UiAction::ClearSearch => {
            window.dispatch_action(Box::new(ClearSearch), cx);
        }
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
            .on_action(cx.listener(|this, action: &Undo, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &StudyAgain, window, cx| {
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
            .when(view != View::Review, |this| this.child(self.render_nav(cx)))
            .child(div().flex_1().overflow_hidden().child(main))
            .children(self.render_overlay(cx))
    }
}

/// Adds one deck's counts onto another, saturating at the type limits.
fn add_deck_stats(total: &mut DeckStats, counts: DeckStats) {
    total.new_cards = total.new_cards.saturating_add(counts.new_cards);
    total.learning_cards =
        total.learning_cards.saturating_add(counts.learning_cards);
    total.review_cards = total.review_cards.saturating_add(counts.review_cards);
    total.total_cards = total.total_cards.saturating_add(counts.total_cards);
}

/// Sums only decks that are not prefixes of another deck.
#[must_use]
pub fn leaf_deck_totals(decks: &[Deck], stats: &[DeckStats]) -> DeckStats {
    let mut total = DeckStats::default();
    for (index, deck) in decks.iter().enumerate() {
        let is_parent = decks.iter().any(|other| {
            other
                .name
                .strip_prefix(deck.name.as_str())
                .is_some_and(|rest| rest.starts_with("::"))
        });
        if is_parent {
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

/// One deck tree node while rows are built.
struct DeckNode<'a> {
    path: String,
    deck: Option<&'a Deck>,
    own: DeckStats,
    total: DeckStats,
    parent: Option<usize>,
}

/// Builds deck rows with virtual parents and stable depth-first ordering.
#[must_use]
pub fn build_deck_rows(
    decks: &[Deck],
    stats: &[DeckStats],
    expanded: &HashSet<String>,
) -> Vec<DeckRow> {
    let capacity = decks.len().saturating_mul(2);
    let mut nodes: Vec<DeckNode<'_>> = Vec::with_capacity(capacity);
    let mut positions: HashMap<String, usize> =
        HashMap::with_capacity(capacity);
    for (deck, stat) in decks.iter().zip(stats.iter()) {
        let path = deck.name.as_str();
        if path.is_empty() {
            continue;
        }
        let mut prefix_end = 0usize;
        let mut segments = path.split("::").peekable();
        while let Some(segment) = segments.next() {
            if prefix_end > 0 {
                prefix_end = prefix_end.saturating_add(2);
            }
            prefix_end = prefix_end.saturating_add(segment.len());
            let Some(prefix) = path.get(..prefix_end) else {
                break;
            };
            let leaf = segments.peek().is_none();
            match positions.get(prefix).copied() {
                Some(position) if leaf => {
                    let placeholder = nodes
                        .get(position)
                        .is_some_and(|node| node.deck.is_none());
                    if placeholder {
                        if let Some(node) = nodes.get_mut(position) {
                            node.deck = Some(deck);
                            node.own = *stat;
                        }
                    } else {
                        nodes.push(DeckNode {
                            path: prefix.to_owned(),
                            deck: Some(deck),
                            own: *stat,
                            total: DeckStats::default(),
                            parent: None,
                        });
                    }
                }
                Some(_) => {}
                None => {
                    let position = nodes.len();
                    nodes.push(DeckNode {
                        path: prefix.to_owned(),
                        deck: leaf.then_some(deck),
                        own: if leaf { *stat } else { DeckStats::default() },
                        total: DeckStats::default(),
                        parent: None,
                    });
                    let _ = positions.insert(prefix.to_owned(), position);
                }
            }
        }
    }
    nodes.sort_by(|left, right| left.path.cmp(&right.path));
    positions.clear();
    for (position, node) in nodes.iter().enumerate() {
        let _ = positions.insert(node.path.clone(), position);
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (position, node) in nodes.iter_mut().enumerate() {
        let parent = node
            .path
            .rsplit_once("::")
            .and_then(|(parent, _)| positions.get(parent).copied());
        node.parent = parent;
        match parent.and_then(|parent| children.get_mut(parent)) {
            Some(list) => list.push(position),
            None => roots.push(position),
        }
    }
    for position in (0..nodes.len()).rev() {
        let Some(node) = nodes.get(position) else {
            continue;
        };
        let mut total = node.total;
        add_deck_stats(&mut total, node.own);
        let parent = node.parent;
        if let Some(node) = nodes.get_mut(position) {
            node.total = total;
        }
        if let Some(node) = parent.and_then(|parent| nodes.get_mut(parent)) {
            add_deck_stats(&mut node.total, total);
        }
    }
    let mut rows = Vec::with_capacity(nodes.len());
    let mut stack = roots
        .iter()
        .rev()
        .map(|position| (*position, 0usize))
        .collect::<Vec<_>>();
    while let Some((position, depth)) = stack.pop() {
        let Some(node) = nodes.get(position) else {
            continue;
        };
        let has_children =
            children.get(position).is_some_and(|list| !list.is_empty());
        let name = node
            .path
            .rsplit("::")
            .next()
            .unwrap_or(&node.path)
            .to_owned();
        let open = expanded.contains(&node.path);
        rows.push(DeckRow {
            id: node.deck.map(|deck| deck.id),
            path: node.path.clone(),
            name,
            depth,
            stats: node.total,
            virtual_group: node.deck.is_none(),
            has_children,
            expanded: open,
        });
        if open && let Some(list) = children.get(position) {
            for child in list.iter().rev() {
                stack.push((*child, depth.saturating_add(1)));
            }
        }
    }
    rows
}

#[must_use]
pub const fn due_total(stats: DeckStats) -> usize {
    stats
        .new_cards
        .saturating_add(stats.learning_cards)
        .saturating_add(stats.review_cards)
}

/// Graded share of the session. An empty queue is never full.
#[must_use]
pub fn session_progress(answered: usize, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    let answered = u32::try_from(answered).unwrap_or(u32::MAX);
    let total = u32::try_from(total).unwrap_or(u32::MAX);
    (answered as f32 / total as f32).clamp(0.0, 1.0)
}

/// Remaining cards in the session, split by queue. The current card counts as
/// remaining until it is graded, so the header never claims a card is done
/// while it still fills the stage.
#[must_use]
pub fn queue_counts(
    current: Option<&Card>,
    queue: &VecDeque<Card>,
) -> DeckStats {
    let mut stats = DeckStats::default();
    for card in current.into_iter().chain(queue.iter()) {
        stats.total_cards = stats.total_cards.saturating_add(1);
        match card.card_type {
            CardType::New => {
                stats.new_cards = stats.new_cards.saturating_add(1);
            }
            CardType::Learning | CardType::Relearning => {
                stats.learning_cards = stats.learning_cards.saturating_add(1);
            }
            CardType::Review => {
                stats.review_cards = stats.review_cards.saturating_add(1);
            }
        }
    }
    stats
}

/// Study time for the session summary: seconds under a minute, then minutes
/// with a seconds remainder, then whole hours.
#[must_use]
pub fn duration_label(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3_600 {
        return format!("{}m {}s", seconds / 60, seconds % 60);
    }
    format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60)
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

/// Matches `needle` inside `haystack` the way a lowercased search would.
fn contains_lowercase(haystack: &str, needle: &str) -> bool {
    if haystack.is_ascii() && needle.is_ascii() {
        let window = needle.len();
        return window > 0
            && haystack
                .as_bytes()
                .windows(window)
                .any(|part| part.eq_ignore_ascii_case(needle.as_bytes()));
    }
    haystack.to_lowercase().contains(needle)
}

/// Keeps a matching row and every ancestor of a match.
#[must_use]
pub fn filter_decks(rows: &[DeckRow], query: &str) -> Vec<DeckRow> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return rows.to_vec();
    }
    let mut keep = HashSet::new();
    for row in rows {
        if contains_lowercase(&row.path, &needle) {
            let _ = keep.insert(row.path.as_str());
            for (index, _) in row.path.match_indices("::") {
                let _ = keep.insert(&row.path[..index]);
            }
        }
    }
    rows.iter()
        .filter(|row| keep.contains(row.path.as_str()))
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

/// Inline validation for the image-occlusion editor.
#[must_use]
pub fn validate_occlusion(
    deck: &Deck,
    model: &NoteModel,
    image: Option<&str>,
    masks: &[OcclusionMask],
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
    if image.is_none_or(str::is_empty) {
        let _ = errors.insert(
            "occlusion".to_owned(),
            "Choose an image from the media directory".to_owned(),
        );
    }
    if masks.is_empty() {
        let _ = errors.insert(
            "masks".to_owned(),
            "Draw at least one mask on the image".to_owned(),
        );
    }
    errors
}

/// Index of the note type with this name, used after a reload.
#[must_use]
pub fn model_index(models: &[NoteModel], name: &str) -> Option<usize> {
    models
        .iter()
        .position(|model| model.name.eq_ignore_ascii_case(name))
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
    matching_rows: usize,
    due: usize,
) -> EmptyKind {
    if deck_count == 0 {
        EmptyKind::NoDecks
    } else if matching_rows == 0 {
        EmptyKind::NoMatch
    } else if due == 0 {
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

/// Study shortcuts stay out of text input and modal dialogs.
const STUDY_CONTEXT: &str = "ranki && !ranki_input && !ranki_dialog";

#[must_use]
pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("space", Reveal, Some(STUDY_CONTEXT)),
        KeyBinding::new("1", GradeAgain, Some(STUDY_CONTEXT)),
        KeyBinding::new("2", GradeHard, Some(STUDY_CONTEXT)),
        KeyBinding::new("3", GradeGood, Some(STUDY_CONTEXT)),
        KeyBinding::new("4", GradeEasy, Some(STUDY_CONTEXT)),
        KeyBinding::new("r", Retry, Some(STUDY_CONTEXT)),
        KeyBinding::new("ctrl-z", Undo, Some(STUDY_CONTEXT)),
        KeyBinding::new("s", StudyAgain, Some(STUDY_CONTEXT)),
        KeyBinding::new("d", OpenDecks, Some(STUDY_CONTEXT)),
        KeyBinding::new("a", OpenAdd, Some(STUDY_CONTEXT)),
        KeyBinding::new("?", ToggleHelp, Some(STUDY_CONTEXT)),
        KeyBinding::new("shift-?", ToggleHelp, Some(STUDY_CONTEXT)),
        KeyBinding::new("f1", ToggleHelp, Some(STUDY_CONTEXT)),
        KeyBinding::new("t", ToggleScheduler, Some(STUDY_CONTEXT)),
        KeyBinding::new("n", NextDeck, Some(STUDY_CONTEXT)),
        KeyBinding::new("esc", EndSession, Some("ranki")),
        KeyBinding::new("cmd-enter", SaveNote, Some("ranki")),
        KeyBinding::new("ctrl-enter", SaveNote, Some("ranki")),
        KeyBinding::new("enter", ConfirmDialog, Some("ranki_dialog")),
        KeyBinding::new("esc", CancelDialog, Some("ranki_dialog")),
        KeyBinding::new("enter", SubmitDeck, Some("ranki_input")),
        KeyBinding::new("ctrl-enter", SaveNote, Some("ranki_input")),
        KeyBinding::new("cmd-enter", SaveNote, Some("ranki_input")),
    ];
    bindings.extend(crate::text_field::key_bindings());
    bindings
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use ranki_model::{
        Card, CardTemplate, CardType, Deck, Id, NoteKind, NoteModel,
        SchedulerKind,
    };

    use crate::Options;

    use super::{
        DeckRow, EmptyKind, Phase, SYNC_STATE, UiAction, View, build_deck_rows,
        collect_tags, date_label, deck_meta, due_total, duration_label,
        empty_copy, empty_kind, filter_decks, focused_delta, model_index,
        models_with_stock, next_due_deck, next_view, note_type_name,
        queue_counts, relative_age, session_progress, status_label,
        sum_leaf_counts, validate_draft, validate_occlusion,
    };
    #[test]
    fn session_progress_never_fills_an_empty_queue() {
        assert_eq!(session_progress(0, 0), 0.0);
        assert_eq!(session_progress(0, 20), 0.0);
        assert_eq!(session_progress(10, 20), 0.5);
        assert_eq!(session_progress(20, 20), 1.0);
        assert_eq!(session_progress(21, 20), 1.0);
    }

    #[test]
    fn duration_labels_read_like_a_session_summary() {
        assert_eq!(duration_label(Duration::from_secs(0)), "0s");
        assert_eq!(duration_label(Duration::from_secs(59)), "59s");
        assert_eq!(duration_label(Duration::from_secs(60)), "1m 0s");
        assert_eq!(duration_label(Duration::from_secs(125)), "2m 5s");
        assert_eq!(duration_label(Duration::from_mins(65)), "1h 5m");
    }

    #[test]
    fn queue_counts_include_the_current_card() {
        let make = |id: Id, kind| Card {
            id,
            note_id: 1,
            deck_id: 1,
            ordinal: 0,
            modified: 0,
            usn: 0,
            card_type: kind,
            queue: ranki_model::CardQueue::New,
            due: 0,
            interval: 0,
            factor: 0,
            reps: 0,
            lapses: 0,
            left: 0,
            original_due: 0,
            original_deck_id: 0,
            flags: 0,
            data: String::new(),
        };
        let mut queue = VecDeque::new();
        queue.push_back(make(2, CardType::Learning));
        queue.push_back(make(3, CardType::Review));
        let stats = queue_counts(Some(&make(1, CardType::New)), &queue);
        assert_eq!(stats.new_cards, 1);
        assert_eq!(stats.learning_cards, 1);
        assert_eq!(stats.review_cards, 1);
        assert_eq!(stats.total_cards, 3);
        let empty = queue_counts(None, &VecDeque::new());
        assert_eq!(empty, ranki_model::DeckStats::default());
    }

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
    fn study_bindings_stay_out_of_text_input() {
        let mut keymap = gpui::Keymap::default();
        keymap.add_bindings(super::key_bindings());
        let typing = [
            gpui::KeyContext::parse("ranki").expect("ranki context"),
            gpui::KeyContext::parse("ranki_input").expect("input context"),
        ];
        let elsewhere =
            [gpui::KeyContext::parse("ranki").expect("ranki context")];
        let keystroke = gpui::Keystroke::parse("a").expect("keystroke");
        let (in_input, _) = keymap
            .bindings_for_input(std::slice::from_ref(&keystroke), &typing);
        assert!(in_input.is_empty());
        let (outside, _) = keymap
            .bindings_for_input(std::slice::from_ref(&keystroke), &elsewhere);
        assert_eq!(outside.len(), 1);
    }

    #[test]
    fn colon_only_deck_names_build_one_row() {
        let decks = [deck(1, ":", 0, 0, 1)];
        let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
        let rows = build_deck_rows(
            &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
            &stats,
            &std::collections::HashSet::default(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, ":");
    }

    #[test]
    fn child_listed_before_parent_replaces_the_virtual_group() {
        let decks = [deck(1, "A::B", 1, 0, 0), deck(2, "A", 0, 0, 0)];
        let stats = decks.iter().map(|(_, stats)| *stats).collect::<Vec<_>>();
        let mut expanded = std::collections::HashSet::new();
        let _ = expanded.insert(String::from("A"));
        let rows = build_deck_rows(
            &decks.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
            &stats,
            &expanded,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "A");
        assert!(!rows[0].virtual_group);
        assert_eq!(rows[1].path, "A::B");
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

    fn row(id: i64, path: &str, depth: usize) -> DeckRow {
        DeckRow {
            id: Some(id),
            path: path.into(),
            name: path.rsplit("::").next().unwrap_or(path).into(),
            depth,
            stats: ranki_model::DeckStats::default(),
            virtual_group: false,
            has_children: false,
            expanded: false,
        }
    }

    #[test]
    fn search_keeps_ancestors_and_sorts_stable() {
        let rows = vec![
            row(1, "A", 0),
            row(2, "A::B", 1),
            row(3, "A::B::C", 2),
            row(4, "A::X", 1),
            row(5, "Z", 0),
        ];
        let paths = |query: &str| {
            filter_decks(&rows, query)
                .iter()
                .map(|row| row.path.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(paths("b"), ["A", "A::B", "A::B::C"]);
        assert_eq!(paths("x"), ["A", "A::X"]);
        assert_eq!(paths("nothing"), Vec::<String>::new());
        assert_eq!(paths("").len(), 5);
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
        assert_eq!(empty_kind(0, 0, 0), EmptyKind::NoDecks);
        assert_eq!(empty_kind(1, 1, 0), EmptyKind::CaughtUp);
        assert_eq!(empty_kind(1, 0, 3), EmptyKind::NoMatch);
        assert_eq!(empty_kind(1, 3, 3), EmptyKind::None);
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
    fn occlusion_validation_needs_an_image_and_a_mask() {
        let deck = Deck {
            id: 1,
            name: "Default".into(),
            description: String::new(),
        };
        let mut model =
            ranki_model::StockModelKind::ImageOcclusion.definition();
        model.id = 5;
        let errors = validate_occlusion(&deck, &model, None, &[]);
        assert_eq!(
            errors.get("occlusion").map(String::as_str),
            Some("Choose an image from the media directory")
        );
        assert_eq!(
            errors.get("masks").map(String::as_str),
            Some("Draw at least one mask on the image")
        );
        let mask = crate::occlusion::default_mask(0);
        let errors = validate_occlusion(&deck, &model, Some("a.png"), &[mask]);
        assert!(errors.is_empty(), "{errors:?}");
        let placeholder = models_with_stock(Vec::new())
            .into_iter()
            .find(ranki_model::NoteModel::is_image_occlusion)
            .expect("stock occlusion");
        let errors =
            validate_occlusion(&deck, &placeholder, Some("a.png"), &[mask]);
        assert_eq!(
            errors.get("model").map(String::as_str),
            Some("This note type is not in the collection")
        );
    }

    #[test]
    fn installing_a_stock_type_keeps_its_tab_selected() {
        let mut installed = ranki_model::StockModelKind::Cloze.definition();
        installed.id = 77;
        let models = models_with_stock(vec![installed]);
        assert_eq!(model_index(&models, "Cloze"), Some(0));
        assert_eq!(models[0].id, 77);
        assert_eq!(model_index(&models, "cloze"), Some(0));
        assert_eq!(model_index(&models, "Nope"), None);
        assert_eq!(models.len(), 4);
        let missing = models_with_stock(Vec::new());
        assert_eq!(model_index(&missing, "Cloze"), Some(2));
        assert_eq!(missing[2].id, 0);
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
