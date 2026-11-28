//! Unit tests for the mobile helpers and store-backed state transitions.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::time::Duration;

use ranki_model::CardQueue;
use ranki_model::Config;
use ranki_model::DataPaths;
use ranki_model::Id;
use ranki_model::NewNote;
use ranki_model::Note;
use ranki_model::Rating;
use ranki_model::RevLog;
use ranki_model::RevLogKind;
use ranki_model::Settings;
use ranki_model::StockModelKind;
use ranki_model::Store;
use ranki_model::ThemeChoice;
use ranki_model::Timing;
use ranki_model::UiState;
use ranki_model::UiView;
use ranki_model::config::parse;
use ranki_storage::Collection;
use ranki_storage::ReopeningCollection;
use tempfile::TempDir;

use crate::mobile::card::media_names;
use crate::mobile::card::plain;
use crate::mobile::card::sides;
use crate::mobile::card::sides_text;
use crate::mobile::platform;
use crate::mobile::state::AppState;
use crate::mobile::state::ReviewPhase;
use crate::mobile::state::Screen;
use crate::mobile::state::Statistics;
use crate::mobile::state::ToastKind;
use crate::mobile::state::browse_answer;
use crate::mobile::state::focused_delta;
use crate::mobile::state::summarize;

fn note(fields: &[&str], model_id: Id) -> Note {
    Note {
        id: 1,
        guid: String::new(),
        model_id,
        modified: 0,
        usn: 0,
        tags: Vec::new(),
        fields: fields.iter().map(|field| (*field).to_owned()).collect(),
        sort_field: String::new(),
        checksum: 0,
        flags: 0,
        data: String::new(),
    }
}

#[test]
fn basic_sides_render_field_references_as_plain_text() {
    let model = StockModelKind::Basic.definition();
    let card = note(&["<b>Question?</b>", "Answer &amp; more"], model.id);

    let (question, answer) = sides(&model, &card, 0);

    assert!(question.contains("Question?"));
    assert!(!question.contains("<b>"));
    assert!(answer.contains("Answer & more"));
}

#[test]
fn cloze_question_hides_the_answer_and_answer_reveals_it() {
    let model = StockModelKind::Cloze.definition();
    let card =
        note(&["The {{c1::capital}} is Paris.", "Remember it"], model.id);

    let (question, answer) = sides(&model, &card, 0);

    assert!(question.contains("_____"));
    assert!(!question.contains("capital"));
    assert!(answer.contains("capital"));
    assert!(answer.contains("Paris"));
    assert!(answer.contains("Remember it"));
}

#[test]
fn plain_strips_markup_but_keeps_line_breaks_and_entities() {
    assert_eq!(
        plain("<div>One<br>Two &amp; Three</div>"),
        "One\nTwo & Three"
    );
}

#[test]
fn plain_keeps_one_blank_line_between_paragraphs() {
    assert_eq!(plain("One<br><br><br>Two"), "One\n\nTwo");
}

#[test]
fn text_filter_uses_the_named_field_and_decodes_once() {
    let mut model = StockModelKind::Basic.definition();
    if let Some(template) = model.templates.first_mut() {
        template.question_format = String::from("{{text:Front}}");
    }
    let card = note(&["Use &lt;word&gt; here", "Answer"], model.id);

    let (question, _) = sides(&model, &card, 0);

    assert_eq!(question, "Use <word> here");
}

#[test]
fn media_names_accept_spaces_around_attribute_equals() {
    let names = media_names("<img src = \"diagram.png\">");
    assert!(names.iter().any(|name| name == "diagram.png"));
}

#[test]
fn media_names_finds_images_and_sounds() {
    let names = media_names("<img src=\"diagram.png\"> [sound:word.mp3]");

    assert!(names.iter().any(|name| name == "diagram.png"));
    assert!(names.iter().any(|name| name == "word.mp3"));
}

struct Fixture {
    dir: TempDir,
    state: AppState,
    deck_id: Id,
}

