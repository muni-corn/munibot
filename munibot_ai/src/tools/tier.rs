use serde::{Deserialize, Serialize};

/// How much authority a tool carries.
///
/// Ordered from least to most dangerous. A tool's own tier is fixed by whoever
/// registers it; a persona's [`crate::tools::ToolSelection`] and an invoker's
/// granted tier both filter against it, and neither can lift a tool above where
/// it was registered.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    /// No side effects reachable from chat alone: the clock, a scratchpad, a
    /// user's own opt-in memory.
    Safe,
    /// Read-only network access: web search, fetching a URL.
    NetworkRead,
    /// munibot's own data, scoped to the invoking user: their profile, recent
    /// messages, balance.
    BotData,
    /// Filesystem and shell access, inside a container only.
    Sandbox,
    /// Actions with consequences outside the conversation: opening a pull
    /// request, messaging a channel, moderating a user. Never reachable
    /// from public chat.
    Privileged,
}

impl RiskTier {
    /// Every tier, in ascending order. Useful for iterating the whole range
    /// rather than naming each variant.
    pub const ALL: [RiskTier; 5] = [
        RiskTier::Safe,
        RiskTier::NetworkRead,
        RiskTier::BotData,
        RiskTier::Sandbox,
        RiskTier::Privileged,
    ];

    /// Parses the `"tier0"` through `"tier4"` shorthand used in persona
    /// configuration, matching this type's declaration order. Returns
    /// `None` for anything else, including the tier's own `snake_case`
    /// serde name - the shorthand is a distinct, config-only spelling.
    pub fn from_shorthand(text: &str) -> Option<Self> {
        match text {
            "tier0" => Some(Self::Safe),
            "tier1" => Some(Self::NetworkRead),
            "tier2" => Some(Self::BotData),
            "tier3" => Some(Self::Sandbox),
            "tier4" => Some(Self::Privileged),
            _ => None,
        }
    }

    /// The inverse of [`Self::from_shorthand`], used to serialize a
    /// [`crate::tools::ToolSelection`] back into the same config-shorthand
    /// form it was read from.
    pub fn shorthand(&self) -> &'static str {
        match self {
            Self::Safe => "tier0",
            Self::NetworkRead => "tier1",
            Self::BotData => "tier2",
            Self::Sandbox => "tier3",
            Self::Privileged => "tier4",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tiers_order_from_safe_to_privileged() {
        assert!(RiskTier::Safe < RiskTier::NetworkRead);
        assert!(RiskTier::NetworkRead < RiskTier::BotData);
        assert!(RiskTier::BotData < RiskTier::Sandbox);
        assert!(RiskTier::Sandbox < RiskTier::Privileged);
    }

    #[test]
    fn test_shorthand_round_trips_declaration_order() {
        for (index, tier) in RiskTier::ALL.iter().enumerate() {
            let shorthand = format!("tier{index}");
            assert_eq!(
                RiskTier::from_shorthand(&shorthand),
                Some(*tier),
                "tier{index} should parse to {tier:?}"
            );
        }
    }

    #[test]
    fn test_unrecognized_shorthand_is_none() {
        assert_eq!(
            RiskTier::from_shorthand("tier5"),
            None,
            "there is no fifth tier"
        );
        assert_eq!(
            RiskTier::from_shorthand("safe"),
            None,
            "the shorthand is tier0, not the serde name"
        );
        assert_eq!(
            RiskTier::from_shorthand("web_search"),
            None,
            "a tool name is not a tier"
        );
    }

    #[test]
    fn test_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&RiskTier::NetworkRead).expect("should serialize");
        assert_eq!(
            encoded, "\"network_read\"",
            "the wire form should be snake_case"
        );
    }

    #[test]
    fn test_shorthand_is_the_exact_inverse_of_from_shorthand() {
        for tier in RiskTier::ALL {
            assert_eq!(
                RiskTier::from_shorthand(tier.shorthand()),
                Some(tier),
                "shorthand() and from_shorthand() must round-trip for {tier:?}"
            );
        }
    }
}
