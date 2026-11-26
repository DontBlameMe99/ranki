#![expect(clippy::expect_used, reason = "test setup panics on failure")]
//! Integration tests for the Anki-compatible collection store.

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use ranki_core::FsrsScheduler;
use ranki_model::CardQueue;
use ranki_model::CardType;
use ranki_model::DeckStats;
use ranki_model::NewNote;
use ranki_model::Rating;
use ranki_model::RevLog;
use ranki_model::RevLogKind;
use ranki_model::ReviewDecision;
use ranki_model::Store;
use ranki_model::StoreError;
use ranki_storage::Collection;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use serde_json::Value;
use sha1::Digest;
use sha1::Sha1;
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("collection.anki2");
    Collection::create(&path).expect("create");
    (directory, path)
}

fn read_only(path: &Path) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("read only")
}

fn now_secs() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock");
    i64::try_from(elapsed.as_secs()).expect("timestamp")
}

fn note(fields: &[&str], tags: &[&str]) -> NewNote {
    NewNote {
        model_id: 1,
        deck_id: 1,
        fields: fields.iter().map(|value| (*value).into()).collect(),
        tags: tags.iter().map(|value| (*value).into()).collect(),
    }
}

#[test]
fn due_query_uses_the_queue_index_for_learning_cards() {
    let (_directory, path) = fixture();
    let store = Collection::open(&path).expect("open");
    let _ = store.due_cards(None, 1, now_secs()).expect("due");
    let conn = read_only(&path);
    let plan: Vec<String> = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT id FROM cards WHERE queue = ?1 AND due <= ?2 \
             ORDER BY due, id LIMIT ?3",
        )
        .expect("prepare")
        .query_map([1_i64, i64::MAX, 20_i64], |row| row.get(3))
        .expect("query")
        .map(|row| row.expect("row"))
        .collect();
    assert!(
        plan.iter()
            .any(|step| step.contains("idx_ranki_cards_queue_due")),
        "{plan:?}"
    );
}
#[test]
fn create_writes_anki_schema_and_default_json() {
    let (_directory, path) = fixture();
    let conn = read_only(&path);
    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
        )
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .map(|row| row.expect("row"))
        .collect();
    for table in ["cards", "col", "graves", "notes", "revlog"] {
        assert!(tables.iter().any(|name| name == table), "missing {table}");
    }
    let indexes: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index'")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .map(|row| row.expect("row"))
        .collect();
    assert!(indexes.contains(&"ix_cards_sched".to_owned()));
    assert!(indexes.contains(&"ix_notes_csum".to_owned()));
    let (configuration, models, decks, dconf, tags): (
        String,
        String,
        String,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT conf, models, decks, dconf, tags FROM col WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("col row");
    for value in [&configuration, &models, &decks, &dconf, &tags] {
        let parsed: Value = serde_json::from_str(value).expect("valid JSON");
        assert!(parsed.is_object());
    }
    let model_json: Value = serde_json::from_str(&models).expect("models");
    assert_eq!(model_json["1"]["name"], "Basic");
    assert_eq!(model_json["1"]["flds"][0]["name"], "Front");
    assert_eq!(model_json["1"]["tmpls"][0]["qfmt"], "{{Front}}");
    let deck_json: Value = serde_json::from_str(&decks).expect("decks");
    assert_eq!(deck_json["1"]["name"], "Default");
}

#[test]
fn created_collection_declares_its_legacy_schema_version() {
    let (_directory, path) = fixture();
    let conn = read_only(&path);
    let version: i64 = conn
        .query_row("SELECT ver FROM col WHERE id = 1", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 11);
    let has_modern_tables: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'notetypes')",
            [],
            |row| row.get(0),
        )
        .expect("modern tables");
    assert!(!has_modern_tables);
}

#[test]
fn review_and_day_learning_dues_use_days_since_collection_creation() {
    let (_directory, path) = fixture();
    let created = 1_685_412_000_i64;
    let now = created.saturating_add(86_400 * 2);
    let today = 2_i64;
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute("UPDATE col SET crt = ?1 WHERE id = 1", [created])
            .expect("set crt");
        let rows = [
            (21_i64, 2_i64, today),
            (22, 2, today.saturating_add(1)),
            (23, 3, today),
            (24, 3, today.saturating_add(1)),
            (25, 1, now),
            (26, 1, now.saturating_add(1)),
        ];
        for (id, queue, due) in rows {
            conn.execute(
                "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, \
                 reps, lapses, left, odue, odid, flags, data) VALUES (?1, 1, 1, 0, 0, 0, ?2, ?3, \
                 ?4, 1, 0, 0, 0, 0, 0, 0, 0, '')",
                [id, 2_i64, queue, due],
            )
            .expect("insert card");
        }
    }
    let store = Collection::open(&path).expect("open");
    assert_eq!(store.collection_created().expect("created"), created);
    let due_ids: Vec<_> = store
        .due_cards(None, 50, now)
        .expect("due")
        .into_iter()
        .map(|card| card.id)
        .collect();
    assert_eq!(due_ids, [25, 23, 21]);
    assert!(!due_ids.contains(&22));
    assert!(!due_ids.contains(&24));

    let card = store.card(21).expect("card").expect("exists");
    let decision = FsrsScheduler::default().schedule(
        &card,
        Rating::Good,
        now,
        store.collection_created().expect("created"),
    );
    let expected = today.saturating_add(i64::from(decision.card.interval));
    assert_eq!(decision.card.due, expected);
}

