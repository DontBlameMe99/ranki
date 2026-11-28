//! Shared review-history statistics.

use std::collections::HashMap;

use crate::{Id, RevLog, RevLogKind};

/// Counts the current review streak using the same due-day rules on every UI.
///
/// A day with nothing due does not add to the streak or end it. An unanswered
/// day with a due card ends it. Today is not counted against the streak because
/// it may still be in progress.
#[must_use]
pub fn review_streak(
    history: &[RevLog],
    day_of: impl Fn(i64) -> i64,
    today: i64,
) -> usize {
    let mut reviewed = HashMap::<i64, usize>::new();
    for log in history {
        if matches!(log.kind, RevLogKind::Manual | RevLogKind::Rescheduled) {
            continue;
        }
        let day = day_of(log.id.checked_div(1_000).unwrap_or_default());
        if day <= today {
            let count = reviewed.entry(day).or_default();
            *count = count.saturating_add(1);
        }
    }
    streak_days(&reviewed, &due_ranges(history, &day_of, today), today)
}

/// Spans where at least one card was due, rebuilt from the review log. An
/// answer makes its card due `interval` days later, and the card stays due until
/// the next answer. Learning intervals are seconds, so those cards only fall due
/// again on the day they were answered.
fn due_ranges(
    history: &[RevLog],
    day_of: &impl Fn(i64) -> i64,
    today: i64,
) -> Vec<(i64, i64)> {
    let mut answers = HashMap::<Id, Vec<(i64, i32)>>::new();
    for log in history {
        if matches!(log.kind, RevLogKind::Manual | RevLogKind::Rescheduled) {
            continue;
        }
        answers.entry(log.card_id).or_default().push((
            day_of(log.id.checked_div(1_000).unwrap_or_default()),
            log.interval,
        ));
    }
    let mut ranges = Vec::with_capacity(answers.len());
    for queue in answers.values_mut() {
        queue.sort_unstable();
        for (index, &(day, interval)) in queue.iter().enumerate() {
            let start = day.saturating_add(i64::from(interval.max(0)));
            let end = queue
                .get(index.saturating_add(1))
                .map_or(today, |next| next.0);
            ranges.extend((start < end).then_some((start, end)));
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

fn streak_days(
    reviewed: &HashMap<i64, usize>,
    due: &[(i64, i64)],
    today: i64,
) -> usize {
    let Some(oldest) = reviewed.keys().copied().min() else {
        return 0;
    };
    let is_due = |day: i64| {
        due.partition_point(|&(start, _)| start <= day)
            .checked_sub(1)
            .is_some_and(|index| {
                due.get(index).is_some_and(|&(_, end)| day < end)
            })
    };
    let mut streak = 0_usize;
    let mut day = today;
    loop {
        if reviewed.contains_key(&day) {
            streak = streak.saturating_add(1);
        } else if day != today && is_due(day) {
            break;
        }
        if day <= oldest {
            break;
        }
        day = day.saturating_sub(1);
    }
    streak
}
