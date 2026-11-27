use ranki_model::{
    Card, CardQueue, CardType, DeckSteps, Rating, RevLogKind, ReviewDecision,
};
use serde_json::{Map, Value};

use crate::shared::{
    FuzzInput, ReviewContext, ReviewOutcome, StepConfig, as_f32, as_i32,
    due_from_days, elapsed_days_from, f32_from_f64, finish, fuzz_interval,
    is_schedulable, minimum_review_fuzz_interval, positive_factor,
    preview_interval, remaining_steps, step_delay_seconds, step_outcome,
    unchanged, unchanged_outcome,
};

/// Anki's default FSRS-6 parameters (21 weights, positive `w20` decay).
pub const ANKI_DEFAULT_PARAMS: [f32; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722,
    0.1666, 0.796, 1.4835, 0.0614, 0.2629, 1.6483, 0.6014, 1.8729, 0.5425,
    0.0912, 0.0658, 0.1542,
];

/// Decay used by FSRS-5 and the 17-weight FSRS-4.5 conversion.
pub const DEFAULT_DECAY: f32 = 0.5;
pub const DEFAULT_DESIRED_RETENTION: f32 = 0.9;
pub const MIN_DESIRED_RETENTION: f32 = 0.7;
pub const MAX_DESIRED_RETENTION: f32 = 0.99;

const FSRS4_PARAMS_LEN: usize = 17;
const FSRS5_PARAMS_LEN: usize = 19;
const FSRS6_PARAMS_LEN: usize = 21;
const S_MIN: f32 = 0.001;
const S_MAX: f32 = 36_500.0;
const D_MIN: f32 = 1.0;
const D_MAX: f32 = 10.0;

/// FSRS weights with the decay Anki derives from `w20`.
#[derive(Clone, Debug, PartialEq)]
pub struct FsrsParams {
    weights: [f32; FSRS6_PARAMS_LEN],
    decay: f32,
}

impl Default for FsrsParams {
    fn default() -> Self {
        Self::from_weights(ANKI_DEFAULT_PARAMS)
    }
}

impl FsrsParams {
    /// Accepts 0 (defaults), 17 (FSRS-4.5), 19 (FSRS-5) or 21 (FSRS-6) weights.
    #[must_use]
    pub fn from_slice(values: &[f32]) -> Option<Self> {
        match values.len() {
            0 => Some(Self::default()),
            FSRS4_PARAMS_LEN => Self::from_fsrs45(values.try_into().ok()?),
            FSRS5_PARAMS_LEN => Self::from_fsrs5(values.try_into().ok()?),
            FSRS6_PARAMS_LEN => Self::from_fsrs6(values.try_into().ok()?),
            _ => None,
        }
    }

    /// Converts fsrs-rs 17-weight FSRS-4.5 parameters to 21 weights.
    #[must_use]
    pub fn from_fsrs45(weights: &[f32; FSRS4_PARAMS_LEN]) -> Option<Self> {
        let w4 = *weights.get(4)?;
        let w5 = *weights.get(5)?;
        let w6 = *weights.get(6)?;
        let mut converted = [0.0_f32; FSRS6_PARAMS_LEN];
        converted
            .iter_mut()
            .zip(weights)
            .for_each(|(slot, value)| *slot = *value);
        *converted.get_mut(4)? = w4 + 2.0 * w5;
        *converted.get_mut(5)? = (3.0 * w5 + 1.0).ln() / 3.0;
        *converted.get_mut(6)? = w6 + 0.5;
        *converted.get_mut(20)? = DEFAULT_DECAY;
        Self::from_valid(converted)
    }

    /// Converts fsrs-rs 19-weight FSRS-5 parameters to 21 weights.
    #[must_use]
    pub fn from_fsrs5(weights: &[f32; FSRS5_PARAMS_LEN]) -> Option<Self> {
        let mut converted = [0.0_f32; FSRS6_PARAMS_LEN];
        converted
            .iter_mut()
            .zip(weights)
            .for_each(|(slot, value)| *slot = *value);
        *converted.get_mut(20)? = DEFAULT_DECAY;
        Self::from_valid(converted)
    }

    /// Uses fsrs-rs 21-weight FSRS-6 parameters directly.
    #[must_use]
    pub fn from_fsrs6(weights: &[f32; FSRS6_PARAMS_LEN]) -> Option<Self> {
        Self::from_valid(*weights)
    }

