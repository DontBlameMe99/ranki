//! Dependency-free FSRS optimizer and auto-optimization trigger policy.
//!
//! Anki only optimizes when the user opens deck options and clicks Optimize.
//! Ranki deliberately simplifies this: auto optimization runs on session
//! start, import and sync once 30 days and 1000 reviews have passed since the
//! last run, with at least 64 usable items and no run already in progress.
//!
//! The fitter follows fsrs-rs 6.6.2 training defaults: 8 epochs, batch 512,
//! seed 2023, learning rate 4e-2 with cosine annealing, gamma 1.0, maximum
//! sequence length 256, a 64-item floor and a 300-item health check. It fits
//! `w0..w3` from review outcomes and keeps `w4..w20` unchanged.
//!
//! This is not Anki equivalent. There is no full 21-weight fit, short-term
//! weight fit, recency weighting, time-series split or parameter clipper, and
//! the health check uses in-sample metrics. Callers can replace [`optimize`]
//! with `fsrs::compute_parameters` later without changing its interface.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use ranki_model::{Rating, RevLogKind};

use crate::fsrs::{ANKI_DEFAULT_PARAMS, FsrsParams};
use crate::shared::f32_from_f64;

/// Anki training epochs used by the pinned fsrs-rs version.
pub const EPOCHS: u32 = 8;
pub const BATCH_SIZE: usize = 512;
pub const RANDOM_SEED: u64 = 2023;
pub const LEARNING_RATE: f64 = 4e-2;
pub const L2_GAMMA: f64 = 1.0;
pub const MAX_SEQUENCE_LENGTH: usize = 256;
pub const MIN_TRAINING_ITEMS: usize = 64;
pub const HEALTH_CHECK_ITEMS: usize = 300;
pub const HEALTH_LOG_LOSS: f64 = 1.11;
pub const HEALTH_RMSE: f64 = 1.53;
pub const AUTO_OPTIMIZE_MIN_DAYS: u32 = 30;
pub const AUTO_OPTIMIZE_MIN_REVIEWS: u32 = 1000;

const STABILITY_MIN: f64 = 0.001;
const STABILITY_MAX: f64 = 100.0;
const PROBABILITY_EPS: f64 = 1e-6;

/// One review in a card history, in the order it was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewRecord {
    pub card_id: i64,
    pub rating: Rating,
    /// Whole days since the previous review of this card.
    pub elapsed_days: u32,
    pub kind: RevLogKind,
}

/// Input to [`optimize`]. Records need not be sorted by card.
#[derive(Clone, Copy, Debug)]
pub struct OptimizationInput<'records> {
    pub records: &'records [ReviewRecord],
    pub current_params: &'records FsrsParams,
}

/// Result of an optimization attempt. `params` stays unchanged on a no-op.
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizationReport {
    pub params: FsrsParams,
    pub items: usize,
    pub accepted: bool,
    pub log_loss_before: f64,
    pub log_loss_after: f64,
    /// `None` below the health-check threshold.
    pub health_check: Option<bool>,
}

/// State checked at session start, import and sync boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptimizationTrigger {
    pub days_since_last_optimization: u32,
    pub reviews_since_last_optimization: u32,
    pub usable_items: usize,
    pub optimization_running: bool,
}

impl OptimizationTrigger {
    #[must_use]
    pub const fn should_optimize(self) -> bool {
        should_auto_optimize(self)
    }
}

/// Cheap day-gate precheck that needs no review log.
///
/// Callers use it to skip loading review history while the policy cannot fire
/// yet. It is the day conjunct of [`should_auto_optimize`].
#[must_use]
pub const fn day_gate(today: u64, last_optimized: u64) -> bool {
    today.saturating_sub(last_optimized) >= AUTO_OPTIMIZE_MIN_DAYS as u64
}
/// Always-on auto-optimize policy, a deliberate simplification of Anki.
#[must_use]
pub const fn should_auto_optimize(trigger: OptimizationTrigger) -> bool {
    !trigger.optimization_running
        && trigger.days_since_last_optimization >= AUTO_OPTIMIZE_MIN_DAYS
        && trigger.reviews_since_last_optimization >= AUTO_OPTIMIZE_MIN_REVIEWS
        && trigger.usable_items >= MIN_TRAINING_ITEMS
}

