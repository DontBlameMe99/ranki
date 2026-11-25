use ranki_model::{Card, CardQueue, CardType, Rating, RevLogKind};

use super::{
    ANKI_DEFAULT_PARAMS, DEFAULT_DECAY, FsrsParams, FsrsPreset, FsrsScheduler,
    MAX_DESIRED_RETENTION, MIN_DESIRED_RETENTION, Scheduler, Sm2Preset,
    Sm2Scheduler, clamp_desired_retention, interval_days_for_stability,
    raw_interval_days_for_stability,
};

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
        NOW.saturating_mul(1_000).saturating_add(600_000)
    );
    assert_eq!(first.revlog.kind, RevLogKind::Learn);
    let second = scheduler.schedule(
        &first.card,
        Rating::Good,
        NOW.saturating_add(600),
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
            assert!(
                decision.card.interval >= 0
                    || decision.card.queue == CardQueue::Learning
            );
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
    let params =
        FsrsParams::from_fsrs5(&LEGACY_FSRS5_WEIGHTS).unwrap_or_default();
    let scheduler =
        FsrsScheduler::new(fsrs_preset(false, 0.9)).with_params(params);
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

#[test]
fn previews_are_pure_and_match_scheduling() {
    let fsrs = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let sm2 = Sm2Scheduler::default();
    for (scheduler, card) in [
        (&fsrs as &dyn Scheduler, fsrs_card()),
        (&sm2 as &dyn Scheduler, new_card()),
    ] {
        for rating in [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy]
        {
            let preview =
                scheduler.preview(&card, rating, NOW, COLLECTION_START);
            assert_eq!(
                preview,
                scheduler.preview(&card, rating, NOW, COLLECTION_START)
            );
            let decision =
                scheduler.schedule(&card, rating, NOW, COLLECTION_START);
            assert_eq!(decision.revlog.interval, preview);
        }
    }
}

#[test]
fn scheduler_is_built_from_settings() {
    let settings = ranki_model::Settings {
        scheduler: ranki_model::SchedulerKind::Sm2,
        ..ranki_model::Settings::default()
    };
    assert_eq!(super::from_settings(&settings).name(), "sm2");

    let card = fsrs_card();
    let standard = ranki_model::Settings {
        scheduler: ranki_model::SchedulerKind::Fsrs,
        desired_retention: 0.9,
        ..ranki_model::Settings::default()
    };
    let easy = ranki_model::Settings {
        desired_retention: 0.8,
        ..standard.clone()
    };
    let standard_interval = super::from_settings(&standard).preview(
        &card,
        Rating::Good,
        NOW,
        COLLECTION_START,
    );
    let easy_interval = super::from_settings(&easy).preview(
        &card,
        Rating::Good,
        NOW,
        COLLECTION_START,
    );
    assert_ne!(standard_interval, easy_interval);
}

const LEGACY_FSRS5_WEIGHTS: [f32; 19] = [
    0.40255, 1.18385, 3.173, 15.69105, 7.1949, 0.5345, 1.4604, 0.0046, 1.54575,
    0.1192, 1.01925, 1.9395, 0.11, 0.29605, 2.2698, 0.2315, 2.9898, 0.51655,
    0.6621,
];

#[test]
fn interval_matches_anki_fsrs5_reference_vectors() {
    for (retention, expected_raw, expected_days) in
        [(0.9, 10.0, 10), (0.8, 23.9803, 24), (0.95, 4.6056, 5)]
    {
        let raw =
            raw_interval_days_for_stability(10.0, retention, DEFAULT_DECAY);
        assert!((f64::from(raw) - expected_raw).abs() < 0.001);
        assert_eq!(
            interval_days_for_stability(10.0, retention, DEFAULT_DECAY, 36_500),
            expected_days
        );
    }
}

#[test]
fn interval_matches_anki_fsrs6_reference_vectors() {
    let decay = ANKI_DEFAULT_PARAMS[20];
    assert!(
        (raw_interval_days_for_stability(10.0, 0.9, decay) - 10.0).abs()
            < 0.001
    );
    assert!(
        (raw_interval_days_for_stability(10.0, 0.95, decay) - 4.0256).abs()
            < 0.001
    );
    assert_eq!(interval_days_for_stability(10.0, 0.95, decay, 36_500), 4);
}

#[test]
fn interval_matches_fsrs_rs_test_next_interval() {
    let decay = ANKI_DEFAULT_PARAMS[20];
    assert_eq!(interval_days_for_stability(1.0, 0.8, decay, 36_500), 3);
    assert_eq!(interval_days_for_stability(1.0, 0.9, decay, 36_500), 1);
}

#[test]
fn interval_is_bounded_and_handles_extreme_inputs() {
    assert_eq!(interval_days_for_stability(1e6, 0.7, 0.1542, 7), 7);
    assert_eq!(interval_days_for_stability(10.0, 0.9, 0.1542, i32::MIN), 1);
    assert_eq!(
        interval_days_for_stability(f32::NAN, 0.9, 0.1542, 36_500),
        1
    );
    assert_eq!(interval_days_for_stability(10.0, 0.9, f32::NAN, 36_500), 10);
}

#[test]
fn fsrs_params_defaults_match_anki() {
    let params = FsrsParams::default();
    assert_eq!(params.weights(), &ANKI_DEFAULT_PARAMS);
    assert_eq!(params.decay(), 0.1542);
}

#[test]
fn fsrs_params_from_fsrs45_matches_fsrs_rs_conversion() {
    let weights = [
        0.4, 0.6, 2.4, 5.8, 4.93, 0.94, 0.86, 0.01, 1.49, 0.14, 0.94, 2.18,
        0.05, 0.34, 1.26, 0.29, 2.61,
    ];
    let expected = [
        0.4,
        0.6,
        2.4,
        5.8,
        6.81,
        0.446_750_13,
        1.36,
        0.01,
        1.49,
        0.14,
        0.94,
        2.18,
        0.05,
        0.34,
        1.26,
        0.29,
        2.61,
        0.0,
        0.0,
        0.0,
        0.5,
    ];
    let params = FsrsParams::from_fsrs45(&weights).unwrap_or_default();
    for (index, expected_weight) in expected.iter().enumerate() {
        assert!(
            (params.weights()[index] - expected_weight).abs() < 1e-5,
            "weight {index} differs"
        );
    }
    assert_eq!(params.decay(), 0.5);
}

#[test]
fn fsrs_params_from_fsrs5_appends_short_term_and_decay() {
    let params =
        FsrsParams::from_fsrs5(&LEGACY_FSRS5_WEIGHTS).unwrap_or_default();
    assert_eq!(&params.weights()[..19], &LEGACY_FSRS5_WEIGHTS);
    assert_eq!(params.weights()[19], 0.0);
    assert_eq!(params.decay(), 0.5);
}

#[test]
fn fsrs_params_from_fsrs6_uses_weights_directly() {
    let params =
        FsrsParams::from_slice(&ANKI_DEFAULT_PARAMS).expect("default weights");
    assert_eq!(params, FsrsParams::default());
    assert_eq!(params.decay(), ANKI_DEFAULT_PARAMS[20]);
}

#[test]
fn fsrs_params_rejects_invalid_weights() {
    assert_eq!(FsrsParams::from_slice(&[]), Some(FsrsParams::default()));
    assert!(FsrsParams::from_slice(&[1.0]).is_none());
    let mut zero_decay = ANKI_DEFAULT_PARAMS;
    zero_decay[20] = 0.0;
    assert!(FsrsParams::from_fsrs6(&zero_decay).is_none());
    let mut non_finite = ANKI_DEFAULT_PARAMS;
    non_finite[3] = f32::NAN;
    assert!(FsrsParams::from_fsrs6(&non_finite).is_none());
}

#[test]
fn desired_retention_is_clamped_to_anki_range() {
    assert_eq!(clamp_desired_retention(0.5), MIN_DESIRED_RETENTION);
    assert_eq!(clamp_desired_retention(0.999), MAX_DESIRED_RETENTION);
    assert_eq!(clamp_desired_retention(f32::NAN), 0.9);
    assert_eq!(clamp_desired_retention(0.85), 0.85);
    let low = FsrsScheduler::new(fsrs_preset(false, 0.1));
    assert_eq!(low.preset().desired_retention, MIN_DESIRED_RETENTION);
    let high = FsrsScheduler::new(fsrs_preset(false, 10.0));
    assert_eq!(high.preset().desired_retention, MAX_DESIRED_RETENTION);
}

#[test]
fn desired_retention_changes_intervals_in_both_directions() {
    let low = interval_days_for_stability(30.0, 0.7, 0.1542, 36_500);
    let middle = interval_days_for_stability(30.0, 0.9, 0.1542, 36_500);
    let high = interval_days_for_stability(30.0, 0.99, 0.1542, 36_500);
    assert!(low > middle && middle > high);
}

#[test]
fn scheduler_applies_parameter_decay_to_intervals() {
    let legacy =
        FsrsParams::from_fsrs5(&LEGACY_FSRS5_WEIGHTS).unwrap_or_default();
    let anki = FsrsScheduler::new(fsrs_preset(false, 0.95));
    let old = anki.clone().with_params(legacy);
    let card = fsrs_card();
    let anki_interval =
        anki.preview(&card, Rating::Good, NOW, COLLECTION_START);
    let old_interval = old.preview(&card, Rating::Good, NOW, COLLECTION_START);
    assert_ne!(anki_interval, old_interval);
}

#[test]
fn scheduler_exposes_params_without_changing_its_preset() {
    let mut scheduler = FsrsScheduler::default();
    assert_eq!(scheduler.params(), &FsrsParams::default());
    let legacy =
        FsrsParams::from_fsrs5(&LEGACY_FSRS5_WEIGHTS).unwrap_or_default();
    scheduler.set_params(legacy.clone());
    assert_eq!(scheduler.params(), &legacy);
    assert_eq!(scheduler.preset(), &FsrsPreset::default());
}

#[test]
fn new_card_good_uses_the_next_learning_step() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let first =
        scheduler.schedule(&new_card(), Rating::Good, NOW, COLLECTION_START);
    assert_eq!(first.card.card_type, CardType::Learning);
    assert_eq!(
        first.card.due,
        NOW.saturating_mul(1_000).saturating_add(600_000),
        "good on a new card must use the second Anki step (10m)"
    );
    assert_eq!(first.revlog.interval, -600);
    assert_eq!(first.revlog.last_interval, 0);

    let second = scheduler.schedule(
        &first.card,
        Rating::Good,
        NOW.saturating_add(600),
        COLLECTION_START,
    );
    assert_eq!(second.card.card_type, CardType::Review);
    assert_eq!(second.revlog.last_interval, -600);
}

