use crate::tools::{RiskTier, ToolSelection};

/// What to do when a moderation *check itself* fails to run - an endpoint
/// outage, a network error, a bad key - never used for a check that ran
/// and actually flagged content, which always refuses regardless of
/// policy (see [`crate::moderation::ModerationGate::check`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModerationPolicy {
    /// Let the turn through anyway, logging a warning. The right choice
    /// for a casual chat persona: a moderation outage should not silence
    /// munibot entirely.
    FailOpen,
    /// Refuse the turn outright. The right choice for a persona that can
    /// reach [`RiskTier::Privileged`] tools: a real-world action going
    /// through unchecked because the check couldn't run is worse than the
    /// turn simply failing.
    FailClosed,
}

impl ModerationPolicy {
    /// The policy a persona gets when it does not explicitly choose one -
    /// see `PersonaConfig::moderation`'s own doc comment for how an
    /// explicit choice overrides this.
    ///
    /// Fail-closed exactly when `tools` broadly reaches
    /// [`RiskTier::Privileged`], matching the milestone 6 plan's own
    /// framing (fail-closed for a "tier 4" persona, fail-open for chat).
    pub fn default_for(tools: &ToolSelection) -> Self {
        if tools.covers_tier_broadly(RiskTier::Privileged) {
            Self::FailClosed
        } else {
            Self::FailOpen
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_persona_with_no_privileged_tools_defaults_to_fail_open() {
        assert_eq!(
            ModerationPolicy::default_for(&ToolSelection::none()),
            ModerationPolicy::FailOpen
        );
    }

    #[test]
    fn test_a_persona_reaching_privileged_tools_defaults_to_fail_closed() {
        assert_eq!(
            ModerationPolicy::default_for(&ToolSelection::tier(RiskTier::Privileged)),
            ModerationPolicy::FailClosed
        );
    }

    #[test]
    fn test_an_all_selection_defaults_to_fail_closed() {
        assert_eq!(
            ModerationPolicy::default_for(&ToolSelection::all()),
            ModerationPolicy::FailClosed
        );
    }
}