    #[must_use]
    pub const fn weights(&self) -> &[f32; FSRS6_PARAMS_LEN] {
        &self.weights
    }

    /// Positive decay derived from `w20` (0.5 for 17 and 19 weight models).
    #[must_use]
    pub const fn decay(&self) -> f32 {
        self.decay
    }

    /// Overrides the decay used by the interval formula and `w20`.
    #[must_use]
    pub fn with_decay(mut self, decay: f32) -> Self {
        let Some(slot) = self.weights.get_mut(FSRS6_PARAMS_LEN - 1) else {
            return self;
        };
        if decay.is_finite() && decay > 0.0 {
            self.decay = decay;
            *slot = decay;
        }
        self
    }

    fn from_weights(weights: [f32; FSRS6_PARAMS_LEN]) -> Self {
        let decay = weights.get(20).copied().unwrap_or(DEFAULT_DECAY);
        Self { weights, decay }
    }

    fn from_valid(weights: [f32; FSRS6_PARAMS_LEN]) -> Option<Self> {
        if weights.iter().any(|weight| !weight.is_finite()) {
            return None;
        }
        let decay = weights.get(20).copied()?;
        if decay <= 0.0 {
            return None;
        }
        Some(Self::from_weights(weights))
    }
}

/// Keeps desired retention inside Anki's preset range, falling back to 0.9.
#[must_use]
pub const fn clamp_desired_retention(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_DESIRED_RETENTION, MAX_DESIRED_RETENTION)
    } else {
        DEFAULT_DESIRED_RETENTION
    }
}

fn difficulty_factor(difficulty: f32) -> u32 {
    if !difficulty.is_finite() || difficulty <= 0.0 {
        return 0;
    }
    let shifted = ((difficulty - 1.0) / 9.0 + 0.1) * 1_000.0;
    let scaled = shifted.round().min(4_294_967_295.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "difficulty is clamped to a non-negative u32 range"
    )]
    let factor = scaled as u32;
    factor
}

fn forgetting_factor(decay: f32) -> f32 {
    (-0.9_f32.ln() / decay).exp() - 1.0
}

/// Raw Anki interval in days: `S / F * (DR^(-1/d) - 1)`.
#[must_use]
pub fn raw_interval_days_for_stability(
    stability: f32,
    desired_retention: f32,
    decay: f32,
) -> f32 {
    let effective_decay = positive_factor(decay, DEFAULT_DECAY);
    let factor = forgetting_factor(effective_decay);
    if !factor.is_finite() || factor <= 0.0 {
        return 0.0;
    }
    let bounded_stability = if stability.is_finite() {
        stability.clamp(S_MIN, S_MAX)
    } else {
        S_MIN
    };
    let retention = clamp_desired_retention(desired_retention);
    bounded_stability / factor * (retention.powf(-1.0 / effective_decay) - 1.0)
}

/// Rounded and bounded Anki interval without fuzz.
#[expect(
    clippy::too_many_arguments,
    reason = "the interval formula takes four scalar inputs"
)]
#[must_use]
pub fn interval_days_for_stability(
    stability: f32,
    desired_retention: f32,
    decay: f32,
    maximum_interval: i32,
) -> i32 {
    let raw =
        raw_interval_days_for_stability(stability, desired_retention, decay);
    let maximum = maximum_interval.max(1);
    if raw.is_nan() {
        return 1;
    }
    let rounded = if raw.is_infinite() {
        maximum
    } else {
        as_i32(raw.round())
    };
    rounded.clamp(1, maximum)
}

/// Settings for the FSRS-5 scheduler.
#[derive(Clone, Debug, PartialEq)]
pub struct FsrsPreset {
    pub desired_retention: f32,
    pub maximum_interval: i32,
    pub learning_steps_seconds: Vec<u32>,
    pub relearning_steps_seconds: Vec<u32>,
    pub enable_fuzzing: bool,
}

impl Default for FsrsPreset {
    fn default() -> Self {
        Self {
            desired_retention: 0.9,
            maximum_interval: 36_500,
            learning_steps_seconds: vec![60, 600],
            relearning_steps_seconds: vec![600],
            enable_fuzzing: true,
        }
    }
}

/// FSRS scheduler with model-driven memory state and deterministic fuzzing.
#[derive(Clone, Debug, PartialEq)]
pub struct FsrsScheduler {
    preset: FsrsPreset,
    params: FsrsParams,
}