#[test]
fn due_forecast_buckets_current_overdue_learning_and_future_cards() {
    let (_directory, path) = fixture();
    let created = 1_685_412_000_i64;
    let now = created.saturating_add(172_800);
    let day_secs = 86_400_i64;
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute("UPDATE col SET crt = ?1 WHERE id = 1", [created])
            .expect("set crt");
        let rows = [
            (31_i64, 2_i64, 1_i64),
            (32, 2, 2),
            (33, 3, 2),
            (34, 1, now),
            (35, 1, created.saturating_add(day_secs.saturating_mul(5))),
            (36, 2, 3),
            (37, 2, 3),
            (38, 3, 4),
            (39, 2, 8),
            (40, 2, 9),
            (41, 0, 3),
            (42, -1, 2),
        ];
        for (id, queue, due) in rows {
            conn.execute(
                "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, \
                 reps, lapses, left, odue, odid, flags, data) VALUES (?1, 1, 1, 0, 0, 0, 2, ?2, \
                 ?3, 1, 0, 0, 0, 0, 0, 0, 0, '')",
                [id, queue, due],
            )
            .expect("insert card");
        }
    }
    let store = Collection::open(&path).expect("open");
    assert_eq!(
        store.due_forecast(7, now).expect("forecast"),
        [4, 2, 1, 0, 0, 0, 1]
    );
    assert_eq!(
        store.due_forecast(0, now).expect("empty forecast"),
        Vec::<usize>::new()
    );
}

#[test]
fn create_rejects_existing_collection_and_open_rejects_missing_file() {
    let (_directory, path) = fixture();
    assert!(matches!(
        Collection::create(&path),
        Err(StoreError::InvalidData(_))
    ));
    let missing = path.with_file_name("missing.anki2");
    assert!(matches!(
        Collection::open(&missing),
        Err(StoreError::Backend(_))
    ));
}

#[test]
fn add_note_stores_anki_fields_tags_sort_field_and_checksum() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let ids = store
        .add_note(&note(&["<b>hello</b>", "world"], &["one", "two"]))
        .expect("add note");
    let card = store.card(ids[0]).expect("card").expect("card exists");
    let saved = store
        .note(card.note_id)
        .expect("note")
        .expect("note exists");
    assert_eq!(saved.fields, ["<b>hello</b>", "world"]);
    assert_eq!(saved.tags, ["one", "two"]);
    assert_eq!(saved.sort_field, "hello");
    let digest = Sha1::digest(b"hello");
    let unsigned =
        u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    let expected = if unsigned > 2_147_483_647 {
        i32::try_from(
            i64::from(unsigned)
                .saturating_sub(i64::from(u32::MAX))
                .saturating_sub(1),
        )
        .expect("checksum")
    } else {
        i32::try_from(unsigned).expect("checksum")
    };
    assert_eq!(saved.checksum, expected);
    let other_ids = store
        .add_note(&note(&["other", "world"], &[]))
        .expect("add");
    let other_card = store.card(other_ids[0]).expect("card").expect("exists");
    let other = store
        .note(other_card.note_id)
        .expect("note")
        .expect("exists");
    assert_ne!(saved.guid, other.guid);
    assert!(!saved.guid.is_empty() && !other.guid.is_empty());
    assert!(saved.guid.is_ascii());
}

#[test]
fn add_note_creates_one_card_per_template() {
    let (_directory, path) = fixture();
    {
        let conn = Connection::open(&path).expect("open");
        let models: String = conn
            .query_row("SELECT models FROM col WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("models");
        let mut model_json: Value =
            serde_json::from_str(&models).expect("parse");
        let template = model_json["1"]["tmpls"][0].clone();
        let templates =
            model_json["1"]["tmpls"].as_array_mut().expect("templates");
        templates.push(template);
        conn.execute(
            "UPDATE col SET models = ?1 WHERE id = 1",
            [serde_json::to_string(&model_json).expect("serialize")],
        )
        .expect("update");
    }
    let mut store = Collection::open(&path).expect("open");
    assert_eq!(store.models().expect("models")[0].templates.len(), 2);
    let ids = store.add_note(&note(&["front", "back"], &[])).expect("add");
    assert_eq!(ids.len(), 2);
    let cards: Vec<_> = ids
        .iter()
        .map(|id| store.card(*id).expect("card").expect("exists"))
        .collect();
    assert_eq!(cards[0].ordinal, 0);
    assert_eq!(cards[1].ordinal, 1);
    assert_eq!(cards[0].note_id, cards[1].note_id);
    assert_ne!(cards[0].due, cards[1].due);
}

#[test]
fn create_deck_builds_hierarchy_and_filters_subdecks() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let parent = store.create_deck("Study").expect("parent");
    let child = store.create_deck("Study::Languages").expect("child");
    let names: Vec<_> = store
        .decks()
        .expect("decks")
        .into_iter()
        .map(|deck| deck.name)
        .collect();
    assert!(names.contains(&"Study".to_owned()));
    assert!(names.contains(&"Study::Languages".to_owned()));
    let mut new_note = note(&["bonjour", "hello"], &[]);
    new_note.deck_id = child.id;
    let ids = store.add_note(&new_note).expect("add");
    let parent_cards = store
        .due_cards(Some(parent.id), 10, now_secs())
        .expect("due");
    assert!(parent_cards.iter().any(|card| card.id == ids[0]));
    let stats = store.stats(Some(parent.id), now_secs()).expect("stats");
    assert_eq!(stats.new_cards, 1);
    let found = store.find_cards("deck:Study").expect("find");
    assert!(found.contains(&ids[0]));
    assert!(store.create_deck("Study").is_err());
    assert!(store.create_deck("Bad::").is_err());
}

#[test]
fn default_deck_is_hidden_only_when_empty_and_childless() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");

    let names: Vec<_> = store
        .decks()
        .expect("decks")
        .into_iter()
        .map(|deck| deck.name)
        .collect();
    assert_eq!(names, ["Default".to_owned()]);

    store.create_deck("Flags").expect("flags");
    let names: Vec<_> = store
        .decks()
        .expect("decks")
        .into_iter()
        .map(|deck| deck.name)
        .collect();
    assert_eq!(names, ["Flags".to_owned()]);

    store
        .add_note(&note(&["front", "back"], &[]))
        .expect("note");
    let names: Vec<_> = store
        .decks()
        .expect("decks")
        .into_iter()
        .map(|deck| deck.name)
        .collect();
    assert!(names.contains(&"Default".to_owned()));

    let (_child_directory, child_path) = fixture();
    let mut child_store = Collection::open(&child_path).expect("open");
    child_store.create_deck("Flags").expect("flags");
    child_store.create_deck("Default::Kids").expect("child");
    let names: Vec<_> = child_store
        .decks()
        .expect("decks")
        .into_iter()
        .map(|deck| deck.name)
        .collect();
    assert!(names.contains(&"Default".to_owned()));
}

