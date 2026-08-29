use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::tools::RiskTier;

/// One entry in a persona's tool selection list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolSelector {
    /// Every registered tool, regardless of tier. Written as `"all"` in
    /// configuration.
    All,
    /// Every tool at exactly this tier. Written as `"tier0"` through `"tier4"`.
    Tier(RiskTier),
    /// One specific tool, by name.
    Named(String),
}

impl ToolSelector {
    /// The exact string this selector reads back from, in configuration.
    ///
    /// Written by hand rather than derived: a derived, `#[serde(untagged)]`
    /// serialization of `Tier(RiskTier)` would emit the tier's own
    /// `snake_case` name (`"network_read"`), which
    /// [`RiskTier::from_shorthand`] does not recognize - breaking the round
    /// trip this type exists to support.
    fn as_config_str(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::All => std::borrow::Cow::Borrowed("all"),
            Self::Tier(tier) => std::borrow::Cow::Borrowed(tier.shorthand()),
            Self::Named(name) => std::borrow::Cow::Borrowed(name),
        }
    }
}

impl Serialize for ToolSelector {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_config_str())
    }
}

/// Which tools a persona may reach for.
///
/// Represents a **union** of selectors, not a choice between them: `["tier0",
/// "web_search"]` grants every [`RiskTier::Safe`] tool plus the specific
/// `web_search` tool, even though `web_search` itself
/// sits at [`RiskTier::NetworkRead`]. Each tier keyword expands to exactly that
/// tier's own tools, not tiers below it - a persona wanting both `Safe` and
/// `NetworkRead` tools lists `"tier0"` and `"tier1"` both, which is exactly
/// what the researcher persona's configuration in `docs/plans/ai/overview.md`
/// does.
///
/// Deserializes directly from a plain TOML list of strings: `"all"` selects
/// everything, `"tier0"` through `"tier4"` expand to a tier, and anything else
/// names one tool.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(transparent)]
pub struct ToolSelection(Vec<ToolSelector>);

impl ToolSelection {
    /// Selects nothing. The default: a persona must opt in to every tool it
    /// wants.
    pub fn none() -> Self {
        Self::default()
    }

    /// Selects every registered tool, regardless of tier.
    pub fn all() -> Self {
        Self(vec![ToolSelector::All])
    }

    /// Selects every tool at exactly the given tier.
    pub fn tier(tier: RiskTier) -> Self {
        Self(vec![ToolSelector::Tier(tier)])
    }

    /// Selects a specific set of tools by name.
    pub fn named(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(
            names
                .into_iter()
                .map(|name| ToolSelector::Named(name.into()))
                .collect(),
        )
    }

    /// Adds a tier to the selection, in addition to whatever it already
    /// selects.
    pub fn with_tier(mut self, tier: RiskTier) -> Self {
        self.0.push(ToolSelector::Tier(tier));
        self
    }

    /// Adds a named tool to the selection, in addition to whatever it already
    /// selects.
    pub fn with_named(mut self, name: impl Into<String>) -> Self {
        self.0.push(ToolSelector::Named(name.into()));
        self
    }

    /// Returns `true` if this selection covers a tool with the given name and
    /// tier.
    ///
    /// A tool is covered if any selector matches it: `All` matches everything,
    /// a tier selector matches a tool at exactly that tier, and a named
    /// selector matches a tool with that exact name regardless of its tier.
    pub fn covers(&self, tool_name: &str, tool_tier: RiskTier) -> bool {
        self.0.iter().any(|selector| match selector {
            ToolSelector::All => true,
            ToolSelector::Tier(tier) => *tier == tool_tier,
            ToolSelector::Named(name) => name == tool_name,
        })
    }

    /// Whether this selection explicitly reaches for `tier` as a whole - via
    /// `"all"` or that tier's own shorthand.
    ///
    /// Deliberately coarser than [`Self::covers`]: a named tool that happens
    /// to sit at `tier` without the tier itself being listed is not detected
    /// here, since that would need the tool registry's own tier for every
    /// name, which this type has no access to. Good enough for a sensible
    /// default (see `crate::moderation::ModerationPolicy::default_for`),
    /// never a security boundary on its own.
    pub fn covers_tier_broadly(&self, tier: RiskTier) -> bool {
        self.0.iter().any(|selector| match selector {
            ToolSelector::All => true,
            ToolSelector::Tier(selected) => *selected == tier,
            ToolSelector::Named(_) => false,
        })
    }
}

