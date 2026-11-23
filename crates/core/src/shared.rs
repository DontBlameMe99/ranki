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
    pub interval: i32,
    pub maximum_interval: i32,
    pub card_id: i64,
    pub reps: u32,
    pub rating: Rating,
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

/// If a delay passes one day, Anki rounds it to whole days so the answer
/// is the same earlier and later in the day.
fn maybe_round_in_days(seconds: u32) -> u32 {
    const DAY: u32 = 86_400;
    if seconds <= DAY {
        return seconds;
    }
    let half = DAY.checked_div(2).unwrap_or(0);
    let days = seconds.saturating_add(half).checked_div(DAY).unwrap_or(0);
    days.saturating_mul(DAY)
}

fn first_step_hard_delay(steps: &[u32]) -> Option<u32> {
    let first = minutes_to_seconds(steps.first().copied().unwrap_or(1));
    let seconds = if let Some(second) = steps.get(1) {
        half_sum(u64::from(first), u64::from(minutes_to_seconds(*second)))?
    } else {
        let cap = first.saturating_add(86_400);
        half_sum(u64::from(first).saturating_mul(3), 0)?.min(cap)
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

/// The step delay the card was sitting on before this answer.
///
/// Anki writes it to `revlog.lastIvl` as negative seconds instead of the
/// card's zero `ivl`, which only exists once a card graduates.
fn learning_last_interval(
    ctx: &ReviewContext<'_>,
    steps: &[u32],
    total: usize,
) -> Option<i32> {
    match ctx.card.card_type {
        CardType::New => Some(0),
        CardType::Review => None,
        CardType::Learning | CardType::Relearning => {
            let remaining = remaining_steps(ctx.card.left, total).max(1);
            let index = total.saturating_sub(remaining);
            Some(steps.get(index).map_or(0, |minutes| {
                i32::try_from(minutes_to_seconds(*minutes))
                    .unwrap_or(i32::MAX)
                    .saturating_neg()
            }))
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
    let last_interval = learning_last_interval(ctx, config.steps, total);
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
            ReviewOutcome {
                card_type: if config.relearning {
                    CardType::Relearning
                } else {
                    CardType::Learning
                },
                queue: CardQueue::Learning,
                due: due_from_seconds(ctx.now_secs, seconds),
                interval: 0,
                lapses: 0,
                left: LEFT_STEP_MARKER.saturating_add(
                    u32::try_from(steps_left).unwrap_or(MAX_STEPS),
                ),
                factor: config.factor,
                data: config.data,
                kind,
                step_seconds: Some(seconds),
                last_interval,
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
            last_interval,
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
    ReviewDecision { card, revlog }
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
    ReviewDecision { card, revlog }
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

fn hash_interval(card_id: i64, reps: u32, rating: Rating) -> u64 {
    let id = u64::from_be_bytes(card_id.to_be_bytes());
    let seed = id
        ^ u64::from(reps).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ u64::from(rating.code()).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mix64(seed)
}

/// Deterministic FSRS fuzz derived from the card and this review.
pub fn fuzz_interval(input: FuzzInput) -> i32 {
    let maximum = input.maximum_interval.max(1);
    let interval = input.interval.clamp(1, maximum);
    if !input.enabled || interval < 3 {
        return interval;
    }
    let days = as_f32(interval);
    let mut delta = 1.0_f32;
    delta += 0.15 * (days.min(7.0) - 2.5).max(0.0);
    delta += 0.10 * (days.min(20.0) - 7.0).max(0.0);
    delta += 0.05 * (days - 20.0).max(0.0);
    let low = as_i32((days - delta).round()).clamp(2, maximum);
    let high = as_i32((days + delta).round()).clamp(low, maximum);
    if high <= low {
        return low;
    }
    let width = i64::from(high)
        .saturating_sub(i64::from(low))
        .saturating_add(1);
    let span = u64::try_from(width).unwrap_or(u64::MAX);
    let pick = hash_interval(input.card_id, input.reps, input.rating)
        .checked_rem(span)
        .unwrap_or(0);
    low.saturating_add(i32::try_from(pick).unwrap_or(0))
}