#[test]
fn due_cards_respects_queue_and_time_windows() {
    let (_directory, path) = fixture();
    let now = now_secs();
    let today = 0_i64;
    let conn = Connection::open(&path).expect("open");
    conn.execute("UPDATE col SET crt = ?1 WHERE id = 1", [now])
        .expect("set crt");
    let rows = [
        (10_i64, 0_i64, 7_i64),
        (11, 1, now),
        (12, 1, now.saturating_add(60)),
        (13, 2, today),
        (14, 2, today.saturating_add(1)),
        (15, 3, today),
        (16, 3, today.saturating_add(1)),
    ];
    for (id, queue, due) in rows {
        conn.execute(
            "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, \
             lapses, left, odue, odid, flags, data) VALUES (?1, 1, 1, 0, 0, 0, 0, ?2, ?3, 0, 0, \
             0, 0, 0, 0, 0, 0, '')",
            [id, queue, due],
        )
        .expect("insert card");
    }
    let store = Collection::open(&path).expect("open");
    let due_ids: Vec<_> = store
        .due_cards(None, 50, now)
        .expect("due")
        .into_iter()
        .map(|card| card.id)
        .collect();
    assert_eq!(due_ids, [11, 15, 13, 10]);
    let limited = store.due_cards(None, 2, now).expect("due");
    assert_eq!(limited.len(), 2);
    assert_eq!(
        store.due_cards(None, 0, now).expect("due"),
        [] as [ranki_model::Card; 0]
    );
}

#[test]
fn stats_count_only_cards_due_at_the_given_time() {
    let (_directory, path) = fixture();
    let now = now_secs();
    let conn = Connection::open(&path).expect("open");
    conn.execute("UPDATE col SET crt = ?1 WHERE id = 1", [now])
        .expect("set crt");
    let rows = [
        (10_i64, 0_i64, 7_i64),
        (11, 1, now),
        (12, 1, now.saturating_add(60)),
        (13, 2, 0),
        (14, 2, 1),
        (15, 3, 0),
        (16, 3, 1),
    ];
    for (id, queue, due) in rows {
        conn.execute(
            "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, \
             lapses, left, odue, odid, flags, data) VALUES (?1, 1, 1, 0, 0, 0, 0, ?2, ?3, 0, 0, \
             0, 0, 0, 0, 0, 0, '')",
            [id, queue, due],
        )
        .expect("insert card");
    }
    let store = Collection::open(&path).expect("open");
    let stats = store.stats(None, now).expect("stats");
    assert_eq!(stats.new_cards, 1);
    assert_eq!(stats.learning_cards, 2);
    assert_eq!(stats.review_cards, 1);
    assert_eq!(stats.total_cards, 7);

    let batch = store.deck_stats(now).expect("deck stats");
    assert_eq!(
        batch
            .into_iter()
            .find(|(id, _)| *id == 1)
            .map(|(_, stats)| stats),
        Some(stats)
    );
}

#[test]
fn find_cards_matches_text_wildcards_and_state() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["alpha question", "answer"], &[]))
        .expect("add");
    store
        .add_note(&note(&["beta question", "answer"], &[]))
        .expect("add");
    assert_eq!(store.find_cards("alpha").expect("find").len(), 1);
    assert_eq!(store.find_cards("question").expect("find").len(), 2);
    assert_eq!(store.find_cards("alpha*").expect("find").len(), 1);
    assert_eq!(store.find_cards("QUESTION").expect("find").len(), 2);
    assert_eq!(store.find_cards("ALPHA").expect("find").len(), 1);
    assert_eq!(store.find_cards("is:new").expect("find").len(), 2);
    assert_eq!(store.find_cards("is:review").expect("find"), [] as [i64; 0]);
    assert!(store.find_cards("is:unknown").is_err());
    let quoted = store.find_cards("\"alpha question\"").expect("find");
    assert_eq!(quoted.len(), 1);
}

#[test]
fn find_cards_matches_deck_tag_and_model() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let deck = store.create_deck("Languages::French").expect("deck");
    let mut new_note = note(&["bonjour", "hello"], &["french"]);
    new_note.deck_id = deck.id;
    let ids = store.add_note(&new_note).expect("add");
    assert_eq!(store.find_cards("tag:french").expect("find"), ids);
    assert_eq!(store.find_cards("tag:FRENCH").expect("find"), ids);
    assert_eq!(store.find_cards("note:Basic").expect("find"), ids);
    assert_eq!(
        store.find_cards("tag:english").expect("find"),
        [] as [i64; 0]
    );
    assert_eq!(store.find_cards("deck:Languages").expect("find"), ids);
    assert_eq!(
        store
            .find_cards("deck:Languages::French tag:french")
            .expect("find"),
        ids
    );
}

#[test]
fn find_cards_does_not_interpret_sql_injection() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["secret", "answer"], &["safe"]))
        .expect("add");
    for query in [
        "' OR 1=1 --",
        "deck:Default' OR 1=1 --",
        "tag:safe' OR '1'='1",
    ] {
        assert!(
            store.find_cards(query).expect("no injection").is_empty(),
            "{query}"
        );
    }
    assert_eq!(store.find_cards("secret").expect("normal search").len(), 1);
    assert_eq!(store.find_cards("").expect("empty query").len(), 1);
}

#[test]
fn apply_review_updates_card_log_and_collection_metadata() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let ids = store
        .add_note(&note(&["review me", "answer"], &[]))
        .expect("add");
    let card = store.card(ids[0]).expect("card").expect("exists");
    let before = store.collection_modified().expect("modified");
    let mut changed = card.clone();
    changed.queue = CardQueue::Review;
    changed.card_type = CardType::Review;
    changed.due = 42;
    changed.interval = 4;
    let decision = ReviewDecision {
        card: changed,
        expected_modified: card.modified,
        expected_usn: card.usn,
        revlog: RevLog {
            id: 9_000,
            card_id: card.id,
            usn: 0,
            ease: Rating::Good,
            interval: 4,
            last_interval: 0,
            factor: 2500,
            time_ms: 500,
            kind: RevLogKind::Review,
        },
    };
    store.apply_review(&decision).expect("apply");
    let saved = store.card(card.id).expect("card").expect("exists");
    assert_eq!(saved.queue, CardQueue::Review);
    assert_eq!(saved.due, 42);
    let log = store.revlog(9_000).expect("revlog").expect("entry");
    assert_eq!(log.card_id, card.id);
    assert_eq!(log.ease, Rating::Good);
    assert!(store.collection_modified().expect("modified") > before);
    store.apply_review(&decision).expect("second review");
    assert_eq!(
        store.review_history().expect("history").len(),
        2,
        "review ids are uniquified"
    );
}