fn seeded_collection(dir: &Path) -> (PathBuf, Id) {
    let data = dir.join("data");
    fs::create_dir_all(data.join("ranki").join("collection.media"))
        .expect("data directory");

    let collection_path = DataPaths::new(data).collection;
    let mut collection =
        Collection::create(&collection_path).expect("new collection");
    let deck = collection.create_deck("Mobile Deck").expect("deck");
    let model = collection
        .ensure_model(StockModelKind::Basic)
        .expect("basic model");
    collection
        .add_note(&NewNote {
            model_id: model.id,
            deck_id: deck.id,
            fields: vec![
                String::from("Seed question"),
                String::from("Seed answer"),
            ],
            tags: Vec::new(),
        })
        .expect("seed note");
    collection.close().expect("close collection");
    (collection_path, deck.id)
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("temp directory");
    let (collection_path, deck_id) = seeded_collection(dir.path());
    let data = dir.path().join("data");
    let settings = Settings::resolve(&Config::default(), DataPaths::new(data));
    let state =
        AppState::new(ReopeningCollection::new(&collection_path), settings)
            .expect("app state");
    Fixture {
        dir,
        state,
        deck_id,
    }
}

fn file_fixture(config: &str, ui_state: &str) -> Fixture {
    let dir = tempfile::tempdir().expect("temp directory");
    let (collection_path, deck_id) = seeded_collection(dir.path());
    let config_dir = dir.path().join("config");
    fs::create_dir_all(&config_dir).expect("config directory");
    fs::write(config_dir.join("config.toml"), config).expect("config file");
    fs::write(config_dir.join("ui-state.toml"), ui_state)
        .expect("ui state file");
    let paths = DataPaths::new(dir.path().join("data"));
    let state = AppState::open(
        ReopeningCollection::new(&collection_path),
        paths,
        &config_dir,
    )
    .expect("app state");
    Fixture {
        dir,
        state,
        deck_id,
    }
}

#[test]
fn refresh_and_screen_selection_use_the_temporary_collection() {
    let mut fixture = fixture();

    assert!(fixture.state.decks.iter().any(|deck| {
        deck.name == "Mobile Deck"
            && deck.id == Some(fixture.deck_id)
            && deck.stats.total_cards == 1
    }));

    fixture
        .state
        .set_screen(Screen::Browse)
        .expect("browse screen");
    assert!(matches!(&fixture.state.screen, Screen::Browse));

    fixture.state.refresh().expect("refresh");
    assert!(fixture.state.decks.iter().any(|deck| {
        deck.name == "Mobile Deck" && deck.stats.new_cards == 1
    }));

    drop(fixture.dir);
}

#[test]
fn rating_and_undo_persist_and_reverse_a_review() {
    let mut fixture = fixture();

    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("start review");
    let card_id = fixture
        .state
        .review
        .current
        .as_ref()
        .map(|card| card.id)
        .expect("current card");
    assert!(matches!(&fixture.state.review.phase, ReviewPhase::Question));

    fixture.state.reveal();
    assert!(matches!(&fixture.state.review.phase, ReviewPhase::Answer));

    fixture.state.rate(Rating::Good).expect("rate card");
    let history = fixture.state.store.review_history().expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].card_id, card_id);

    let reviewed = fixture
        .state
        .store
        .card(card_id)
        .expect("load reviewed card")
        .expect("reviewed card exists");
    assert_ne!(reviewed.queue, CardQueue::New);

    fixture.state.undo().expect("undo review");
    assert_eq!(
        fixture
            .state
            .store
            .review_history()
            .expect("history after undo"),
        Vec::new()
    );
    let restored = fixture
        .state
        .store
        .card(card_id)
        .expect("load restored card")
        .expect("restored card exists");
    assert_eq!(restored.queue, CardQueue::New);

    drop(fixture.dir);
}

