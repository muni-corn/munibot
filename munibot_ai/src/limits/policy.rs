use std::time::Duration;

/// The limits configured for one scope kind (every
/// [`crate::limits::Scope::User`] instance, say, regardless of which specific
/// user).
///
/// Every field is optional: unset means no limit at that scope for that
/// dimension, matching how [`crate::persona::config::BudgetConfig`]'s own
/// fields work.
#[derive(Clone, Copy, Debug)]
pub struct RateLimitPolicy {
    /// How many turns may be requested within `window`.
    pub max_requests: Option<u32>,
    /// How many tokens may be spent within `window`, checked against usage
    /// already recorded from *previous* turns - a turn in progress has no
    /// way to know its own token cost before the model answers.
    pub max_tokens: Option<u64>,
    /// How many turns for this scope may be in flight at once, checked
    /// entirely in memory rather than against the database.
    pub max_concurrent_turns: Option<u32>,
    pub window: Duration,
}

impl Default for RateLimitPolicy {
    /// No limits at all, with a one-minute window - a policy this permissive
    /// is only ever meaningful once at least one field above is actually
    /// set.
    fn default() -> Self {
        Self {
            max_requests: None,
            max_tokens: None,
            max_concurrent_turns: None,
            window: Duration::from_secs(60),
        }
    }
}

impl RateLimitPolicy {
    /// Whether every limit is unset - a scope with nothing to check at all,
    /// so [`crate::limits::RateLimiter`] can skip it without a database
    /// round trip.
    pub fn is_unlimited(&self) -> bool {
        self.max_requests.is_none()
            && self.max_tokens.is_none()
            && self.max_concurrent_turns.is_none()
    }
}

/// The three scope kinds' policies together, for
/// [`crate::limits::RateLimiter::new`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ScopePolicies {
    pub user: RateLimitPolicy,
    pub guild: RateLimitPolicy,
    pub global: RateLimitPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy_is_unlimited() {
        assert!(RateLimitPolicy::default().is_unlimited());
    }

    #[test]
    fn test_any_single_limit_makes_a_policy_not_unlimited() {
        assert!(
            !RateLimitPolicy {
                max_requests: Some(10),
                ..RateLimitPolicy::default()
            }
            .is_unlimited()
        );
        assert!(
            !RateLimitPolicy {
                max_tokens: Some(1000),
                ..RateLimitPolicy::default()
            }
            .is_unlimited()
        );
        assert!(
            !RateLimitPolicy {
                max_concurrent_turns: Some(1),
                ..RateLimitPolicy::default()
            }
            .is_unlimited()
        );
    }
}
