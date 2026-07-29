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
    tools::{ToolCtx, ToolRegistry},
    types::{AiError, CompletionRequest, ContentBlock, Cost, Message, Role, Usage},
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

    /// Runs one full turn: call the provider, dispatch every tool call it asks
    /// for, and repeat until it answers with plain text.
    ///
    /// Budget enforcement, handoff validation, cancellation, and parallel tool
    /// dispatch all arrive in later commits; this one covers the loop's
    /// basic shape and sequential tool calls only.
    pub async fn run_turn(&self, request: TurnRequest) -> Result<TurnOutcome, AiError> {
        let tool_schemas = self
            .tools
            .schemas_for(&request.tools, request.ctx.granted_tier);

        let mut history = request.history.clone();
        let mut iterations = 0usize;
        let mut total_usage = Usage::default();
        let mut total_cost = Cost::ZERO;

        loop {
            iterations += 1;

            let mut completion_request =
                CompletionRequest::new(request.model.clone(), history.clone())
                    .with_tools(tool_schemas.clone())
                    .with_params(request.params.clone());
            if let Some(system) = &request.system {
                completion_request = completion_request.with_system(system.clone());
            }

            let response = self.provider.complete(completion_request).await?;
            total_usage += response.usage;
            total_cost += estimate_cost(&request.model, &response.usage);

            if !response.stop_reason.wants_another_iteration() {
                return Ok(TurnOutcome::text(
                    response.text(),
                    total_usage,
                    total_cost,
                    iterations,
                ));
            }

            // capture what each call needs before response.content moves into history below
            let calls: Vec<(String, String, serde_json::Value)> = response
                .tool_uses()
                .iter()
                .filter_map(|block| block.as_tool_use())
                .map(|(call_id, name, arguments)| {
                    (call_id.to_string(), name.to_string(), arguments.clone())
                })
                .collect();

            history.push(Message::new(Role::Assistant, response.content));

            let mut results = Vec::with_capacity(calls.len());
            for (call_id, name, arguments) in calls {
                match self.dispatch_tool(&name, arguments, &request.ctx).await {
                    crate::tools::ToolOutcome::Ok(text) => {
                        results.push(ContentBlock::tool_result(call_id, text));
                    }
                    crate::tools::ToolOutcome::Err(text) => {
                        results.push(ContentBlock::tool_error(call_id, text));
                    }
                    crate::tools::ToolOutcome::Fatal(error) => return Err(error),
                }
            }
            history.push(Message::tool_results(results));
        }
    }

    /// Looks a tool up and runs it, or produces a model-visible error naming
    /// the available tools when the model asks for one that does not exist.
    /// A persona misconfiguration or a model hallucinating a tool name is
    /// something the model itself can recover from, given a clear
    /// enough error, so this is never a hard failure.
    async fn dispatch_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
        ctx: &ToolCtx,
    ) -> crate::tools::ToolOutcome {
        match self.tools.get(name) {
            Some(tool) => tool.invoke(arguments, ctx).await,
            None => {
                let available = self.tools.names().join(", ");
                crate::tools::ToolOutcome::err(format!(
                    "no such tool {name:?} :< available tools are: {available}"
                ))
            }
        }
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

    /// Always succeeds, echoing a fixed reply so tests can assert on it
    /// downstream.
    struct EchoTool {
        name: &'static str,
        reply: &'static str,
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for EchoTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "echoes a fixed reply"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            crate::tools::ToolOutcome::ok(self.reply)
        }
    }

    /// Always aborts the turn, for testing that a Fatal outcome stops the loop.
    struct FatalTool;

    #[async_trait::async_trait]
    impl crate::tools::Tool for FatalTool {
        fn name(&self) -> &str {
            "explode"
        }

        fn description(&self) -> &str {
            "always fails fatally"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            crate::tools::ToolOutcome::fatal(AiError::Cancelled)
        }
    }

    #[tokio::test]
    async fn test_run_turn_dispatches_a_registered_tool_and_returns_final_text() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is 12:00"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("it is 12:00"));
        assert_eq!(
            outcome.iterations, 2,
            "one tool round trip plus the final text round trip"
        );

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");
        assert!(
            matches!(last_message.role, crate::types::Role::Tool),
            "the tool result should have been appended as a Tool-role message"
        );
        let ContentBlock::ToolResult { content, .. } = &last_message.content[0] else {
            panic!("expected the appended message to carry a tool result block");
        };
        assert_eq!(
            content, "12:00",
            "the tool's own reply should be what the model sees back"
        );
    }

    #[tokio::test]
    async fn test_run_turn_unknown_tool_is_recoverable_not_fatal() {
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "does_not_exist", serde_json::json!({}))
                .respond_text("okay, moving on"),
        );

        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));
        let outcome = harness
            .run_turn(request())
            .await
            .expect("an unknown tool must not be fatal");

        assert_eq!(outcome.text.as_deref(), Some("okay, moving on"));

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");
        let ContentBlock::ToolResult {
            content, is_error, ..
        } = &last_message.content[0]
        else {
            panic!("expected the appended message to carry a tool result block");
        };
        assert!(
            *is_error,
            "an unknown tool should be reported as an error result"
        );
        assert!(
            content.contains("no such tool"),
            "the model should see why its call failed, so it can pick a real tool next time: \
             {content:?}"
        );
    }

    #[tokio::test]
    async fn test_run_turn_fatal_tool_outcome_aborts_the_turn() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FatalTool));

        let provider: Arc<MockProvider> =
            Arc::new(MockProvider::new().respond_tool_use("c1", "explode", serde_json::json!({})));
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let result = harness.run_turn(turn_request).await;

        assert!(
            result.is_err(),
            "a Fatal tool outcome must abort the whole turn"
        );
    }

    #[tokio::test]
    async fn test_run_turn_dispatches_every_call_in_a_multi_tool_response() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "first",
            reply: "one",
        }));
        registry.register(Arc::new(EchoTool {
            name: "second",
            reply: "two",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::tool_use("c1", "first", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "second", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("both done"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["first", "second"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("both done"));

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");
        assert_eq!(
            last_message.content.len(),
            2,
            "both tool calls from the one response should produce one result each"
        );
    }

    #[tokio::test]
    async fn test_run_turn_accumulates_usage_and_cost_across_iterations() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![ContentBlock::tool_use(
                        "c1",
                        "current_time",
                        serde_json::json!({}),
                    )],
                    crate::types::StopReason::ToolUse,
                    Usage::new(100, 100),
                )))
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![ContentBlock::text("done")],
                    crate::types::StopReason::EndTurn,
                    Usage::new(50, 50),
                ))),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(
            outcome.usage,
            Usage::new(150, 150),
            "usage from both iterations should be summed, not just the last one"
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
