#![expect(
    clippy::redundant_pub_crate,
    reason = "helpers are shared only with the crate root"
)]
//! Auto-optimization at session and sync boundaries.

use ranki_core::AUTO_OPTIMIZE_MIN_DAYS;
use ranki_core::FsrsParams;
use ranki_core::MIN_TRAINING_ITEMS;
use ranki_core::OptimizationInput;
use ranki_core::ReviewRecord;
use ranki_core::should_auto_optimize;
use ranki_model::Config;
use ranki_model::RevLog;
use ranki_model::RevLogKind;
use ranki_model::Store;

const SECONDS_PER_DAY: i64 = 86_400;
const MILLIS_PER_DAY: i64 = 86_400_000;

/// Runs one optimizer pass when the documented trigger policy allows it.
///
/// The config is updated with the fitted weights and reset counters when a
/// pass ran. The config-only part of the trigger policy is checked before the
/// review log is read, so the day gate rejects most sessions without a query.
/// Review counters refresh once that gate passes. Loading the whole log after
/// the gate costs one query per session, because the [`Store`] trait exposes
/// no review count.
pub(super) fn maybe_auto_optimize(
    store: &dyn Store,
    config: &mut Config,
    now_secs: i64,
) {
    if !config.scheduler.auto_optimize {
        return;
    }
    let today = day_number(now_secs);
    if today.saturating_sub(config.scheduler.last_optimized)
        < AUTO_OPTIMIZE_MIN_DAYS
    {
        return;
    }
    let history = match store.review_history() {
        Ok(history) => history,
        Err(error) => {
            eprintln!("ranki: auto-optimize: {error}");
            return;
        }
    };
    let records = review_records(&history);
    let usable_items = records
        .iter()
        .filter(|record| {
            record.elapsed_days > 0
                && matches!(
                    record.kind,
                    RevLogKind::Learn
                        | RevLogKind::Review
                        | RevLogKind::Relearn
                )
        })
        .count();
    let reviews_since =
        reviews_since(&history, config.scheduler.last_optimized);
    let trigger = ranki_core::OptimizationTrigger {
        days_since_last_optimization: today
            .saturating_sub(config.scheduler.last_optimized),
        reviews_since_last_optimization: reviews_since,
        usable_items,
        optimization_running: false,
    };
    config.scheduler.reviews_since = reviews_since;
    if !should_auto_optimize(trigger) {
        return;
    }
    let current = configured_params(config);
    let report = ranki_core::optimize(&OptimizationInput {
        records: &records,
        current_params: &current,
    });
    if report.items < MIN_TRAINING_ITEMS {
        return;
    }
    config.scheduler.params = report.params.weights().to_vec();
    config.scheduler.decay = report.params.decay();
    config.scheduler.last_optimized = today;
    config.scheduler.reviews_since = 0;
}

fn day_number(now_secs: i64) -> u32 {
    u32::try_from(now_secs.div_euclid(SECONDS_PER_DAY)).unwrap_or(u32::MAX)
}