/// Fits initial stabilities from review records without changing other weights.
#[must_use]
pub fn optimize(input: &OptimizationInput<'_>) -> OptimizationReport {
    let items = training_items(input.records);
    let decay = f64::from(input.current_params.decay());
    let before = log_loss(&items, input.current_params.weights(), decay);
    let can_fit = items.len() >= MIN_TRAINING_ITEMS
        && items.iter().any(|item| item.sequence_length > 1);
    if !can_fit {
        return OptimizationReport {
            params: input.current_params.clone(),
            items: items.len(),
            accepted: false,
            log_loss_before: before,
            log_loss_after: before,
            health_check: None,
        };
    }

    let candidate = fit(&items, input.current_params);
    let after = log_loss(&items, candidate.weights(), decay);
    let accepted = after < before;
    let params = if accepted {
        candidate
    } else {
        input.current_params.clone()
    };
    let health_check = health_check(&items, &params);
    OptimizationReport {
        params,
        items: items.len(),
        accepted,
        log_loss_before: before,
        log_loss_after: after,
        health_check,
    }
}

#[derive(Clone, Copy)]
struct TrainingItem {
    initial_rating: Rating,
    rating: Rating,
    elapsed_days: u32,
    sequence_length: usize,
}

fn training_items(records: &[ReviewRecord]) -> Vec<TrainingItem> {
    let mut grouped: BTreeMap<i64, Vec<ReviewRecord>> = BTreeMap::new();
    for record in records {
        if matches!(
            record.kind,
            RevLogKind::Learn | RevLogKind::Review | RevLogKind::Relearn
        ) {
            grouped.entry(record.card_id).or_default().push(*record);
        }
    }
    let mut items = Vec::new();
    for group in grouped.values() {
        let Some(initial) = group.first() else {
            continue;
        };
        let prefixes = group.iter().enumerate().take(MAX_SEQUENCE_LENGTH);
        for (index, record) in
            prefixes.filter(|(_, item)| item.elapsed_days > 0)
        {
            items.push(TrainingItem {
                initial_rating: initial.rating,
                rating: record.rating,
                elapsed_days: record.elapsed_days,
                sequence_length: index.saturating_add(1),
            });
        }
    }
    items
}

fn stability_index(rating: Rating) -> usize {
    usize::from(rating.code()).saturating_sub(1).min(3)
}

fn observed(rating: Rating) -> f64 {
    if rating == Rating::Again { 0.0 } else { 1.0 }
}

fn forgetting_factor(decay: f64) -> f64 {
    (-0.9_f64.ln() / decay).exp() - 1.0
}

fn retrievability(item: &TrainingItem, weights: &[f32], decay: f64) -> f64 {
    let index = stability_index(item.initial_rating);
    let stability = f64::from(weights.get(index).copied().unwrap_or(0.001))
        .max(STABILITY_MIN);
    let factor = forgetting_factor(decay);
    (1.0 + factor * f64::from(item.elapsed_days) / stability).powf(-decay)
}

fn count(items: &[TrainingItem]) -> f64 {
    f64::from(u32::try_from(items.len()).unwrap_or(u32::MAX)).max(1.0)
}

fn log_loss(items: &[TrainingItem], weights: &[f32], decay: f64) -> f64 {
    let mut total = 0.0;
    for item in items {
        let predicted = retrievability(item, weights, decay)
            .clamp(PROBABILITY_EPS, 1.0 - PROBABILITY_EPS);
        let actual = observed(item.rating);
        total -=
            actual * predicted.ln() + (1.0 - actual) * (1.0 - predicted).ln();
    }
    total / count(items)
}

fn root_mean_square_error(
    items: &[TrainingItem],
    weights: &[f32],
    decay: f64,
) -> f64 {
    let mut total = 0.0;
    for item in items {
        let difference =
            retrievability(item, weights, decay) - observed(item.rating);
        total += difference * difference;
    }
    (total / count(items)).sqrt()
}

fn batch_gradient(
    items: &[TrainingItem],
    weights: &[f32],
    decay: f64,
) -> [f64; 4] {
    let mut gradient = [0.0_f64; 4];
    let factor = forgetting_factor(decay);
    for item in items {
        let index = stability_index(item.initial_rating);
        let stability = f64::from(weights.get(index).copied().unwrap_or(0.001))
            .max(STABILITY_MIN);
        let predicted = retrievability(item, weights, decay)
            .clamp(PROBABILITY_EPS, 1.0 - PROBABILITY_EPS);
        let actual = observed(item.rating);
        let ratio = factor * f64::from(item.elapsed_days) / stability;
        let loss_slope =
            -(actual / predicted - (1.0 - actual) / (1.0 - predicted));
        let curve_slope = decay * predicted * ratio / (1.0 + ratio);
        if let Some(slot) = gradient.get_mut(index) {
            *slot += loss_slope * curve_slope * stability;
        }
    }
    let divisor = count(items);
    for (index, slot) in gradient.iter_mut().enumerate() {
        let prior =
            f64::from(ANKI_DEFAULT_PARAMS.get(index).copied().unwrap_or(0.001))
                .ln();
        let current =
            f64::from(weights.get(index).copied().unwrap_or(0.001)).ln();
        *slot = *slot / divisor + L2_GAMMA * (current - prior) / divisor;
    }
    gradient
}

