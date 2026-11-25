use ranki_model::next_id;

use ranki_model::{
    Card, CardQueue, CardType, Rating, RevLog, RevLogKind, ReviewDecision,
};

use crate::{DEFAULT_FACTOR, LEFT_STEP_MARKER, SECONDS_PER_DAY};

const I32_MAX_F32: f32 = 2_147_483_647.0;
const I32_MIN_F32: f32 = -2_147_483_648.0;

const MAX_STEPS: u32 = u32::MAX - LEFT_STEP_MARKER;

pub struct ReviewContext<'card> {
    pub card: &'card Card,
    pub rating: Rating,
    pub now_secs: i64,
    pub collection_start_secs: i64,
}

pub struct ReviewOutcome {
    pub card_type: CardType,
    pub queue: CardQueue,
    pub due: i64,
    pub interval: i32,
    pub lapses: u32,
    pub left: u32,
    pub factor: u32,
    pub data: String,
    pub kind: RevLogKind,
    pub step_seconds: Option<u32>,
    /// Interval before this answer: days, or negative learning seconds.
    pub last_interval: Option<i32>,
}

pub struct StepConfig<'steps> {
    pub steps: &'steps [u32],
    pub relearning: bool,
    pub graduation_interval: i32,
    pub factor: u32,
    pub data: String,
}

#[derive(Clone, Copy)]
pub struct FuzzInput {
    /// Raw, unrounded interval in days.
    pub interval: f32,
    pub minimum: u32,
    pub maximum: u32,
    pub card_id: i64,
    pub reps: u32,

    pub enabled: bool,
}

/// Suspended, buried and preview cards are left untouched.
pub const fn is_schedulable(card: &Card) -> bool {
    card.queue.is_due()
}

pub const fn day_number(now_secs: i64, collection_start_secs: i64) -> i64 {
    now_secs
        .saturating_sub(collection_start_secs)
        .div_euclid(SECONDS_PER_DAY)
}

pub fn elapsed_days(card: &Card, now_secs: i64) -> u32 {
    let elapsed = now_secs.saturating_sub(card.modified);
    u32::try_from(elapsed.div_euclid(SECONDS_PER_DAY)).unwrap_or(u32::MAX)
}

pub fn due_from_days(
    now_secs: i64,
    collection_start_secs: i64,
    interval: i32,
) -> i64 {
    day_number(now_secs, collection_start_secs)
        .saturating_add(i64::from(interval))
}

pub fn due_from_seconds(now_secs: i64, seconds: u32) -> i64 {
    now_secs
        .saturating_mul(1_000)
        .saturating_add(i64::from(seconds).saturating_mul(1_000))
}

/// Converts Anki's packed `left` value into a step count.
pub fn remaining_steps(left: u32, total: usize) -> usize {
    let cap = u32::try_from(total).unwrap_or(u32::MAX);
    let packed = if left >= LEFT_STEP_MARKER {
        left.saturating_sub(LEFT_STEP_MARKER)
    } else {
        left
    };
    usize::try_from(packed.min(cap)).unwrap_or(total)
}

/// Step presets are minutes, while Anki stores step delays in seconds.
const fn minutes_to_seconds(minutes: u32) -> u32 {
    minutes.saturating_mul(60)
}

fn half_sum(first: u64, second: u64) -> Option<u32> {
    #[expect(
        clippy::integer_division,
        reason = "Anki averages adjacent learning steps"
    )]
    let half = first.saturating_add(second) / 2;
    u32::try_from(half).ok()
}

#[expect(
    clippy::integer_division,
    reason = "Anki rounds learning day delays to whole days"
)]
const fn maybe_round_in_days(seconds: u32) -> u32 {
    const DAY: u32 = 86_400;
    if seconds <= DAY {
        return seconds;
    }
    let half = DAY / 2;
    let days = seconds.saturating_add(half) / DAY;
    days.saturating_mul(DAY)
}

fn first_step_hard_delay(steps: &[u32]) -> Option<u32> {
    let first = minutes_to_seconds(steps.first().copied().unwrap_or(1));
    let seconds = if let Some(second) = steps.get(1) {
        half_sum(u64::from(first), u64::from(minutes_to_seconds(*second)))?
    } else {
        half_sum(u64::from(first).saturating_mul(3), 0)?
            .min(first.saturating_add(86_400))
    };
    Some(maybe_round_in_days(seconds))
}

/// `None` graduates the card; `Some` keeps it in a learning step.
pub fn step_delay_seconds(
    steps: &[u32],
    remaining: usize,
    rating: Rating,
) -> Option<u32> {
    if steps.is_empty() || remaining == 0 {
        return None;
    }
    let index = steps.len().saturating_sub(remaining);
    match rating {
        Rating::Easy => None,
        Rating::Again => steps.first().copied().map(minutes_to_seconds),
        Rating::Good => {
            if remaining <= 1 {
                None
            } else {
                steps
                    .get(index.saturating_add(1))
                    .copied()
                    .map(minutes_to_seconds)
            }
        }
        Rating::Hard => {
            if remaining >= steps.len() {
                first_step_hard_delay(steps)
            } else {
                steps.get(index).copied().map(minutes_to_seconds)
            }
        }
    }
}