#[test]
fn undo_review_restores_card_and_deletes_log() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["undo me", "answer"], &[]))
        .expect("add");
    let now = now_secs();
    let before = store
        .due_cards(None, 1, now)
        .expect("due")
        .pop()
        .expect("due card");
    let decision = FsrsScheduler::default().schedule(
        &before,
        Rating::Good,
        now,
        store.collection_created().expect("created"),
    );
    store.apply_review(&decision).expect("apply");
    assert_ne!(
        store.card(before.id).expect("card").expect("exists"),
        before
    );
    assert_eq!(store.review_history().expect("history").len(), 1);

    assert!(
        store
            .undo_review(&before, decision.revlog.id)
            .expect("undo")
    );
    let mut restored = store.card(before.id).expect("card").expect("exists");
    assert_eq!(restored.usn, -1, "local undo stays pending sync");
    restored.usn = before.usn;
    assert_eq!(restored, before);
    assert_eq!(
        store.review_history().expect("history"),
        [] as [ranki_model::RevLog; 0]
    );

    let unchanged = store.card(before.id).expect("card").expect("exists");
    assert!(
        !store
            .undo_review(&before, decision.revlog.id)
            .expect("retry")
    );
    assert_eq!(
        store.card(before.id).expect("card").expect("exists"),
        unchanged
    );
}

#[test]
fn grade_undo_grade_uses_the_restored_revision() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["again", "answer"], &[]))
        .expect("add");
    let now = now_secs();
    let before = store
        .due_cards(None, 1, now)
        .expect("due")
        .pop()
        .expect("card");
    let scheduler = FsrsScheduler::default();
    let decision = scheduler.schedule(
        &before,
        Rating::Good,
        now,
        store.collection_created().expect("created"),
    );
    store.apply_review(&decision).expect("grade");
    assert!(
        store
            .undo_review(&before, decision.revlog.id)
            .expect("undo")
    );
    let restored = store.card(before.id).expect("card").expect("exists");
    let retry = scheduler.schedule(
        &restored,
        Rating::Good,
        now,
        store.collection_created().expect("created"),
    );
    store.apply_review(&retry).expect("grade after undo");
    assert_eq!(store.review_history().expect("history").len(), 1);
}

#[test]
fn stale_review_writes_are_rejected() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["stale", "answer"], &[]))
        .expect("add");
    let now = now_secs();
    let card = store
        .due_cards(None, 1, now)
        .expect("due")
        .pop()
        .expect("card");
    let decision = FsrsScheduler::default().schedule(
        &card,
        Rating::Good,
        now,
        store.collection_created().expect("created"),
    );
    let conn = Connection::open(&path).expect("open");
    conn.execute("UPDATE cards SET mod = mod + 10 WHERE id = ?1", [card.id])
        .expect("external write");
    drop(conn);
    assert!(store.apply_review(&decision).is_err());
    assert_eq!(
        store.review_history().expect("history"),
        [] as [ranki_model::RevLog; 0]
    );
}

#[test]
fn repeated_review_gets_a_unique_log_id() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["race", "answer"], &[]))
        .expect("add");
    let card = store
        .due_cards(None, 1, now_secs())
        .expect("due")
        .pop()
        .expect("card");
    let now = card.modified;
    let scheduler = FsrsScheduler::default();
    let first = scheduler.schedule(
        &card,
        Rating::Good,
        now,
        store.collection_created().expect("created"),
    );
    let second = first.clone();
    store.apply_review(&first).expect("first write");
    store.apply_review(&second).expect("second write");
    assert_eq!(store.review_history().expect("history").len(), 2);
}

#[test]
fn review_of_an_older_card_is_accepted() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["older", "answer"], &[]))
        .expect("add");
    let now = now_secs();
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute("UPDATE cards SET mod = ?1", [now.saturating_sub(3600)])
            .expect("age card");
    }
    let card = store
        .due_cards(None, 1, now)
        .expect("due")
        .pop()
        .expect("card");
    let decision = FsrsScheduler::default().schedule(
        &card,
        Rating::Good,
        now,
        store.collection_created().expect("created"),
    );
    store.apply_review(&decision).expect("apply older card");
    assert_eq!(store.review_history().expect("history").len(), 1);
}

#[test]
fn stats_excludes_future_day_learning_cards() {
    let (_directory, path) = fixture();
    let created = 1_685_412_000_i64;
    let now = created.saturating_add(86_400);
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute("UPDATE col SET crt = ?1 WHERE id = 1", [created])
            .expect("set crt");
        for (id, due) in [(31_i64, 1_i64), (32, 2)] {
            conn.execute(
                "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, \
                 reps, lapses, left, odue, odid, flags, data) VALUES (?1, 1, 1, 0, 0, 0, 2, 3, \
                 ?2, 1, 0, 0, 0, 0, 0, 0, 0, '')",
                [id, due],
            )
            .expect("insert");
        }
    }
    let store = Collection::open(&path).expect("open");
    let stats = store.stats(None, now).expect("stats");
    assert_eq!(stats.total_cards, 2);
    assert_eq!(stats.learning_cards, 1);
}

