use std::time::Duration;

/// How long a scope is refused once abuse detection trips, and how that
/// grows with repeat offences.
///
/// Escalating rather than fixed: a first trip is very plausibly a false
/// positive (see [`crate::abuse`]'s own doc comment) and deserves a short,
/// forgettable cooldown, but a scope that keeps tripping is showing a
/// pattern, not bad luck, and should cool down for longer each time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CooldownPolicy {
    /// The first strike's cooldown.
    pub base: Duration,
    /// The longest a cooldown may ever grow to, regardless of how many
    /// strikes have accumulated.
    pub max: Duration,
    /// How long a scope must go without a new strike before its strike
    /// count resets to zero. Without this, one bad afternoon a year ago
    /// would cool a scope down at the maximum forever.
    pub reset_after: Duration,
}

impl Default for CooldownPolicy {
    /// A minute, doubling up to an hour, forgiven after a day of clean
    /// behaviour.
    fn default() -> Self {
        Self {
            base: Duration::from_secs(60),
            max: Duration::from_secs(60 * 60),
            reset_after: Duration::from_secs(60 * 60 * 24),
        }
    }
}

impl CooldownPolicy {
    /// The cooldown duration for the `strike`th strike (1-indexed): doubles
    /// from `base` each additional strike, capped at `max`.
    ///
    /// `strike` is expected to start at 1; `0` is treated the same as `1`
    /// (no such thing as a "zeroth" strike having its own, shorter
    /// cooldown).
    pub fn duration_for(&self, strike: u32) -> Duration {
        // capped well below any real strike count could reach anyway, purely
        // to keep the shift itself from ever overflowing
        let shift = strike.saturating_sub(1).min(32);
        let multiplier = 1u64 << shift;
        let scaled_secs = (self.base.as_secs()).saturating_mul(multiplier);
        Duration::from_secs(scaled_secs).min(self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_first_strike_uses_the_base_duration() {
        let policy = CooldownPolicy::default();
        assert_eq!(policy.duration_for(1), policy.base);
    }

    #[test]
    fn test_a_zeroth_strike_is_treated_like_the_first() {
        let policy = CooldownPolicy::default();
        assert_eq!(policy.duration_for(0), policy.duration_for(1));
    }

    #[test]
    fn test_each_strike_doubles_the_previous_cooldown() {
        let policy = CooldownPolicy {
            base: Duration::from_secs(10),
            max: Duration::from_secs(10_000),
            reset_after: Duration::from_secs(1),
        };
        assert_eq!(policy.duration_for(1), Duration::from_secs(10));
        assert_eq!(policy.duration_for(2), Duration::from_secs(20));
        assert_eq!(policy.duration_for(3), Duration::from_secs(40));
        assert_eq!(policy.duration_for(4), Duration::from_secs(80));
    }

    #[test]
    fn test_the_cooldown_never_exceeds_the_configured_max() {
        let policy = CooldownPolicy {
            base: Duration::from_secs(60),
            max: Duration::from_secs(300),
            reset_after: Duration::from_secs(1),
        };
        assert_eq!(policy.duration_for(100), Duration::from_secs(300));
    }

    #[test]
    fn test_an_enormous_strike_count_never_overflows() {
        let policy = CooldownPolicy::default();
        // mainly a doesn't-panic check
        assert_eq!(policy.duration_for(u32::MAX), policy.max);
    }
}