fn update_weights(weights: &mut [f32], gradient: &[f64; 4], rate: f64) {
    for (index, slot) in weights.iter_mut().take(4).enumerate() {
        let slope = gradient.get(index).copied().unwrap_or(0.0);
        let current = f64::from(*slot).max(STABILITY_MIN);
        let updated = (current.ln() - rate * slope)
            .clamp(STABILITY_MIN.ln(), STABILITY_MAX.ln())
            .exp();
        *slot = f32_from_f64(updated.clamp(STABILITY_MIN, STABILITY_MAX));
    }
}

fn learning_rate(epoch: u32) -> f64 {
    let steps = f64::from(EPOCHS.saturating_sub(1));
    let progress = f64::from(epoch) / steps;
    LEARNING_RATE * 0.5 * (1.0 + (PI * progress).cos())
}

struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    const fn next_u64(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.0 = state;
        state
    }
}

fn shuffle<T>(items: &mut [T], rng: &mut Rng) {
    for index in (1..items.len()).rev() {
        let span = u64::from(u32::try_from(index).unwrap_or(u32::MAX))
            .saturating_add(1);
        let pick =
            usize::try_from(rng.next_u64().checked_rem(span).unwrap_or(0))
                .unwrap_or(0);
        items.swap(index, pick);
    }
}

fn fit(items: &[TrainingItem], current: &FsrsParams) -> FsrsParams {
    let mut weights = *current.weights();
    let mut order = items.to_vec();
    let mut rng = Rng::new(RANDOM_SEED);
    let decay = f64::from(current.decay());
    for epoch in 0..EPOCHS {
        shuffle(&mut order, &mut rng);
        let rate = learning_rate(epoch);
        for batch in order.chunks(BATCH_SIZE) {
            let gradient = batch_gradient(batch, &weights, decay);
            update_weights(&mut weights, &gradient, rate);
        }
    }
    FsrsParams::from_fsrs6(&weights).unwrap_or_else(|| current.clone())
}

