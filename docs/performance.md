# Performance

Ranki measures every user-facing surface in release builds and gates pull
requests against the best merged measurement. The check is named
`Performance audit`. Add it to the branch ruleset to block merges.

## Surfaces

`crates/bench` runs 20 workloads through the same release code the app ships:

- CLI: cold process start plus `decks` and `search`.
- AnkiConnect: server startup, `version`, and `findCards` round trips over HTTP.
- MCP: server startup, `tools/list`, and a `list_decks` tool call over stdio.
- GUI: one review frame, and one answer-reveal sample that runs two frame
  calls (reveal, then the redraw), driven through the same render-model
  helpers as the Wayland window with no display or GPU in the loop. Native compositing is outside the measurement; templates, card markup,
  and the deck-row model are inside it. The `gui` cargo feature gates the pair.
- Storage: collection open, common and missing searches over 2,000 notes, due
  cards, statistics, and a persisted review.
- Scheduling: FSRS and SM-2 decisions. Sinks keep the optimizer from removing
  the work.
- Packages: `.apkg` export and import of the seeded collection.

Every workload asserts that it produced the expected result. A broken fixture
fails the benchmark instead of reporting a plausible number.

## Method

One `ranki-bench measure` run declares a budget, a criterion, warmup calls,
samples, and repetitions per workload. It times each operation separately with
`Instant`, records the median, p95, and the largest timer gap seen by a 1 ms
watchdog, and writes one JSON report. The timing `method` is 1. Change it when
the way a workload is timed changes, so old ceilings stop gating the new
method.

CI builds each variant once in a parallel matrix. A shared Cargo target cache
keyed by variant, `Cargo.lock`, `flake.lock`, and the GUI feature keeps warm
builds out of the release compiler. Audit builds disable LTO and use 16 codegen
units, so every variant uses the same faster profile.
Each measurement shard downloads the head and one reference binary, then
runs both inside the head Nix development shell, which realizes the same store
paths used by every planned variant. Head and reference rounds alternate on
one runner. The gate stitches each workload's head result with the ceiling
measured in that shard, so a runner's speed cancels out and a ceiling is never
compared with a number from another runner.
A pull request must keep the timing method, sample count, repetitions, warmup,
and workload definition hash of the recorded ceiling. Otherwise a regression
could hide behind a cheaper measurement. The oldest variants run the same
harness with the GUI feature off when their commit lacks the headless GUI hook,
so their own review code is never replaced.

A workload fails when its best-of-three median, p95, or worst gap is more than
5% above the ceiling's worst run, or above that plus either side's spread
between rounds, whichever is larger. It also fails when it loses a workload
that has a ceiling, declares no budget or criterion, or raises a budget. An
over-budget workload fails only when the ceiling is within budget on the same
runner. Warnings never fail the check. The full table is posted as a pull
request comment.
The measurement job uploads the exact floor file it planned against, and the
audit uses that snapshot so a floor update between measurement and publish
cannot change the rules.

Ceilings live in `.github/performance-floor.json` on the separate `floors`
branch, so a pull request cannot edit them. Each workload records the commit
that produced its ceiling. CI re-measures each recorded commit beside the pull
request and uses that workload's own measurement as the comparison. Successful
pushes to `main` lower each ceiling only when that run's worst measurement beats
the recorded best. Budgets only tighten.

## Trust model

The audit and planning jobs check out the gate script from the default branch.
Aggregation uses that copy when it has the `stitch` command and otherwise uses
the pull request's copy to assemble reports. The audit still applies the default
branch rules and never executes measured code. The measurement itself runs PR
code, so the audit treats pull requests from forks as untrusted: it posts a
failing check explaining that a maintainer must rerun the Performance workflow
on a branch in this repository. Same-repository pull requests are gated on the
uploaded measurement. A failed comparison, a missing measurement, or a missing
re-measured ceiling publishes a failing check. Add `Performance audit` to the
branch ruleset to block merges.

## Reproduce

```sh
nix develop
CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  cargo build --release -p ranki -p ranki-bench
mkdir -p .tmp/performance
sha="$(git rev-parse HEAD)"
mkdir -p .tmp/performance/aggregated
for round in 1 2 3; do
  RANKI_BIN="$PWD/target/release/ranki" \
    target/release/ranki-bench measure --out ".tmp/performance/$sha-$round.json"
done
```

The CI gate scripts consume those reports directly:

```sh
python3 .github/scripts/performance.py aggregate \
  --dir .tmp/performance --out .tmp/performance/aggregated --rounds 3
python3 .github/scripts/performance.py compare \
  --head .tmp/performance/aggregated/$(git rev-parse HEAD).json \
  --comment .tmp/performance/comment.md \
  --result .tmp/performance/result.txt
python3 .github/scripts/performance_test.py
```

Set `RANKI_BENCH_VERBOSE=1` to print each workload as it starts.
