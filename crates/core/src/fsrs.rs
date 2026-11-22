use ranki_model::{
    Card, CardQueue, CardType, Rating, RevLogKind, ReviewDecision,
};
use serde_json::{Map, Number, Value};

use crate::Scheduler;
use crate::shared::{
    FuzzInput, ReviewContext, ReviewOutcome, StepConfig, as_f32, as_i32,
    due_from_days, elapsed_days, f32_from_f64, factor_or_default, finish,
    fuzz_interval, is_schedulable, positive_factor, preview_interval,
    step_outcome, unchanged, unchanged_outcome,
};

/// FSRS-5 model weights and the fixed forgetting curve constants.
const WEIGHTS: [f32; 19] = [
    0.40255, 1.18385, 3.173, 15.69105, 7.1949, 0.5345, 1.4604, 0.0046, 1.54575,
    0.1192, 1.01925, 1.9395, 0.11, 0.29605, 2.2698, 0.2315, 2.9898, 0.51655,
    0.6621,
];
const DECAY: f32 = -0.5;
const FACTOR: f32 = 19.0 / 81.0;
const S_MIN: f32 = 0.001;
const S_MAX: f32 = 36_500.0;
const D_MIN: f32 = 1.0;
const D_MAX: f32 = 10.0;

/// Settings for the FSRS-5 scheduler.
#[derive(Clone, Debug, PartialEq)]
pub struct FsrsPreset {
    pub desired_retention: f32,
    pub maximum_interval: i32,
    pub learning_steps_minutes: Vec<u32>,
    pub relearning_steps_minutes: Vec<u32>,
    pub enable_fuzzing: bool,
}

impl Default for FsrsPreset {
    fn default() -> Self {
        Self {
            desired_retention: 0.9,
            maximum_interval: 36_500,
            learning_steps_minutes: vec![1, 10],
            relearning_steps_minutes: vec![10],
            enable_fuzzing: true,
        }
    }
}

/// FSRS-5 scheduler with deterministic interval fuzzing.
#[derive(Clone, Debug, PartialEq)]
pub struct FsrsScheduler {
    preset: FsrsPreset,
}

struct Prepared {
    data: String,
    factor: u32,
    graduation: i32,
}

#[derive(Clone, Copy)]
struct IntervalInput {
    stability: f32,
    card_id: i64,
    reps: u32,
    rating: Rating,
}

impl FsrsScheduler {
    #[must_use]
    pub const fn new(preset: FsrsPreset) -> Self {
        Self { preset }
    }

    #[must_use]
    pub fn default_preset() -> FsrsPreset {
        FsrsPreset::default()
    }

    #[must_use]
    pub const fn preset(&self) -> &FsrsPreset {
        &self.preset
    }

    fn interval_days(&self, input: IntervalInput) -> i32 {
        let retention = positive_factor(self.preset.desired_retention, 0.9)
            .clamp(0.5, 0.999);
        let raw =
            input.stability / FACTOR * (retention.powf(1.0 / DECAY) - 1.0);
        let maximum = self.preset.maximum_interval.max(1);
        let interval = as_i32(raw.round()).clamp(1, maximum);
        fuzz_interval(FuzzInput {
            interval,
            maximum_interval: self.preset.maximum_interval,
            card_id: input.card_id,
            reps: input.reps,
            rating: input.rating,
            enabled: self.preset.enable_fuzzing,
        })
    }

    fn prepare(&self, ctx: &ReviewContext<'_>) -> Prepared {
        let memory = read_memory(&ctx.card.data).map_or_else(
            || initial_memory(ctx.rating),
            |current| {
                update_memory(
                    current,
                    elapsed_days(ctx.card, ctx.now_secs),
                    ctx.rating,
                )
            },
        );
        let raw_retention = positive_factor(self.preset.desired_retention, 0.9);
        let graduation = self.interval_days(IntervalInput {
            stability: memory.stability,
            card_id: ctx.card.id,
            reps: ctx.card.reps,
            rating: ctx.rating,
        });
        Prepared {
            data: write_memory(&ctx.card.data, memory, raw_retention),
            factor: factor_or_default(ctx.card.factor),
            graduation,
        }
    }

    fn plan(&self, ctx: &ReviewContext<'_>) -> ReviewOutcome {
        if !is_schedulable(ctx.card) {
            return unchanged_outcome(ctx);
        }
        let prepared = self.prepare(ctx);
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
            &self.preset.relearning_steps_minutes
        } else {
            &self.preset.learning_steps_minutes
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
        let steps = &self.preset.relearning_steps_minutes;
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
        }
    }
}