fn health_check(items: &[TrainingItem], params: &FsrsParams) -> Option<bool> {
    if items.len() <= HEALTH_CHECK_ITEMS {
        return None;
    }
    let decay = f64::from(params.decay());
    let loss = log_loss(items, params.weights(), decay);
    let error = root_mean_square_error(items, params.weights(), decay);
    Some(loss <= HEALTH_LOG_LOSS || error <= HEALTH_RMSE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture describes one card pair"
    )]
    fn pair(
        card_id: i64,
        initial: Rating,
        outcome: Rating,
        days: u32,
    ) -> [ReviewRecord; 2] {
        [
            ReviewRecord {
                card_id,
                rating: initial,
                elapsed_days: 0,
                kind: RevLogKind::Learn,
            },
            ReviewRecord {
                card_id,
                rating: outcome,
                elapsed_days: days,
                kind: RevLogKind::Review,
            },
        ]
    }

    fn paired_dataset(count: usize, days: u32) -> Vec<ReviewRecord> {
        let mut records = Vec::new();
        for index in 0..count {
            let id = i64::try_from(index).unwrap_or(0);
            records.extend(pair(id, Rating::Good, Rating::Good, days));
        }
        records
    }

    fn report(records: &[ReviewRecord]) -> OptimizationReport {
        let current = FsrsParams::default();
        optimize(&OptimizationInput {
            records,
            current_params: &current,
        })
    }

    #[test]
    fn trigger_requires_thirty_days() {
        let base = OptimizationTrigger {
            days_since_last_optimization: 29,
            reviews_since_last_optimization: 1_000,
            usable_items: 64,
            optimization_running: false,
        };
        assert!(!base.should_optimize());
        assert!(
            OptimizationTrigger {
                days_since_last_optimization: 30,
                ..base
            }
            .should_optimize()
        );
        assert!(
            OptimizationTrigger {
                days_since_last_optimization: 31,
                ..base
            }
            .should_optimize()
        );
    }

    #[test]
    fn trigger_requires_one_thousand_reviews() {
        let base = OptimizationTrigger {
            days_since_last_optimization: 30,
            reviews_since_last_optimization: 999,
            usable_items: 64,
            optimization_running: false,
        };
        assert!(!should_auto_optimize(base));
        assert!(
            OptimizationTrigger {
                reviews_since_last_optimization: 1_000,
                ..base
            }
            .should_optimize()
        );
    }

    #[test]
    fn trigger_requires_sixty_four_items() {
        let base = OptimizationTrigger {
            days_since_last_optimization: 30,
            reviews_since_last_optimization: 1_000,
            usable_items: 63,
            optimization_running: false,
        };
        assert!(!base.should_optimize());
        assert!(
            OptimizationTrigger {
                usable_items: 64,
                ..base
            }
            .should_optimize()
        );
    }

    #[test]
    fn trigger_skips_a_running_optimization() {
        let trigger = OptimizationTrigger {
            days_since_last_optimization: 90,
            reviews_since_last_optimization: 5_000,
            usable_items: 500,
            optimization_running: true,
        };
        assert!(!trigger.should_optimize());
    }

    #[test]
    fn optimizer_constants_match_anki_defaults() {
        assert_eq!(EPOCHS, 8);
        assert_eq!(BATCH_SIZE, 512);
        assert_eq!(RANDOM_SEED, 2023);
        assert_eq!(LEARNING_RATE, 4e-2);
        assert_eq!(L2_GAMMA, 1.0);
        assert_eq!(MAX_SEQUENCE_LENGTH, 256);
        assert_eq!(MIN_TRAINING_ITEMS, 64);
        assert_eq!(HEALTH_CHECK_ITEMS, 300);
        assert_eq!(AUTO_OPTIMIZE_MIN_DAYS, 30);
        assert_eq!(AUTO_OPTIMIZE_MIN_REVIEWS, 1_000);
    }

    #[test]
    fn optimizer_is_a_noop_without_records() {
        let result = report(&[]);
        assert_eq!(result.items, 0);
        assert!(!result.accepted);
        assert_eq!(result.params, FsrsParams::default());
        assert_eq!(result.log_loss_before, 0.0);
        assert_eq!(result.log_loss_after, 0.0);
        assert_eq!(result.health_check, None);
    }

    #[test]
    fn optimizer_is_a_noop_below_sixty_four_items() {
        let result = report(&paired_dataset(63, 30));
        assert_eq!(result.items, 63);
        assert!(!result.accepted);
        assert_eq!(result.params, FsrsParams::default());
    }

    #[test]
    fn optimizer_is_a_noop_for_all_first_review_sets() {
        let records: Vec<ReviewRecord> = (0..80)
            .map(|index| ReviewRecord {
                card_id: index,
                rating: Rating::Good,
                elapsed_days: 10,
                kind: RevLogKind::Review,
            })
            .collect();
        let result = report(&records);
        assert_eq!(result.items, 80);
        assert!(!result.accepted);
        assert_eq!(result.params, FsrsParams::default());
    }

    #[test]
    fn optimizer_ignores_filtered_and_manual_reviews() {
        let mut records = paired_dataset(10, 30);
        for record in &mut records {
            record.kind = RevLogKind::Filtered;
        }
        let result = report(&records);
        assert_eq!(result.items, 0);
        assert!(!result.accepted);
    }

    fn long_history(card_id: i64) -> Vec<ReviewRecord> {
        let mut records = vec![ReviewRecord {
            card_id,
            rating: Rating::Good,
            elapsed_days: 0,
            kind: RevLogKind::Learn,
        }];
        for seq in 0_u32..260 {
            records.push(ReviewRecord {
                card_id,
                rating: Rating::Good,
                elapsed_days: seq.saturating_add(1),
                kind: RevLogKind::Review,
            });
        }
        records
    }

    #[test]
    fn optimizer_skips_same_day_reviews_and_caps_sequences() {
        let records: Vec<ReviewRecord> =
            (0..4).flat_map(long_history).collect();
        let items = training_items(&records);
        assert_eq!(items.len(), 4 * (MAX_SEQUENCE_LENGTH - 1));
        assert!(items.iter().all(|item| item.elapsed_days > 0));
    }

    #[test]
    fn optimizer_fits_stabilities_from_review_outcomes() {
        let result = report(&paired_dataset(80, 30));
        assert!(result.accepted);
        assert!(result.log_loss_after < result.log_loss_before);
        assert_eq!(result.items, 80);
        assert_eq!(result.health_check, None);
        assert_ne!(result.params, FsrsParams::default());
    }

    #[test]
    fn optimizer_changes_only_initial_stabilities() {
        let result = report(&paired_dataset(80, 30));
        let before = FsrsParams::default();
        for index in 4..21 {
            assert_eq!(result.params.weights()[index], before.weights()[index]);
        }
        assert_eq!(result.params.decay(), before.decay());
    }

    #[test]
    fn optimizer_is_deterministic() {
        let records = paired_dataset(100, 21);
        assert_eq!(report(&records), report(&records));
    }

    #[test]
    fn optimizer_runs_a_health_check_above_300_items() {
        assert_eq!(report(&paired_dataset(300, 30)).health_check, None);
        let result = report(&paired_dataset(301, 30));
        assert!(result.health_check.is_some());
    }

    #[test]
    fn optimizer_noop_keeps_custom_current_params() {
        let current = FsrsParams::from_fsrs5(&[1.0; 19]).unwrap_or_default();
        let result = optimize(&OptimizationInput {
            records: &[],
            current_params: &current,
        });
        assert!(!result.accepted);
        assert_eq!(result.params, current);
    }
}