struct Prepared {
    data: String,
    factor: u32,
    graduation: i32,
}

impl FsrsScheduler {
    #[must_use]
    pub fn new(preset: FsrsPreset) -> Self {
        let clamped = FsrsPreset {
            desired_retention: clamp_desired_retention(
                preset.desired_retention,
            ),
            ..preset
        };
        Self {
            preset: clamped,
            params: FsrsParams::default(),
        }
    }

    #[must_use]
    pub fn default_preset() -> FsrsPreset {
        FsrsPreset::default()
    }

    #[must_use]
    pub const fn preset(&self) -> &FsrsPreset {
        &self.preset
    }

    #[must_use]
    pub const fn params(&self) -> &FsrsParams {
        &self.params
    }

    pub const fn set_params(&mut self, params: FsrsParams) {
        self.params = params;
    }

    #[must_use]
    pub const fn with_params(mut self, params: FsrsParams) -> Self {
        self.params = params;
        self
    }

    /// Applies a deck's imported learning and relearning steps.
    #[must_use]
    pub fn with_steps(&self, steps: &DeckSteps) -> Self {
        let mut preset = self.preset.clone();
        preset.learning_steps_seconds.clone_from(&steps.learning);
        preset
            .relearning_steps_seconds
            .clone_from(&steps.relearning);
        Self {
            preset,
            params: self.params.clone(),
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Anki's button minimum needs the card state and both bounds"
    )]
    fn interval_minimum(
        index: usize,
        raw: f32,
        previous: u32,
        maximum: u32,
        earlier_days: u32,
    ) -> u32 {
        let minimum = minimum_review_fuzz_interval(raw, previous, maximum);
        if index < 2 {
            minimum.max(1).min(maximum)
        } else {
            minimum.max(earlier_days).min(maximum)
        }
    }

    fn rating_count(rating: Rating) -> usize {
        usize::from(rating.code())
            .saturating_sub(1)
            .clamp(0, 3)
            .saturating_add(1)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "shared interval computation needs the card, state, elapsed days and count"
    )]
    fn button_intervals(
        &self,
        ctx: &ReviewContext<'_>,
        current: Option<Memory>,
        elapsed: u32,
        needed: usize,
    ) -> ([i32; 4], Memory) {
        let maximum = u32::try_from(self.preset.maximum_interval.max(1))
            .unwrap_or(u32::MAX);
        let previous =
            u32::try_from(ctx.card.interval.max(0)).unwrap_or(u32::MAX);
        let ratings = [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy];
        let mut intervals = [0_i32; 4];
        let mut memory = initial_memory(&self.params, ctx.rating);
        let mut previous_interval = 0_i32;
        for (index, slot) in intervals.iter_mut().take(needed).enumerate() {
            let rating = ratings.get(index).copied().unwrap_or(ctx.rating);
            memory = current.map_or_else(
                || initial_memory(&self.params, rating),
                |memory| update_memory(&self.params, memory, elapsed, rating),
            );
            let raw = raw_interval_days_for_stability(
                memory.stability,
                self.preset.desired_retention,
                self.params.decay(),
            );
            let earlier = u32::try_from(previous_interval)
                .unwrap_or(maximum)
                .saturating_add(1)
                .min(maximum);
            let minimum =
                Self::interval_minimum(index, raw, previous, maximum, earlier);
            *slot = fuzz_interval(FuzzInput {
                interval: raw,
                minimum,
                maximum,
                card_id: ctx.card.id,
                reps: ctx.card.reps,
                enabled: self.preset.enable_fuzzing,
            });
            previous_interval = *slot;
        }
        (intervals, memory)
    }

    fn prepare(
        &self,
        ctx: &ReviewContext<'_>,
        card_data: CardData,
    ) -> Prepared {
        let needed = Self::rating_count(ctx.rating);
        let elapsed = card_data.elapsed_days(ctx.card.modified, ctx.now_secs);
        let CardData {
            object,
            memory: current,
        } = card_data;
        let (intervals, memory) =
            self.button_intervals(ctx, current, elapsed, needed);
        Prepared {
            data: write_memory(
                object,
                memory,
                MemoryWrite {
                    desired_retention: clamp_desired_retention(
                        self.preset.desired_retention,
                    ),
                    decay: self.params.decay(),
                    now_secs: ctx.now_secs,
                },
            ),
            factor: difficulty_factor(memory.difficulty),
            graduation: intervals
                .get(needed.saturating_sub(1))
                .copied()
                .unwrap_or(1)
                .max(1),
        }
    }

    fn plan(&self, ctx: &ReviewContext<'_>) -> ReviewOutcome {
        if !is_schedulable(ctx.card) {
            return unchanged_outcome(ctx);
        }
        let card_data = CardData::parse(&ctx.card.data);
        let prepared = self.prepare(ctx, card_data);
        match ctx.card.card_type {
            CardType::New | CardType::Learning | CardType::Relearning => {
                self.schedule_steps(ctx, prepared)
            }
            CardType::Review => self.schedule_review(ctx, prepared),
        }
    }

    fn schedule_steps(
        &self,
        ctx: &ReviewContext<'_>,
        prepared: Prepared,
    ) -> ReviewOutcome {
        let relearning = ctx.card.card_type == CardType::Relearning;
        let steps = if relearning {
            &self.preset.relearning_steps_seconds
        } else {
            &self.preset.learning_steps_seconds
        };
        step_outcome(
            ctx,
            StepConfig {
                steps,
                relearning,
                graduation_interval: prepared.graduation,
                factor: prepared.factor,
                data: prepared.data,
            },
        )
    }

    fn schedule_review(
        &self,
        ctx: &ReviewContext<'_>,
        prepared: Prepared,
    ) -> ReviewOutcome {
        let steps = &self.preset.relearning_steps_seconds;
        if ctx.rating == Rating::Again && !steps.is_empty() {
            let mut outcome = step_outcome(
                ctx,
                StepConfig {
                    steps,
                    relearning: true,
                    graduation_interval: prepared.graduation,
                    factor: prepared.factor,
                    data: prepared.data,
                },
            );
            outcome.lapses = 1;
            return outcome;
        }
        ReviewOutcome {
            card_type: CardType::Review,
            queue: CardQueue::Review,
            due: due_from_days(
                ctx.now_secs,
                ctx.collection_start_secs,
                prepared.graduation,
            ),
            interval: prepared.graduation,
            lapses: u32::from(ctx.rating == Rating::Again),
            left: 0,
            factor: prepared.factor,
            data: prepared.data,
            kind: if ctx.rating == Rating::Again {
                RevLogKind::Relearn
            } else {
                RevLogKind::Review
            },
            step_seconds: None,
            last_interval: None,
        }
    }
}

