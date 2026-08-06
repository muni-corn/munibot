use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::limits::Scope;

/// The in-memory piece of the rate limiter: how many turns are in flight
/// right now for each scope. Never persisted - concurrency is inherently
/// live state that doesn't need to survive a restart, and checking it costs
/// nothing next to the database round trip a request/token check needs.
#[derive(Clone, Default)]
pub(crate) struct ConcurrencyTracker(Arc<Mutex<HashMap<Scope, u32>>>);

impl ConcurrencyTracker {
    /// Reserves one slot for `scope` if fewer than `max` are already in
    /// flight, returning a guard that releases it again when dropped.
    pub(crate) fn try_acquire(&self, scope: Scope, max: u32) -> Option<ConcurrencyGuard> {
        let mut in_flight = self.0.lock().unwrap();
        let current = in_flight.entry(scope).or_insert(0);
        if *current >= max {
            return None;
        }
        *current += 1;
        Some(ConcurrencyGuard {
            tracker: Some((self.clone(), scope)),
        })
    }
}

/// Releases a reserved concurrency slot when dropped.
///
/// `None` when no concurrency limit was configured for the checked scope -
/// [`crate::limits::RateLimiter::check`] still returns one either way, so a
/// caller always holds *something* for the turn's own lifetime rather than
/// needing to branch on whether concurrency was actually being tracked.
pub struct ConcurrencyGuard {
    tracker: Option<(ConcurrencyTracker, Scope)>,
}

impl ConcurrencyGuard {
    /// A guard that releases nothing, for a scope with no concurrency limit
    /// configured at all.
    pub(crate) fn inert() -> Self {
        Self { tracker: None }
    }
}

impl Drop for ConcurrencyGuard {
    fn drop(&mut self) {
        let Some((tracker, scope)) = self.tracker.take() else {
            return;
        };
        let mut in_flight = tracker.0.lock().unwrap();
        if let Some(count) = in_flight.get_mut(&scope) {
            *count = count.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acquiring_under_the_max_succeeds() {
        let tracker = ConcurrencyTracker::default();
        assert!(tracker.try_acquire(Scope::User(1), 2).is_some());
        assert!(tracker.try_acquire(Scope::User(1), 2).is_some());
    }

    #[test]
    fn test_acquiring_at_the_max_is_refused() {
        let tracker = ConcurrencyTracker::default();
        let _first = tracker
            .try_acquire(Scope::User(1), 1)
            .expect("should succeed");
        assert!(
            tracker.try_acquire(Scope::User(1), 1).is_none(),
            "a second concurrent turn should be refused once the max is reached"
        );
    }

    #[test]
    fn test_dropping_a_guard_frees_its_slot() {
        let tracker = ConcurrencyTracker::default();
        let first = tracker
            .try_acquire(Scope::User(1), 1)
            .expect("should succeed");
        assert!(tracker.try_acquire(Scope::User(1), 1).is_none());

        drop(first);

        assert!(
            tracker.try_acquire(Scope::User(1), 1).is_some(),
            "dropping the first guard should free its slot"
        );
    }

    #[test]
    fn test_scopes_are_tracked_independently() {
        let tracker = ConcurrencyTracker::default();
        let _user = tracker
            .try_acquire(Scope::User(1), 1)
            .expect("should succeed");
        assert!(
            tracker.try_acquire(Scope::Guild(1), 1).is_some(),
            "a different scope should have its own independent slot"
        );
    }

    #[test]
    fn test_an_inert_guard_frees_nothing_on_drop() {
        // mainly a compile-and-doesn't-panic check: dropping an inert guard
        // must never touch a tracker at all
        let guard = ConcurrencyGuard::inert();
        drop(guard);
    }
}
