//! Root application state, keyboard dispatcher, and store-backed actions.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    env,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    KeyBinding, MouseButton, MouseDownEvent, PathPromptOptions, Pixels, Point,
    Render, Window, WindowAppearance, actions, div, point, prelude::*, px,
};
use ranki_core::{FsrsScheduler, from_settings};
use ranki_model::{
    Card, CardQueue, CardType, Deck, DeckStats, Id, NewNote, Note, NoteModel,
    Rating, ReviewDecision, StockModelKind, Store,
};

use crate::{
    Options, SettingsSaver, ThemeChoice,
    browser_view::BrowseState,
    card_view::{self, RenderedCard},
    occlusion::{
        self, MaskShape, NormalizedRect, OcclusionMask, default_mask,
        image_occlusion_fields,
    },
    statistics_view::{Statistics, build_statistics},
    text_field::{EditHook, TextField},
    theme::{self, Palette, Theme, interval_label},
};

/// Anki shows learning cards due within this window when the queue runs dry.
pub const LEARN_AHEAD_SECS: i64 = 20 * 60;
pub const DEBOUNCE: Duration = Duration::from_millis(120);
pub const FORECAST_DAYS: usize = 7;
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
        Reload,
        Undo,
        NextField,
        PreviousField,
        StudyAgain,
        EndSession,
        OpenDecks,
        OpenStatistics,
        OpenBrowse,
        OpenAdd,
        CreateDeck,
        OpenSettings,
        UseLightTheme,
        UseDarkTheme,
        UseSystemTheme,
        SubmitDeck,
        SaveNote,
        CancelAdd,
        ToggleHelp,
        ConfirmDialog,
        CancelDialog,
        NextDeck,
        ClearSearch,
        ClearBrowseSearch,
        ToggleSync,
        SyncSignIn,
    ]
);

/// Which full-view screen is visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Decks,
    Browse,
    Review,
    AddCard,
    Settings,
    Statistics,
}

impl View {
    const fn key_context(self) -> &'static str {
        match self {
            Self::Review => "ranki ranki_review",
            Self::Decks
            | Self::Browse
            | Self::AddCard
            | Self::Settings
            | Self::Statistics => "ranki",
        }
    }
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

/// Open deck actions menu and the window point that opened it.
#[derive(Clone, Copy, Debug)]
pub struct DeckMenu {
    pub id: Id,
    pub anchor: Point<Pixels>,
}

/// Pending destructive action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmation {
    RemoveDeck { id: Id, name: String },
    Reload,
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

/// `AnkiWeb` sign-in state kept only for the lifetime of the window.
pub struct SyncUiState {
    pub username: Entity<TextField>,
    pub password: Entity<TextField>,
    pub signed_in: bool,
    pub busy: bool,
}
impl Toast {
    /// A toast outlives its text only by the timer that clears it.
    fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.created_at) >= TOAST_LIFETIME
    }
}

/// Image-occlusion editor state.
#[derive(Default)]
pub struct OcclusionState {
    /// Media file name chosen for the `Image` field.
    pub image: Option<String>,
    /// Natural image size, used to align masks inside a letterboxed canvas.
    pub image_size: Option<(f32, f32)>,
    pub masks: Vec<OcclusionMask>,
    pub selected: Option<usize>,
    /// Drag in progress, in image-relative coordinates.
    pub drag: Option<(f32, f32, f32, f32)>,
    /// Screen rectangle of the drawing area, from the last prepaint.
    pub bounds: Option<Bounds<Pixels>>,
}

impl OcclusionState {
    /// Rectangle of the fitted image inside the drawing area.
    pub(crate) fn image_rect(&self) -> Option<NormalizedRect> {
        let bounds = self.bounds?;
        let Some(size) = self.image_size else {
            return Some(NormalizedRect {
                left: 0.0,
                top: 0.0,
                width: 1.0,
                height: 1.0,
            });
        };
        Some(occlusion::contain_rect(
            size,
            (f32::from(bounds.size.width), f32::from(bounds.size.height)),
        ))
    }

    fn raw_pointer(&self, position: (f32, f32)) -> Option<(f32, f32)> {
        let bounds = self.bounds?;
        let rect = self.image_rect()?;
        let width = rect.width * f32::from(bounds.size.width);
        let height = rect.height * f32::from(bounds.size.height);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let origin_x = f32::from(bounds.origin.x)
            + rect.left * f32::from(bounds.size.width);
        let origin_y = f32::from(bounds.origin.y)
            + rect.top * f32::from(bounds.size.height);
        Some((
            (position.0 - origin_x) / width,
            (position.1 - origin_y) / height,
        ))
    }