impl Default for FsrsScheduler {
    fn default() -> Self {
        Self::new(FsrsPreset::default())
    }
}

impl FsrsScheduler {
    /// Schedules `card` after `rating` at `now_secs` Unix seconds.
    ///
    /// Review dues are days since `collection_start_secs`.
    #[expect(
        clippy::too_many_arguments,
        reason = "the scheduling signature is shared by every caller"
    )]
    #[must_use]
    pub fn schedule(
        &self,
        card: &Card,
        rating: Rating,
        now_secs: i64,
        collection_start_secs: i64,
    ) -> ReviewDecision {
        let ctx = ReviewContext {
            card,
            rating,
            now_secs,
            collection_start_secs,
        };
        if !is_schedulable(card) {
            return unchanged(&ctx);
        }
        finish(&ctx, self.plan(&ctx))
    }

    /// Returns the interval a rating would produce without writing a log.
    ///
    /// Negative intervals are learning delays in seconds.
    #[expect(
        clippy::too_many_arguments,
        reason = "the preview signature mirrors schedule"
    )]
    #[must_use]
    pub fn preview(
        &self,
        card: &Card,
        rating: Rating,
        now_secs: i64,
        collection_start_secs: i64,
    ) -> i32 {
        let ctx = ReviewContext {
            card,
            rating,
            now_secs,
            collection_start_secs,
        };
        preview_interval(&self.plan(&ctx))
    }

    /// Returns the four rating intervals from one shared memory pass.
    #[expect(
        clippy::too_many_arguments,
        reason = "the batch preview signature mirrors preview"
    )]
    #[must_use]
    pub fn preview_all(
        &self,
        card: &Card,
        now_secs: i64,
        collection_start_secs: i64,
    ) -> [i32; 4] {
        if !is_schedulable(card) {
            return [card.interval; 4];
        }
        let ratings = [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy];
        let card_data = CardData::parse(&card.data);
        let elapsed = card_data.elapsed_days(card.modified, now_secs);
        let ctx = ReviewContext {
            card,
            rating: Rating::Again,
            now_secs,
            collection_start_secs,
        };
        let (intervals, _) = self.button_intervals(
            &ctx,
            card_data.memory,
            elapsed,
            ratings.len(),
        );
        let mut previews = [0_i32; 4];
        for ((preview, rating), interval) in
            previews.iter_mut().zip(ratings).zip(intervals)
        {
            let ctx = ReviewContext {
                card,
                rating,
                now_secs,
                collection_start_secs,
            };
            *preview = self.preview_interval_for(&ctx, interval.max(1));
        }
        previews
    }

    fn preview_interval_for(
        &self,
        ctx: &ReviewContext<'_>,
        graduation: i32,
    ) -> i32 {
        if ctx.card.card_type == CardType::Review
            && (ctx.rating != Rating::Again
                || self.preset.relearning_steps_seconds.is_empty())
        {
            return graduation;
        }
        let steps = if ctx.card.card_type == CardType::Review
            || ctx.card.card_type == CardType::Relearning
        {
            &self.preset.relearning_steps_seconds
        } else {
            &self.preset.learning_steps_seconds
        };
        step_preview_interval(ctx, steps, graduation)
    }
}

