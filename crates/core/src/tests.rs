use ranki_model::{Card, CardQueue, CardType, Rating, RevLogKind};

use super::{FsrsPreset, FsrsScheduler, Scheduler, Sm2Preset, Sm2Scheduler};

const NOW: i64 = 1_700_000_000;

const COLLECTION_START: i64 = 1_685_412_000;
fn base_card() -> Card {
    Card {
        id: 7,
        note_id: 3,
        deck_id: 1,
        ordinal: 0,
        modified: NOW,
        usn: -1,
        card_type: CardType::Review,
        queue: CardQueue::Review,
        due: 0,
        interval: 10,
        factor: 2_500,
        reps: 5,
        lapses: 0,
        left: 0,
        original_due: 0,
        original_deck_id: 0,
        flags: 0,
        data: String::new(),
    }
}

fn fsrs_card() -> Card {
    Card {
        data: String::from(r#"{"s":10.0,"d":5.0}"#),
        ..base_card()
    }
}

fn fsrs_preset(fuzz: bool, retention: f32) -> FsrsPreset {
    FsrsPreset {
        desired_retention: retention,
        maximum_interval: 36_500,
        learning_steps_minutes: vec![1, 10],
        relearning_steps_minutes: vec![10],
        enable_fuzzing: fuzz,
    }
}

fn new_card() -> Card {
    Card {
        card_type: CardType::New,
        queue: CardQueue::New,
        due: 0,
        interval: 0,
        left: 0,
        reps: 0,
        ..base_card()
    }
}

fn advance(card: &Card, now_secs: &mut i64) {
    *now_secs = now_secs
        .saturating_add(i64::from(card.interval).saturating_mul(86_400));
}

fn stored_stability(data: &str) -> f64 {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| value.get("s").and_then(serde_json::Value::as_f64))
        .unwrap_or(0.0)
}

#[test]
fn fsrs_repeated_good_intervals_grow() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let mut card = fsrs_card();
    let mut now = NOW;
    let mut previous = 0;
    for _ in 0..6 {
        let decision =
            scheduler.schedule(&card, Rating::Good, now, COLLECTION_START);
        assert!(decision.card.interval > previous);
        previous = decision.card.interval;
        advance(&decision.card, &mut now);
        card = decision.card;
    }
}

#[test]
fn fsrs_again_gives_shorter_interval_than_good() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let card = fsrs_card();
    let again = scheduler.schedule(&card, Rating::Again, NOW, COLLECTION_START);
    let good = scheduler.schedule(&card, Rating::Good, NOW, COLLECTION_START);
    assert!(again.card.interval < good.card.interval);
    assert_eq!(again.card.card_type, CardType::Relearning);
}

#[test]
fn fsrs_higher_desired_retention_shortens_intervals() {
    let low = FsrsScheduler::new(fsrs_preset(false, 0.8));
    let high = FsrsScheduler::new(fsrs_preset(false, 0.95));
    let card = fsrs_card();
    let loose = low.schedule(&card, Rating::Good, NOW, COLLECTION_START);
    let strict = high.schedule(&card, Rating::Good, NOW, COLLECTION_START);
    assert!(strict.card.interval < loose.card.interval);
}

#[test]
fn fsrs_learning_steps_and_graduation() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let first =
        scheduler.schedule(&new_card(), Rating::Good, NOW, COLLECTION_START);
    assert_eq!(first.card.card_type, CardType::Learning);
    assert_eq!(first.card.queue, CardQueue::Learning);
    assert_eq!(first.card.left, 1_001);
    assert_eq!(
        first.card.due,
        NOW.saturating_mul(1_000).saturating_add(60_000)
    );
    assert_eq!(first.revlog.kind, RevLogKind::Learn);
    let second = scheduler.schedule(
        &first.card,
        Rating::Good,
        NOW.saturating_add(60),
        COLLECTION_START,
    );
    assert_eq!(second.card.card_type, CardType::Review);
    assert_eq!(second.card.queue, CardQueue::Review);
    assert_eq!(second.card.left, 0);
    assert!(second.card.interval >= 1);
}

#[test]
fn fsrs_lapse_enters_relearning() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let decision =
        scheduler.schedule(&fsrs_card(), Rating::Again, NOW, COLLECTION_START);
    assert_eq!(decision.card.card_type, CardType::Relearning);
    assert_eq!(decision.card.queue, CardQueue::Learning);
    assert_eq!(decision.card.lapses, 1);
    assert_eq!(decision.revlog.kind, RevLogKind::Relearn);
}

