//! Unit tests for the mobile helpers and store-backed state transitions.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use ranki_model::CardQueue;
use ranki_model::Config;
use ranki_model::DataPaths;
use ranki_model::Id;
use ranki_model::NewNote;
use ranki_model::Note;
use ranki_model::Rating;
use ranki_model::Settings;
use ranki_model::StockModelKind;
use ranki_model::Store;
use ranki_storage::Collection;
use ranki_storage::ReopeningCollection;
use tempfile::TempDir;

use crate::card::media_names;
use crate::card::plain;
use crate::card::sides;
use crate::platform;
use crate::state::AppState;
use crate::state::ReviewPhase;
use crate::state::Screen;

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

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("temp directory");
    let data = dir.path().join("data");
    fs::create_dir_all(data.join("ranki").join("collection.media"))
        .expect("data directory");

    let collection_path = DataPaths::new(data.clone()).collection;
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

    let settings = Settings::resolve(&Config::default(), DataPaths::new(data));
    let state =
        AppState::new(ReopeningCollection::new(&collection_path), settings)
            .expect("app state");

    Fixture {
        dir,
        state,
        deck_id: deck.id,
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

    fixture.state.add_note().expect("add note");
    fixture.state.refresh().expect("refresh after add");
    fixture
        .state
        .set_browse_query(String::from("Second"))
        .expect("browse query");

    assert!(fixture.state.browse.rows.iter().any(|row| {
        row.front.contains("Second question") && row.deck == "Mobile Deck"
    }));
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