#[test]
fn due_stats_count_only_cards_due_now() {
    let (_directory, path) = fixture();
    let created = 1_685_412_000_i64;
    let now = created.saturating_add(86_400);
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute("UPDATE col SET crt = ?1 WHERE id = 1", [created])
            .expect("set crt");
        for (id, queue, due) in [
            (21_i64, 0_i64, 0_i64),
            (22, 1, now),
            (23, 3, 1_i64),
            (24, 2, 2_i64),
            (25, 2, 1_i64),
        ] {
            conn.execute(
                "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, \
                 reps, lapses, left, odue, odid, flags, data) VALUES (?1, 1, 1, 0, 0, 0, ?2, ?3, \
                 ?4, 1, 0, 0, 0, 0, 0, 0, 0, '')",
                [id, 2_i64, queue, due],
            )
            .expect("insert");
        }
    }
    let store = Collection::open(&path).expect("open");
    let all = store.stats(None, now).expect("stats");
    let due = store
        .deck_stats(now)
        .expect("deck stats")
        .into_iter()
        .find(|(id, _)| *id == 1)
        .map(|(_, stats)| stats)
        .unwrap_or_default();
    assert_eq!(all.total_cards, 5);
    assert_eq!(all.new_cards, 1);
    assert_eq!(due.new_cards, 1);
    assert_eq!(due.learning_cards, 2);
    assert_eq!(due.review_cards, 1);
    assert_eq!(
        due.new_cards
            .saturating_add(due.learning_cards)
            .saturating_add(due.review_cards),
        4
    );
}

#[test]
fn stats_and_collection_modified_track_writes() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let before = store.collection_modified().expect("modified");
    store.add_note(&note(&["one", "answer"], &[])).expect("add");
    let stats = store.stats(None, now_secs()).expect("stats");
    assert_eq!(stats.new_cards, 1);
    assert_eq!(stats.total_cards, 1);
    assert!(stats.learning_cards == 0 && stats.review_cards == 0);
    assert!(store.collection_modified().expect("modified") > before);
    assert_eq!(
        store
            .stats(Some(999), now_secs())
            .expect("stats")
            .total_cards,
        0
    );
}

#[test]
fn collection_modified_increases_with_consecutive_writes() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let mut previous = store.collection_modified().expect("modified");
    for _ in 0..3 {
        store.add_note(&note(&["one", "answer"], &[])).expect("add");
        let current = store.collection_modified().expect("modified");
        assert!(current > previous, "{current} <= {previous}");
        previous = current;
    }
}

#[test]
fn collection_modified_saturates_at_the_end_of_the_range() {
    let (_directory, path) = fixture();
    let conn = Connection::open(&path).expect("open");
    conn.execute("UPDATE col SET mod = ?1 WHERE id = 1", [i64::MAX - 1])
        .expect("seed mod");
    drop(conn);
    let mut store = Collection::open(&path).expect("open");
    for _ in 0..2 {
        store.add_note(&note(&["one", "answer"], &[])).expect("add");
        assert_eq!(
            store.collection_modified().expect("modified"),
            i64::MAX,
            "saturating bump past i64::MAX - 1 stays at the limit"
        );
    }
}

#[test]
fn checkpoint_and_close_leave_a_readable_collection() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["durable", "answer"], &[]))
        .expect("add");
    store.checkpoint().expect("checkpoint");
    assert_eq!(store.path(), path);
    store.close().expect("close");
    let reopened = Collection::open(&path).expect("reopen");
    assert_eq!(reopened.find_cards("durable").expect("find").len(), 1);
}

#[test]
fn open_does_not_rewrite_existing_collection_json() {
    let (_directory, path) = fixture();
    let conn = Connection::open(&path).expect("open");
    conn.execute(
        "UPDATE col SET conf = ?1 WHERE id = 1",
        ["{\"nextPos\": 77, \"custom\": true}"],
    )
    .expect("update");
    drop(conn);
    let store = Collection::open(&path).expect("open");
    store.checkpoint().expect("checkpoint");
    drop(store);
    let conn = read_only(&path);
    let configuration: String = conn
        .query_row("SELECT conf FROM col WHERE id = 1", [], |row| row.get(0))
        .expect("conf");
    let parsed: Value = serde_json::from_str(&configuration).expect("JSON");
    assert_eq!(parsed["nextPos"], 77);
    assert_eq!(parsed["custom"], true);
}

#[test]
fn reads_a_real_anki_collection_without_touching_the_original() {
    let Some(source) = env::var_os("RANKI_TEST_COLLECTION")
        .map(PathBuf::from)
        .filter(|path| path.exists())
    else {
        eprintln!("skipping: real Anki collection not present");
        return;
    };
    let original = fs::read(&source).expect("read original");
    let original_hash = Sha1::digest(&original);
    let directory = tempfile::tempdir().expect("tempdir");
    let copy = directory.path().join("collection.anki2");
    fs::copy(&source, &copy).expect("copy");
    let store = Collection::open(&copy).expect("open copy");
    let conn = read_only(&copy);
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .map(|row| row.expect("row"))
        .collect();
    for table in ["cards", "col", "graves", "notes", "revlog"] {
        assert!(tables.iter().any(|name| name == table), "missing {table}");
    }
    let decks = store.decks().expect("decks");
    assert!(decks.len() > 1);
    assert!(!decks.iter().any(|deck| deck.name == "Default"));
    assert!(decks.iter().any(|deck| deck.name.contains("::")));
    let first_card: i64 = conn
        .query_row("SELECT id FROM cards ORDER BY id LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("card id");
    assert!(store.card(first_card).expect("card").is_some());
    assert!(store.models().expect("models").len() > 1);
    let due_now = {
        let crt: i64 = conn
            .query_row("SELECT crt FROM col WHERE id = 1", [], |row| row.get(0))
            .expect("crt");
        let maximum_due: i64 = conn
            .query_row(
                "SELECT MAX(due) FROM cards WHERE queue IN (2, 3)",
                [],
                |row| row.get(0),
            )
            .expect("maximum due");
        crt.saturating_add(maximum_due.saturating_mul(86_400))
    };
    assert_ne!(
        store.due_cards(None, 5, due_now).expect("due"),
        [] as [ranki_model::Card; 0]
    );
    drop(conn);
    drop(store);
    assert_eq!(
        Sha1::digest(fs::read(&source).expect("read again")),
        original_hash
    );
}

#[test]
fn ids_are_unique_across_storage_core_and_manual_logs() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let note_ids = store
        .add_note(&note(&["front", "back"], &[]))
        .expect("add note");
    let card_id = note_ids.last().copied().expect("card id");
    let card = store.card(card_id).expect("card").expect("card exists");
    let start = store.collection_created().expect("collection start");
    let decision = FsrsScheduler::default().schedule(
        &card,
        Rating::Good,
        now_secs(),
        start,
    );
    let manual_id = ranki_model::next_id(now_secs().saturating_mul(1_000));

    let mut ids = note_ids;
    ids.push(decision.revlog.id);
    ids.push(manual_id);
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len());
}

