//! Anki-compatible FSRS scheduling for cards.
//! Step delays use Unix seconds, while review intervals use day numbers.

mod fsrs;
mod optimizer;
pub(crate) mod shared;

#[cfg(test)]
mod tests;

pub use fsrs::{
    ANKI_DEFAULT_PARAMS, DEFAULT_DECAY, FsrsParams, FsrsPreset, FsrsScheduler,
    MAX_DESIRED_RETENTION, MIN_DESIRED_RETENTION, clamp_desired_retention,
    interval_days_for_stability, raw_interval_days_for_stability,
};
pub use optimizer::{
    AUTO_OPTIMIZE_MIN_DAYS, AUTO_OPTIMIZE_MIN_REVIEWS, BATCH_SIZE, EPOCHS,
    HEALTH_CHECK_ITEMS, L2_GAMMA, LEARNING_RATE, MAX_SEQUENCE_LENGTH,
    MIN_TRAINING_ITEMS, OptimizationInput, OptimizationReport,
    OptimizationTrigger, RANDOM_SEED, ReviewRecord, day_gate, optimize,
    should_auto_optimize,
};

use ranki_model::{Card, Settings, Store};

/// Builds the FSRS scheduler from resolved settings.
///
/// Anki defaults apply unless the settings provide parameters, decay, or a
/// desired retention.
#[must_use]
pub fn from_settings(settings: &Settings) -> FsrsScheduler {
    let params = FsrsParams::from_slice(&settings.fsrs_params)
        .unwrap_or_default()
        .with_decay(settings.fsrs_decay);
    FsrsScheduler::new(FsrsPreset {
        desired_retention: settings.desired_retention,
        ..FsrsPreset::default()
    })
    .with_params(params)
}

/// Applies the card's deck preset to a scheduler.
///
/// A store without deck presets keeps the scheduler's default steps.
#[must_use]
pub fn scheduler_for_card<S: Store + ?Sized>(
    scheduler: &FsrsScheduler,
    store: &S,
    card: &Card,
) -> FsrsScheduler {
    let steps = store.deck_steps(card.deck_id).unwrap_or_default();
    let max_taken = store.max_taken_secs(card.deck_id).unwrap_or(60);
    scheduler.with_steps(&steps).with_max_taken_secs(max_taken)
}