fn step_preview_interval(
    ctx: &ReviewContext<'_>,
    steps: &[u32],
    graduation: i32,
) -> i32 {
    let total = steps.len();
    let remaining = if ctx.card.card_type == CardType::New {
        total
    } else {
        remaining_steps(ctx.card.left, total).max(1)
    };
    step_delay_seconds(steps, remaining, ctx.rating)
        .map_or(graduation, |seconds| {
            i32::try_from(seconds).unwrap_or(i32::MAX).saturating_neg()
        })
}

#[derive(Clone, Copy)]
struct Memory {
    stability: f32,
    difficulty: f32,
}

struct CardData {
    object: Map<String, Value>,
    memory: Option<Memory>,
}

impl CardData {
    fn parse(data: &str) -> Self {
        let object: Map<String, Value> =
            serde_json::from_str(data).ok().unwrap_or_default();
        let memory = read_memory(&object);
        Self { object, memory }
    }

    fn elapsed_days(&self, modified: i64, now_secs: i64) -> u32 {
        let last = self
            .object
            .get("lrt")
            .and_then(Value::as_i64)
            .unwrap_or(modified);
        elapsed_days_from(last, now_secs)
    }
}

fn weight(params: &FsrsParams, index: usize) -> f32 {
    params.weights().get(index).copied().unwrap_or(0.0)
}

fn initial_difficulty(params: &FsrsParams, rating: Rating) -> f32 {
    let grade = f32::from(rating.code().saturating_sub(1));
    (weight(params, 4) - (weight(params, 5) * grade).exp() + 1.0)
        .clamp(D_MIN, D_MAX)
}

fn initial_memory(params: &FsrsParams, rating: Rating) -> Memory {
    let index = usize::from(rating.code()).saturating_sub(1);
    Memory {
        stability: weight(params, index).clamp(S_MIN, S_MAX),
        difficulty: initial_difficulty(params, rating),
    }
}

fn next_difficulty(
    params: &FsrsParams,
    difficulty: f32,
    rating: Rating,
) -> f32 {
    let grade = f32::from(rating.code());
    let delta = -weight(params, 6) * (grade - 3.0);
    let damped = difficulty + delta * (10.0 - difficulty) / 9.0;
    (weight(params, 7) * initial_difficulty(params, Rating::Easy)
        + (1.0 - weight(params, 7)) * damped)
        .clamp(D_MIN, D_MAX)
}

fn retention(params: &FsrsParams, stability: f32, days: u32) -> f32 {
    let elapsed = as_f32(i32::try_from(days).unwrap_or(i32::MAX)).max(0.0);
    let decay = positive_factor(params.decay(), DEFAULT_DECAY);
    let factor = forgetting_factor(decay);
    if !factor.is_finite() || factor <= 0.0 || stability <= 0.0 {
        return 1.0;
    }
    (1.0 + factor * elapsed / stability).powf(-decay)
}