#[test]
fn rename_deck_renames_subdecks_and_preserves_ids() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let parent = store.create_deck("Study").expect("parent");
    let child = store.create_deck("Study::Languages").expect("child");
    let grandchild = store
        .create_deck("Study::Languages::French")
        .expect("grandchild");
    let mut entry = note(&["bonjour", "hello"], &[]);
    entry.deck_id = grandchild.id;
    let card_id = store.add_note(&entry).expect("add")[0];

    let renamed = store.rename_deck(parent.id, "Learning").expect("rename");
    assert_eq!(renamed.id, parent.id);
    assert_eq!(renamed.name, "Learning");

    let decks = store.decks().expect("decks");
    let names: Vec<_> = decks.iter().map(|deck| deck.name.as_str()).collect();
    assert!(names.contains(&"Learning"));
    assert!(names.contains(&"Learning::Languages"));
    assert!(names.contains(&"Learning::Languages::French"));
    assert!(!names.iter().any(|name| name.starts_with("Study")));
    for expected in [&child, &grandchild] {
        assert!(decks.iter().any(|deck| deck.id == expected.id));
    }
    let card = store.card(card_id).expect("card").expect("exists");
    assert_eq!(card.deck_id, grandchild.id);

    let conn = read_only(&path);
    let decks_json: String = conn
        .query_row("SELECT decks FROM col WHERE id = 1", [], |row| row.get(0))
        .expect("decks json");
    let parsed: Value = serde_json::from_str(&decks_json).expect("valid JSON");
    let deck_name = |id: i64| {
        parsed
            .get(id.to_string())
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(deck_name(parent.id), "Learning");
    assert_eq!(deck_name(child.id), "Learning::Languages");
    assert_eq!(deck_name(grandchild.id), "Learning::Languages::French");
    let index: String = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'ix_cards_sched'",
            [],
            |row| row.get(0),
        )
        .expect("index");
    assert_eq!(index, "ix_cards_sched");
}

#[test]
fn rename_deck_rejects_empty_and_colliding_names() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let parent = store.create_deck("A").expect("parent");
    store.create_deck("A::B").expect("child");
    store.create_deck("B").expect("other");
    assert!(store.rename_deck(parent.id, "  ").is_err());
    assert!(store.rename_deck(parent.id, "A::").is_err());
    assert!(store.rename_deck(parent.id, "B").is_err());
    assert!(store.rename_deck(999, "Renamed").is_err());

    let source = store.create_deck("Q").expect("source");
    store.create_deck("Q::Z").expect("source child");
    store.create_deck("Z::Z").expect("collision");
    assert!(store.rename_deck(source.id, "Z").is_err());
    assert!(
        store
            .decks()
            .expect("decks")
            .iter()
            .any(|deck| deck.name == "Q")
    );
}

#[test]
fn remove_deck_deletes_subtree_cards_notes_revlog_and_writes_graves() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let parent = store.create_deck("Work").expect("parent");
    let child = store.create_deck("Work::Notes").expect("child");
    let mut entry = note(&["question", "answer"], &["tag"]);
    entry.deck_id = child.id;
    let card_id = store.add_note(&entry).expect("add")[0];
    let card = store.card(card_id).expect("card").expect("exists");
    let note_id = card.note_id;
    let log_id = 9_002_i64;
    let decision = ReviewDecision {
        card: card.clone(),
        expected_modified: card.modified,
        expected_usn: card.usn,
        revlog: RevLog {
            id: log_id,
            card_id,
            usn: 0,
            ease: Rating::Good,
            interval: 4,
            last_interval: 0,
            factor: 2500,
            time_ms: 500,
            kind: RevLogKind::Review,
        },
    };
    store.apply_review(&decision).expect("review");

    store.remove_deck(parent.id).expect("remove");

    assert_eq!(store.decks().expect("decks").len(), 1);
    assert!(store.card(card_id).expect("card").is_none());
    assert!(store.note(note_id).expect("note").is_none());
    assert!(store.revlog(log_id).expect("revlog").is_none());
    let conn = read_only(&path);
    let mut statement = conn
        .prepare("SELECT oid, type FROM graves ORDER BY type, oid")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .expect("query");
    let graves: Vec<_> = rows.map(|row| row.expect("row")).collect();
    assert!(graves.contains(&(card_id, 0)), "{graves:?}");
    assert!(graves.contains(&(note_id, 1)), "{graves:?}");
    assert!(graves.contains(&(parent.id, 2)), "{graves:?}");
    assert!(graves.contains(&(child.id, 2)), "{graves:?}");
    let decks_json: String = conn
        .query_row("SELECT decks FROM col WHERE id = 1", [], |row| row.get(0))
        .expect("decks json");
    let parsed: Value = serde_json::from_str(&decks_json).expect("JSON");
    assert!(parsed.get(parent.id.to_string()).is_none());
    assert!(parsed.get(child.id.to_string()).is_none());
    assert!(parsed.get("1").is_some());
}

#[test]
fn remove_deck_protects_default_and_rejects_unknown_decks() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    assert!(matches!(
        store.remove_deck(1),
        Err(StoreError::InvalidData(_))
    ));
    assert!(matches!(
        store.remove_deck(999),
        Err(StoreError::NotFound(_))
    ));
    assert!(
        store
            .decks()
            .expect("decks")
            .iter()
            .any(|deck| deck.id == 1)
    );
}