pub fn step_outcome(
    ctx: &ReviewContext<'_>,
    config: StepConfig<'_>,
) -> ReviewOutcome {
    let total = config.steps.len();
    let remaining = if ctx.card.card_type == CardType::New {
        total
    } else {
        remaining_steps(ctx.card.left, total).max(1)
    };
    let kind = if config.relearning {
        RevLogKind::Relearn
    } else {
        RevLogKind::Learn
    };
    match step_delay_seconds(config.steps, remaining, ctx.rating) {
        Some(seconds) => {
            let steps_left = match ctx.rating {
                Rating::Again => total,
                Rating::Hard => remaining,
                Rating::Good => remaining.saturating_sub(1),
                Rating::Easy => 0,
            };
            let interval =
                i32::try_from(seconds).unwrap_or(i32::MAX).saturating_neg();
            ReviewOutcome {
                card_type: if config.relearning {
                    CardType::Relearning
                } else {
                    CardType::Learning
                },
                queue: CardQueue::Learning,
                due: due_from_seconds(ctx.now_secs, seconds),
                interval,
                lapses: 0,
                left: LEFT_STEP_MARKER.saturating_add(
                    u32::try_from(steps_left).unwrap_or(MAX_STEPS),
                ),
                factor: config.factor,
                data: config.data,
                kind,
                step_seconds: Some(seconds),
                last_interval: Some(ctx.card.interval),
            }
        }
        None => ReviewOutcome {
            card_type: CardType::Review,
            queue: CardQueue::Review,
            due: due_from_days(
                ctx.now_secs,
                ctx.collection_start_secs,
                config.graduation_interval,
            ),
            interval: config.graduation_interval,
            lapses: 0,
            left: 0,
            factor: config.factor,
            data: config.data,
            kind,
            step_seconds: None,
            last_interval: Some(ctx.card.interval),
        },
    }
}

/// Encodes the review interval the same way [`finish`] writes it to the log.
#[must_use]
pub fn preview_interval(outcome: &ReviewOutcome) -> i32 {
    outcome.step_seconds.map_or(outcome.interval, |seconds| {
        i32::try_from(seconds).unwrap_or(i32::MAX).saturating_neg()
    })
}

/// Outcome for a card that is not due for review.
#[must_use]
pub fn unchanged_outcome(ctx: &ReviewContext<'_>) -> ReviewOutcome {
    ReviewOutcome {
        card_type: ctx.card.card_type,
        queue: ctx.card.queue,
        due: ctx.card.due,
        interval: ctx.card.interval,
        lapses: ctx.card.lapses,
        left: ctx.card.left,
        factor: ctx.card.factor,
        data: ctx.card.data.clone(),
        kind: RevLogKind::Manual,
        step_seconds: None,
        last_interval: None,
    }
}
pub fn finish(
    ctx: &ReviewContext<'_>,
    outcome: ReviewOutcome,
) -> ReviewDecision {
    let interval = preview_interval(&outcome);
    let mut card = ctx.card.clone();
    card.modified = ctx.now_secs;
    card.usn = -1;
    card.card_type = outcome.card_type;
    card.queue = outcome.queue;
    card.due = outcome.due;
    card.interval = outcome.interval;
    card.factor = outcome.factor;
    card.reps = card.reps.saturating_add(1);
    card.lapses = card.lapses.saturating_add(outcome.lapses);
    card.left = outcome.left;
    card.data = outcome.data;

    let revlog = RevLog {
        id: next_id(ctx.now_secs.saturating_mul(1_000)),
        card_id: card.id,
        usn: -1,
        ease: ctx.rating,
        interval,
        last_interval: outcome
            .last_interval
            .unwrap_or_else(|| ctx.card.interval.max(0)),
        factor: outcome.factor,
        time_ms: 0,
        kind: outcome.kind,
    };
    ReviewDecision {
        card,
        revlog,
        expected_modified: ctx.card.modified,
        expected_usn: ctx.card.usn,
    }
}

/// Keeps an unschedulable card unchanged and records a manual log entry.
pub fn unchanged(ctx: &ReviewContext<'_>) -> ReviewDecision {
    let card = ctx.card.clone();
    let revlog = RevLog {
        id: next_id(ctx.now_secs.saturating_mul(1_000)),
        card_id: card.id,
        usn: -1,
        ease: ctx.rating,
        interval: card.interval,
        last_interval: card.interval.max(0),
        factor: card.factor,
        time_ms: 0,
        kind: RevLogKind::Manual,
    };
    ReviewDecision {
        card,
        revlog,
        expected_modified: ctx.card.modified,
        expected_usn: ctx.card.usn,
    }
}

