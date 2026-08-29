use std::{collections::HashMap, sync::Arc};

use crate::{
    tools::{
        CurrentTimeTool, ExaBackend, ExaClient, RiskTier, TodoWriteTool, Tool, ToolSelection,
        WebFetchTool, WebSearchTool,
    },
    types::ToolSchema,
};

/// Every tool munibot knows how to run, keyed by name.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Builds an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a registry with every tool this milestone ships, given the
    /// current environment: `current_time` and `todo_write` unconditionally,
    /// and `web_search`/`web_fetch` only when `EXA_API_KEY` is set.
    ///
    /// A missing key is not a startup failure - it just means a smaller set
    /// of tools is available, the same tradeoff
    /// [`crate::provider::ProviderRegistry::from_env`] makes for model
    /// providers.
    pub fn from_env() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(CurrentTimeTool));
        registry.register(Arc::new(TodoWriteTool::new()));

        match std::env::var("EXA_API_KEY") {
            Ok(api_key) => {
                let backend: Arc<dyn ExaBackend> = Arc::new(ExaClient::new(api_key));
                registry.register(Arc::new(WebSearchTool::new(backend.clone())));
                registry.register(Arc::new(WebFetchTool::new(backend)));
            }
            Err(_) => {
                tracing::debug!(
                    "EXA_API_KEY not set; web_search and web_fetch tools are unavailable"
                );
            }
        }

        registry
    }

    /// Registers a tool. A second registration under the same name replaces the
    /// first.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Looks a tool up by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Every tool name currently registered, for diagnostics and startup
    /// logging.
    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    /// Builds a fresh registry with every tool already in this one, plus
    /// `extra` registered on top - a second registration under a name
    /// already present replaces the first, the same as [`Self::register`].
    ///
    /// Used for a sandboxed turn: the six sandbox tools are per-turn (they
    /// close over one running container's own
    /// [`crate::sandbox::rpc::RpcClient`]), so they are layered onto the
    /// shared base registry fresh for each such turn rather than ever being
    /// registered into it directly.
    pub fn with_overlay(&self, extra: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        let mut overlaid = Self {
            tools: self.tools.clone(),
        };
        for tool in extra {
            overlaid.register(tool);
        }
        overlaid
    }

    /// The schemas a provider should be offered: every registered tool that a
    /// persona's selection covers *and* that the invoker's granted tier
    /// permits.
    ///
    /// Both checks matter independently. A persona's [`ToolSelection`] can name
    /// tools the current invoker is not authorized for - the same persona
    /// might be shared between an admin and a regular user - and the
    /// invoker's granted tier can never be widened by what the persona asks
    /// for. A tool is offered only when both agree; this is what keeps
    /// [`ToolCtx::require_tier`] from ever needing to reject a call the
    /// model should not have been able to make in the first place.
    ///
    /// [`ToolCtx::require_tier`]: crate::tools::ToolCtx::require_tier
    pub fn schemas_for(&self, selection: &ToolSelection, granted: RiskTier) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|tool| tool.tier() <= granted && selection.covers(tool.name(), tool.tier()))
            .map(|tool| tool.schema())
            .collect()
    }

    /// Resolves `name` to a registered tool, but only if `selection` and
    /// `granted` would actually have offered it - the same two-part check
    /// [`Self::schemas_for`] already applies to what a provider sees, now
    /// also applied to what a call is allowed to *dispatch*.
    ///
    /// Without this, a call naming a tool that exists in this registry
    /// (registered for some other persona sharing it) but was never in
    /// `selection`, or above `granted`, would still resolve and run: the
    /// registry itself has no notion of "whose call this is", and a tool's
    /// own [`crate::tools::ToolCtx::require_tier`] check - the other half
    /// of this defense - is only as good as every tool implementation
    /// remembering to call it. This closes the same gap unconditionally,
    /// for every tool, tier-checking or not.
    pub fn get_authorized(
        &self,
        name: &str,
        selection: &ToolSelection,
        granted: RiskTier,
    ) -> Option<Arc<dyn Tool>> {
        self.get(name)
            .filter(|tool| tool.tier() <= granted && selection.covers(tool.name(), tool.tier()))
    }

    /// Every tool name `selection` and `granted` would actually offer - the
    /// same filter [`Self::schemas_for`] applies, without building a full
    /// schema for each. Used to name what *is* available when a call is
    /// refused, without leaking the existence of a tool this call was never
    /// authorized to even be told about.
    pub fn names_for(&self, selection: &ToolSelection, granted: RiskTier) -> Vec<&str> {
        self.tools
            .values()
            .filter(|tool| tool.tier() <= granted && selection.covers(tool.name(), tool.tier()))
            .map(|tool| tool.name())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::*;
    use crate::tools::{ToolCtx, ToolOutcome};

    struct StubTool {
        name: &'static str,
        tier: RiskTier,
    }

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "a stub tool for registry tests"
        }

        fn tier(&self) -> RiskTier {
            self.tier
        }

        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }

        async fn invoke(&self, _input: Value, _ctx: &ToolCtx) -> ToolOutcome {
            ToolOutcome::ok("stub")
        }
    }

    /// A registry with one tool registered at each of the five tiers, named
    /// after its tier for readability in test failure output.
    fn registry_with_one_tool_per_tier() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        for tier in RiskTier::ALL {
            registry.register(Arc::new(StubTool {
                name: tier.shorthand(),
                tier,
            }));
        }
        registry
    }

    fn names_of(schemas: &[ToolSchema]) -> Vec<&str> {
        let mut names: Vec<&str> = schemas.iter().map(|schema| schema.name.as_str()).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn test_register_and_get_round_trip() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(StubTool {
            name: "ping",
            tier: RiskTier::Safe,
        }));

        let tool = registry
            .get("ping")
            .expect("should find the registered tool");
        assert_eq!(tool.name(), "ping");
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn test_registering_the_same_name_twice_replaces_the_first() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(StubTool {
            name: "ping",
            tier: RiskTier::Safe,
        }));
        registry.register(Arc::new(StubTool {
            name: "ping",
            tier: RiskTier::Privileged,
        }));

        assert_eq!(
            registry.names().len(),
            1,
            "a second registration should replace, not duplicate"
        );
        assert_eq!(
            registry.get("ping").expect("should exist").tier(),
            RiskTier::Privileged
        );
    }

    /// The filtering matrix: (selection, granted tier) -> which of the five
    /// per-tier stub tools should be offered. Table-driven, per the plan,
    /// so every combination of persona selection and invoker authorization
    /// is exercised in one place.
    #[test]
    fn test_schemas_for_filtering_matrix() {
        let registry = registry_with_one_tool_per_tier();

        let cases: &[(&str, ToolSelection, RiskTier, &[&str])] = &[
            (
                "selecting everything but granted only Safe offers only tier0",
                ToolSelection::all(),
                RiskTier::Safe,
                &["tier0"],
            ),
            (
                "selecting everything and granted Privileged offers every tier",
                ToolSelection::all(),
                RiskTier::Privileged,
                &["tier0", "tier1", "tier2", "tier3", "tier4"],
            ),
            (
                "selecting a specific high tier but granted low offers nothing",
                ToolSelection::tier(RiskTier::Privileged),
                RiskTier::Safe,
                &[],
            ),
            (
                "selecting tier0 and tier1 with full authorization offers both, not tier2",
                ToolSelection::none()
                    .with_tier(RiskTier::Safe)
                    .with_tier(RiskTier::NetworkRead),
                RiskTier::Privileged,
                &["tier0", "tier1"],
            ),
            (
                "naming one high tier tool directly, with sufficient authorization, offers it \
                 alone",
                ToolSelection::named(["tier3"]),
                RiskTier::Privileged,
                &["tier3"],
            ),
            (
                "naming a tool the invoker is not authorized for offers nothing",
                ToolSelection::named(["tier4"]),
                RiskTier::Safe,
                &[],
            ),
            (
                "an empty selection offers nothing regardless of authorization",
                ToolSelection::none(),
                RiskTier::Privileged,
                &[],
            ),
        ];

        for (description, selection, granted, expected) in cases {
            let schemas = registry.schemas_for(selection, *granted);
            assert_eq!(names_of(&schemas), *expected, "case failed: {description}");
        }
    }

    #[test]
    fn test_with_overlay_keeps_the_base_tools_and_adds_the_extra_ones() {
        let mut base = ToolRegistry::new();
        base.register(Arc::new(StubTool {
            name: "base_tool",
            tier: RiskTier::Safe,
        }));

        let overlaid = base.with_overlay([Arc::new(StubTool {
            name: "extra_tool",
            tier: RiskTier::Sandbox,
        }) as Arc<dyn Tool>]);

        assert!(overlaid.get("base_tool").is_some());
        assert!(overlaid.get("extra_tool").is_some());
    }

    #[test]
    fn test_with_overlay_does_not_mutate_the_base_registry() {
        let base = ToolRegistry::new();
        let overlaid = base.with_overlay([Arc::new(StubTool {
            name: "extra_tool",
            tier: RiskTier::Sandbox,
        }) as Arc<dyn Tool>]);

        assert!(base.get("extra_tool").is_none());
        assert!(overlaid.get("extra_tool").is_some());
    }

    #[test]
    fn test_with_overlay_replaces_a_base_tool_of_the_same_name() {
        let mut base = ToolRegistry::new();
        base.register(Arc::new(StubTool {
            name: "shared_name",
            tier: RiskTier::Safe,
        }));

        let overlaid = base.with_overlay([Arc::new(StubTool {
            name: "shared_name",
            tier: RiskTier::Sandbox,
        }) as Arc<dyn Tool>]);

        assert_eq!(
            overlaid.get("shared_name").unwrap().tier(),
            RiskTier::Sandbox
        );
    }

    #[test]
    fn test_names_lists_every_registered_tool() {
        let registry = registry_with_one_tool_per_tier();
        let mut names = registry.names();
        names.sort_unstable();
        assert_eq!(names, vec!["tier0", "tier1", "tier2", "tier3", "tier4"]);
    }
}
