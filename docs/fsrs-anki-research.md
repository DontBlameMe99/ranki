# FSRS desired retention and optimizer research for Ranki

Scope: Anki source at pinned commit `be56085c32a9d02b506fa880cfa63c756c1779fc`, which pins `fsrs` crate version `6.6.2`. Ranki files read: `crates/core/src/{lib,fsrs,shared,tests}.rs`, `crates/model/src/config.rs`, `crates/bin/src/config.rs`, `docs/anki-compatibility.md`, `docs/card-types-research.md`.

## 1. What Ranki does today

`FsrsScheduler` uses one fixed FSRS-5 weight array of 19 values, a fixed decay of 0.5, and a fixed factor of `19/81`. `desired_retention` comes from global TOML config, defaults to 0.9, and is validated to `0.5..=0.999`. The scheduler clamps it to the same range. There is no per-deck retention, no parameter storage, no optimizer, and no load balancing. `write_memory` stores `s`, `d`, and `dr` in `cards.data`, but not `decay`. Fuzzing uses a local hash of card id, reps, and rating. See `crates/core/src/fsrs.rs:16`, `crates/core/src/shared.rs:303`, `crates/model/src/config.rs:249`.

## 2. How desired retention feeds Anki's interval

Anki uses the Rust `fsrs` crate. Parameter length selects the model (fsrs-rs `model.rs`, `check_and_fill_parameters`): 17 values convert to 21 (FSRS-4.5), 19 values append decay 0.5 (FSRS-5), 21 values are used directly (FSRS-6). An empty list uses defaults. Model defaults are:

```text
[0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001,
 1.8722, 0.1666, 0.796, 1.4835, 0.0614, 0.2629, 1.6483, 0.6014,
 1.8729, 0.5425, 0.0912, 0.0658, 0.1542]
```

`w20` is stored positive. fsrs-rs sets a local `decay = -w20`, so the equivalent positive-decay curve is:

```text
d      = w20                       # positive, default 0.1542 for FSRS-6
F      = 0.9^(-1/d) - 1            # 0.98034649 for d=0.1542
R(t)   = (1 + F*t/S)^(-d)
I_raw  = S/F * (DR^(-1/d) - 1)     # DR in (0,1)
```

fsrs-rs writes the same calculation as `decay = -w[20]`, `factor = exp(ln(0.9)/decay) - 1`, `I_raw = S/factor * (DR^(1/decay) - 1)`. Stability is clamped to `0.001..36500` days and difficulty to `1..10`.

Exact Anki steps after answering a review card:

1. `next_states` advances memory for each rating using elapsed days, then calls `next_interval` for each resulting stability. `next_interval` returns an unrounded `f32`.
2. `passing_fsrs_review_intervals` computes a minimum for Hard of `max(1, minimum_review_fuzz_interval(hard_raw, previous_interval, max_ivl))`. Good uses `max(hard + 1, ...)`, Easy uses `max(good + 1, ...)`. This enforces strictly increasing Hard, Good, and Easy day counts.
3. `constrain_passing_interval` calls `with_review_fuzz` with that minimum and the configured `maxIvl`, default 36500. With no fuzz factor it returns `round(interval).clamp(minimum, maximum)`.
4. Fuzz starts at 2.5 days. Delta is `1 + 0.15*(min(I,7)-2.5).max(0) + 0.10*(min(I,20)-7).max(0) + 0.05*(I-20).max(0)`. Bounds are `round(I-delta)` and `round(I+delta)`, clamped to the minimum and maximum. Result is `floor(lower + fuzz_factor*(1 + upper - lower))`. When load balancing is enabled, Anki asks the load balancer for a due day inside those bounds instead.
5. On config or retention changes with rescheduling on, `update_memory_state` computes `I_raw` from stored stability with rating 0 and the card's cached `dr`, applies the same minimum and fuzz or load balancer, writes a new due date, and logs a rescheduled review. Only type Review and non-suspended cards with a last review are rescheduled.

Verified numeric vectors, fuzz off:

```text
FSRS-5 d=0.5, F=19/81:
  S=10, DR=0.90 -> 10.0000 -> 10 days
  S=10, DR=0.80 -> 23.9803 -> 24 days
  S=10, DR=0.95 ->  4.6056 ->  5 days

FSRS-6 d=0.1542, F=0.98034649:
  S=10, DR=0.90 -> 10.0000 -> 10 days
  S=10, DR=0.95 ->  4.0256 ->  4 days

fsrs-rs v6.6.2 test_next_interval, default params, S=1, round().max(1):
  DR=0.8 -> 3, DR=0.9 -> 1
```

The fsrs-rs vector is in `src/inference.rs`, `test_next_interval`. The other values use the published formula in f64. f32 arithmetic can shift boundary cases by one day.

Sources:
`https://github.com/open-spaced-repetition/fsrs-rs/blob/v6.6.2/src/model.rs`
`https://github.com/open-spaced-repetition/fsrs-rs/blob/v6.6.2/src/inference.rs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/scheduler/states/review.rs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/scheduler/states/fuzz.rs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/scheduler/fsrs/memory_state.rs`

## 3. Desired retention value and change behavior

Default desired retention is 0.9. Preset validation is `0.7..=0.99` in `deckconfig/mod.rs`, and the UI spin box uses `min=0.7 max=0.99`. Warnings appear below 0.8 or above 0.95, with danger styling below 0.7 or above 0.97. The manual recommends staying below 0.97. Historical retention is separate, default 0.9 and validated to `0.7..=0.97`.

A normal deck may override the preset value. Schema 11 stores the override in `decks.desiredRetention` as an integer percent; Anki converts it with `/100`. Effective retention is deck override if present, otherwise preset `desiredRetention`.

When desired retention changes, Anki updates the cached `dr` and `decay` values on affected cards. Due dates change only when "Reschedule cards on change" is enabled. The default is off, so future reviews use the new retention and existing workload stays stable. With rescheduling on, Anki recomputes eligible review cards, writes new due dates, and adds rescheduled revlog entries. Learning and relearning cards are not rescheduled by this path. Load balancing is a separate global setting, `loadBalancerEnabled`, default true. It only selects a due day within the computed fuzz range.

Source:
`https://docs.ankiweb.net/deck-options.html#fsrs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/ts/routes/deck-options/FsrsOptions.svelte`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/deckconfig/mod.rs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/decks/schema11.rs`

## 4. Anki's FSRS optimizer in practice

Anki does not run the optimizer automatically. There is no sync trigger, daily job, or review-count job. The deck options screen shows a reminder when `daysSinceLastOptimization > 30`. The user clicks "Optimize" for a preset or "Save and Optimize" for all presets. Only the optimize-all path writes `lastFsrsOptimize` in the pinned source. Manual docs recommend optimization about once per month.

Reviews used by one preset:

- Default search is `preset:"<preset name>" -is:suspended`, over all decks assigned to the preset.
- `revlog_for_srs` reads revlogs for matching cards. `ignoreRevlogsBeforeDate` can cut off older history.
- `reviews_for_fsrs` skips cramming reviews, drops manual or rescheduled entries that do not affect scheduling, and drops history before a card reset. Training requires a last group of learning entries after the cutoff.
- Each training item is an expanding prefix of a card's history with `delta_t` in whole days relative to the next day boundary. Only prefixes ending in a review with `delta_t > 0` become training items.
- fsrs-rs applies recency weights and drops items with more than `max_seq_len` reviews.

Exact thresholds and constants:

```text
fsrs-rs version                    6.6.2
Anki training epochs               8
fsrs-rs default epochs             5
batch size                         512
random seed                        2023
learning rate                      4e-2, cosine annealing
gamma (L2)                         1.0
max sequence length                256
No training if train_set < 64 items, or all items are first-review only
Zero training items return current params
Health check only when train_set > 300 items
Health check pass: adjusted_log_loss <= 1.11 OR adjusted_rmse <= 1.53
30 day UI reminder threshold
FSRS_BATCH_SIZE=1000 is a memory-state batch size, not an optimizer gate
```

There is no 100 or 1000 review minimum in this source. Older Anki versions used 1000, then 400, as documented in the FSRS4Anki tutorial. Those are historical limits.

Parameter fit:

- Anki calls `fsrs::compute_parameters` with `enable_short_term: true` and the preset's sub-1-day relearning step count.
- It fits all 21 values for FSRS-6, including `w17`, `w18`, `w19`, and `w20`. `w20` is positive decay, clamped to `0.1..0.8`.
- Clamps: `w0..w3` in `0.001..100`, `w4` in `1..10`, `w5..w6` in `0.001..4`, `w7` in `0.001..0.75`, `w8` in `0..4.5`, `w9` in `0..0.8`, `w10` in `0.001..3.5`, `w11` in `0.001..5`, `w12` in `0.001..0.25`, `w13` in `0.001..0.9`, `w14` in `0..4`, `w15` in `0..1`, `w16` in `1..6`, `w17` and `w18` in `0..ceiling`, `w19` in `0.01..0.8`, `w20` in `0.1..0.8`. The ceiling is 2, or a smaller value derived from relearning steps.
- After fitting, Anki compares log loss of current and optimized params on the same training items. If current is no worse, it keeps current, with an extra short-term stability check for more than one relearning step.
- Optional time-series health check uses 5 splits and adjusted metrics.

Anki depends on the Rust `fsrs` crate, not the Python `fsrs-optimizer` repository. Sources:
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/Cargo.toml`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/scheduler/fsrs/params.rs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/deckconfig/update.rs`
`https://github.com/open-spaced-repetition/fsrs-rs/blob/v6.6.2/src/training.rs`
`https://github.com/open-spaced-repetition/fsrs-rs/blob/v6.6.2/src/parameter_clipper.rs`
`https://github.com/open-spaced-repetition/fsrs4anki/blob/main/docs/tutorial.md`