pub const fn factor_or_default(factor: u32) -> u32 {
    if factor == 0 { DEFAULT_FACTOR } else { factor }
}

pub fn positive_factor(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "interval arithmetic only needs f32 precision"
)]
pub const fn as_f32(value: i32) -> f32 {
    value as f32
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "values are rounded and clamped before conversion"
)]
pub const fn as_i32(value: f32) -> i32 {
    value.clamp(I32_MIN_F32, I32_MAX_F32) as i32
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Anki stores memory state as f32"
)]
pub const fn f32_from_f64(value: f64) -> f32 {
    value as f32
}

const fn mix64(value: u64) -> u64 {
    let mut state = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    state = (state ^ (state >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    state = (state ^ (state >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    state ^ (state >> 31)
}

fn hash_interval(card_id: i64, reps: u32) -> u64 {
    let id = u64::from_be_bytes(card_id.to_be_bytes());
    let seed = id ^ u64::from(reps).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    mix64(seed)
}

fn as_u32(value: f32) -> u32 {
    u32::try_from(as_i32(value.round())).unwrap_or(0)
}

fn as_f32_u32(value: u32) -> f32 {
    as_f32(i32::try_from(value).unwrap_or(i32::MAX))
}

/// Anki's lower bound for a review interval, derived from its previous due day.
#[must_use]
pub fn minimum_review_fuzz_interval(
    interval: f32,
    previous_interval: u32,
    maximum_interval: u32,
) -> u32 {
    let maximum = maximum_interval.max(1);
    let upper =
        fuzz_bounds(interval.clamp(0.0, as_f32_u32(maximum)), 1, maximum).1;
    let rounded = as_u32(interval.round());
    if rounded > previous_interval {
        previous_interval.saturating_add(1).min(maximum)
    } else if previous_interval <= upper {
        previous_interval.min(maximum)
    } else {
        0
    }
}

fn fuzz_delta(interval: f32) -> f32 {
    if interval < 2.5 {
        return 0.0;
    }
    1.0 + 0.15 * (interval.min(7.0) - 2.5).max(0.0)
        + 0.10 * (interval.min(20.0) - 7.0).max(0.0)
        + 0.05 * (interval - 20.0).max(0.0)
}

fn fuzz_bounds(interval: f32, minimum: u32, maximum: u32) -> (u32, u32) {
    let delta = fuzz_delta(interval);
    let lower = as_u32(interval - delta).clamp(minimum, maximum);
    let mut upper = as_u32(interval + delta).clamp(minimum, maximum);
    if upper == lower && upper > 2 && upper < maximum {
        upper = upper.saturating_add(1);
    }
    (lower, upper)
}

/// Applies Anki's review fuzz to a raw interval, using a deterministic factor.
#[must_use]
pub fn fuzz_interval(input: FuzzInput) -> i32 {
    let maximum = input.maximum.max(1);
    let minimum = input.minimum.min(maximum);
    let raw = input
        .interval
        .clamp(as_f32_u32(minimum), as_f32_u32(maximum));
    if !input.enabled {
        return i32::try_from(as_u32(raw.round()).clamp(minimum, maximum))
            .unwrap_or(i32::MAX);
    }
    let (lower, upper) = fuzz_bounds(raw, minimum, maximum);
    let seed = hash_interval(input.card_id, input.reps)
        .checked_rem(1_000_000)
        .unwrap_or(0);
    let factor = as_f32_u32(u32::try_from(seed).unwrap_or(0)) / 1_000_000.0;
    let span = as_f32_u32(upper.saturating_sub(lower).saturating_add(1));
    let picked = (as_f32_u32(lower) + factor * span).floor();
    i32::try_from(as_u32(picked).clamp(minimum, maximum)).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{fuzz_bounds, fuzz_delta, minimum_review_fuzz_interval};

    #[test]
    fn review_fuzz_bounds_match_anki() {
        assert_eq!(fuzz_delta(2.49), 0.0);
        assert_eq!(fuzz_bounds(2.49, 1, 1000), (2, 2));
        assert_eq!(fuzz_bounds(2.5, 1, 1000), (2, 4));
        assert_eq!(fuzz_bounds(7.0, 1, 1000), (5, 9));
        assert_eq!(fuzz_bounds(17.0, 1, 1000), (14, 20));
        assert_eq!(fuzz_bounds(37.0, 1, 1000), (33, 41));
    }

    #[test]
    fn previous_interval_limits_the_fuzz_range() {
        assert_eq!(minimum_review_fuzz_interval(2.726_948_3, 4, 36_500), 4);
        assert_eq!(minimum_review_fuzz_interval(2.726_948_3, 5, 36_500), 0);
        assert_eq!(minimum_review_fuzz_interval(4.591_988, 4, 36_500), 5);
    }
}