#[test]
fn rating_loads_the_next_card_on_the_question_side() {
    let mut fixture = fixture();
    let model_id = fixture
        .state
        .add
        .models
        .first()
        .map(|model| model.id)
        .expect("basic model");
    fixture
        .state
        .store
        .add_note(&NewNote {
            model_id,
            deck_id: fixture.deck_id,
            fields: vec![
                String::from("Second question"),
                String::from("Second answer"),
            ],
            tags: Vec::new(),
        })
        .expect("second note");
    fixture.state.refresh().expect("refresh");
    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("start review");

    fixture.state.reveal();
    fixture.state.rate(Rating::Good).expect("rate card");

    assert!(fixture.state.review.current.is_some());
    assert!(matches!(fixture.state.review.phase, ReviewPhase::Question));
    drop(fixture.dir);
}

#[test]
fn study_all_loads_cards_from_every_deck() {
    let mut fixture = fixture();
    let model_id = fixture
        .state
        .add
        .models
        .first()
        .map(|model| model.id)
        .expect("basic model");
    let other = fixture
        .state
        .store
        .create_deck("Other Deck")
        .expect("other deck");
    fixture
        .state
        .store
        .add_note(&NewNote {
            model_id,
            deck_id: other.id,
            fields: vec![
                String::from("Other question"),
                String::from("Other answer"),
            ],
            tags: Vec::new(),
        })
        .expect("other note");
    fixture.state.refresh().expect("refresh");

    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("study one deck");
    fixture.state.start_review(None).expect("study all");

    assert_eq!(fixture.state.selected_deck, None);
    let loaded = usize::from(fixture.state.review.current.is_some())
        + fixture.state.review.queue.len();
    assert_eq!(loaded, 2);
    drop(fixture.dir);
}

#[test]
fn pending_learning_card_waits_for_its_due_time() {
    let mut fixture = fixture();
    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("start review");
    let mut pending =
        fixture.state.review.current.clone().expect("current card");
    pending.queue = CardQueue::Learning;
    pending.due = i64::MAX;
    fixture.state.review.current = None;
    fixture.state.review.queue.clear();
    fixture.state.review.pending = vec![pending];

    fixture.state.take_next_card();

    assert!(fixture.state.review.current.is_none());
    assert_eq!(fixture.state.review.pending.len(), 1);
    drop(fixture.dir);
}

#[test]
fn undo_does_not_duplicate_a_restored_current_card() {
    let mut fixture = fixture();
    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("start review");
    let before = fixture.state.review.current.clone().expect("current card");
    fixture.state.reveal();
    fixture.state.rate(Rating::Again).expect("rate card");
    assert_eq!(
        fixture.state.store.review_history().expect("history").len(),
        1
    );
    fixture.state.review.queue.clear();
    fixture.state.review.current = Some(before);

    fixture.state.undo().expect("undo review");

    assert_eq!(
        fixture
            .state
            .store
            .review_history()
            .expect("history after undo"),
        Vec::new()
    );
    assert_eq!(fixture.state.review.queue, Vec::new());
    assert!(fixture.state.review.current.is_some());
    drop(fixture.dir);
}

