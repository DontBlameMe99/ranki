use ranki_model::{
    Card, CardQueue, CardType, Rating, RevLogKind, ReviewDecision,
};

use crate::Scheduler;
use crate::shared::{
    ReviewContext, ReviewOutcome, StepConfig, as_f32, as_i32, due_from_days,
    factor_or_default, finish, is_schedulable, positive_factor,
    preview_interval, step_outcome, unchanged, unchanged_outcome,
};

const DEFAULT_MINIMUM_EASE: u32 = 1_300;
const DEFAULT_EASY_BONUS: f32 = 1.3;
const MAX_INTERVAL: i32 = 36_500;
const EASY_GRADUATION_INTERVAL: i32 = 4;

/// Settings for Anki's SM-2 scheduler.
#[derive(Clone, Debug, PartialEq)]
pub struct Sm2Preset {
    pub learning_steps_minutes: Vec<u32>,
    pub relearning_steps_minutes: Vec<u32>,
    pub minimum_ease: u32,
    pub easy_bonus: f32,
}

impl Default for Sm2Preset {
    fn default() -> Self {
        Self {
            learning_steps_minutes: vec![1, 10],
            relearning_steps_minutes: vec![10],
            minimum_ease: DEFAULT_MINIMUM_EASE,
            easy_bonus: DEFAULT_EASY_BONUS,
        }
    }
}

/// Anki-style SM-2 scheduler.
#[derive(Clone, Debug, PartialEq)]
pub struct Sm2Scheduler {
    preset: Sm2Preset,
}

struct Prepared {
    factor: u32,
    data: String,
}

impl Sm2Scheduler {
    #[must_use]
    pub const fn new(preset: Sm2Preset) -> Self {
        Self { preset }
    }

    #[must_use]
    pub const fn preset(&self) -> &Sm2Preset {
        &self.preset
    }

    fn review_interval(&self, card: &Card, rating: Rating) -> i32 {
        let factor =
            adjusted_factor(card.factor, self.preset.minimum_ease, rating);
        let bonus = positive_factor(self.preset.easy_bonus, DEFAULT_EASY_BONUS)
            .clamp(1.0, 5.0);
        let candidate = match card.reps {
            0 => first_interval(rating),
            1 => second_interval(rating, bonus),
            _ => later_interval(
                card.interval.max(0),
                factor,
                multiplier(rating, bonus),
            ),
        };
        as_i32(candidate.round()).clamp(1, MAX_INTERVAL)
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
                graduation_interval: graduation_interval(ctx.rating),
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
                    graduation_interval: graduation_interval(ctx.rating),
                    factor: prepared.factor,
                    data: prepared.data,
                },
            );
            outcome.lapses = 1;
            outcome.kind = RevLogKind::Review;
            return outcome;
        }
        let interval = self.review_interval(ctx.card, ctx.rating);
        ReviewOutcome {
            card_type: CardType::Review,
            queue: CardQueue::Review,
            due: due_from_days(
                ctx.now_secs,
                ctx.collection_start_secs,
                interval,
            ),
            interval,
            lapses: u32::from(ctx.rating == Rating::Again),
            left: 0,
            factor: prepared.factor,
            data: prepared.data,
            kind: RevLogKind::Review,
            step_seconds: None,
            last_interval: None,
        }
    }

    fn plan(&self, ctx: &ReviewContext<'_>) -> ReviewOutcome {
        if !is_schedulable(ctx.card) {
            return unchanged_outcome(ctx);
        }
        let prepared = Prepared {
            factor: adjusted_factor(
                ctx.card.factor,
                self.preset.minimum_ease,
                ctx.rating,
            ),
            data: ctx.card.data.clone(),
        };
        match ctx.card.card_type {
            CardType::New | CardType::Learning | CardType::Relearning => {
                self.schedule_steps(ctx, prepared)
            }
            CardType::Review => self.schedule_review(ctx, prepared),
        }
    }
}

impl Default for Sm2Scheduler {
    fn default() -> Self {
        Self::new(Sm2Preset::default())
    }
}

impl Scheduler for Sm2Scheduler {
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
        "sm2"
    }
}

fn graduation_interval(rating: Rating) -> i32 {
    if rating == Rating::Easy {
        EASY_GRADUATION_INTERVAL
    } else {
        1
    }
}

fn first_interval(rating: Rating) -> f32 {
    if rating == Rating::Easy { 4.0 } else { 1.0 }
}

fn second_interval(rating: Rating, bonus: f32) -> f32 {
    match rating {
        Rating::Hard => 3.0,
        Rating::Good => 6.0,
        Rating::Easy => 6.0 * bonus,
        Rating::Again => 1.0,
    }
}

const fn multiplier(rating: Rating, bonus: f32) -> f32 {
    match rating {
        Rating::Hard => 1.2,
        Rating::Easy => bonus,
        Rating::Good | Rating::Again => 1.0,
    }
}

fn later_interval(previous: i32, factor: u32, factor_multiplier: f32) -> f32 {
    as_f32(previous) * as_f32(i32::try_from(factor).unwrap_or(i32::MAX))
        / 1_000.0
        * factor_multiplier
}

fn adjusted_factor(card_factor: u32, minimum_ease: u32, rating: Rating) -> u32 {
    let current = factor_or_default(card_factor);
    let change = match rating {
        Rating::Again => -200,
        Rating::Hard => -150,
        Rating::Good => 0,
        Rating::Easy => 150,
    };
    let floor = i64::from(minimum_ease.max(1));
    let adjusted = i64::from(current)
        .saturating_add(change)
        .clamp(floor, i64::from(u32::MAX));
    u32::try_from(adjusted).unwrap_or(u32::MAX)
}