#[test]
fn remove_deck_keeps_notes_that_still_have_cards() {
    let (_directory, path) = fixture();
    {
        let conn = Connection::open(&path).expect("open");
        let models: String = conn
            .query_row("SELECT models FROM col WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("models");
        let mut model_json: Value =
            serde_json::from_str(&models).expect("parse");
        let template = model_json["1"]["tmpls"][0].clone();
        model_json["1"]["tmpls"]
            .as_array_mut()
            .expect("templates")
            .push(template);
        conn.execute(
            "UPDATE col SET models = ?1 WHERE id = 1",
            [serde_json::to_string(&model_json).expect("serialize")],
        )
        .expect("update");
    }
    let mut store = Collection::open(&path).expect("open");
    let source = store.create_deck("Source").expect("source");
    let other = store.create_deck("Other").expect("other");
    let mut entry = note(&["front", "back"], &[]);
    entry.deck_id = source.id;
    let cards = store.add_note(&entry).expect("add");
    assert_eq!(cards.len(), 2);
    let note_id = store.card(cards[0]).expect("card").expect("exists").note_id;

    let mut moved = store.card(cards[1]).expect("card").expect("exists");
    moved.deck_id = other.id;
    let decision = ReviewDecision {
        card: moved.clone(),
        expected_modified: moved.modified,
        expected_usn: moved.usn,
        revlog: RevLog {
            id: 9_003_i64,
            card_id: cards[1],
            usn: 0,
            ease: Rating::Good,
            interval: 0,
            last_interval: 0,
            factor: 2500,
            time_ms: 0,
            kind: RevLogKind::Manual,
        },
    };
    store.apply_review(&decision).expect("move");
    store.remove_deck(source.id).expect("remove");

    assert!(store.card(cards[0]).expect("card").is_none());
    assert_eq!(
        store.card(cards[1]).expect("card").expect("exists").deck_id,
        other.id
    );
    assert!(store.note(note_id).expect("note").is_some());
}

#[test]
fn rename_and_remove_work_with_modern_deck_tables() {
    let (_directory, path) = fixture();
    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch(
            "UPDATE col SET decks = '{}' WHERE id = 1;
             CREATE TABLE decks (
                 id integer primary key,
                 name text not null,
                 mtime_secs integer not null,
                 usn integer not null,
                 common blob not null,
                 kind integer not null
             );
             INSERT INTO decks VALUES (10, 'Modern', 0, 0, x'', 0);
             INSERT INTO decks VALUES (11, 'Modern\u{001f}Child', 0, 0, x'', 0);",
        )
        .expect("modern schema");
    }
    let mut store = Collection::open(&path).expect("open");
    let parent = store
        .decks()
        .expect("decks")
        .into_iter()
        .find(|deck| deck.name == "Modern")
        .expect("parent");
    let renamed = store.rename_deck(parent.id, "Current").expect("rename");
    assert_eq!(renamed.name, "Current");
    let names: Vec<_> = store
        .decks()
        .expect("decks")
        .into_iter()
        .map(|deck| deck.name)
        .collect();
    assert!(names.contains(&"Current::Child".to_owned()));

    let conn = read_only(&path);
    let raw: String = conn
        .query_row("SELECT name FROM decks WHERE id = 11", [], |row| row.get(0))
        .expect("encoded name");
    assert_eq!(raw, "Current\u{001f}Child");
    drop(conn);

    store.remove_deck(parent.id).expect("remove");
    let conn = read_only(&path);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM decks", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 0);
}

#[test]
fn deck_stats_matches_per_deck_stats_for_a_hierarchy() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let root = store.create_deck("Root").expect("root");
    let child = store.create_deck("Root::Child").expect("child");
    let leaf = store.create_deck("Root::Child::Leaf").expect("leaf");
    for (index, deck_id) in
        [root.id, child.id, leaf.id, 1].into_iter().enumerate()
    {
        store
            .add_note(&NewNote {
                model_id: 1,
                deck_id,
                fields: vec![format!("front {index}"), "back".to_owned()],
                tags: Vec::new(),
            })
            .expect("add");
    }
    let batch = store.deck_stats(now_secs()).expect("deck stats");
    let decks = store.decks().expect("decks");
    assert_eq!(batch.len(), decks.len());
    for (id, stats) in batch {
        assert_eq!(
            stats,
            store.stats(Some(id), now_secs()).expect("stats"),
            "deck {id}"
        );
    }
    let root_stats = store
        .deck_stats(now_secs())
        .expect("deck stats")
        .into_iter()
        .find(|(id, _)| *id == root.id)
        .expect("root entry");
    assert_eq!(root_stats.1.total_cards, 3);
    assert_eq!(root_stats.1.new_cards, 3);
}

#[test]
fn deck_stats_counts_children_of_names_ending_in_a_colon() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let parent = store.create_deck("A:").expect("parent");
    let child = store.create_deck("A:::B").expect("child");
    store
        .add_note(&NewNote {
            model_id: 1,
            deck_id: child.id,
            fields: vec!["front".to_owned(), "back".to_owned()],
            tags: Vec::new(),
        })
        .expect("add");
    let batch = store.deck_stats(now_secs()).expect("deck stats");
    let totals = batch
        .iter()
        .find(|(id, _)| *id == parent.id)
        .map(|(_, stats)| *stats)
        .expect("parent entry");
    assert_eq!(
        totals,
        store.stats(Some(parent.id), now_secs()).expect("stats")
    );
    assert_eq!(totals.new_cards, 1);
}

#[test]
fn apply_reviews_writes_a_batch_atomically() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    let mut decisions = Vec::with_capacity(1000);
    for index in 0..1000_u32 {
        let ids = store
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![format!("card {index}"), "back".to_owned()],
                tags: Vec::new(),
            })
            .expect("add");
        let card = store.card(ids[0]).expect("card").expect("exists");
        let mut changed = card.clone();
        changed.queue = CardQueue::Review;
        changed.card_type = CardType::Review;
        changed.due = i64::from(index);
        changed.interval = 4;
        decisions.push(ReviewDecision {
            card: changed,
            expected_modified: card.modified,
            expected_usn: card.usn,
            revlog: RevLog {
                id: 20_000 + i64::from(index),
                card_id: card.id,
                usn: 0,
                ease: Rating::Good,
                interval: 4,
                last_interval: 0,
                factor: 2500,
                time_ms: 400,
                kind: RevLogKind::Review,
            },
        });
    }
    store.apply_reviews(&decisions).expect("batch");
    for (index, decision) in decisions.iter().enumerate() {
        let saved =
            store.card(decision.card.id).expect("card").expect("exists");
        assert_eq!(saved.queue, CardQueue::Review);
        assert_eq!(saved.interval, 4);
        assert_eq!(saved.due, i64::from(u32::try_from(index).expect("index")));
        let log = store
            .revlog(20_000 + i64::try_from(index).expect("index"))
            .expect("revlog")
            .expect("entry");
        assert_eq!(log.card_id, decision.card.id);
    }

    let mut failing = decisions[0].clone();
    failing.revlog.id = 90_000;
    failing.card.interval = 9;
    let mut missing = failing.clone();
    missing.card.id = -1;
    missing.revlog.id = 90_001;
    assert!(store.apply_reviews(&[failing, missing]).is_err());
    let saved = store
        .card(decisions[0].card.id)
        .expect("card")
        .expect("exists");
    assert_eq!(saved.interval, 4, "a failed batch writes nothing");
    assert!(store.revlog(90_000).expect("revlog").is_none());
    assert!(store.revlog(90_001).expect("revlog").is_none());
}