#[test]
fn adding_a_note_makes_it_browsable() {
    let mut fixture = fixture();

    fixture.state.set_screen(Screen::Add).expect("add screen");
    let deck_index = fixture
        .state
        .add
        .decks
        .iter()
        .position(|deck| deck.id == fixture.deck_id)
        .expect("deck in add state");
    fixture.state.set_add_deck(deck_index);
    let model_index = fixture
        .state
        .add
        .models
        .iter()
        .position(|model| model.name == "Basic")
        .expect("basic model in add state");
    fixture.state.set_add_model(model_index);
    assert!(fixture.state.add.fields.len() >= 2);
    fixture
        .state
        .set_add_field(0, String::from("Second question"));
    fixture
        .state
        .set_add_field(1, String::from("Second answer"));

    fixture.state.save_note().expect("add note");
    fixture.state.refresh().expect("refresh after add");
    fixture
        .state
        .set_browse_query(String::from("Second"))
        .expect("browse query");

    assert!(fixture.state.browse.rows.iter().any(|row| {
        row.front.contains("Second question")
            && row.back == "Second answer"
            && row.deck == "Mobile Deck"
            && !row.back.contains('<')
    }));
    assert_eq!(fixture.state.browse.total, 1);
    assert_eq!(
        fixture
            .state
            .store
            .stats(Some(fixture.deck_id), 0)
            .expect("deck stats")
            .total_cards,
        2
    );

    drop(fixture.dir);
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

struct EnvGuard(Vec<(&'static str, Option<OsString>)>);

impl EnvGuard {
    #[expect(
        unsafe_code,
        reason = "path resolution reads process environment variables"
    )]
    fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
        let saved = vars
            .iter()
            .map(|(key, _)| (*key, env::var_os(key)))
            .collect::<Vec<_>>();
        for (key, new_value) in vars {
            match new_value {
                // SAFETY: the caller holds `ENV_LOCK` for the whole test.
                Some(text) => unsafe { env::set_var(key, text) },
                // SAFETY: the caller holds `ENV_LOCK` for the whole test.
                None => unsafe { env::remove_var(key) },
            }
        }
        Self(saved)
    }
}

impl Drop for EnvGuard {
    #[expect(
        unsafe_code,
        reason = "path resolution reads process environment variables"
    )]
    fn drop(&mut self) {
        for (key, old_value) in &self.0 {
            match old_value {
                // SAFETY: this guard drops before the lock guard declared with it.
                Some(text) => unsafe { env::set_var(key, text) },
                // SAFETY: this guard drops before the lock guard declared with it.
                None => unsafe { env::remove_var(key) },
            }
        }
    }
}

#[test]
fn take_next_card_preserves_the_current_phase() {
    let mut fixture = fixture();
    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("start review");
    let current = fixture.state.review.current.clone().expect("current card");
    fixture.state.review.queue = vec![current];
    fixture.state.review.current = None;
    fixture.state.review.phase = ReviewPhase::Answer;

    fixture.state.take_next_card();

    assert!(fixture.state.review.current.is_some());
    assert!(matches!(fixture.state.review.phase, ReviewPhase::Answer));
    drop(fixture.dir);
}

#[test]
fn pending_card_advances_when_due() {
    let mut fixture = fixture();
    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("start review");
    let mut pending =
        fixture.state.review.current.clone().expect("current card");
    pending.queue = CardQueue::Learning;
    pending.due = i64::MIN;
    fixture.state.review.current = None;
    fixture.state.review.queue.clear();
    fixture.state.review.pending = vec![pending];
    fixture.state.review.phase = ReviewPhase::Waiting;

    fixture.state.poll_pending();

    assert!(fixture.state.review.current.is_some());
    assert!(matches!(fixture.state.review.phase, ReviewPhase::Question));
    drop(fixture.dir);
}