#[test]
fn fsrs_fuzz_is_deterministic_and_bounded() {
    let scheduler = FsrsScheduler::new(fsrs_preset(true, 0.9));
    let card = fsrs_card();
    let first = scheduler.schedule(&card, Rating::Good, NOW, COLLECTION_START);
    let second = scheduler.schedule(&card, Rating::Good, NOW, COLLECTION_START);
    assert_eq!(first.card.interval, second.card.interval);
    assert!((2..=36_500).contains(&first.card.interval));
}

#[test]
fn fsrs_same_day_uses_short_term_scaling() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let card = fsrs_card();
    let same_day =
        scheduler.schedule(&card, Rating::Good, NOW, COLLECTION_START);
    let later = scheduler.schedule(
        &card,
        Rating::Good,
        NOW.saturating_add(86_400),
        COLLECTION_START,
    );
    let same_stability = stored_stability(&same_day.card.data);
    let later_stability = stored_stability(&later.card.data);
    assert!(same_stability > 0.0);
    assert!(later_stability > 0.0);
    assert!((same_stability - later_stability).abs() > 0.001);
}

#[test]
fn suspended_buried_and_preview_cards_are_unchanged() {
    let fsrs = FsrsScheduler::default();
    let sm2 = Sm2Scheduler::default();
    for queue in [CardQueue::Suspended, CardQueue::Buried, CardQueue::Preview] {
        let card = Card {
            queue,
            ..fsrs_card()
        };
        let fsrs_decision =
            fsrs.schedule(&card, Rating::Good, NOW, COLLECTION_START);
        let sm2_decision =
            sm2.schedule(&card, Rating::Good, NOW, COLLECTION_START);
        assert_eq!(fsrs_decision.card, card);
        assert_eq!(sm2_decision.card, card);
        assert_eq!(fsrs_decision.revlog.kind, RevLogKind::Manual);
        assert_eq!(sm2_decision.revlog.kind, RevLogKind::Manual);
    }
}

#[test]
fn extreme_inputs_do_not_panic() {
    let presets = [
        fsrs_preset(true, 0.9),
        FsrsPreset {
            maximum_interval: i32::MIN,
            learning_steps_minutes: vec![u32::MAX, 0],
            relearning_steps_minutes: vec![u32::MAX],
            ..fsrs_preset(true, f32::NAN)
        },
    ];
    let schedulers: Vec<FsrsScheduler> =
        presets.into_iter().map(FsrsScheduler::new).collect();
    let extreme = Card {
        id: i64::MIN,
        modified: i64::MIN,
        interval: i32::MAX,
        factor: u32::MAX,
        reps: u32::MAX,
        lapses: u32::MAX,
        left: u32::MAX,
        card_type: CardType::Learning,
        queue: CardQueue::Learning,
        data: String::from("not json"),
        ..base_card()
    };
    let cases = [i64::MIN, 0, i64::MAX.saturating_div(2)]
        .into_iter()
        .flat_map(|now| {
            [fsrs_card(), extreme.clone()]
                .into_iter()
                .flat_map(move |card| {
                    [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy]
                        .into_iter()
                        .map(move |rating| (now, card.clone(), rating))
                })
        });
    for (now, card, rating) in cases {
        for scheduler in &schedulers {
            let decision =
                scheduler.schedule(&card, rating, now, COLLECTION_START);
            assert_ne!(decision.revlog.id, 0);
            assert!(decision.card.interval >= 0);
        }
    }
    let sm2 = Sm2Scheduler::new(Sm2Preset {
        learning_steps_minutes: vec![u32::MAX],
        relearning_steps_minutes: vec![u32::MAX],
        minimum_ease: u32::MAX,
        easy_bonus: f32::NAN,
    });
    for rating in [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy] {
        let decision =
            sm2.schedule(&extreme, rating, i64::MAX, COLLECTION_START);
        assert_eq!(decision.card.factor, u32::MAX);
    }
}

#[test]
fn revlog_matches_contract() {
    let scheduler = FsrsScheduler::default();
    let card = fsrs_card();
    let decision =
        scheduler.schedule(&card, Rating::Good, NOW, COLLECTION_START);
    assert_ne!(decision.revlog.id, 0);
    assert_eq!(decision.revlog.card_id, card.id);
    assert_eq!(decision.revlog.usn, -1);
    assert_eq!(decision.revlog.time_ms, 0);
    assert_eq!(decision.revlog.ease, Rating::Good);
    assert_eq!(decision.revlog.kind, RevLogKind::Review);
}