#[test]
fn learning_again_resets_to_the_first_step() {
    let scheduler = FsrsScheduler::new(FsrsPreset {
        learning_steps_minutes: vec![1, 20],
        ..fsrs_preset(false, 0.9)
    });
    let first =
        scheduler.schedule(&new_card(), Rating::Good, NOW, COLLECTION_START);
    assert_eq!(first.revlog.interval, -1_200);

    let again = scheduler.schedule(
        &first.card,
        Rating::Again,
        NOW.saturating_add(1_200),
        COLLECTION_START,
    );
    assert_eq!(again.revlog.interval, -60);
    assert_eq!(again.card.left, 1_002);

    let good = scheduler.schedule(
        &again.card,
        Rating::Good,
        NOW.saturating_add(1_260),
        COLLECTION_START,
    );
    assert_eq!(good.revlog.interval, -1_200);
    let graduated = scheduler.schedule(
        &good.card,
        Rating::Good,
        NOW.saturating_add(2_460),
        COLLECTION_START,
    );
    assert_eq!(graduated.card.card_type, CardType::Review);
}

#[test]
fn fsrs6_same_day_stability_matches_fsrs_rs_reference() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let card = Card {
        data: String::from(r#"{"s":5.0,"d":5.0}"#),
        ..fsrs_card()
    };
    for (rating, expected) in [
        (Rating::Again, 1.596_818_f64),
        (Rating::Hard, 5.0),
        (Rating::Good, 5.0),
        (Rating::Easy, 8.129_61),
    ] {
        let decision = scheduler.schedule(&card, rating, NOW, COLLECTION_START);
        let stability = stored_stability(&decision.card.data);
        assert!(
            (stability - expected).abs() < 0.001,
            "{rating:?} wrote {stability}, expected {expected}"
        );
    }
}