## 5. Config and storage keys

Schema 11 JSON keys, all preserved through migration:

- `col.conf`: `fsrs` boolean, `fsrsHealthCheck`, `loadBalancerEnabled`, `lastFsrsOptimize` day number. `lastFsrsOptimize` comes from `I32ConfigKey` with camelCase serialization.
- `col.dconf` per preset: `fsrsWeights` (FSRS-4.5), `fsrsParams5` (19 floats), `fsrsParams6` (21 floats), `desiredRetention`, `sm2Retention`, `weightSearch`, `ignoreRevlogsBeforeDate`, `maxIvl`. `fsrsParams6` wins, then `fsrsParams5`, then `fsrsWeights`.
- `col.decks` per deck: `desiredRetention` as integer percent.
- `cards.data`: `s` (stability, 4 decimals), `d` (difficulty, 3), `dr` (desired retention, 2), `decay` (3), `lrt` (last review time), `cd` (custom data). Anki caches `dr` and `decay` for search, stats, and add-ons. Scheduler uses params and preset or deck config, not the cached values.
- Schema 18 moves config into normalized `config`, `deck_config`, and `decks` tables. Writing only `col.conf` or `col.dconf` has no effect there.

Sources:
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/config/number.rs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/config/bool.rs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/deckconfig/schema11.rs`
`https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/rslib/src/storage/card/data.rs`

## 6. Minimal "always auto-optimize" for Ranki

Proposed trigger, close to Anki's cadence and cheap to implement:

1. Config flag `[scheduler] auto_optimize = true` by default.
2. On session start and after import or sync, if `today - last_optimize_day >= 30` and at least 1000 reviews exist since the last optimization, start one synchronous optimization on a worker thread. The 1000 review count is a conservative policy choice, not an Anki threshold. Fall back to 64 usable FSRS items if a smaller gate is wanted.
3. Do nothing when params are missing and fewer than 64 usable FSRS items exist. Use default FSRS-6 params until the first successful run.
4. On success, persist 21 floats, `last_optimize_day`, and reset the review counter. Keep the old params on failure. Do not reschedule existing cards.
5. Publish progress and cancellation through the existing review loop. Only one optimization may run at a time.

Simplification and compatibility loss:

- A lightweight implementation that estimates only initial stabilities `w0..w3` from first-review recall and keeps `w4..w20` at defaults is not Anki-equivalent.
- No log-loss guard, health check, time-series split, recency weighting, or clipper means worse parameters can be accepted.
- No per-preset or per-deck params means one fit is shared by the whole collection.
- No rescheduling means existing due dates keep their old intervals until reviewed.
- No load balancing means due days use deterministic fuzz only.

If exact optimization compatibility matters later, replace the lightweight fitter with the Rust `fsrs` crate's `compute_parameters`. Do not add the Python optimizer.

## 7. Concrete Ranki integration plan

ranki-core:

1. Add `FsrsParams` holding `Vec<f32>` or `[f32; 21]`, plus `from_slice` accepting 0, 17, 19, or 21 values with fsrs-rs conversion rules. Expose `decay()` as positive `w20` and defaults as the 21 Anki values.
2. Replace `WEIGHTS`, `DECAY`, and `FACTOR` in `fsrs.rs` with model-driven values. Add a pure `interval_days_for_stability(stability, desired_retention, params, maximum_interval)` that implements the formula and bounds without fuzz. Keep fuzz as a separate step.
3. Add `src/optimizer.rs` with `OptimizationInput`, `OptimizationReport`, `OptimizationTrigger`, and a `fit_initial_stabilities` function. Add a trait so a full fsrs-rs fitter can replace it later.
4. Add positive `decay` to `write_memory` and read it from `cards.data`. Keep `dr` on write.
5. Add a `maybe_auto_optimize` call at session start, import, and sync boundaries. It should take the current review count and collection data, run the fitter off the UI thread, and return params for persistence.

ranki-model and ranki-bin config:

6. Extend `SchedulerConfig` with `auto_optimize: bool`, `fsrs_params: Vec<f32>`, `last_optimize_day: u32`, and `reviews_since_optimize: u32`. Add all keys to `SCHEDULER_KEYS`. Validate retention to `0.7..=0.99` to match Anki, while accepting old `0.5..0.999` files as 0.9 with a warning if needed.
7. Carry the new fields through `Settings` and `Settings::resolve`. Add CLI flags only if the existing `Cli` already exposes scheduler options.

Tests:

8. Unit test the interval vectors in section 2 with fuzz off, including FSRS-5 and FSRS-6 decay and 17/19/21 param conversion.
9. Test retention bounds, default 0.9, invalid fallback, and config round trip.
10. Test trigger logic at 29, 30, and 31 days, 999 and 1000 reviews, 63 and 64 usable items, and one run in progress.
11. Test persistence of params and counters, failure keeping old params, and that no card due dates change on optimization.
12. Test reading and writing `s`, `d`, `dr`, `decay`, and unknown fields in `cards.data`.

## 8. Pitfalls

- FSRS-5 has 19 weights and decay 0.5. FSRS-6 has 21 weights and variable decay. Ranki's current fixed 19-weight model will not interpret a 21-value Anki preset.
- Variable decay changes interval scaling. With `d=0.1542`, `S=10`, and `DR=0.95`, the interval is 4 days. With `d=0.5`, it is 5 days.
- Same-day reviews use short-term stability formulas with `w17`, `w18`, and `w19`. Ranki's `same_day_stability` uses only two short-term weights and does not include `S^(-w19)`.
- Anki fuzzes raw `f32` intervals and preserves the previous interval through `minimum_review_fuzz_interval`. Ranki rounds before fuzzing, has no strict Hard/Good/Easy ordering, and starts fuzz at 3 days instead of 2.5.
- Anki's load balancer changes due-day selection inside fuzz bounds. Matching it requires more than matching the interval formula.
- Cached card `dr` and `decay` are compatibility metadata. Do not use a stale card value after a preset change without updating it.

## Sources

Pinned Anki prefix: `https://github.com/ankitects/anki/blob/be56085c32a9d02b506fa880cfa63c756c1779fc/`

- `rslib/src/scheduler/fsrs/{params,memory_state}.rs`
- `rslib/src/scheduler/states/{review,fuzz}.rs`
- `rslib/src/deckconfig/{mod,schema11,update}.rs`
- `rslib/src/decks/schema11.rs`
- `rslib/src/config/{bool,number}.rs`
- `rslib/src/storage/card/data.rs`
- `ts/routes/deck-options/FsrsOptions.svelte`
- `https://docs.ankiweb.net/deck-options.html#fsrs`
- `https://github.com/open-spaced-repetition/fsrs-rs/tree/v6.6.2`
- `https://github.com/open-spaced-repetition/fsrs4anki/blob/main/docs/tutorial.md`