fn reviews_since(history: &[RevLog], last_optimized: u32) -> u32 {
    let cutoff = u64::from(last_optimized)
        .saturating_mul(u64::try_from(MILLIS_PER_DAY).unwrap_or(u64::MAX));
    let count = history
        .iter()
        .filter(|log| u64::try_from(log.id).unwrap_or_default() > cutoff)
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn review_records(history: &[RevLog]) -> Vec<ReviewRecord> {
    let mut records = Vec::with_capacity(history.len());
    let mut previous: Option<(i64, i64)> = None;
    for log in history {
        let elapsed_days = previous
            .filter(|(card_id, _)| *card_id == log.card_id)
            .map_or(0, |(_, previous_id)| {
                let millis = log.id.saturating_sub(previous_id);
                u32::try_from(millis.div_euclid(MILLIS_PER_DAY))
                    .unwrap_or(u32::MAX)
            });
        previous = Some((log.card_id, log.id));
        records.push(ReviewRecord {
            card_id: log.card_id,
            rating: log.ease,
            elapsed_days,
            kind: log.kind,
        });
    }
    records
}

fn configured_params(config: &Config) -> FsrsParams {
    if config.scheduler.params.is_empty() {
        return FsrsParams::default();
    }
    let mut values = config.scheduler.params.clone();
    match values.len() {
        19 => values.push(config.scheduler.decay),
        21 => {
            if let Some(slot) = values.get_mut(20) {
                *slot = config.scheduler.decay;
            }
        }
        _ => {}
    }
    FsrsParams::from_slice(&values).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ranki_model::NewNote;
    use ranki_model::Rating;
    use ranki_model::RevLog;
    use ranki_model::RevLogKind;
    use ranki_model::ReviewDecision;
    use ranki_model::Store;
    use ranki_storage::Collection;

    use super::maybe_auto_optimize;

    const NOW: i64 = 2_000_000_000;

    fn collection_with_reviews(path: &Path, count: usize) -> Collection {
        let mut collection =
            Collection::create(&path.join("collection.anki2")).unwrap();
        let ids = collection
            .add_note(&NewNote {
                model_id: 1,
                deck_id: 1,
                fields: vec![String::from("front"), String::from("back")],
                tags: Vec::new(),
            })
            .unwrap();
        let card = collection.card(ids[0]).unwrap().unwrap();
        let base = NOW.saturating_mul(1_000);
        let decisions = (0..count)
            .map(|index| ReviewDecision {
                card: card.clone(),
                revlog: RevLog {
                    id: i64::try_from(index)
                        .unwrap_or(0)
                        .saturating_mul(super::MILLIS_PER_DAY)
                        .saturating_add(base),
                    card_id: card.id,
                    usn: 0,
                    ease: Rating::Good,
                    interval: 1,
                    last_interval: 0,
                    factor: 2_500,
                    time_ms: 100,
                    kind: RevLogKind::Review,
                },
            })
            .collect::<Vec<_>>();
        collection.apply_reviews(&decisions).unwrap();
        collection
    }

    #[test]
    fn trigger_requires_thirty_days() {
        let directory = tempfile::tempdir().unwrap();
        let collection = collection_with_reviews(directory.path(), 1_000);
        let mut config = ranki_model::Config::default();
        config.scheduler.last_optimized =
            super::day_number(NOW).saturating_sub(29);
        maybe_auto_optimize(&collection, &mut config, NOW);
        assert_eq!(config.scheduler.params.len(), 0);
        // The day gate returns before the review log is read.
        assert_eq!(config.scheduler.reviews_since, 0);

        config.scheduler.last_optimized =
            super::day_number(NOW).saturating_sub(30);
        maybe_auto_optimize(&collection, &mut config, NOW);
        assert_eq!(config.scheduler.params.len(), 21);
        assert_eq!(config.scheduler.last_optimized, super::day_number(NOW));
        assert_eq!(config.scheduler.reviews_since, 0);
    }

    #[test]
    fn trigger_requires_one_thousand_reviews() {
        let first = tempfile::tempdir().unwrap();
        let collection = collection_with_reviews(first.path(), 999);
        let mut config = ranki_model::Config::default();
        config.scheduler.last_optimized = 0;
        maybe_auto_optimize(&collection, &mut config, NOW);
        assert_eq!(config.scheduler.params.len(), 0);
        assert_eq!(config.scheduler.reviews_since, 999);

        let second = tempfile::tempdir().unwrap();
        let collection = collection_with_reviews(second.path(), 1_000);
        let mut config = ranki_model::Config::default();
        config.scheduler.last_optimized = 0;
        maybe_auto_optimize(&collection, &mut config, NOW);
        assert_eq!(config.scheduler.params.len(), 21);
    }

    #[test]
    fn disabled_optimizer_leaves_config_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let collection = collection_with_reviews(directory.path(), 1_000);
        let mut config = ranki_model::Config::default();
        config.scheduler.auto_optimize = false;
        let before = config.clone();
        maybe_auto_optimize(&collection, &mut config, NOW);
        assert_eq!(config, before);
    }

    #[test]
    fn optimizer_persists_configured_decay_and_params() {
        let directory = tempfile::tempdir().unwrap();
        let collection = collection_with_reviews(directory.path(), 1_000);
        let mut config = ranki_model::Config::default();
        config.scheduler.params = vec![2.0; 21];
        config.scheduler.decay = 0.4;
        config.scheduler.last_optimized = 0;
        maybe_auto_optimize(&collection, &mut config, NOW);
        assert_eq!(config.scheduler.params.len(), 21);
        assert!((0.1..=0.8).contains(&config.scheduler.decay));
        assert_eq!(config.scheduler.last_optimized, super::day_number(NOW));
        assert_eq!(config.scheduler.reviews_since, 0);
    }
}