#[test]
fn review_scope_is_kept_separate_from_the_deck_picker() {
    let mut fixture = fixture();
    fixture.state.start_review(None).expect("study all");
    assert_eq!(fixture.state.review_scope, None);
    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("study one deck");
    assert_eq!(fixture.state.review_scope, Some(fixture.deck_id));
    drop(fixture.dir);
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn explicit_directories_win_over_xdg_and_home() {
    let _guard = env_lock();
    let _env = EnvGuard::set(&[
        ("RANKI_DATA_DIR", Some("/ranki-explicit/data")),
        ("RANKI_CONFIG_DIR", Some("/ranki-explicit/config")),
        ("XDG_DATA_HOME", Some("/xdg/data")),
        ("XDG_CONFIG_HOME", Some("/xdg/config")),
        ("HOME", Some("/home/test")),
    ]);

    let (data, config) = platform::initial_paths();

    assert_eq!(data.as_deref(), Some(Path::new("/ranki-explicit/data")));
    assert_eq!(config.as_deref(), Some(Path::new("/ranki-explicit/config")));
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn xdg_directories_are_used_without_explicit_values() {
    let _guard = env_lock();
    let _env = EnvGuard::set(&[
        ("RANKI_DATA_DIR", None),
        ("RANKI_CONFIG_DIR", None),
        ("XDG_DATA_HOME", Some("/xdg/data")),
        ("XDG_CONFIG_HOME", Some("/xdg/config")),
        ("HOME", Some("/home/test")),
    ]);

    let (data, config) = platform::initial_paths();

    assert_eq!(data.as_deref(), Some(Path::new("/xdg/data/ranki")));
    assert_eq!(config.as_deref(), Some(Path::new("/xdg/config/ranki")));
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn home_directories_are_the_last_fallback() {
    let _guard = env_lock();
    let _env = EnvGuard::set(&[
        ("RANKI_DATA_DIR", None),
        ("RANKI_CONFIG_DIR", None),
        ("XDG_DATA_HOME", None),
        ("XDG_CONFIG_HOME", None),
        ("HOME", Some("/home/test")),
    ]);

    let (data, config) = platform::initial_paths();

    assert_eq!(
        data.as_deref(),
        Some(Path::new("/home/test/.local/share/ranki"))
    );
    assert_eq!(
        config.as_deref(),
        Some(Path::new("/home/test/.config/ranki"))
    );
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[test]
fn missing_directories_have_no_path() {
    let _guard = env_lock();
    let _env = EnvGuard::set(&[
        ("RANKI_DATA_DIR", None),
        ("RANKI_CONFIG_DIR", None),
        ("XDG_DATA_HOME", None),
        ("XDG_CONFIG_HOME", None),
        ("HOME", None),
    ]);

    let (data, config) = platform::initial_paths();

    assert_eq!(data, None);
    assert_eq!(config, None);
}

#[test]
fn sides_text_keeps_encoded_image_text() {
    let model = StockModelKind::Basic.definition();
    let note = note(
        &["&lt;img src=&quot;example.png&quot;&gt;", "Answer"],
        model.id,
    );
    let (question, _) = sides_text(&model, &note, 0);
    assert_eq!(question, "<img src=\"example.png\">");
}

#[test]
fn browse_answer_keeps_independent_answers() {
    assert_eq!(browse_answer("cat", "caterpillar", false), "caterpillar");
    assert_eq!(browse_answer("cat", "cat", false), "cat");
    assert_eq!(
        browse_answer("Question?", "Question?\nAnswer!", true),
        "Answer!"
    );
}

const START: i64 = 1_700_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

fn log(day: i64, rating: Rating, millis: u32) -> RevLog {
    RevLog {
        id: START
            .saturating_add(day.saturating_mul(SECONDS_PER_DAY))
            .saturating_add(3_600)
            .saturating_mul(1_000),
        card_id: 1,
        usn: 0,
        ease: rating.code(),
        interval: 1,
        last_interval: 0,
        factor: 2_500,
        time_ms: millis,
        kind: RevLogKind::Review,
    }
}

#[test]
fn statistics_start_empty() {
    let timing = Timing::v1(START, START);

    let stats = summarize(&[], &timing);

    assert_eq!(stats, Statistics::default());
    assert_eq!(stats.accuracy_percent(), None);
}

#[test]
fn statistics_count_today_time_accuracy_and_streak() {
    let now = START
        .saturating_add(SECONDS_PER_DAY.saturating_mul(2))
        .saturating_add(3_600);
    let timing = Timing::v1(now, START);
    let history = [
        log(0, Rating::Good, 1_000),
        log(1, Rating::Again, 500),
        log(2, Rating::Good, 2_000),
        log(2, Rating::Easy, 3_000),
    ];

    let stats = summarize(&history, &timing);

    assert_eq!(stats.reviews_today, 2);
    assert_eq!(stats.millis_today, 5_000);
    assert_eq!(stats.ratings_today, [0, 0, 1, 1]);
    assert_eq!(stats.accuracy_percent(), Some(100));
    assert_eq!(stats.streak, 3);
    assert_eq!(stats.forecast, Vec::<usize>::new());
}

#[test]
fn statistics_treat_manual_logs_as_scheduling_only() {
    let timing = Timing::v1(START, START);
    let mut manual = log(0, Rating::Again, 1_000);
    manual.kind = RevLogKind::Manual;

    let stats = summarize(&[manual], &timing);

    assert_eq!(stats.reviews_today, 0);
    assert_eq!(stats.accuracy_percent(), None);
    assert_eq!(stats.streak, 0);
}

#[test]
fn statistics_screen_loads_totals_and_forecast() {
    let mut fixture = fixture();
    fixture
        .state
        .set_screen(Screen::Statistics)
        .expect("statistics screen");
    assert_eq!(fixture.state.statistics.reviews_today, 0);
    assert_eq!(fixture.state.statistics.streak, 0);
    assert_eq!(fixture.state.statistics.accuracy_percent(), None);
    assert_eq!(fixture.state.statistics.forecast.len(), 7);

    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("start review");
    fixture.state.reveal();
    fixture.state.rate(Rating::Good).expect("rate card");
    fixture
        .state
        .set_screen(Screen::Statistics)
        .expect("statistics screen");

    assert_eq!(fixture.state.statistics.reviews_today, 1);
    assert_eq!(fixture.state.statistics.accuracy_percent(), Some(100));
    assert_eq!(fixture.state.statistics.streak, 1);
    assert_eq!(fixture.state.statistics.forecast.len(), 7);
    drop(fixture.dir);
}

#[test]
fn settings_and_ui_state_load_from_the_config_directory() {
    let fixture = file_fixture(
        "[ui]\ntheme = \"light\"\n",
        "view = \"browse\"\ndeck_query = \"fraction\"\nbrowse_query = \"tag:hard\"\nreview_all_decks = true\n",
    );

    assert_eq!(fixture.state.settings.theme, ThemeChoice::Light);
    assert_eq!(fixture.state.deck_query, "fraction");
    assert_eq!(fixture.state.browse.query, "tag:hard");
    assert_eq!(fixture.state.review_scope, None);
    drop(fixture.dir);
}

#[test]
fn theme_and_ui_state_are_saved_beside_the_config() {
    let mut fixture = file_fixture("", "");
    let config_dir = fixture.dir.path().join("config");

    fixture.state.set_theme(ThemeChoice::Dark);
    fixture.state.set_deck_query(String::from("fraction"));
    fixture
        .state
        .set_browse_query(String::from("tag:hard"))
        .expect("browse query");
    fixture
        .state
        .set_screen(Screen::Statistics)
        .expect("statistics screen");

    let text = fs::read_to_string(config_dir.join("ui-state.toml"))
        .expect("ui state file");
    let saved: UiState = toml::from_str(&text).expect("parsed ui state");
    assert_eq!(saved.theme, Some(ThemeChoice::Dark));
    assert_eq!(saved.view, UiView::Statistics);
    assert_eq!(saved.deck_query, "fraction");
    assert_eq!(saved.browse_query, "tag:hard");
    drop(fixture.dir);
}

#[test]
fn signing_out_updates_the_config_sync_identity() {
    let mut fixture =
        file_fixture("[sync]\nenabled = true\nusername = \"alice\"\n", "");
    let config_dir = fixture.dir.path().join("config");
    assert!(fixture.state.settings.sync.enabled);
    assert_eq!(
        fixture.state.settings.sync.username.as_deref(),
        Some("alice")
    );

    fixture.state.sign_out().expect("sign out");

    let text = fs::read_to_string(config_dir.join("config.toml"))
        .expect("config file");
    let saved = parse(&text);
    assert!(!saved.sync.enabled);
    assert_eq!(saved.sync.username, None);
    assert!(!text.contains("password"));
    drop(fixture.dir);
}

#[test]
fn saved_views_round_trip_through_the_ui_state() {
    for view in [
        UiView::Decks,
        UiView::Browse,
        UiView::Review,
        UiView::AddCard,
        UiView::Settings,
        UiView::Statistics,
    ] {
        assert_eq!(UiView::from(Screen::from(view)), view);
    }
}

#[test]
fn tapping_a_browse_row_edits_the_note_instead_of_adding_one() {
    let mut fixture = fixture();
    fixture
        .state
        .set_screen(Screen::Browse)
        .expect("browse screen");
    let row = fixture
        .state
        .browse
        .rows
        .first()
        .cloned()
        .expect("browse row");
    assert_eq!(row.front, "Seed question");

    fixture.state.start_edit(row.card_id).expect("start edit");
    fixture
        .state
        .start_edit(row.card_id)
        .expect("second edit is rejected");
    assert!(matches!(
        fixture.state.toast.as_ref().map(|toast| toast.kind),
        Some(ToastKind::Error)
    ));
    assert!(matches!(fixture.state.screen, Screen::Add));
    assert!(fixture.state.add.editing.is_some());
    assert_eq!(
        fixture.state.add.fields.first().map(String::as_str),
        Some("Seed question")
    );
    assert_eq!(
        fixture.state.add.fields.get(1).map(String::as_str),
        Some("Seed answer")
    );
    assert_eq!(
        fixture
            .state
            .add
            .decks
            .get(fixture.state.add.deck_index)
            .map(|deck| deck.id),
        Some(fixture.deck_id)
    );
    assert_eq!(
        fixture
            .state
            .add
            .models
            .get(fixture.state.add.model_index)
            .map(|model| model.name.as_str()),
        Some("Basic")
    );

    fixture
        .state
        .set_add_field(0, String::from("Edited question"));
    fixture.state.save_note().expect("save note");

    assert!(matches!(fixture.state.screen, Screen::Browse));
    assert!(fixture.state.add.editing.is_none());
    assert!(fixture.state.add.fields.iter().all(String::is_empty));
    assert!(
        fixture
            .state
            .browse
            .rows
            .iter()
            .any(|row| row.front == "Edited question"
                && row.back == "Seed answer")
    );
    assert_eq!(fixture.state.store.find_cards("").expect("cards").len(), 1);
    assert_eq!(
        fixture
            .state
            .store
            .stats(Some(fixture.deck_id), 0)
            .expect("deck stats")
            .total_cards,
        1
    );
    drop(fixture.dir);
}

#[test]
fn cancelling_an_edit_returns_to_browse_and_keeps_the_note() {
    let mut fixture = fixture();
    fixture
        .state
        .set_screen(Screen::Browse)
        .expect("browse screen");
    let row = fixture
        .state
        .browse
        .rows
        .first()
        .cloned()
        .expect("browse row");

    fixture.state.start_edit(row.card_id).expect("start edit");
    fixture.state.set_add_field(0, String::from("Discarded"));
    fixture.state.cancel_edit();

    assert!(matches!(fixture.state.screen, Screen::Browse));
    assert!(fixture.state.add.editing.is_none());
    assert!(fixture.state.add.fields.iter().all(String::is_empty));
    assert_eq!(fixture.state.add.tags, String::new());
    assert!(
        fixture
            .state
            .browse
            .rows
            .iter()
            .any(|row| row.front == "Seed question")
    );
    assert_eq!(fixture.state.store.find_cards("").expect("cards").len(), 1);
    drop(fixture.dir);
}

#[test]
fn decks_are_created_and_renamed_with_validated_names() {
    let mut fixture = fixture();
    fixture.state.deck_draft.name = String::from("  Study Deck  ");
    fixture.state.save_deck().expect("create deck");
    let id = fixture
        .state
        .decks
        .iter()
        .find(|row| row.name == "Study Deck")
        .and_then(|row| row.id)
        .expect("created deck id");

    fixture.state.start_deck_rename(id);
    assert_eq!(fixture.state.deck_draft.name, "Study Deck");
    fixture.state.deck_draft.name = String::from("Renamed Deck");
    fixture.state.save_deck().expect("rename deck");
    assert!(
        fixture
            .state
            .decks
            .iter()
            .any(|row| row.id == Some(id) && row.name == "Renamed Deck")
    );

    fixture.state.deck_draft.name = String::from("   ");
    fixture.state.save_deck().expect("empty name");
    assert!(matches!(
        fixture.state.toast.as_ref().map(|toast| toast.kind),
        Some(ToastKind::Error)
    ));
    assert!(
        fixture
            .state
            .decks
            .iter()
            .any(|row| row.id == Some(id) && row.name == "Renamed Deck")
    );
    drop(fixture.dir);
}

#[test]
fn removing_a_deck_needs_confirmation_and_deletes_its_cards() {
    let mut fixture = fixture();
    assert_eq!(fixture.state.store.find_cards("").expect("cards").len(), 1);

    fixture.state.request_remove_deck(fixture.deck_id);
    assert_eq!(
        fixture.state.deck_draft.confirm_remove,
        Some(fixture.deck_id)
    );
    assert!(
        fixture
            .state
            .decks
            .iter()
            .any(|row| row.id == Some(fixture.deck_id))
    );

    fixture.state.cancel_remove_deck();
    assert!(fixture.state.deck_draft.confirm_remove.is_none());
    assert_eq!(fixture.state.store.find_cards("").expect("cards").len(), 1);

    fixture.state.request_remove_deck(fixture.deck_id);
    fixture.state.confirm_remove_deck().expect("remove deck");
    assert!(fixture.state.deck_draft.confirm_remove.is_none());
    assert!(
        !fixture
            .state
            .decks
            .iter()
            .any(|row| row.id == Some(fixture.deck_id))
    );
    assert_eq!(fixture.state.store.find_cards("").expect("cards").len(), 0);
    assert_eq!(fixture.state.browse.rows, Vec::new());
    drop(fixture.dir);
}

#[test]
fn rating_records_the_focused_answer_time_and_undo_restores_it() {
    let mut fixture = fixture();
    fixture
        .state
        .start_review(Some(fixture.deck_id))
        .expect("start review");
    fixture.state.reveal();
    fixture.state.review.focused = Duration::from_secs(7);
    fixture.state.rate(Rating::Good).expect("rate card");

    let history = fixture.state.store.review_history().expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].time_ms, 7_000);
    assert_eq!(fixture.state.review.focused, Duration::ZERO);
    assert_eq!(
        fixture.state.review.last.as_ref().map(|last| last.spent),
        Some(Duration::from_secs(7))
    );

    fixture.state.undo().expect("undo review");
    assert_eq!(fixture.state.review.focused, Duration::from_secs(7));
    drop(fixture.dir);
}

#[test]
fn focused_answer_time_freezes_when_the_screen_is_not_focused() {
    let base = Duration::from_secs(3);
    assert_eq!(
        focused_delta(Duration::from_secs(2), true, base),
        Duration::from_secs(5)
    );
    assert_eq!(focused_delta(Duration::from_secs(2), false, base), base);
    assert_eq!(
        focused_delta(Duration::from_secs(3_600), true, base),
        Duration::from_secs(60)
    );
}

#[test]
fn image_occlusion_notes_need_the_desktop_editor() {
    let mut fixture = fixture();
    let model = fixture
        .state
        .store
        .ensure_model(StockModelKind::ImageOcclusion)
        .expect("image occlusion model");
    fixture.state.refresh().expect("refresh");
    let index = fixture
        .state
        .add
        .models
        .iter()
        .position(|candidate| candidate.id == model.id)
        .expect("image occlusion model in add state");
    fixture.state.set_add_model(index);
    let before = fixture.state.store.find_cards("").expect("cards").len();

    fixture.state.save_note().expect("save note");

    assert!(matches!(
        fixture.state.toast.as_ref().map(|toast| toast.kind),
        Some(ToastKind::Error)
    ));
    assert_eq!(
        fixture
            .state
            .store
            .find_cards("")
            .expect("cards after save")
            .len(),
        before
    );
    drop(fixture.dir);
}