impl Default for FsrsScheduler {
    fn default() -> Self {
        Self::new(FsrsPreset::default())
    }
}

impl Scheduler for FsrsScheduler {
    fn schedule(
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

    fn preview(
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

    fn name(&self) -> &'static str {
        "fsrs"
    }
}

#[derive(Clone, Copy)]
struct Memory {
    stability: f32,
    difficulty: f32,
}

fn weight(index: usize) -> f32 {
    WEIGHTS.get(index).copied().unwrap_or(0.0)
}

fn initial_difficulty(rating: Rating) -> f32 {
    let grade = f32::from(rating.code().saturating_sub(1));
    (weight(4) - (weight(5) * grade).exp() + 1.0).clamp(D_MIN, D_MAX)
}

fn initial_memory(rating: Rating) -> Memory {
    let index = usize::from(rating.code()).saturating_sub(1);
    Memory {
        stability: weight(index).clamp(S_MIN, S_MAX),
        difficulty: initial_difficulty(rating),
    }
}

fn next_difficulty(difficulty: f32, rating: Rating) -> f32 {
    let grade = f32::from(rating.code());
    let delta = -weight(6) * (grade - 3.0);
    let damped = difficulty + delta * (10.0 - difficulty) / 9.0;
    (weight(7) * initial_difficulty(Rating::Easy) + (1.0 - weight(7)) * damped)
        .clamp(D_MIN, D_MAX)
}

fn retention(stability: f32, days: u32) -> f32 {
    let elapsed = as_f32(i32::try_from(days).unwrap_or(i32::MAX)).max(0.0);
    (1.0 + FACTOR * elapsed / stability).powf(DECAY)
}

fn recall_stability(
    memory: Memory,
    retrievability: f32,
    rating: Rating,
) -> f32 {
    let hard = if rating == Rating::Hard {
        weight(15)
    } else {
        1.0
    };
    let easy = if rating == Rating::Easy {
        weight(16)
    } else {
        1.0
    };
    let increase = weight(8).exp()
        * (11.0 - memory.difficulty)
        * memory.stability.powf(-weight(9))
        * ((1.0 - retrievability) * weight(10)).exp_m1()
        * hard
        * easy;
    memory.stability * (1.0 + increase)
}

fn forget_stability(memory: Memory, retrievability: f32) -> f32 {
    let long_term = weight(11)
        * memory.difficulty.powf(-weight(12))
        * ((memory.stability + 1.0).powf(weight(13)) - 1.0)
        * ((1.0 - retrievability) * weight(14)).exp();
    let short_term_cap = memory.stability / (weight(17) * weight(18)).exp();
    long_term.min(short_term_cap)
}

fn same_day_stability(stability: f32, rating: Rating) -> f32 {
    let grade = f32::from(rating.code());
    let raw_factor = (weight(17) * (grade - 3.0 + weight(18))).exp();
    let factor = if rating.code() >= 2 {
        raw_factor.max(1.0)
    } else {
        raw_factor
    };
    (stability * factor).clamp(S_MIN, S_MAX)
}

fn update_memory(memory: Memory, days: u32, rating: Rating) -> Memory {
    let difficulty = next_difficulty(memory.difficulty, rating);
    let stability = if days == 0 {
        same_day_stability(memory.stability, rating)
    } else {
        let retrievability = retention(memory.stability, days);
        let next = if rating == Rating::Again {
            forget_stability(memory, retrievability)
        } else {
            recall_stability(memory, retrievability, rating)
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
fn read_memory(data: &str) -> Option<Memory> {
    let object: Map<String, Value> = serde_json::from_str(data).ok()?;
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

fn json_number(value: f32) -> Value {
    Value::Number(
        Number::from_f64(f64::from(value)).unwrap_or_else(|| Number::from(0)),
    )
}

fn write_memory(data: &str, memory: Memory, desired_retention: f32) -> String {
    let mut object: Map<String, Value> =
        serde_json::from_str(data).ok().unwrap_or_default();
    object.insert(String::from("s"), json_number(memory.stability));
    object.insert(String::from("d"), json_number(memory.difficulty));
    object.insert(
        String::from("dr"),
        json_number(desired_retention.clamp(0.5, 0.999)),
    );
    serde_json::to_string(&object).unwrap_or_default()
}