#[test]
fn review_intervals_keep_anki_button_order() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let card = fsrs_card();
    let interval = |rating| {
        scheduler
            .schedule(&card, rating, NOW, COLLECTION_START)
            .card
            .interval
    };
    let hard = interval(Rating::Hard);
    let good = interval(Rating::Good);
    let easy = interval(Rating::Easy);
    assert!(hard < good, "hard {hard}, good {good}");
    assert!(good < easy, "good {good}, easy {easy}");
}

#[test]
fn review_intervals_never_shrink_below_the_previous_interval() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let card = Card {
        interval: 1,
        ..fsrs_card()
    };
    let good = scheduler.schedule(&card, Rating::Good, NOW, COLLECTION_START);
    assert!(good.card.interval >= card.interval);
}

#[test]
fn fuzzing_keeps_the_answer_button_order() {
    let scheduler = FsrsScheduler::new(fsrs_preset(true, 0.9));
    for id in 1..=64 {
        let card = Card {
            id,
            interval: 4,
            ..fsrs_card()
        };
        let interval = |rating| {
            scheduler
                .schedule(&card, rating, NOW, COLLECTION_START)
                .card
                .interval
        };
        let hard = interval(Rating::Hard);
        let good = interval(Rating::Good);
        let easy = interval(Rating::Easy);
        assert!(hard < good, "card {id}: hard {hard}, good {good}");
        assert!(good < easy, "card {id}: good {good}, easy {easy}");
    }
}

#[test]
fn revlog_last_interval_uses_the_previous_card_interval() {
    let scheduler = FsrsScheduler::new(fsrs_preset(false, 0.9));
    let reviewed = Card {
        interval: 10,
        ..fsrs_card()
    };
    let again =
        scheduler.schedule(&reviewed, Rating::Again, NOW, COLLECTION_START);
    assert_eq!(again.revlog.last_interval, 10);

    let hard =
        scheduler.schedule(&new_card(), Rating::Hard, NOW, COLLECTION_START);
    let good = scheduler.schedule(
        &hard.card,
        Rating::Good,
        NOW.saturating_add(330),
        COLLECTION_START,
    );
    assert_eq!(good.revlog.last_interval, -330);
}