impl<'de> Deserialize<'de> for ToolSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<String>::deserialize(deserializer)?;
        let selectors = entries
            .into_iter()
            .map(|entry| match entry.as_str() {
                "all" => ToolSelector::All,
                _ => match RiskTier::from_shorthand(&entry) {
                    Some(tier) => ToolSelector::Tier(tier),
                    None => ToolSelector::Named(entry),
                },
            })
            .collect();

        Ok(ToolSelection(selectors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserializes_a_mixed_list_as_a_union() {
        // exactly the companion persona's configuration from docs/plans/ai/overview.md
        let selection: ToolSelection =
            serde_json::from_value(serde_json::json!(["tier0", "web_search"]))
                .expect("should parse");

        assert!(
            selection.covers("current_time", RiskTier::Safe),
            "tier0 should cover every Safe tool"
        );
        assert!(
            selection.covers("web_search", RiskTier::NetworkRead),
            "web_search should be covered by its explicit name even though it is not Safe"
        );
        assert!(
            !selection.covers("get_balance", RiskTier::BotData),
            "an unlisted tier and an unlisted name should not be covered"
        );
    }

    #[test]
    fn test_two_tiers_are_both_covered_when_both_are_listed() {
        // the researcher persona's configuration: tier0 and tier1 both listed, proving
        // each expands to exactly its own tier rather than tier1 alone being
        // assumed cumulative
        let selection: ToolSelection =
            serde_json::from_value(serde_json::json!(["tier0", "tier1"])).expect("should parse");

        assert!(selection.covers("current_time", RiskTier::Safe));
        assert!(selection.covers("web_search", RiskTier::NetworkRead));
        assert!(
            !selection.covers("read_recent_messages", RiskTier::BotData),
            "an untier'd tier should not be implicitly covered"
        );
    }

    #[test]
    fn test_all_covers_every_tier() {
        let selection = ToolSelection::all();
        for tier in RiskTier::ALL {
            assert!(
                selection.covers("anything", tier),
                "\"all\" should cover tier {tier:?}"
            );
        }
    }

    #[test]
    fn test_none_covers_nothing() {
        let selection = ToolSelection::none();
        assert!(!selection.covers("current_time", RiskTier::Safe));
    }

    #[test]
    fn test_builder_methods_accumulate() {
        let selection = ToolSelection::none()
            .with_tier(RiskTier::Safe)
            .with_named("web_search");

        assert!(selection.covers("current_time", RiskTier::Safe));
        assert!(selection.covers("web_search", RiskTier::NetworkRead));
    }

    #[test]
    fn test_named_selector_does_not_cover_by_tier_alone() {
        let selection = ToolSelection::named(["web_search"]);
        assert!(
            !selection.covers("web_fetch", RiskTier::NetworkRead),
            "naming one tool must not implicitly grant every tool at its tier"
        );
    }

    #[test]
    fn test_deserializing_the_literal_all_keyword() {
        let selection: ToolSelection =
            serde_json::from_value(serde_json::json!(["all"])).expect("should parse");
        assert!(selection.covers("anything_at_all", RiskTier::Privileged));
    }

    #[test]
    fn test_empty_list_deserializes_to_none() {
        let selection: ToolSelection =
            serde_json::from_value(serde_json::json!([])).expect("should parse");
        assert!(!selection.covers("current_time", RiskTier::Safe));
    }

    #[test]
    fn test_covers_tier_broadly_matches_an_explicit_tier_selector() {
        let selection = ToolSelection::tier(RiskTier::Privileged);
        assert!(selection.covers_tier_broadly(RiskTier::Privileged));
        assert!(!selection.covers_tier_broadly(RiskTier::Safe));
    }

    #[test]
    fn test_covers_tier_broadly_matches_all() {
        let selection = ToolSelection::all();
        assert!(selection.covers_tier_broadly(RiskTier::Privileged));
    }

    #[test]
    fn test_covers_tier_broadly_ignores_a_named_tool_at_that_tier() {
        // a coarse heuristic, not a security boundary - see the method's
        // own doc comment for why a named tool alone doesn't count
        let selection = ToolSelection::named(["some_privileged_tool"]);
        assert!(!selection.covers_tier_broadly(RiskTier::Privileged));
    }

    #[test]
    fn test_mixed_selection_round_trips_through_serialization() {
        let original = ToolSelection::none()
            .with_tier(RiskTier::Safe)
            .with_named("web_search");

        let encoded = serde_json::to_value(&original).expect("should serialize");
        assert_eq!(
            encoded,
            serde_json::json!(["tier0", "web_search"]),
            "a tier selector must serialize back to its shorthand, not RiskTier's own snake_case \
             name"
        );

        let decoded: ToolSelection = serde_json::from_value(encoded).expect("should deserialize");
        assert_eq!(decoded, original, "the round trip should be exact");
    }

    #[test]
    fn test_all_selector_round_trips() {
        let original = ToolSelection::all();
        let encoded = serde_json::to_value(&original).expect("should serialize");
        assert_eq!(encoded, serde_json::json!(["all"]));

        let decoded: ToolSelection = serde_json::from_value(encoded).expect("should deserialize");
        assert_eq!(decoded, original);
    }
}