#[test]
fn sm2_reference_intervals_are_one_six_fifteen() {
    let scheduler = Sm2Scheduler::default();
    let mut card = Card {
        interval: 0,
        reps: 0,
        ..base_card()
    };
    let mut now = NOW;
    for expected in [1, 6, 15] {
        let decision =
            scheduler.schedule(&card, Rating::Good, now, COLLECTION_START);
        assert_eq!(decision.card.interval, expected);
        advance(&decision.card, &mut now);
        card = decision.card;
    }
}

#[test]
fn sm2_again_resets_to_relearning_and_reduces_factor() {
    let scheduler = Sm2Scheduler::default();
    let card = Card {
        interval: 30,
        ..base_card()
    };
    let decision =
        scheduler.schedule(&card, Rating::Again, NOW, COLLECTION_START);
    assert_eq!(decision.card.card_type, CardType::Relearning);
    assert_eq!(decision.card.queue, CardQueue::Learning);
    assert_eq!(decision.card.lapses, 1);
    assert_eq!(decision.card.factor, 2_300);
    assert_eq!(decision.revlog.kind, RevLogKind::Relearn);
}

#[test]
fn sm2_learning_steps_and_easy_graduation() {
    let scheduler = Sm2Scheduler::default();
    let first =
        scheduler.schedule(&new_card(), Rating::Good, NOW, COLLECTION_START);
    assert_eq!(first.card.card_type, CardType::Learning);
    assert_eq!(first.card.left, 1_001);
    let graduated = scheduler.schedule(
        &first.card,
        Rating::Easy,
        NOW.saturating_add(60),
        COLLECTION_START,
    );
    assert_eq!(graduated.card.card_type, CardType::Review);
    assert_eq!(graduated.card.interval, 4);
}

#[test]
fn sm2_ease_floor_and_easy_bonus() {
    let scheduler = Sm2Scheduler::new(Sm2Preset {
        minimum_ease: 2_400,
        easy_bonus: 1.5,
        ..Sm2Preset::default()
    });
    let low_factor = Card {
        factor: 1_000,
        ..base_card()
    };
    let lapse =
        scheduler.schedule(&low_factor, Rating::Again, NOW, COLLECTION_START);
    assert_eq!(lapse.card.factor, 2_400);

    let plain = Sm2Scheduler::default();
    let good =
        plain.schedule(&base_card(), Rating::Good, NOW, COLLECTION_START);
    let easy =
        plain.schedule(&base_card(), Rating::Easy, NOW, COLLECTION_START);
    assert!(easy.card.interval > good.card.interval);
}

#[test]
fn scheduler_names_and_presets() {
    let fsrs = FsrsScheduler::default();
    let sm2 = Sm2Scheduler::default();
    assert_eq!(fsrs.name(), "fsrs");
    assert_eq!(sm2.name(), "sm2");
    assert_eq!(fsrs.preset(), &FsrsScheduler::default_preset());
    assert_eq!(sm2.preset(), &Sm2Preset::default());
}

#[test]
fn fsrs_maximum_interval_is_respected() {
    let scheduler = FsrsScheduler::new(FsrsPreset {
        maximum_interval: 7,
        ..fsrs_preset(false, 0.9)
    });
    let card = Card {
        data: String::from(r#"{"s":1000.0,"d":3.0}"#),
        ..base_card()
    };
    let decision =
        scheduler.schedule(&card, Rating::Easy, NOW, COLLECTION_START);
    assert_eq!(decision.card.interval, 7);
}

#[test]
fn revlog_ids_are_unique_within_one_second() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let first =
        scheduler.schedule(&fsrs_card(), Rating::Good, NOW, COLLECTION_START);
    let second =
        scheduler.schedule(&fsrs_card(), Rating::Good, NOW, COLLECTION_START);
    assert_ne!(first.revlog.id, second.revlog.id);
}

#[test]
fn fsrs_same_day_good_and_easy_do_not_shrink() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let card = fsrs_card();
    for rating in [Rating::Good, Rating::Easy] {
        let same_day = scheduler.schedule(&card, rating, NOW, COLLECTION_START);
        let later = scheduler.schedule(
            &card,
            rating,
            NOW.saturating_add(86_400),
            COLLECTION_START,
        );
        assert!(stored_stability(&same_day.card.data) >= 10.0);
        assert!(same_day.card.interval >= later.card.interval);
    }
}
