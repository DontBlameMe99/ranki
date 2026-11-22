use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;

use crate::Id;

static LAST_ID: AtomicI64 = AtomicI64::new(0);

/// Allocates a unique, increasing millisecond id.
///
/// `at_millis` is the caller's clock reading. All modules share this counter so
/// notes, cards, review logs, and manual logs cannot collide. Once the counter
/// reaches `i64::MAX` it stays there instead of wrapping into negative ids.
#[must_use]
pub fn next_id(at_millis: i64) -> Id {
    const MAX_AT_MILLIS: i64 = i64::MAX - 1_000_000_000;
    allocate(&LAST_ID, at_millis.min(MAX_AT_MILLIS))
}

fn allocate(counter: &AtomicI64, at_millis: i64) -> Id {
    let previous = counter
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
            Some(last.max(at_millis).saturating_add(1))
        })
        .unwrap_or(i64::MAX);
    previous.max(at_millis).saturating_add(1)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicI64;

    use super::{allocate, next_id};

    #[test]
    fn ids_are_unique_and_increasing_across_call_sites() {
        let a = next_id(1_700_000_000_000);
        let b = next_id(1_700_000_000_000);
        let c = next_id(1_699_999_999_999);
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn ids_never_wrap() {
        let counter = AtomicI64::new(i64::MAX - 2);
        assert_eq!(allocate(&counter, i64::MAX - 2), i64::MAX - 1);
        assert_eq!(allocate(&counter, i64::MAX - 2), i64::MAX);
        assert_eq!(allocate(&counter, i64::MAX), i64::MAX);
    }
}
