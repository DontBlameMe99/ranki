//! Scheduling algorithms for Anki-compatible cards.
//!
//! Step delays use Unix milliseconds, while review intervals use day numbers.

mod fsrs;
mod optimizer;
mod shared;
mod sm2;

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
pub use sm2::{Sm2Preset, Sm2Scheduler};

use std::sync::Arc;

use ranki_model::{Card, Rating, ReviewDecision, Settings};

/// Builds the scheduler configured in the resolved settings.
#[must_use]
pub fn from_settings(settings: &Settings) -> Arc<dyn Scheduler> {
    match settings.scheduler {
        ranki_model::SchedulerKind::Fsrs => {
            Arc::new(FsrsScheduler::new(FsrsPreset {
                desired_retention: settings.desired_retention,
                ..FsrsPreset::default()
            }))
        }
        ranki_model::SchedulerKind::Sm2 => {
            Arc::new(Sm2Scheduler::new(Sm2Preset::default()))
        }
    }
}

/// Chooses the next review for an answered card.
pub trait Scheduler: Send + Sync {
    /// Schedules `card` after a `rating` at `now_secs` Unix seconds.
    ///
    /// Review dues use days since `collection_start_secs`.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the scheduler signature is part of the public API"
    )]
    fn schedule(
        &self,
        card: &Card,
        rating: Rating,
        now_secs: i64,
        collection_start_secs: i64,
    ) -> ReviewDecision;

    /// Returns the interval a rating would produce without writing a log.
    ///
    /// Negative intervals are learning delays in seconds.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the preview signature mirrors schedule"
    )]
    fn preview(
        &self,
        card: &Card,
        rating: Rating,
        now_secs: i64,
        collection_start_secs: i64,
    ) -> i32;

    /// Stable name of the scheduling algorithm.
    #[must_use]
    fn name(&self) -> &'static str;
}

/// Length of a day in the Unix time scale used by Anki's day numbers.
const SECONDS_PER_DAY: i64 = 86_400;

/// Anki stores learning steps as `1000 + remaining steps` in `card.left`.
const LEFT_STEP_MARKER: u32 = 1_000;

/// Anki's default ease factor before a card has been graded.
const DEFAULT_FACTOR: u32 = 2_500;
