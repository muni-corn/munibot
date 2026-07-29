//! The agent loop: model to tools to handoff, with budgets and events.
//!
//! This is the one place that drives a [`crate::provider::Provider`] and a
//! [`crate::tools::ToolRegistry`] together. Everything above it - personas,
//! platform adapters, the eventual pipeline - talks to the harness and never to
//! a provider or a tool directly.

pub mod budget;
pub mod event;
pub mod turn;

use std::sync::Arc;

pub use budget::{Budget, BudgetTracker};
pub use event::HarnessEvent;
pub use turn::{HandoffSchema, TurnOutcome, TurnRequest};

use crate::{
    provider::{Provider, estimate_cost},
    tools::ToolRegistry,
    types::{AiError, CompletionRequest},
};

/// Drives a [`Provider`] and a [`ToolRegistry`] together into one full agent
/// turn.
///
/// This is the only thing above it - personas, platform adapters, the eventual
/// pipeline - ever needs to hold. Nothing else touches a provider or a tool
/// directly.
pub struct Harness {
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
}

impl Harness {
    /// Builds a harness over a provider and the tool registry it may draw from.
    pub fn new(provider: Arc<dyn Provider>, tools: Arc<ToolRegistry>) -> Self {
        Self { provider, tools }
    }

    /// Runs one full turn.
    ///
    /// This commit covers exactly the case every real turn starts with: build a
    /// request, offer the tools the persona's selection and the invoker's
    /// tier both permit, and call the provider once. A response that does
    /// not ask for a tool ends the turn with its text.
    ///
    /// A response that *does* ask for a tool currently fails outright, rather
    /// than looping - tool dispatch is a separate, later commit, and a stub
    /// "loop forever without ever calling a tool" implementation would be
    /// strictly worse than an honest error naming what is missing.
    pub async fn run_turn(&self, request: TurnRequest) -> Result<TurnOutcome, AiError> {
        let tool_schemas = self
            .tools
            .schemas_for(&request.tools, request.ctx.granted_tier);

        let mut completion_request =
            CompletionRequest::new(request.model.clone(), request.history.clone())
                .with_tools(tool_schemas)
                .with_params(request.params.clone());
        if let Some(system) = &request.system {
            completion_request = completion_request.with_system(system.clone());
        }

        let response = self.provider.complete(completion_request).await?;
        let cost = estimate_cost(&request.model, &response.usage);

        if response.stop_reason.wants_another_iteration() {
            return Err(AiError::Other(
                "the model asked for a tool call, but this harness does not dispatch tools yet :<"
                    .to_string(),
            ));
        }

        Ok(TurnOutcome::text(response.text(), response.usage, cost, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        provider::MockProvider,
        tools::{ConversationId, Platform, RiskTier, ToolCtx, ToolSelection},
        types::{History, Message, ModelRef},
    };

    fn ctx(granted_tier: RiskTier) -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Discord,
            granted_tier,
            guild_id: None,
            conversation_id: ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }

    fn request() -> TurnRequest {
        TurnRequest::new(
            ModelRef::new("anthropic", "claude-opus-5"),
            History::from(vec![Message::user("hi")]),
            ctx(RiskTier::Safe),
        )
    }

    #[tokio::test]
    async fn test_run_turn_returns_text_on_end_turn() {
        let provider = Arc::new(MockProvider::new().respond_text("hello there"));
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let outcome = harness.run_turn(request()).await.expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("hello there"));
        assert_eq!(outcome.iterations, 1);
        assert!(!outcome.is_handoff());
    }

    #[tokio::test]
    async fn test_run_turn_computes_cost_from_usage() {
        let provider = Arc::new(MockProvider::new().respond(Ok(
            crate::types::CompletionResponse::new(
                vec![crate::types::ContentBlock::text("hi")],
                crate::types::StopReason::EndTurn,
                crate::types::Usage::new(1_000_000, 1_000_000),
            ),
        )));
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let outcome = harness.run_turn(request()).await.expect("should succeed");

        assert!(
            outcome.cost > crate::types::Cost::ZERO,
            "a priced model with real usage should produce a nonzero cost"
        );
    }

    #[tokio::test]
    async fn test_run_turn_propagates_provider_errors() {
        let provider =
            Arc::new(MockProvider::new().respond_error(AiError::Rejected("bad key".to_string())));
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let result = harness.run_turn(request()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_turn_fails_honestly_on_tool_use_for_now() {
        let provider = Arc::new(MockProvider::new().respond_tool_use(
            "c1",
            "current_time",
            serde_json::json!({}),
        ));
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let result = harness.run_turn(request()).await;
        assert!(
            result.is_err(),
            "tool dispatch is not implemented yet, so this must fail rather than hang or silently \
             drop the call"
        );
    }

    #[tokio::test]
    async fn test_run_turn_offers_only_tools_the_selection_and_tier_both_permit() {
        use async_trait::async_trait;
        use serde_json::{Value, json};

        struct StubTool;

        #[async_trait]
        impl crate::tools::Tool for StubTool {
            fn name(&self) -> &str {
                "web_search"
            }

            fn description(&self) -> &str {
                "search the web"
            }

            fn tier(&self) -> RiskTier {
                RiskTier::NetworkRead
            }

            fn input_schema(&self) -> Value {
                json!({"type": "object", "properties": {}})
            }

            async fn invoke(&self, _input: Value, _ctx: &ToolCtx) -> crate::tools::ToolOutcome {
                crate::tools::ToolOutcome::ok("unused")
            }
        }

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(StubTool));

        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_text("hi"));
        let harness = Harness::new(provider.clone(), Arc::new(registry));

        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::NetworkRead);
        turn_request.ctx = ctx(RiskTier::Safe); // authorized only for Safe, not NetworkRead

        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        assert!(
            sent.tools.is_empty(),
            "web_search should not be offered when the invoker is only authorized for Safe"
        );
    }
}