#[test]
fn stats_counts_boundaries_within_a_deck_subtree() {
    let (_directory, path) = fixture();
    let now = now_secs();
    let conn = Connection::open(&path).expect("open");
    conn.execute("UPDATE col SET crt = ?1 WHERE id = 1", [now])
        .expect("set crt");
    drop(conn);
    let mut store = Collection::open(&path).expect("open");
    let parent = store.create_deck("Study").expect("parent");
    let child = store.create_deck("Study::Kids").expect("child");

    let conn = Connection::open(&path).expect("open");
    for (id, did, queue, due) in [
        (10_i64, parent.id, 0_i64, 7_i64),
        (11, parent.id, 1, now),
        (12, parent.id, 1, now.saturating_add(1)),
        (13, parent.id, 2, 0),
        (14, parent.id, 2, 1),
        (15, parent.id, 3, 0),
        (20, child.id, 0, 7),
        (21, child.id, 1, now),
        (22, child.id, 1, now.saturating_add(1)),
        (23, child.id, 2, 0),
        (24, child.id, 3, 1),
    ] {
        conn.execute(
            "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, \
             lapses, left, odue, odid, flags, data) VALUES (?1, 1, ?2, 0, 0, 0, 0, ?3, ?4, 0, 0, \
             0, 0, 0, 0, 0, 0, '')",
            [id, did, queue, due],
        )
        .expect("insert card");
    }
    drop(conn);

    let store = Collection::open(&path).expect("open");
    let all = store.stats(Some(parent.id), now).expect("parent stats");
    assert_eq!(all.new_cards, 2);
    assert_eq!(all.learning_cards, 3);
    assert_eq!(all.review_cards, 2);
    assert_eq!(all.total_cards, 11);

    let subtree = store.stats(Some(child.id), now).expect("child stats");
    assert_eq!(subtree.new_cards, 1);
    assert_eq!(subtree.learning_cards, 1);
    assert_eq!(subtree.review_cards, 1);
    assert_eq!(subtree.total_cards, 5);

    let collection = store.stats(None, now).expect("collection stats");
    assert_eq!(collection, all);
    assert_eq!(
        store.stats(Some(999), now).expect("missing deck"),
        DeckStats::default()
    );
}

#[test]
fn due_cards_probe_keeps_priority_and_all_new_order() {
    let (_directory, path) = fixture();
    let now = now_secs();
    let today = 0_i64;
    let conn = Connection::open(&path).expect("open");
    conn.execute("UPDATE col SET crt = ?1 WHERE id = 1", [now])
        .expect("set crt");
    for (id, queue, due) in [(10_i64, 0_i64, 7_i64), (11, 0, 3), (30, 2, today)]
    {
        conn.execute(
            "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, \
             lapses, left, odue, odid, flags, data) VALUES (?1, 1, 1, 0, 0, 0, 0, ?2, ?3, 0, 0, \
             0, 0, 0, 0, 0, 0, '')",
            [id, queue, due],
        )
        .expect("insert card");
    }
    conn.execute("DELETE FROM cards WHERE id = 30", [])
        .expect("remove review card");
    drop(conn);

    let store = Collection::open(&path).expect("open");
    let due: Vec<_> = store
        .due_cards(None, 2, now)
        .expect("due")
        .into_iter()
        .map(|card| card.id)
        .collect();
    assert_eq!(due, [11, 10]);

    let conn = Connection::open(&path).expect("open");
    conn.execute(
        "INSERT INTO cards (id, nid, did, ord, mod, usn, type, queue, due, ivl, factor, reps, \
         lapses, left, odue, odid, flags, data) VALUES (30, 1, 1, 0, 0, 0, 0, 2, 0, 0, 0, 0, \
         0, 0, 0, 0, 0, '')",
        [],
    )
    .expect("insert review card");
    drop(conn);
    let due: Vec<_> = store
        .due_cards(None, 2, now)
        .expect("due")
        .into_iter()
        .map(|card| card.id)
        .collect();
    assert_eq!(due, [30, 11]);
}

#[test]
fn apply_review_reports_stale_and_missing_cards() {
    let (_directory, path) = fixture();
    let mut store = Collection::open(&path).expect("open");
    store
        .add_note(&note(&["review", "answer"], &[]))
        .expect("add");
    let now = now_secs();
    let card = store
        .due_cards(None, 1, now)
        .expect("due")
        .pop()
        .expect("card");
    let decision = FsrsScheduler::default().schedule(
        &card,
        Rating::Good,
        now,
        store.collection_created().expect("created"),
    );

    let mut stale = decision.clone();
    stale.expected_modified = card.modified.saturating_add(1);
    let error = store.apply_review(&stale).expect_err("stale review");
    assert!(
        matches!(&error, StoreError::InvalidData(message) if message == "card changed since it was loaded"),
        "{error:?}"
    );

    let mut missing = decision;
    missing.card.id = -1;
    let error = store.apply_review(&missing).expect_err("missing review");
    assert!(
        matches!(&error, StoreError::InvalidData(message) if message == "card changed since it was loaded"),
        "{error:?}"
    );
    assert_eq!(store.review_history().expect("history"), [] as [RevLog; 0]);
}