    fn pointer(&self, position: (f32, f32)) -> Option<(f32, f32)> {
        self.raw_pointer(position)
            .map(|(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
    }

    /// Topmost mask under an image-relative point, if any.
    #[must_use]
    pub fn mask_at(&self, point: (f32, f32)) -> Option<usize> {
        self.masks
            .iter()
            .enumerate()
            .rev()
            .find(|(_, mask)| mask.rect.contains(point))
            .map(|(index, _)| index)
    }

    /// Removes one mask and keeps the selection valid.
    fn remove_mask(&mut self, index: usize) -> bool {
        if index >= self.masks.len() {
            return false;
        }
        let _ = self.masks.remove(index);
        self.selected = self.masks.len().checked_sub(1);
        true
    }

    /// Removes the topmost mask under an image-relative point.
    fn remove_mask_at(&mut self, point: (f32, f32)) -> bool {
        self.mask_at(point)
            .is_some_and(|index| self.remove_mask(index))
    }

    /// Drops the image and the masks that belonged to it.
    fn clear_image(&mut self) {
        self.image = None;
        self.image_size = None;
        self.masks.clear();
        self.selected = None;
        self.drag = None;
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
    pub deck_id: Option<Id>,
    pub model_index: usize,
    pub fields: Vec<Entity<TextField>>,
    pub tags: Entity<TextField>,
    pub tag_chips: Vec<String>,
    pub errors: BTreeMap<String, String>,
    pub media_filter: Entity<TextField>,
    pub deck_filter: Entity<TextField>,
    pub deck_picker: bool,
    pub deck_picker_anchor: Point<Pixels>,
    pub deck_picker_dismissed_press: Option<MouseDownEvent>,
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
    /// Whether this grade left the card in the session for another step.
    pub requeued: bool,
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
    /// Learning cards waiting for their next step inside this session.
    pub pending: VecDeque<Card>,
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
    /// Cards finished for session progress. Learning repeats stay pending.
    pub completed: usize,
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
    pub previews: [i32; 4],
    pub more_due: bool,
    pub load_errors: usize,
    pub load_error: Option<String>,
}

/// Root entity. The store boundary stays identical to the previous UI.
pub struct Root {
    pub(crate) store: Box<dyn Store>,
    pub(crate) options: Options,
    pub(crate) focus: FocusHandle,
    pub(crate) dialog_focus: FocusHandle,
    pub(crate) settings_focus: FocusHandle,
    pub(crate) system_dark: bool,
    pub(crate) view: View,
    pub(crate) previous_view: View,
    pub(crate) decks: Vec<Deck>,
    pub(crate) models: Vec<NoteModel>,
    pub(crate) stats: Vec<DeckStats>,
    /// Cards scheduled in each of the next seven Anki days, today first.
    pub(crate) forecast: Vec<usize>,
    pub(crate) statistics: Statistics,
    pub(crate) deck_rows: Vec<DeckRow>,
    pub(crate) selected: Option<usize>,
    pub(crate) expanded: HashSet<String>,
    pub(crate) deck_search: Entity<TextField>,
    pub(crate) browse: BrowseState,
    pub(crate) create_field: Entity<TextField>,
    pub(crate) create_modal: bool,
    pub(crate) rename_field: Entity<TextField>,
    pub(crate) deck_edit: Option<DeckEdit>,
    pub(crate) deck_menu: Option<DeckMenu>,
    pub(crate) deck_menu_dismissed_press: Option<MouseDownEvent>,
    pub(crate) confirmation: Option<Confirmation>,
    pub(crate) review: ReviewState,
    pub(crate) add: AddState,
    pub(crate) scheduler: FsrsScheduler,
    pub(crate) toast: Option<Toast>,
    pub(crate) help_open: bool,
    pub(crate) dev_reveal: bool,
    pub(crate) external_change: bool,
    pub(crate) collection_modified: i64,
    pub(crate) collection_start: i64,
    pub(crate) sync: SyncUiState,
    pub(crate) save_settings: SettingsSaver,
    pub(crate) images: Entity<card_view::LruImageCache>,
    pub(crate) font_family: &'static str,
    pub(crate) last_change_check: Instant,
}

/// Values the window needs before the root entity exists.
pub struct Launch {
    pub store: Box<dyn Store>,
    pub options: Options,
    pub save_settings: SettingsSaver,
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

/// Uses the stored card when available so a requeued card carries its saved
/// revision into the next grade.
fn refreshed_card(
    current: &Card,
    load: impl FnOnce(Id) -> Option<Card>,
) -> Card {
    load(current.id).unwrap_or_else(|| current.clone())
}
impl Root {
    #[must_use]
    pub fn new(
        launch: Launch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Launch {
            store,
            options,
            save_settings,
        } = launch;
        let focus = cx.focus_handle();
        let dialog_focus = cx.focus_handle();
        let settings_focus = cx.focus_handle();
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
        let browse_search = cx.new(|cx| TextField::new(cx, "Search cards"));
        let browse_hook = Self::browse_edit_hook(cx);
        browse_search.update(cx, |field, _| field.set_edit_hook(browse_hook));
        let create_field = cx.new(|cx| make_field(cx, "Deck name"));
        let deck_filter = cx.new(|cx| make_field(cx, "Search decks"));
        let media_filter = cx.new(|cx| make_field(cx, "Filter images"));
        let rename_field = cx.new(|cx| make_field(cx, "New name"));
        let tags = cx.new(|cx| make_field(cx, "Add a tag"));
        let tag_hook = Self::tag_hook(cx);
        tags.update(cx, |field, _| field.set_edit_hook(tag_hook));
        let sync_username = cx.new(|cx| make_field(cx, "AnkiWeb username"));
        let sync_password = cx.new(|cx| {
            let mut field = TextField::new(cx, "AnkiWeb password");
            field.set_secure(true, cx);
            field
        });
        let sync_state = crate::sync_view::read_state(&options.collection);
        let signed_in = sync_state
            .as_ref()
            .is_some_and(|state| !state.host_key.is_empty());
        if let Some(name) = options.sync.username.clone() {
            sync_username.update(cx, |field, cx| field.seed(name, cx));
        }
        let mut root = Self {
            scheduler: from_settings(&options),
            store,
            options,
            focus,
            dialog_focus,
            settings_focus,
            system_dark,
            view: View::Decks,
            previous_view: View::Decks,
            decks: Vec::new(),
            models: Vec::new(),
            stats: Vec::new(),
            forecast: Vec::new(),
            statistics: Statistics::default(),
            deck_rows: Vec::new(),
            selected: None,
            expanded: HashSet::new(),
            deck_search,
            browse: BrowseState {
                search: browse_search,
                rows: Vec::new(),
                total: 0,
                selected: None,
                error: None,
                generation: 0,
                loading: false,
            },
            create_field,
            create_modal: false,
            rename_field,
            deck_edit: None,
            deck_menu: None,
            deck_menu_dismissed_press: None,
            confirmation: None,
            review: ReviewState::default(),
            add: AddState {
                deck_id: None,
                model_index: 0,
                fields: Vec::new(),
                tags,
                tag_chips: Vec::new(),
                errors: BTreeMap::new(),
                media_filter,
                deck_filter,
                deck_picker: false,
                deck_picker_anchor: point(px(0.), px(0.)),
                deck_picker_dismissed_press: None,
                occlusion: OcclusionState::default(),
            },
            toast: None,
            help_open: false,
            dev_reveal: env::var("RANKI_UI_DEV_REVEAL")
                .is_ok_and(|value| value == "1"),
            external_change: false,
            collection_modified: 0,
            collection_start: 0,
            sync: SyncUiState {
                username: sync_username,
                password: sync_password,
                signed_in,
                busy: false,
            },
            save_settings,
            images: card_view::LruImageCache::new(cx),
            last_change_check: Instant::now(),
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
            Ok("statistics") => root.view = View::Statistics,
            Ok("add") => root.view = View::AddCard,
            Ok("settings") => root.view = View::Settings,
            Ok("browse") => {
                root.view = View::Browse;
                root.refresh_browse(cx);
            }
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
        if let Ok(path) = env::var("RANKI_UI_DEV_STUDY_DECK")
            && let Some(id) = root
                .deck_rows
                .iter()
                .find(|row| row.path == path)
                .and_then(|row| row.id)
            && let Some(index) =
                root.decks.iter().position(|deck| deck.id == id)
        {
            root.study_deck(index, cx);
        }
        if let Ok(theme) = env::var("RANKI_UI_DEV_THEME") {
            let choice = match theme.as_str() {
                "light" => Some(ThemeChoice::Light),
                "dark" => Some(ThemeChoice::Dark),
                "system" => Some(ThemeChoice::System),
                _ => None,
            };
            if let Some(choice) = choice {
                root.set_theme(choice, cx);
            }
        }
        if root.view == View::Decks
            && env::var("RANKI_UI_DEV_CREATE").is_ok_and(|value| value == "1")
        {
            root.create_modal = true;
            window.focus(root.create_field.read(cx).focus_handle());
        }
        if env::var("RANKI_UI_DEV_MENU").is_ok_and(|value| value == "1") {
            root.view = View::Decks;
            let viewport = window.viewport_size();
            root.deck_menu = root.decks.first().map(|deck| DeckMenu {
                id: deck.id,
                anchor: point(viewport.width - px(79.), px(423.)),
            });
        }
        if env::var("RANKI_UI_DEV_DECK_PICKER").is_ok_and(|value| value == "1")
        {
            root.view = View::AddCard;
            root.add.deck_picker = true;
            root.add.deck_picker_anchor =
                point(px(theme::NAV_WIDTH + theme::PAGE_X + 12.), px(172.));
            window.focus(root.add.deck_filter.read(cx).focus_handle());
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(crate) fn set_theme(
        &mut self,
        choice: ThemeChoice,
        cx: &mut Context<Self>,
    ) {
        if self.options.theme == choice {
            return;
        }
        self.options.theme = choice;
        self.refresh_field_colors(cx);
        cx.notify();
    }

    pub(crate) fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .try_into()
            .unwrap_or(0)
    }

    fn record_local_write(&mut self) {
        if let Ok(stamp) = self.store.collection_modified() {
            self.collection_modified = stamp;
        }
        self.last_change_check = Instant::now();
    }

    fn poll_external_change(&mut self, cx: &mut Context<Self>) {
        if self.external_change
            || self.last_change_check.elapsed() < Duration::from_secs(1)
        {
            return;
        }
        self.last_change_check = Instant::now();
        if self
            .store
            .collection_modified()
            .is_ok_and(|stamp| stamp != self.collection_modified)
        {
            self.external_change = true;
            cx.notify();
        }
    }

    pub(crate) fn start_external_change_poll(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, Self::poll_external_change).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn reload_collection(&mut self, cx: &mut Context<Self>) {
        self.review = ReviewState::default();
        self.view = View::Decks;
        self.previous_view = View::Decks;
        self.add.deck_picker = false;
        self.reload(cx);
    }

    fn request_reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.current.is_some() || self.review.answered > 0 {
            self.confirmation = Some(Confirmation::Reload);
            window.focus(&self.dialog_focus);
        } else {
            self.reload_collection(cx);
        }
        cx.notify();
    }
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let selected_id = self
            .selected
            .and_then(|index| self.decks.get(index))
            .map(|deck| deck.id);
        let draft_fields = self
            .add
            .fields
            .iter()
            .map(|field| field.read(cx).text())
            .collect::<Vec<_>>();
        let previous_fields = self
            .models
            .get(self.add.model_index)
            .map(|model| model.fields.clone())
            .unwrap_or_default();
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
        if let Some(name) = chosen.as_ref()
            && let Some(index) = model_index(&self.models, name)
        {
            self.add.model_index = index;
        }
        if self.add.model_index >= self.models.len() {
            self.add.model_index = 0;
        }
        if let Some(preferred) = crate::add_card_view::preferred_model_index(
            &self.models,
            self.add.model_index,
        ) {
            self.add.model_index = preferred;
        }
        self.refresh_stats();
        self.collection_start = self.store.collection_created().unwrap_or(0);
        self.refresh_statistics(cx);
        self.collection_modified =
            self.store.collection_modified().unwrap_or(0);
        self.external_change = false;
        if let Some(id) = selected_id {
            self.selected = self.decks.iter().position(|deck| deck.id == id);
        }
        if self.selected.is_none() && !self.decks.is_empty() {
            self.selected = Some(0);
        }
        if self.add.deck_id.is_some() && self.add_deck_index().is_none() {
            self.add.deck_id = self
                .selected
                .and_then(|index| self.decks.get(index))
                .map(|deck| deck.id);
        }
        if self.add.deck_id.is_none() && !self.decks.is_empty() {
            self.add.deck_id = self
                .selected
                .and_then(|index| self.decks.get(index))
                .map(|deck| deck.id);
        }
        self.reset_fields(cx);
        let model_fields = self
            .models
            .get(self.add.model_index)
            .map(|model| model.fields.clone())
            .unwrap_or_default();
        for (index, field) in self.add.fields.iter().enumerate() {
            let Some(name) = model_fields.get(index) else {
                continue;
            };
            let Some(old_index) = previous_fields
                .iter()
                .position(|old| old.eq_ignore_ascii_case(name))
            else {
                continue;
            };
            if let Some(value) = draft_fields.get(old_index) {
                field.update(cx, |field, cx| field.seed(value.clone(), cx));
            }
        }
        cx.notify();
    }

    /// Recomputes due counts, the forecast, and deck rows.
    fn refresh_stats(&mut self) {
        self.stats = vec![ranki_model::DeckStats::default(); self.decks.len()];
        self.rebuild_deck_rows();
        let now = Self::now();
        self.stats = match self.store.deck_stats(now) {
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
        self.forecast = self
            .store
            .due_forecast(FORECAST_DAYS, now)
            .unwrap_or_default();
        self.rebuild_deck_rows();
    }

    /// Rebuilds the collection-wide review summary from the store's log.
    fn refresh_statistics(&mut self, cx: &mut Context<Self>) {
        let now = Self::now();
        match self.store.review_history() {
            Ok(history) => {
                self.statistics =
                    build_statistics(&history, now, self.collection_start);
            }
            Err(error) => {
                self.statistics = Statistics::default();
                self.show_toast(
                    ToastKind::Error,
                    format!("Could not load review history: {error}"),
                    cx,
                );
            }
        }
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
    pub(crate) fn add_deck_index(&self) -> Option<usize> {
        self.add
            .deck_id
            .and_then(|id| self.decks.iter().position(|deck| deck.id == id))
    }
    pub(crate) fn select_add_deck(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index >= self.decks.len() {
            return;
        }
        self.add.deck_id = self.decks.get(index).map(|deck| deck.id);
        self.add.deck_picker = false;
        let _ = self.add.errors.remove("deck");
        cx.notify();
    }

    pub(crate) fn toggle_add_deck_picker(
        &mut self,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add.deck_picker = !self.add.deck_picker;
        if self.add.deck_picker {
            self.add.deck_picker_anchor = anchor;
            self.add.deck_picker_dismissed_press = None;
            let filter = self.add.deck_filter.read(cx).focus_handle().clone();
            window.focus(&filter);
        } else {
            window.focus(&self.focus);
        }
        cx.notify();
    }

    /// Copies a chosen image into the media directory for the occlusion editor.
    pub(crate) fn upload_occlusion_image(&mut self, cx: &mut Context<Self>) {
        let media_dir = self.options.media_dir.clone();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose image".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => occlusion::import_image(&path, &media_dir)
                        .map(Some)
                        .map_err(|error| error.to_string()),
                    None => Ok(None),
                },
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |root, cx| match result {
                Ok(Some(name)) => root.choose_occlusion_image(name, cx),
                Ok(None) => {}
                Err(error) => root.show_toast(
                    ToastKind::Error,
                    format!("Could not upload image: {error}"),
                    cx,
                ),
            });
        })
        .detach();
    }

    pub(crate) fn choose_occlusion_image(
        &mut self,
        name: String,
        cx: &mut Context<Self>,
    ) {
        if self.add.occlusion.image.is_some()
            && self.add.occlusion.image.as_deref() != Some(name.as_str())
        {
            self.add.occlusion.masks.clear();
            self.add.occlusion.selected = None;
        }
        let size = image::image_dimensions(self.options.media_dir.join(&name))
            .ok()
            .map(|(width, height)| (width as f32, height as f32));
        self.add.occlusion.image_size = size;
        self.add.occlusion.image = Some(name);
        let _ = self.add.errors.remove("occlusion");
        cx.notify();
    }

    pub(crate) fn remove_occlusion_image(&mut self, cx: &mut Context<Self>) {
        self.add.occlusion.clear_image();
        let _ = self.add.errors.remove("occlusion");
        let _ = self.add.errors.remove("masks");
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
        if self.add.occlusion.image.is_none() {
            return;
        }
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
        self.remove_mask(index, cx);
    }

    fn remove_mask(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.add.occlusion.remove_mask(index) {
            cx.notify();
        }
    }

    /// Removes the mask under a right-click on the canvas.
    pub(crate) fn remove_mask_at(
        &mut self,
        position: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let Some(point) = self.add.occlusion.raw_pointer(position) else {
            return;
        };
        if !(0.0..=1.0).contains(&point.0) || !(0.0..=1.0).contains(&point.1) {
            return;
        }
        if self.add.occlusion.remove_mask_at(point) {
            let _ = self.add.errors.remove("masks");
            cx.notify();
        }
    }

    pub(crate) fn clear_occlusion_masks(&mut self, cx: &mut Context<Self>) {
        self.add.occlusion.masks.clear();
        self.add.occlusion.selected = None;
        self.add.occlusion.drag = None;
        cx.notify();
    }

    /// Records the drawing area so pointer positions can be normalized.
    pub(crate) fn set_occlusion_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.add.occlusion.bounds != Some(bounds) {
            self.add.occlusion.bounds = Some(bounds);
            cx.notify();
        }
    }

    pub(crate) fn occlusion_press(
        &mut self,
        position: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        if self.add.occlusion.image.is_some()
            && let Some(pointer) = self.add.occlusion.pointer(position)
        {
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

    /// Image Occlusion field values in the stock model order.
    pub(crate) fn occlusion_fields(
        &self,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
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
                    self.review.load_error = Some(error.to_string());
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
                self.review.load_error = Some(error.to_string());
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
        self.review.pending.clear();
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
            &self.browse.search,
            &self.create_field,
            &self.rename_field,
            &self.add.tags,
            &self.add.media_filter,
            &self.add.deck_filter,
        ];
        for field in fields {
            field.update(cx, |field, _| field.set_placeholder_color(color));
        }
        for field in &self.add.fields {
            field.update(cx, |field, _| field.set_placeholder_color(color));
        }
    }

    /// Picks the earliest learning card due by `cutoff_secs`.
    ///
    /// Learning dues are Unix milliseconds; review dues are day numbers, so only
    /// queued learning cards can be pulled forward.
    fn due_learning_card(
        pending: &VecDeque<Card>,
        cutoff_secs: i64,
    ) -> Option<Card> {
        let cutoff_ms = cutoff_secs.saturating_mul(1_000);
        pending
            .iter()
            .filter(|card| {
                matches!(
                    card.queue,
                    CardQueue::Learning | CardQueue::DayLearning
                ) && card.due <= cutoff_ms
            })
            .min_by_key(|card| card.due)
            .cloned()
    }

    fn take_learning_card(
        &mut self,
        cutoff_secs: i64,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(card) =
            Self::due_learning_card(&self.review.pending, cutoff_secs)
        else {
            return false;
        };
        self.review.pending.retain(|entry| entry.id != card.id);
        let _ = self.review.seen.remove(&card.id);
        if !self.load_card(&card, cx) {
            self.review.load_errors = self.review.load_errors.saturating_add(1);
        }
        true
    }

    fn ensure_current(&mut self, cx: &mut Context<Self>) {
        let now = Self::now();
        while self.review.current.is_none() && self.take_learning_card(now, cx)
        {
        }
        while self.review.current.is_none()
            && let Some(card) = self.review.queue.pop_front()
        {
            if self.review.seen.contains(&card.id) {
                continue;
            }
            let _ = self.review.seen.insert(card.id);
            if !self.load_card(&card, cx) {
                self.review.load_errors =
                    self.review.load_errors.saturating_add(1);
                continue;
            }
            break;
        }
        while self.review.current.is_none()
            && self.take_learning_card(now.saturating_add(LEARN_AHEAD_SECS), cx)
        {
        }
        if self.review.current.is_none() {
            self.review.question = None;
            self.review.answer = None;
            self.review.note = None;
            self.review.progress_from = self.progress_fraction();
            if self.review.load_error.is_none() {
                let scope = if self.review.scope_all {
                    None
                } else {
                    self.selected
                        .and_then(|index| self.decks.get(index))
                        .map(|deck| deck.id)
                };
                self.review.more_due = self
                    .store
                    .due_cards(scope, 1, Self::now())
                    .is_ok_and(|cards| !cards.is_empty());
            }
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
        let Some(note) = self.store.note(card.note_id).ok().flatten() else {
            self.show_toast(
                ToastKind::Error,
                format!("Card {} has no readable note", card.id),
                cx,
            );
            return None;
        };
        let Some(model) =
            self.models.iter().find(|model| model.id == note.model_id)
        else {
            self.show_toast(
                ToastKind::Error,
                format!("Card {} has no note type", card.id),
                cx,
            );
            return None;
        };
        let question = card_view::render_card(
            model,
            &note,
            card_view::CardRequest::question(card.ordinal),
        );
        let answer = card_view::render_card(
            model,
            &note,
            card_view::CardRequest::answer_only(card.ordinal),
        );
        Some(LoadedCard {
            note: Some(note),
            question: Some(question),
            answer: Some(answer),
        })
    }

    /// Puts a prepared card on the stage and restarts its timers.
    fn apply_loaded(
        &mut self,
        card: &Card,
        loaded: LoadedCard,
        _cx: &mut Context<Self>,
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
        self.refresh_previews();
    }

    pub(crate) fn reveal(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Review {
            return;
        }
        match self.review.phase {
            Phase::Question => {
                self.review.phase = Phase::Answer;
                self.review.last_tick = Some(Instant::now());
                cx.notify();
            }
            Phase::Answer => self.grade(ranki_model::Rating::Good, cx),
        }
    }

    pub(crate) fn toggle_reveal(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Review || self.review.current.is_none() {
            return;
        }
        self.review.phase = match self.review.phase {
            Phase::Question => Phase::Answer,
            Phase::Answer => Phase::Question,
        };
        self.review.last_tick = Some(Instant::now());
        cx.notify();
    }

    fn refresh_previews(&mut self) {
        let Some(card) = self.review.current.as_ref() else {
            return;
        };
        let now = Self::now();
        let start = self.collection_start;
        self.review.previews = [
            self.scheduler.preview(
                card,
                ranki_model::Rating::Again,
                now,
                start,
            ),
            self.scheduler
                .preview(card, ranki_model::Rating::Hard, now, start),
            self.scheduler
                .preview(card, ranki_model::Rating::Good, now, start),
            self.scheduler
                .preview(card, ranki_model::Rating::Easy, now, start),
        ];
    }

    pub(crate) fn grade(&mut self, rating: Rating, cx: &mut Context<Self>) {
        if self.view != View::Review {
            return;
        }
        let Some(current) = self.review.current.clone() else {
            return;
        };
        let card =
            refreshed_card(&current, |id| self.store.card(id).ok().flatten());
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
        self.poll_external_change(cx);
        if let Err(error) = self.store.apply_review(&decision) {
            self.show_toast(
                ToastKind::Error,
                format!("Could not save review: {error}"),
                cx,
            );
            cx.notify();
            return;
        }

        self.record_local_write();
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
        let requeued = matches!(
            decision.card.queue,
            CardQueue::Learning | CardQueue::DayLearning
        );
        if !requeued {
            self.review.completed = self.review.completed.saturating_add(1);
        }
        self.review.last_grade = Some(UndoEntry {
            card,
            revlog_id: decision.revlog.id,
            rating,
            spent: self.review.focused_elapsed,
            requeued,
        });
        if requeued {
            let _ = self.review.seen.remove(&decision.card.id);
            self.review.pending.push_back(decision.card);
        }
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

    /// Fraction of the session that is finished, clamped for both ends of the
    /// queue. Repeats stay pending, so they leave the bar unchanged.
    pub(crate) fn progress_fraction(&self) -> f32 {
        session_progress(self.review.completed, self.review.session_total)
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
        self.poll_external_change(cx);
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
                    self.review.pending.retain(|card| card.id != current.id);
                    if current.id != entry.card.id {
                        self.review.queue.push_front(current);
                    }
                }
                self.review.pending.retain(|card| card.id != entry.card.id);
                if !entry.requeued {
                    self.review.completed =
                        self.review.completed.saturating_sub(1);
                }
                let _ = self.review.seen.remove(&entry.card.id);
                let Some(restored) =
                    self.store.card(entry.card.id).ok().flatten()
                else {
                    self.show_toast(
                        ToastKind::Error,
                        "Could not reload the undone card",
                        cx,
                    );
                    return;
                };
                let Some(loaded) = self.render_state(&restored, cx) else {
                    return;
                };
                self.apply_loaded(&restored, loaded, cx);
                self.review.last_grade_at = None;
                self.record_local_write();
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
            Some(Confirmation::Reload) => self.reload_collection(cx),
            None => {}
        }
        cx.notify();
    }

    pub(crate) fn open_create_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_field.update(cx, TextField::clear);
        self.create_modal = true;
        window.focus(self.create_field.read(cx).focus_handle());
        cx.notify();
    }

    pub(crate) fn create_deck(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self.create_field.read(cx).text().trim().to_owned();
        if name.is_empty() {
            self.show_toast(ToastKind::Error, "Deck name cannot be empty", cx);
            cx.notify();
            return;
        }
        match self.store.create_deck(&name) {
            Ok(deck) => {
                for (index, _) in deck.name.match_indices("::") {
                    let _ = self.expanded.insert(deck.name[..index].to_owned());
                }
                self.create_field
                    .update(cx, super::text_field::TextField::clear);
                self.create_modal = false;
                self.clear_toast();
                self.reload(cx);
                window.focus(&self.focus);
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

    pub(crate) fn cancel_create(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_field.update(cx, TextField::clear);
        self.create_modal = false;
        window.focus(&self.focus);
        cx.notify();
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
        if self.view != View::AddCard {
            return;
        }
        let deck_index = self.add_deck_index();
        let Some(deck) =
            deck_index.and_then(|index| self.decks.get(index)).cloned()
        else {
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
        let note = NewNote {
            model_id: model.id,
            deck_id: deck.id,
            fields,
            tags,
        };
        self.poll_external_change(cx);
        match self.store.add_note(&note) {
            Ok(ids) => {
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
                self.refresh_stats();
                self.add.occlusion = OcclusionState::default();
                self.record_local_write();
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
        if self.deck_menu.take().is_some() {
            self.deck_menu_dismissed_press = None;
            cx.notify();
        }
        let had_dialog = self.confirmation.is_some();
        match action {
            UiAction::Reveal => self.reveal(cx),
            UiAction::Grade(Rating::Again) => self.grade(Rating::Again, cx),
            UiAction::Grade(Rating::Hard) => self.grade(Rating::Hard, cx),
            UiAction::Grade(Rating::Good) => self.grade(Rating::Good, cx),
            UiAction::Grade(Rating::Easy) => self.grade(Rating::Easy, cx),
            UiAction::Retry => self.toggle_reveal(cx),
            UiAction::Reload => self.request_reload(window, cx),
            UiAction::Undo => self.undo_grade(cx),
            UiAction::StudyAgain => {
                if self.view == View::Review
                    && self.review.current.is_none()
                    && (self.review.answered > 0
                        || self.review.load_errors > 0
                        || self.review.load_error.is_some())
                {
                    self.study_again(cx);
                }
            }
            UiAction::EndSession => {
                if self.view == View::Review {
                    self.end_session(cx);
                } else if self.view == View::AddCard && self.add.deck_picker {
                    self.add.deck_picker = false;
                    window.focus(&self.focus);
                    cx.notify();
                } else if self.confirmation.is_some()
                    || self.help_open
                    || self.create_modal
                {
                    self.confirmation = None;
                    self.help_open = false;
                    if self.create_modal {
                        self.cancel_create(window, cx);
                    }
                    cx.notify();
                }
            }
            UiAction::OpenDecks => {
                self.view = next_view(self.view, View::Decks);
                self.refresh_stats();
                window.focus(&self.focus);
                cx.notify();
            }
            UiAction::OpenStatistics => {
                self.view = next_view(self.view, View::Statistics);
                self.refresh_statistics(cx);
                cx.notify();
            }
            UiAction::OpenBrowse => {
                self.view = View::Browse;
                let field = self.browse.search.clone();
                self.refresh_browse(cx);
                cx.notify();
                window.defer(cx, move |window, cx| {
                    window.focus(field.read(cx).focus_handle());
                });
            }
            UiAction::OpenAdd => {
                if self.view != View::AddCard {
                    if let Some(deck) =
                        self.selected.and_then(|index| self.decks.get(index))
                    {
                        self.add.deck_id = Some(deck.id);
                    }
                    self.add.deck_picker = false;
                    self.previous_view = self.view;
                }
                self.view = next_view(self.view, View::AddCard);
                self.add.errors.clear();
                window.focus(&self.focus);
                cx.notify();
            }
            UiAction::OpenSettings => {
                self.view = next_view(self.view, View::Settings);
                window.focus(&self.settings_focus);
                cx.notify();
            }
            UiAction::SetTheme(choice) => {
                if self.view == View::Settings {
                    window.focus(&self.settings_focus);
                }
                self.set_theme(choice, cx);
            }
            UiAction::CreateDeck => self.open_create_modal(window, cx),
            UiAction::SubmitDeck => {
                if self.create_modal {
                    self.create_deck(window, cx);
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
            UiAction::Confirm => {
                if self.create_modal {
                    self.create_deck(window, cx);
                } else {
                    self.confirm(cx);
                }
            }
            UiAction::CancelDialog => {
                self.confirmation = None;
                if self.create_modal {
                    self.cancel_create(window, cx);
                } else {
                    cx.notify();
                }
            }
            UiAction::NextDeck => {
                self.refresh_stats();
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
            UiAction::ClearBrowseSearch => {
                self.browse
                    .search
                    .update(cx, super::text_field::TextField::clear);
                self.refresh_browse(cx);
                window.focus(self.browse.search.read(cx).focus_handle());
                cx.notify();
            }
            UiAction::ToggleSync => {
                let enabled = !self.options.sync.enabled;
                self.set_sync_enabled(enabled, cx);
            }
            UiAction::SyncSignIn => self.sign_in(cx),
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
    Reload,
    Undo,
    StudyAgain,
    EndSession,
    OpenDecks,
    OpenStatistics,
    OpenBrowse,
    OpenAdd,
    OpenSettings,
    SetTheme(ThemeChoice),
    CreateDeck,
    SubmitDeck,
    SaveNote,
    CancelAdd,
    ToggleHelp,
    Confirm,
    CancelDialog,
    NextDeck,
    ClearSearch,
    ClearBrowseSearch,
    ToggleSync,
    SyncSignIn,
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
impl From<&Reload> for UiAction {
    fn from(_: &Reload) -> Self {
        Self::Reload
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
impl From<&OpenSettings> for UiAction {
    fn from(_: &OpenSettings) -> Self {
        Self::OpenSettings
    }
}
impl From<&UseLightTheme> for UiAction {
    fn from(_: &UseLightTheme) -> Self {
        Self::SetTheme(ThemeChoice::Light)
    }
}
impl From<&UseDarkTheme> for UiAction {
    fn from(_: &UseDarkTheme) -> Self {
        Self::SetTheme(ThemeChoice::Dark)
    }
}
impl From<&UseSystemTheme> for UiAction {
    fn from(_: &UseSystemTheme) -> Self {
        Self::SetTheme(ThemeChoice::System)
    }
}
impl From<&OpenStatistics> for UiAction {
    fn from(_: &OpenStatistics) -> Self {
        Self::OpenStatistics
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
impl From<&OpenBrowse> for UiAction {
    fn from(_: &OpenBrowse) -> Self {
        Self::OpenBrowse
    }
}

impl From<&ClearBrowseSearch> for UiAction {
    fn from(_: &ClearBrowseSearch) -> Self {
        Self::ClearBrowseSearch
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
        UiAction::Reload => window.dispatch_action(Box::new(Reload), cx),
        UiAction::Undo => window.dispatch_action(Box::new(Undo), cx),
        UiAction::StudyAgain => {
            window.dispatch_action(Box::new(StudyAgain), cx);
        }
        UiAction::EndSession => {
            window.dispatch_action(Box::new(EndSession), cx);
        }
        UiAction::OpenDecks => window.dispatch_action(Box::new(OpenDecks), cx),
        UiAction::OpenStatistics => {
            window.dispatch_action(Box::new(OpenStatistics), cx);
        }
        UiAction::OpenBrowse => {
            window.dispatch_action(Box::new(OpenBrowse), cx);
        }
        UiAction::OpenAdd => window.dispatch_action(Box::new(OpenAdd), cx),
        UiAction::OpenSettings => {
            window.dispatch_action(Box::new(OpenSettings), cx);
        }
        UiAction::SetTheme(ThemeChoice::Light) => {
            window.dispatch_action(Box::new(UseLightTheme), cx);
        }
        UiAction::SetTheme(ThemeChoice::Dark) => {
            window.dispatch_action(Box::new(UseDarkTheme), cx);
        }
        UiAction::SetTheme(ThemeChoice::System) => {
            window.dispatch_action(Box::new(UseSystemTheme), cx);
        }
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
        UiAction::ClearBrowseSearch => {
            window.dispatch_action(Box::new(ClearBrowseSearch), cx);
        }
        UiAction::ToggleSync => {
            window.dispatch_action(Box::new(ToggleSync), cx);
        }
        UiAction::SyncSignIn => {
            window.dispatch_action(Box::new(SyncSignIn), cx);
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
        self.poll_external_change(cx);
        let colors = self.colors();
        let view = self.view;
        let main = match view {
            View::Decks => self.render_decks(cx).into_any_element(),
            View::Browse => self.render_browse(window, cx).into_any_element(),
            View::Review => self.render_review(window, cx).into_any_element(),
            View::AddCard => {
                self.render_add_card(window, cx).into_any_element()
            }
            View::Settings => self.render_settings().into_any_element(),
            View::Statistics => self.render_statistics(cx).into_any_element(),
        };
        div()
            .id("root")
            .size_full()
            .flex()
            .key_context(view.key_context())
            .bg(colors.base)
            .text_color(colors.text)
            .font_family(self.font_family)
            .track_focus(&self.focus)
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                let menu = this.deck_menu_dismissed_press.take().is_some();
                let picker =
                    this.add.deck_picker_dismissed_press.take().is_some();
                if menu || picker {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let menu = this.deck_menu_dismissed_press.take().is_some();
                    let picker =
                        this.add.deck_picker_dismissed_press.take().is_some();
                    if menu || picker {
                        cx.notify();
                    }
                }),
            )
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
            .on_action(cx.listener(|this, action: &Reload, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &Undo, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|_, _: &NextField, window, _| {
                window.focus_next();
            }))
            .on_action(cx.listener(|_, _: &PreviousField, window, _| {
                window.focus_prev();
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
            .on_action(cx.listener(
                |this, action: &OpenStatistics, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .on_action(cx.listener(|this, action: &OpenBrowse, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, action: &OpenAdd, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(
                |this, action: &OpenSettings, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .on_action(cx.listener(
                |this, action: &UseLightTheme, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .on_action(cx.listener(
                |this, action: &UseDarkTheme, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .on_action(cx.listener(
                |this, action: &UseSystemTheme, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
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
            .on_action(cx.listener(
                |this, action: &ClearBrowseSearch, window, cx| {
                    this.dispatch(UiAction::from(action), window, cx);
                },
            ))
            .on_action(cx.listener(|this, action: &NextDeck, window, cx| {
                this.dispatch(UiAction::from(action), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSync, window, cx| {
                this.dispatch(UiAction::ToggleSync, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SyncSignIn, window, cx| {
                this.dispatch(UiAction::SyncSignIn, window, cx);
            }))
            .when(view != View::Review, |this| this.child(self.render_nav()))
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
    let names = decks
        .iter()
        .map(|deck| deck.name.as_str())
        .collect::<HashSet<_>>();
    let mut total = DeckStats::default();
    for (index, deck) in decks.iter().enumerate() {
        let is_nested = deck
            .name
            .match_indices("::")
            .any(|(index, _)| names.contains(&deck.name[..index]));
        if is_nested {
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
        let total = if node.deck.is_some() {
            node.own
        } else {
            node.total
        };
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

/// Total cards in the forecast window.
#[must_use]
pub fn forecast_total(counts: &[usize]) -> usize {
    counts
        .iter()
        .fold(0_usize, |total, count| total.saturating_add(*count))
}

/// Height of one forecast bar. Zero days keep a small baseline.
#[must_use]
pub fn forecast_bar_height(count: usize, peak: usize) -> f32 {
    if count == 0 || peak == 0 {
        return 6.0;
    }
    18.0 + (count as f32 / peak as f32) * 42.0
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

/// Remaining cards in the session, split by queue. Learning cards waiting for
/// their next step and the current card count as remaining, so the header never
/// claims a card is done while it will appear again.
#[must_use]
pub fn queue_counts(
    current: Option<&Card>,
    queue: &VecDeque<Card>,
    pending: &VecDeque<Card>,
) -> DeckStats {
    let mut stats = DeckStats::default();
    for card in current
        .into_iter()
        .chain(queue.iter())
        .chain(pending.iter())
    {
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

/// Short label for a note-type tab.
#[must_use]
pub fn note_type_name(model: &NoteModel) -> String {
    let name = model.name.to_lowercase();
    if model.is_image_occlusion() || name.contains("occlusion") {
        String::from("Image Occlusion")
    } else if name.contains("cloze") {
        String::from("Cloze")
    } else if name.contains("reversed") {
        String::from("Reversed")
    } else if name.starts_with("basic") {
        String::from("Basic")
    } else {
        model.name.clone()
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
    if model.card_ordinals(fields).is_empty() {
        let _ = errors.insert(
            "fields".to_owned(),
            "This note type would not create any cards".to_owned(),
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
        let _ = errors
            .insert("occlusion".to_owned(), "Upload an image first".to_owned());
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

/// Installed note types plus every stock type the collection is missing.
///
/// Missing stock types keep placeholder id 0, so the add-card screen can show
/// their fields before the collection has the model.
#[must_use]
pub fn models_with_stock(installed: Vec<NoteModel>) -> Vec<NoteModel> {
    let mut models = installed;
    for kind in ranki_model::StockModelKind::ALL {
        let known = models
            .iter()
            .any(|model| ranki_model::StockModelKind::of(model) == Some(kind));
        if !known {
            models.push(kind.definition());
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

/// Study shortcuts stay out of text input, dialogs, and the settings screen.
const STUDY_CONTEXT: &str =
    "ranki && !ranki_input && !ranki_dialog && !ranki_settings";
/// Settings shortcuts only apply while the settings screen is mounted.
const SETTINGS_CONTEXT: &str =
    "ranki > ranki_settings && !ranki_input && !ranki_dialog";
const STATISTICS_CONTEXT: &str =
    "ranki && !ranki_review && !ranki_input && !ranki_dialog";

/// Tab navigation stays out of modal dialogs.
const FOCUS_CONTEXT: &str = "ranki && !ranki_dialog";

#[must_use]
pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("space", Reveal, Some(STUDY_CONTEXT)),
        KeyBinding::new("1", GradeAgain, Some(STUDY_CONTEXT)),
        KeyBinding::new("2", GradeHard, Some(STUDY_CONTEXT)),
        KeyBinding::new("3", GradeGood, Some(STUDY_CONTEXT)),
        KeyBinding::new("4", GradeEasy, Some(STUDY_CONTEXT)),
        KeyBinding::new("r", Retry, Some(STUDY_CONTEXT)),
        KeyBinding::new("enter", Reveal, Some(STUDY_CONTEXT)),
        KeyBinding::new("f5", Reload, Some("ranki")),
        KeyBinding::new("ctrl-z", Undo, Some(STUDY_CONTEXT)),
        KeyBinding::new("s", StudyAgain, Some(STUDY_CONTEXT)),
        KeyBinding::new("d", OpenDecks, Some(STUDY_CONTEXT)),
        KeyBinding::new("b", OpenBrowse, Some(STUDY_CONTEXT)),
        KeyBinding::new("a", OpenAdd, Some(STUDY_CONTEXT)),
        KeyBinding::new(",", OpenSettings, Some(STUDY_CONTEXT)),
        KeyBinding::new("?", ToggleHelp, Some(STUDY_CONTEXT)),
        KeyBinding::new("shift-?", ToggleHelp, Some(STUDY_CONTEXT)),
        KeyBinding::new("f1", ToggleHelp, Some(STUDY_CONTEXT)),
        KeyBinding::new("t", OpenStatistics, Some(STATISTICS_CONTEXT)),
        KeyBinding::new("n", NextDeck, Some(STUDY_CONTEXT)),
        KeyBinding::new("d", OpenDecks, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("a", OpenAdd, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("esc", EndSession, Some("ranki")),
        KeyBinding::new("tab", NextField, Some(FOCUS_CONTEXT)),
        KeyBinding::new("shift-tab", PreviousField, Some(FOCUS_CONTEXT)),
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
    use gpui::AppContext as _;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::Duration;

    use ranki_model::Store as _;
    use ranki_model::{
        Card, CardQueue, CardTemplate, CardType, Deck, Id, NewNote, Note,
        NoteKind, NoteModel, ReviewDecision, StoreError,
    };

    use crate::Options;

    use super::{
        DeckRow, EmptyKind, MaskShape, NormalizedRect, OcclusionMask,
        OcclusionState, Phase, UiAction, View, build_deck_rows, collect_tags,
        date_label, deck_meta, due_total, duration_label, empty_copy,
        empty_kind, filter_decks, focused_delta, forecast_bar_height,
        forecast_total, model_index, models_with_stock, next_due_deck,
        next_view, queue_counts, refreshed_card, session_progress,
        sum_leaf_counts, validate_draft, validate_occlusion,
    };
    use gpui::{point, px, size};

    struct ModalStore;

    impl ranki_model::Store for ModalStore {
        fn decks(&self) -> Result<Vec<Deck>, StoreError> {
            Ok(vec![Deck {
                id: 1,
                name: String::from("Default"),
                description: String::new(),
            }])
        }

        fn create_deck(&mut self, name: &str) -> Result<Deck, StoreError> {
            Ok(Deck {
                id: 2,
                name: name.to_owned(),
                description: String::new(),
            })
        }

        fn models(&self) -> Result<Vec<NoteModel>, StoreError> {
            Ok(ranki_model::stock_models())
        }

        fn find_cards(&self, _: &str) -> Result<Vec<Id>, StoreError> {
            Ok(Vec::new())
        }

        fn due_cards(
            &self,
            _: Option<Id>,
            _: usize,
            _: i64,
        ) -> Result<Vec<Card>, StoreError> {
            Ok(Vec::new())
        }

        fn card(&self, _: Id) -> Result<Option<Card>, StoreError> {
            Ok(None)
        }

        fn note(&self, _: Id) -> Result<Option<Note>, StoreError> {
            Ok(None)
        }

        fn add_note(&mut self, _: &NewNote) -> Result<Vec<Id>, StoreError> {
            Ok(Vec::new())
        }

        fn apply_review(
            &mut self,
            _: &ReviewDecision,
        ) -> Result<(), StoreError> {
            Ok(())
        }

        fn stats(
            &self,
            _: Option<Id>,
            _: i64,
        ) -> Result<ranki_model::DeckStats, StoreError> {
            Ok(ranki_model::DeckStats::default())
        }

        fn collection_modified(&self) -> Result<i64, StoreError> {
            Ok(0)
        }

        fn collection_created(&self) -> Result<i64, StoreError> {
            Ok(0)
        }
    }

    #[gpui::test]
    fn create_deck_modal_closes_only_on_outside_click(
        cx: &mut gpui::TestAppContext,
    ) {
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(ModalStore),
                    options: Options::default(),
                    save_settings: std::sync::Arc::new(|_| Ok(())),
                },
                window,
                cx,
            )
        });
        cx.run_until_parked();
        cx.dispatch_action(super::CreateDeck);
        cx.run_until_parked();
        assert!(cx.update(|_, app| root.read(app).create_modal));

        let center = cx.update(|window, _| {
            let size = window.viewport_size();
            point(
                px(f32::from(size.width) / 2.),
                px(f32::from(size.height) / 2.),
            )
        });
        cx.simulate_click(center, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            cx.update(|_, app| root.read(app).create_modal),
            "clicking inside the modal must keep it open"
        );

        cx.simulate_click(point(px(4.), px(4.)), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            !cx.update(|_, app| root.read(app).create_modal),
            "clicking outside the modal must close it"
        );
    }
    #[gpui::test]
    fn deck_picker_floats_above_the_form(cx: &mut gpui::TestAppContext) {
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(ModalStore),
                    options: Options::default(),
                    save_settings: std::sync::Arc::new(|_| Ok(())),
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
        cx.update_entity(&root, |root, cx| {
            root.view = super::View::AddCard;
            cx.notify();
        });
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(1280.), px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let button = cx
            .debug_bounds("add-deck-picker")
            .expect("deck picker button bounds");

        let center = button.center();
        cx.simulate_mouse_move(center, None, gpui::Modifiers::none());
        cx.simulate_mouse_down(
            center,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_mouse_up(
            center,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        let after = cx
            .debug_bounds("add-deck-picker")
            .expect("deck picker button bounds after opening");
        assert_eq!(button.origin, after.origin);
        assert_eq!(button.size, after.size);

        let popup = cx
            .debug_bounds("add-deck-picker-popup")
            .expect("deck picker popup bounds");
        assert!(
            popup.size.width <= px(360.),
            "popup was {} wide",
            popup.size.width
        );
        assert!(popup.origin.y >= after.bottom());
    }
    #[gpui::test]
    fn deck_picker_clicks_do_not_reach_the_editor(
        cx: &mut gpui::TestAppContext,
    ) {
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(ModalStore),
                    options: Options::default(),
                    save_settings: std::sync::Arc::new(|_| Ok(())),
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
        cx.update_entity(&root, |root, cx| {
            root.view = super::View::AddCard;
            root.seed_fields("untouched", cx);
            root.add.deck_picker = true;
            root.add.deck_picker_anchor = point(px(270.), px(224.));
            cx.notify();
        });
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(1280.), px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        let row = cx
            .debug_bounds("add-deck-0")
            .expect("first deck row bounds");
        let bold = cx.debug_bounds("add-format-B").expect("bold button bounds");
        assert!(
            row.intersects(&bold),
            "row {row:?} must overlap bold {bold:?}"
        );
        let click = point(
            px(f32::midpoint(
                f32::from(row.origin.x.max(bold.origin.x)),
                f32::from(row.bottom_right().x.min(bold.bottom_right().x)),
            )),
            px(f32::midpoint(
                f32::from(row.origin.y.max(bold.origin.y)),
                f32::from(row.bottom_right().y.min(bold.bottom_right().y)),
            )),
        );
        cx.simulate_mouse_move(click, None, gpui::Modifiers::none());
        cx.simulate_mouse_down(
            click,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_mouse_up(
            click,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();

        let (text, picker_open) = cx.read_entity(&root, |root, cx| {
            (root.add.fields[0].read(cx).text(), root.add.deck_picker)
        });
        assert!(!picker_open, "selecting a deck must close the picker");
        assert_eq!(text, "untouched");
    }

    fn click_test_point(
        cx: &mut gpui::VisualTestContext,
        at: gpui::Point<gpui::Pixels>,
    ) {
        cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
        cx.simulate_mouse_down(
            at,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_mouse_up(
            at,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    #[gpui::test]
    fn deck_picker_dismisses_on_outside_click(cx: &mut gpui::TestAppContext) {
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(ModalStore),
                    options: Options::default(),
                    save_settings: std::sync::Arc::new(|_| Ok(())),
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(px(1280.), px(800.)));
        cx.update_entity(&root, |root, cx| {
            root.view = super::View::AddCard;
            cx.notify();
        });
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(1280.), px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let button = cx
            .debug_bounds("add-deck-picker")
            .expect("deck picker button bounds")
            .center();

        click_test_point(cx, button);
        assert!(cx.read_entity(&root, |root, _| root.add.deck_picker));

        click_test_point(cx, point(px(1100.), px(600.)));
        assert!(!cx.read_entity(&root, |root, _| root.add.deck_picker));

        click_test_point(cx, button);
        assert!(cx.read_entity(&root, |root, _| root.add.deck_picker));

        click_test_point(cx, button);
        assert!(
            !cx.read_entity(&root, |root, _| root.add.deck_picker),
            "the dismissing press must not reopen the picker"
        );
    }

    #[gpui::test]
    fn deck_picker_closes_when_the_form_scrolls(cx: &mut gpui::TestAppContext) {
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(ModalStore),
                    options: Options::default(),
                    save_settings: std::sync::Arc::new(|_| Ok(())),
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(px(1280.), px(800.)));
        cx.update_entity(&root, |root, cx| {
            root.view = super::View::AddCard;
            root.add.deck_picker = true;
            root.add.deck_picker_anchor = point(px(270.), px(224.));
            cx.notify();
        });
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(1280.), px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let popup = cx
            .debug_bounds("add-deck-picker-popup")
            .expect("deck picker popup bounds");
        let scroll = |cx: &mut gpui::VisualTestContext, at| {
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: at,
                delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-20.))),
                modifiers: gpui::Modifiers::none(),
                touch_phase: gpui::TouchPhase::Moved,
            });
            cx.run_until_parked();
        };

        scroll(
            cx,
            point(popup.origin.x + px(160.), popup.origin.y + px(70.)),
        );
        assert!(
            cx.read_entity(&root, |root, _| root.add.deck_picker),
            "scrolling the deck list must keep the picker open"
        );

        scroll(cx, point(px(1100.), px(600.)));
        assert!(!cx.read_entity(&root, |root, _| root.add.deck_picker));
    }

    #[test]
    fn session_progress_never_fills_an_empty_queue() {
        assert_eq!(session_progress(0, 0), 0.0);
        assert_eq!(session_progress(0, 20), 0.0);
        assert_eq!(session_progress(10, 20), 0.5);
        assert_eq!(session_progress(20, 20), 1.0);
        assert_eq!(session_progress(21, 20), 1.0);
    }

    #[gpui::test]
    fn repeated_hard_answers_leave_progress_unchanged(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("collection.anki2");
        let mut store =
            ranki_storage::Collection::create(&path).expect("collection");
        let _ = store
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![
                    String::from("Question?"),
                    String::from("Answer!"),
                ],
                tags: Vec::new(),
            })
            .expect("note");
        let options = Options {
            collection: path,
            ..Options::default()
        };
        let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(store),
                    options,
                    save_settings,
                },
                window,
                cx,
            )
        });
        cx.update_entity(&root, super::Root::study_all);

        let mut grade = |rating| {
            cx.update_entity(&root, |root, cx| {
                root.review.phase = Phase::Answer;
                root.review.last_grade_at = None;
                root.grade(rating, cx);
            });
            cx.read_entity(&root, |root, _| {
                (root.progress_fraction(), root.review.answered)
            })
        };
        assert_eq!(grade(ranki_model::Rating::Hard), (0.0, 1));
        assert_eq!(grade(ranki_model::Rating::Hard), (0.0, 2));
        assert_eq!(grade(ranki_model::Rating::Easy), (1.0, 3));
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
        let mut pending = VecDeque::new();
        pending.push_back(make(4, CardType::Learning));
        let stats =
            queue_counts(Some(&make(1, CardType::New)), &queue, &pending);
        assert_eq!(stats.new_cards, 1);
        assert_eq!(stats.learning_cards, 2);
        assert_eq!(stats.review_cards, 1);
        assert_eq!(stats.total_cards, 4);
        let empty = queue_counts(None, &VecDeque::new(), &VecDeque::new());
        assert_eq!(empty, ranki_model::DeckStats::default());
    }

    #[test]
    fn review_uses_anki_fsrs_defaults() {
        let scheduler = super::from_settings(&Options::default());
        assert_eq!(scheduler.params(), &ranki_core::FsrsParams::default());
        assert_eq!(scheduler.preset().desired_retention, 0.9);
    }

    #[test]
    fn grading_uses_the_stored_card_revision() {
        let current = Card {
            id: 7,
            note_id: 1,
            deck_id: 1,
            ordinal: 0,
            modified: 100,
            usn: -1,
            card_type: CardType::Learning,
            queue: CardQueue::Learning,
            due: 101_000,
            interval: -60,
            factor: 2_500,
            reps: 1,
            lapses: 0,
            left: 1_001,
            original_due: 0,
            original_deck_id: 0,
            flags: 0,
            data: String::new(),
        };
        let mut saved = current.clone();
        saved.usn = 4;
        assert_eq!(refreshed_card(&current, |_| Some(saved.clone())).usn, 4);
        assert_eq!(refreshed_card(&current, |_| None), current);
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
            deck(1, "A", 1, 1, 4),
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
    fn review_keys_are_bound_where_they_are_advertised() {
        let mut keymap = gpui::Keymap::default();
        keymap.add_bindings(super::key_bindings());
        let root = [gpui::KeyContext::parse("ranki").expect("ranki context")];
        for (key, expected) in [("enter", 1), ("r", 1), ("f5", 1)] {
            let stroke = gpui::Keystroke::parse(key).expect("keystroke");
            let (bindings, _) = keymap.bindings_for_input(&[stroke], &root);
            assert_eq!(bindings.len(), expected, "{key}");
        }
    }

    #[test]
    fn tab_moves_focus_in_and_out_of_text_inputs() {
        let mut keymap = gpui::Keymap::default();
        keymap.add_bindings(super::key_bindings());
        let root = [gpui::KeyContext::parse("ranki").expect("ranki context")];
        let typing = [
            gpui::KeyContext::parse("ranki").expect("ranki context"),
            gpui::KeyContext::parse("ranki_input").expect("input context"),
        ];
        let dialog = [
            gpui::KeyContext::parse("ranki").expect("ranki context"),
            gpui::KeyContext::parse("ranki_dialog").expect("dialog context"),
        ];
        for key in ["tab", "shift-tab"] {
            let stroke = gpui::Keystroke::parse(key).expect("keystroke");
            let (from_root, _) =
                keymap.bindings_for_input(std::slice::from_ref(&stroke), &root);
            let (from_input, _) = keymap
                .bindings_for_input(std::slice::from_ref(&stroke), &typing);
            assert_eq!(from_root.len(), 1, "{key}");
            assert_eq!(from_input.len(), 1, "{key}");
            let (in_dialog, _) = keymap
                .bindings_for_input(std::slice::from_ref(&stroke), &dialog);
            assert!(in_dialog.is_empty(), "{key}");
        }
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
    fn theme_has_no_number_shortcuts() {
        let mut keymap = gpui::Keymap::default();
        keymap.add_bindings(super::key_bindings());
        let settings = [
            gpui::KeyContext::parse("ranki").expect("ranki context"),
            gpui::KeyContext::parse("ranki_settings")
                .expect("settings context"),
        ];
        for key in ["1", "2", "3"] {
            let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
            let (in_settings, _) = keymap.bindings_for_input(
                std::slice::from_ref(&keystroke),
                &settings,
            );
            assert!(in_settings.is_empty());
        }
    }

    #[test]
    fn statistics_key_is_only_bound_in_statistics() {
        let mut keymap = gpui::Keymap::default();
        keymap.add_bindings(super::key_bindings());
        let keystroke = gpui::Keystroke::parse("t").expect("keystroke");
        let stats = [gpui::KeyContext::parse(View::Statistics.key_context())
            .expect("stats context")];
        let review = [gpui::KeyContext::parse(View::Review.key_context())
            .expect("review context")];
        let typing = [
            gpui::KeyContext::parse(View::Statistics.key_context())
                .expect("stats context"),
            gpui::KeyContext::parse("ranki_input").expect("input context"),
        ];
        let (on_stats, _) =
            keymap.bindings_for_input(std::slice::from_ref(&keystroke), &stats);
        let (in_review, _) = keymap
            .bindings_for_input(std::slice::from_ref(&keystroke), &review);
        let (in_input, _) = keymap
            .bindings_for_input(std::slice::from_ref(&keystroke), &typing);
        assert_eq!(on_stats.len(), 1);
        assert!(in_review.is_empty());
        assert!(in_input.is_empty());
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
    fn leaf_totals_keep_cards_owned_by_a_parent_deck() {
        let decks = [deck(1, "A", 1, 0, 1), deck(2, "A::B", 1, 0, 0)];
        let stats = [
            ranki_model::DeckStats {
                new_cards: 2,
                total_cards: 2,
                ..Default::default()
            },
            ranki_model::DeckStats {
                new_cards: 1,
                total_cards: 1,
                ..Default::default()
            },
        ];
        let deck_list = decks
            .iter()
            .map(|(deck, _)| deck.clone())
            .collect::<Vec<_>>();
        let total = super::leaf_deck_totals(&deck_list, &stats);
        assert_eq!(due_total(total), 2);
        assert_eq!(total.total_cards, 2);
    }

    #[test]
    fn explicit_parent_stats_are_not_added_to_children_twice() {
        let decks = [deck(1, "A", 1, 0, 0), deck(2, "A::B", 1, 0, 0)];
        let stats = [
            ranki_model::DeckStats {
                new_cards: 2,
                total_cards: 2,
                ..Default::default()
            },
            ranki_model::DeckStats {
                new_cards: 1,
                total_cards: 1,
                ..Default::default()
            },
        ];
        let rows = build_deck_rows(
            &decks
                .iter()
                .map(|(deck, _)| deck.clone())
                .collect::<Vec<_>>(),
            &stats,
            &std::collections::HashSet::default(),
        );
        assert_eq!(rows[0].stats.new_cards, 2);
        assert_eq!(rows[0].stats.total_cards, 2);
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
    fn forecast_bars_scale_to_the_busiest_day() {
        let days = [5, 2, 0, 3, 1, 0, 4];
        assert_eq!(forecast_total(&days), 15);
        assert_eq!(forecast_total(&[]), 0);
        assert_eq!(forecast_bar_height(0, 5), 6.0);
        assert_eq!(forecast_bar_height(2, 0), 6.0);
        assert_eq!(forecast_bar_height(1, 4), 28.5);
        assert_eq!(forecast_bar_height(4, 4), 60.0);
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
    fn occlusion_drag_creates_one_mask_on_release() {
        let mut state = OcclusionState {
            bounds: Some(gpui::Bounds {
                origin: point(px(100.), px(50.)),
                size: size(px(200.), px(100.)),
            }),
            ..OcclusionState::default()
        };
        let start = state.pointer((125., 75.)).expect("start point");
        let end = state.pointer((175., 125.)).expect("end point");
        state.drag = Some((start.0, start.1, end.0, end.1));
        let rect = state.drag_rect().expect("drag preview");
        assert_eq!((rect.left, rect.top), (0.125, 0.25));
        assert_eq!((rect.width, rect.height), (0.25, 0.5));
        state.finish_drag();
        assert_eq!(state.masks.len(), 1);
        assert_eq!(state.selected, Some(0));
        assert_eq!(state.drag, None);
    }

    #[test]
    fn occlusion_pointer_uses_the_fitted_image_area() {
        let state = OcclusionState {
            image_size: Some((200.0, 100.0)),
            bounds: Some(gpui::Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(400.), px(400.)),
            }),
            masks: vec![OcclusionMask {
                shape: MaskShape::Rect,
                rect: NormalizedRect {
                    left: 0.0,
                    top: 0.0,
                    width: 0.2,
                    height: 1.0,
                },
            }],
            ..OcclusionState::default()
        };
        let rect = state.image_rect().expect("fitted image");
        assert_eq!((rect.top, rect.height), (0.25, 0.5));
        assert_eq!(state.pointer((200.0, 100.0)), Some((0.5, 0.0)));
        assert_eq!(state.pointer((200.0, 300.0)), Some((0.5, 1.0)));
        let outside = state.raw_pointer((40.0, 50.0)).expect("raw point");
        assert!(outside.1 < 0.0);
        assert_eq!(
            state.mask_at(state.pointer((40.0, 50.0)).unwrap()),
            Some(0)
        );
        assert_eq!(state.mask_at(outside), None);
    }

    #[test]
    fn mask_hit_test_finds_the_newest_overlap_and_clears_with_the_image() {
        let mask = |left, top| OcclusionMask {
            shape: MaskShape::Rect,
            rect: NormalizedRect {
                left,
                top,
                width: 0.4,
                height: 0.4,
            },
        };
        let mut state = OcclusionState {
            image: Some("diagram.png".into()),
            masks: vec![mask(0.1, 0.1), mask(0.3, 0.3)],
            ..OcclusionState::default()
        };
        assert_eq!(state.mask_at((0.2, 0.2)), Some(0));
        assert_eq!(state.mask_at((0.35, 0.35)), Some(1));
        assert_eq!(state.mask_at((0.9, 0.9)), None);
        assert!(state.remove_mask_at((0.35, 0.35)));
        assert_eq!(state.masks.len(), 1);
        assert_eq!(state.selected, Some(0));
        assert!(!state.remove_mask_at((0.9, 0.9)));
        state.clear_image();
        assert_eq!(state.image, None);
        assert_eq!(state.masks.len(), 0);
        assert_eq!(state.selected, None);
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
            Some("Upload an image first")
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

    #[gpui::test]
    fn saving_an_occlusion_note_clears_the_chosen_image(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("collection.anki2");
        let store = Box::new(
            ranki_storage::Collection::create(&path).expect("collection"),
        );
        let options = Options {
            collection: path,
            ..Options::default()
        };
        let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store,
                    options,
                    save_settings,
                },
                window,
                cx,
            )
        });
        cx.update_entity(&root, |root, cx| {
            root.view = super::View::AddCard;
            let occlusion = root
                .models
                .iter()
                .position(ranki_model::NoteModel::is_image_occlusion)
                .expect("stock occlusion");
            root.select_model(occlusion, cx);
            root.add.occlusion.image = Some(String::from("diagram.png"));
            root.add.occlusion.masks = vec![crate::occlusion::default_mask(0)];
            root.add.occlusion.selected = Some(0);
            cx.notify();
        });
        assert!(
            cx.read_entity(&root, |root, _| root.add.occlusion.image.is_some())
        );

        cx.update_entity(&root, super::Root::save_note);

        let state = cx.read_entity(&root, |root, _| {
            (
                root.add.occlusion.image.clone(),
                root.add.occlusion.masks.len(),
                root.add.occlusion.selected,
            )
        });
        assert_eq!(state.0, None);
        assert_eq!(state.1, 0);
        assert_eq!(state.2, None);
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

    #[test]
    fn learn_ahead_pulls_learning_cards_within_twenty_minutes() {
        let now = 1_700_000_000;
        let card = |id, queue, due| Card {
            id,
            note_id: 0,
            deck_id: 0,
            ordinal: 0,
            modified: now,
            usn: -1,
            card_type: CardType::Learning,
            queue,
            due,
            interval: -600,
            factor: 2_500,
            reps: 1,
            lapses: 0,
            left: 1_001,
            original_due: 0,
            original_deck_id: 0,
            flags: 0,
            data: String::new(),
        };
        let mut pending = VecDeque::new();
        pending.push_back(card(
            1,
            CardQueue::Learning,
            (now + 10 * 60) * 1_000,
        ));
        pending.push_back(card(2, CardQueue::Review, (now + 60) * 1_000));
        pending.push_back(card(4, CardQueue::Learning, (now + 60) * 1_000));
        pending.push_back(card(
            3,
            CardQueue::Learning,
            (now + 21 * 60) * 1_000,
        ));
        let picked = super::Root::due_learning_card(
            &pending,
            now + super::LEARN_AHEAD_SECS,
        );
        assert_eq!(picked.map(|card| card.id), Some(4));
    }

    #[gpui::test]
    fn clicking_create_button_saves_the_deck(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("collection.anki2");
        let store = Box::new(
            ranki_storage::Collection::create(&path).expect("collection"),
        );
        let options = Options {
            collection: path,
            ..Options::default()
        };
        let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store,
                    options,
                    save_settings,
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
        cx.update_entity(&root, |root, cx| {
            root.create_modal = true;
            root.create_field.update(cx, |field, cx| {
                field.set_text("Click Deck", cx);
            });
            cx.notify();
        });
        let focus = cx.read_entity(&root, |root, cx| {
            root.create_field.read(cx).focus_handle().clone()
        });
        cx.update(|window, _| window.focus(&focus));
        cx.draw(
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::size(gpui::px(1280.), gpui::px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let button = cx
            .debug_bounds("create-deck-primary")
            .expect("create button bounds");
        cx.simulate_mouse_move(button.center(), None, gpui::Modifiers::none());
        cx.simulate_mouse_down(
            button.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_mouse_up(
            button.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        let names = cx.read_entity(&root, |root, _| {
            root.decks
                .iter()
                .map(|deck| deck.name.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(names, ["Click Deck"]);
    }
    #[gpui::test]
    fn clicking_a_deck_row_studies_that_deck(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("collection.anki2");
        let mut store =
            ranki_storage::Collection::create(&path).expect("collection");
        let cards = store
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![
                    String::from("Question?"),
                    String::from("Answer!"),
                ],
                tags: Vec::new(),
            })
            .expect("note");
        assert_eq!(cards.len(), 1);
        let options = Options {
            collection: path,
            ..Options::default()
        };
        let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(store),
                    options,
                    save_settings,
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(1280.), px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let row = cx
            .debug_bounds("deck-row-Default")
            .expect("deck row bounds");
        cx.simulate_click(row.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let state = cx.read_entity(&root, |root, _| {
            (
                root.view,
                root.selected,
                root.review.scope_all,
                root.review.session_total,
                root.review.current.is_some(),
            )
        });
        assert_eq!(state.0, View::Review);
        assert_eq!(state.1, Some(0));
        assert!(!state.2);
        assert_eq!(state.3, 1);
        assert!(state.4);
    }

    #[gpui::test]
    fn deck_row_controls_do_not_start_a_session(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("collection.anki2");
        let mut store =
            ranki_storage::Collection::create(&path).expect("collection");
        let _ = store.create_deck("Topic::Child").expect("deck");
        let _ = store
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![
                    String::from("Question?"),
                    String::from("Answer!"),
                ],
                tags: Vec::new(),
            })
            .expect("note");
        let options = Options {
            collection: path,
            ..Options::default()
        };
        let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(store),
                    options,
                    save_settings,
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(1280.), px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        let chevron = cx
            .debug_bounds("decks-chevron-Topic")
            .expect("chevron bounds");
        cx.simulate_click(chevron.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let (view, expanded) = cx.read_entity(&root, |root, _| {
            (root.view, root.expanded.contains("Topic"))
        });
        assert_eq!(view, View::Decks);
        assert!(expanded);

        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let menu = cx.debug_bounds("decks-menu-1").expect("menu bounds");
        cx.simulate_click(menu.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let (view, menu_open) = cx.read_entity(&root, |root, _| {
            (root.view, root.deck_menu.is_some())
        });
        assert_eq!(view, View::Decks);
        assert!(menu_open);
    }
    #[gpui::test]
    fn right_clicking_a_deck_opens_its_actions_menu(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("collection.anki2");
        let mut store =
            ranki_storage::Collection::create(&path).expect("collection");
        let cards = store
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![
                    String::from("Question?"),
                    String::from("Answer!"),
                ],
                tags: Vec::new(),
            })
            .expect("note");
        assert_eq!(cards.len(), 1);
        let options = Options {
            collection: path,
            ..Options::default()
        };
        let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(store),
                    options,
                    save_settings,
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(1280.), px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        let row = cx
            .debug_bounds("deck-row-Default")
            .expect("deck row bounds");
        cx.simulate_mouse_down(
            row.center(),
            gpui::MouseButton::Right,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_up(
            row.center(),
            gpui::MouseButton::Right,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        let state = cx.read_entity(&root, |root, _| {
            (root.view, root.deck_menu.map(|menu| menu.id))
        });
        assert_eq!(state.0, View::Decks);
        assert_eq!(state.1, Some(1));
        assert!(cx.debug_bounds("deck-actions-menu").is_some());
    }
    #[gpui::test]
    fn clicking_the_rename_field_keeps_the_decks_view(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("collection.anki2");
        let mut store =
            ranki_storage::Collection::create(&path).expect("collection");
        let _ = store
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![
                    String::from("Question?"),
                    String::from("Answer!"),
                ],
                tags: Vec::new(),
            })
            .expect("note");
        let options = Options {
            collection: path,
            ..Options::default()
        };
        let save_settings: crate::SettingsSaver = Arc::new(|_| Ok(()));
        let (root, cx) = cx.add_window_view(|window, cx| {
            super::Root::new(
                super::Launch {
                    store: Box::new(store),
                    options,
                    save_settings,
                },
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
        cx.update_entity(&root, |root, cx| {
            let deck = root.decks[0].clone();
            root.start_rename(&deck, cx);
        });
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(1280.), px(800.)),
            |_, _| root.clone(),
        );
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let field = cx
            .debug_bounds("deck-rename-field")
            .expect("rename field bounds");
        cx.simulate_click(field.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let (view, editing) = cx.read_entity(&root, |root, _| {
            (root.view, root.deck_edit.is_some())
        });
        assert_eq!(view, View::Decks);
        assert!(editing);
    }
}
