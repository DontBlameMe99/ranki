//! Anki code mappings and config fallback behavior.

use ranki_model::CardQueue;
use ranki_model::CardType;
use ranki_model::Rating;
use ranki_model::RevLogKind;
use ranki_model::ThemeChoice;
use ranki_model::config::parse;

#[test]
fn anki_codes_round_trip() {
    for (kind, code) in [
        (CardType::New, 0),
        (CardType::Learning, 1),
        (CardType::Review, 2),
        (CardType::Relearning, 3),
    ] {
        assert_eq!(kind.code(), code);
        assert_eq!(CardType::from_code(code), kind);
    }
    assert_eq!(CardType::from_code(99), CardType::New);

    for (queue, code) in [
        (CardQueue::New, 0),
        (CardQueue::Learning, 1),
        (CardQueue::Review, 2),
        (CardQueue::DayLearning, 3),
        (CardQueue::Suspended, -1),
        (CardQueue::Buried, -2),
        (CardQueue::Preview, -3),
    ] {
        assert_eq!(queue.code(), code);
        assert_eq!(CardQueue::from_code(code), queue);
    }
    assert_eq!(CardQueue::from_code(99), CardQueue::Suspended);
    assert!(CardQueue::DayLearning.is_due());
    assert!(!CardQueue::Suspended.is_due());
}

#[test]
fn rating_and_review_kinds_cover_every_code() {
    for (rating, code) in [
        (Rating::Again, 1),
        (Rating::Hard, 2),
        (Rating::Good, 3),
        (Rating::Easy, 4),
    ] {
        assert_eq!(rating.code(), code);
        assert_eq!(Rating::from_code(code), Some(rating));
    }
    assert_eq!(Rating::from_code(0), None);
    assert_eq!(Rating::from_code(5), None);

    for (kind, code) in [
        (RevLogKind::Learn, 0),
        (RevLogKind::Review, 1),
        (RevLogKind::Relearn, 2),
        (RevLogKind::Filtered, 3),
        (RevLogKind::Manual, 4),
    ] {
        assert_eq!(kind.code(), code);
        assert_eq!(RevLogKind::from_code(code), kind);
    }
    assert_eq!(RevLogKind::from_code(99), RevLogKind::Learn);
}

#[test]
fn invalid_config_values_fall_back_field_by_field() {
    let config = parse(
        "[general]\ncollection = ''\n[ui]\ntheme = ''\nshow_keyboard_hints = 'no'\n[scheduler]\ndesired_retention = 'no'\n",
    );
    assert_eq!(config.general.collection, None);
    assert_eq!(config.ui.theme, ThemeChoice::System);
    assert!(config.ui.show_keyboard_hints);
    assert_eq!(config.scheduler.desired_retention, 0.9);
}

#[test]
fn invalid_sections_keep_their_defaults() {
    let config = parse("[ui]\ntheme = 5\n");
    assert_eq!(config.ui.theme, ThemeChoice::System);

    let config = parse("ui = 5\n");
    assert_eq!(config.ui.theme, ThemeChoice::System);

    let config = parse("this is not TOML");
    assert_eq!(config.general.collection, None);
}
