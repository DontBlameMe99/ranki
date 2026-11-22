//! Scheduling algorithms for Anki-compatible cards.
//!
//! Step delays use Unix milliseconds, while review intervals use day numbers.

mod fsrs;
mod shared;
mod sm2;

#[cfg(test)]
mod tests;

pub use fsrs::{FsrsPreset, FsrsScheduler};
pub use sm2::{Sm2Preset, Sm2Scheduler};

use ranki_model::{Card, Rating, ReviewDecision};

/// Chooses the next review for an answered card.
pub trait Scheduler {
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