#[expect(
    clippy::too_many_arguments,
    reason = "memory update needs the model, state, elapsed days and rating"
)]
fn recall_stability(
    params: &FsrsParams,
    memory: Memory,
    retrievability: f32,
    rating: Rating,
) -> f32 {
    let hard = if rating == Rating::Hard {
        weight(params, 15)
    } else {
        1.0
    };
    let easy = if rating == Rating::Easy {
        weight(params, 16)
    } else {
        1.0
    };
    let increase = weight(params, 8).exp()
        * (11.0 - memory.difficulty)
        * memory.stability.powf(-weight(params, 9))
        * ((1.0 - retrievability) * weight(params, 10)).exp_m1()
        * hard
        * easy;
    memory.stability * (1.0 + increase)
}

fn forget_stability(
    params: &FsrsParams,
    memory: Memory,
    retrievability: f32,
) -> f32 {
    let long_term = weight(params, 11)
        * memory.difficulty.powf(-weight(params, 12))
        * ((memory.stability + 1.0).powf(weight(params, 13)) - 1.0)
        * ((1.0 - retrievability) * weight(params, 14)).exp();
    let short_term_cap =
        memory.stability / (weight(params, 17) * weight(params, 18)).exp();
    long_term.min(short_term_cap)
}

fn same_day_stability(
    params: &FsrsParams,
    stability: f32,
    rating: Rating,
) -> f32 {
    let grade = f32::from(rating.code());
    let raw_factor = (weight(params, 17) * (grade - 3.0 + weight(params, 18)))
        .exp()
        * stability.powf(-weight(params, 19).max(0.0));
    let factor = if rating.code() >= 2 {
        raw_factor.max(1.0)
    } else {
        raw_factor
    };
    (stability * factor).clamp(S_MIN, S_MAX)
}

#[expect(
    clippy::too_many_arguments,
    reason = "memory update needs the model, state, elapsed days and rating"
)]
fn update_memory(
    params: &FsrsParams,
    memory: Memory,
    days: u32,
    rating: Rating,
) -> Memory {
    let difficulty = next_difficulty(params, memory.difficulty, rating);
    let stability = if days == 0 {
        same_day_stability(params, memory.stability, rating)
    } else {
        let retrievability = retention(params, memory.stability, days);
        let next = if rating == Rating::Again {
            forget_stability(params, memory, retrievability)
        } else {
            recall_stability(params, memory, retrievability, rating)
        };
        next.clamp(S_MIN, S_MAX)
    };
    Memory {
        stability,
        difficulty,
    }
}

const fn finite_f32(value: f64) -> Option<f32> {
    if value.is_finite() {
        Some(f32_from_f64(value))
    } else {
        None
    }
}

/// Reads the `s` and `d` fields Anki uses in `cards.data`.
fn read_memory(object: &Map<String, Value>) -> Option<Memory> {
    let stability = object.get("s")?.as_f64().and_then(finite_f32)?;
    let difficulty = object.get("d")?.as_f64().and_then(finite_f32)?;
    if stability <= 0.0 || difficulty <= 0.0 {
        return None;
    }
    Some(Memory {
        stability: stability.clamp(S_MIN, S_MAX),
        difficulty: difficulty.clamp(D_MIN, D_MAX),
    })
}

fn json_number(value: f32, decimals: usize) -> Value {
    let text = format!("{value:.decimals$}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}
fn rounded(value: f32, factor: f32) -> f32 {
    (value * factor).round() / factor
}

#[derive(Clone, Copy)]
struct MemoryWrite {
    desired_retention: f32,
    decay: f32,
    now_secs: i64,
}

fn write_memory(
    mut object: Map<String, Value>,
    memory: Memory,
    write: MemoryWrite,
) -> String {
    let stability = rounded(memory.stability, 10_000.0);
    let difficulty = rounded(memory.difficulty, 1_000.0);
    let retention =
        rounded(clamp_desired_retention(write.desired_retention), 100.0);
    let rounded_decay =
        rounded(positive_factor(write.decay, DEFAULT_DECAY), 1_000.0);
    let _ = object.insert(String::from("s"), json_number(stability, 4));
    let _ = object.insert(String::from("d"), json_number(difficulty, 3));
    let _ = object.insert(String::from("dr"), json_number(retention, 2));
    let _ = object.insert(String::from("decay"), json_number(rounded_decay, 3));
    let _ = object.insert(String::from("lrt"), Value::from(write.now_secs));
    serde_json::to_string(&object).unwrap_or_default()
}
