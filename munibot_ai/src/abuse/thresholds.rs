use std::time::Duration;

/// How bursty a scope's own recent activity has to be before
/// [`crate::abuse::AbuseDetector`] treats it as a strike, for the two
/// heuristics that need recent history rather than a single message
/// ([`crate::abuse::injection_signature`] needs neither: one match is
/// always enough).
///
/// Every field has a sensible built-in default (see [`Self::default`]),
/// the same ergonomic-config-then-resolve shape
/// [`crate::limits::RateLimitPolicy`] already uses - an operator overrides
/// only what they actually want tuned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectionThresholds {
    /// How many times a (normalized) prompt may repeat within
    /// `duplicate_window` before it counts as a strike.
    pub duplicate_threshold: u32,
    pub duplicate_window: Duration,
    /// How many distinct personas a scope may use within
    /// `persona_switch_window` before it counts as a strike.
    pub persona_switch_threshold: u32,
    pub persona_switch_window: Duration,
}

impl Default for DetectionThresholds {
    /// Three repeats of the same prompt inside two minutes, or four
    /// distinct personas inside one minute - loose enough that a genuine
    /// conversation (retrying a typo, comparing two personas' answers)
    /// never trips either one, tight enough to catch a deliberate script.
    fn default() -> Self {
        Self {
            duplicate_threshold: 3,
            duplicate_window: Duration::from_secs(120),
            persona_switch_threshold: 4,
            persona_switch_window: Duration::from_secs(60),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_thresholds_are_loose_but_finite() {
        let defaults = DetectionThresholds::default();
        assert!(defaults.duplicate_threshold > 1);
        assert!(defaults.persona_switch_threshold > 1);
    }
}
