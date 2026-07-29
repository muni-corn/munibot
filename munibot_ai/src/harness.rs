//! The agent loop: model to tools to handoff, with budgets and events.
//!
//! This is the one place that drives a [`crate::provider::Provider`] and a
//! [`crate::tools::ToolRegistry`] together. Everything above it - personas,
//! platform adapters, the eventual pipeline - talks to the harness and never to
//! a provider or a tool directly.

pub mod budget;
pub mod event;
pub mod turn;
pub mod validate;

use std::sync::Arc;

pub use budget::{Budget, BudgetTracker};
pub use event::HarnessEvent;
pub use turn::{HandoffSchema, TurnOutcome, TurnRequest};
pub use validate::validate_tool_arguments;

use crate::{
    provider::{Provider, estimate_cost},
    tools::{ToolCtx, ToolRegistry},
    types::{AiError, CompletionRequest, ContentBlock, Message, Role},
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
    /// Handoff validation and cancellation arrive in later commits; this one
    /// covers the loop's basic shape, tool dispatch and argument
    /// validation, and general budget enforcement.
    pub async fn run_turn(&self, request: TurnRequest) -> Result<TurnOutcome, AiError> {
        let tool_schemas = self
            .tools
            .schemas_for(&request.tools, request.ctx.granted_tier);

        let mut history = request.history.clone();
        let mut tracker = BudgetTracker::new(request.budget.clone());
        let mut tool_retries = 0usize;
        let mut last_text = String::new();

        loop {
            // don't start another round trip once a prior iteration already spent the
            // budget
            if let Err(reason) = tracker.check() {
                return Ok(Self::truncated_outcome(last_text, &tracker, &reason));
            }

            let mut completion_request =
                CompletionRequest::new(request.model.clone(), history.clone())
                    .with_tools(tool_schemas.clone())
                    .with_params(request.params.clone());
            if let Some(system) = &request.system {
                completion_request = completion_request.with_system(system.clone());
            }

            let response = self.provider.complete(completion_request).await?;
            tracker.record(
                response.usage,
                estimate_cost(&request.model, &response.usage),
            );
            last_text = response.text();

            // this iteration alone may have just spent what was left
            if let Err(reason) = tracker.check() {
                return Ok(Self::truncated_outcome(last_text, &tracker, &reason));
            }

            if !response.stop_reason.wants_another_iteration() {
                return Ok(TurnOutcome::text(
                    response.text(),
                    tracker.usage(),
                    tracker.cost(),
                    tracker.iterations(),
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

            // resolve and validate every call before dispatching any of them: an unknown
            // tool name or arguments that fail schema validation never reach
            // invoke() at all, and the model sees exactly why, so it can
            // correct itself on the next iteration
            let mut pending: Vec<Option<ContentBlock>> = calls.iter().map(|_| None).collect();
            let mut dispatchable: Vec<(
                usize,
                String,
                Arc<dyn crate::tools::Tool>,
                serde_json::Value,
            )> = Vec::new();

            for (index, (call_id, name, arguments)) in calls.iter().enumerate() {
                match self.tools.get(name) {
                    None => {
                        let available = self.tools.names().join(", ");
                        pending[index] = Some(ContentBlock::tool_error(
                            call_id.clone(),
                            format!("no such tool {name:?} :< available tools are: {available}"),
                        ));
                    }
                    Some(tool) => match validate_tool_arguments(&tool.input_schema(), arguments) {
                        Ok(()) => {
                            dispatchable.push((index, call_id.clone(), tool, arguments.clone()));
                        }
                        Err(validation_error) => {
                            tool_retries += 1;
                            if let Some(max) = request.budget.max_tool_retries
                                && tool_retries > max
                            {
                                return Err(AiError::SchemaViolation(format!(
                                    "gave up after {tool_retries} invalid tool calls in this turn \
                                     :< {validation_error}"
                                )));
                            }
                            pending[index] = Some(ContentBlock::tool_error(
                                call_id.clone(),
                                format!("invalid arguments for {name:?} :< {validation_error}"),
                            ));
                        }
                    },
                }
            }

            let to_dispatch: Vec<(Arc<dyn crate::tools::Tool>, serde_json::Value)> = dispatchable
                .iter()
                .map(|(_, _, tool, arguments)| (Arc::clone(tool), arguments.clone()))
                .collect();
            let outcomes = self.dispatch_calls(&to_dispatch, &request.ctx).await;

            for ((index, call_id, ..), outcome) in dispatchable.into_iter().zip(outcomes) {
                pending[index] = Some(match outcome {
                    crate::tools::ToolOutcome::Ok(text) => ContentBlock::tool_result(call_id, text),
                    crate::tools::ToolOutcome::Err(text) => ContentBlock::tool_error(call_id, text),
                    crate::tools::ToolOutcome::Fatal(error) => return Err(error),
                });
            }

            let results: Vec<ContentBlock> = pending
                .into_iter()
                .map(|result| {
                    result.expect("every call index is set during either validation or dispatch")
                })
                .collect();
            history.push(Message::tool_results(results));
        }
    }

    /// Builds the outcome for a turn that stopped early because its budget ran
    /// out, rather than because the model finished answering.
    ///
    /// A partial answer beats no answer: whatever text the last response
    /// carried is kept and marked as truncated, naming which limit was hit,
    /// rather than the turn failing outright. When no text has been
    /// produced yet at all - a degenerately tight budget tripping before the
    /// first response even arrives - the marker stands alone.
    fn truncated_outcome(
        last_text: String,
        tracker: &BudgetTracker,
        reason: &AiError,
    ) -> TurnOutcome {
        let text = if last_text.is_empty() {
            format!("(no response yet :< {reason})")
        } else {
            format!("{last_text}\n\n(response truncated :< {reason})")
        };
        TurnOutcome::text(text, tracker.usage(), tracker.cost(), tracker.iterations())
    }

    /// Dispatches every already-resolved, already-validated call, preserving
    /// result order to match call order regardless of which ones ran
    /// concurrently.
    ///
    /// Calls to a tool that reports [`crate::tools::Tool::is_serial`] run one
    /// at a time, in their relative order; every other call runs
    /// concurrently via [`futures::future::join_all`]. A tool with shared
    /// mutable state across calls - a persistent shell session inside one
    /// sandbox is the motivating case - is the only thing that needs the serial
    /// path; independent calls (a search alongside a file read) gain
    /// nothing from being serialized.
    ///
    /// A `Fatal` outcome is not short-circuited: every already-dispatched call
    /// in this batch is allowed to finish before `run_turn` aborts the
    /// turn, since cancelling siblings mid-flight is a separate concern for
    /// a later commit.
    async fn dispatch_calls(
        &self,
        calls: &[(Arc<dyn crate::tools::Tool>, serde_json::Value)],
        ctx: &ToolCtx,
    ) -> Vec<crate::tools::ToolOutcome> {
        let mut parallel_indices = Vec::new();
        let mut serial_indices = Vec::new();

        for (index, (tool, _)) in calls.iter().enumerate() {
            if tool.is_serial() {
                serial_indices.push(index);
            } else {
                parallel_indices.push(index);
            }
        }

        let parallel_futures = parallel_indices.iter().map(|&index| {
            let (tool, arguments) = &calls[index];
            tool.invoke(arguments.clone(), ctx)
        });
        let parallel_outcomes = futures::future::join_all(parallel_futures).await;

        let mut serial_outcomes = Vec::with_capacity(serial_indices.len());
        for &index in &serial_indices {
            let (tool, arguments) = &calls[index];
            serial_outcomes.push(tool.invoke(arguments.clone(), ctx).await);
        }

        let mut outcomes: Vec<Option<crate::tools::ToolOutcome>> =
            (0..calls.len()).map(|_| None).collect();
        for (index, outcome) in parallel_indices.into_iter().zip(parallel_outcomes) {
            outcomes[index] = Some(outcome);
        }
        for (index, outcome) in serial_indices.into_iter().zip(serial_outcomes) {
            outcomes[index] = Some(outcome);
        }

        outcomes
            .into_iter()
            .map(|outcome| {
                outcome
                    .expect("every call index is assigned by exactly one of the two groups above")
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        provider::MockProvider,
        tools::{ConversationId, Platform, RiskTier, ToolCtx, ToolSelection},
        types::{History, Message, ModelRef, Usage},
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

    /// Requires a string `query` argument; echoes it back on success.
    struct SchemaedTool;

    #[async_trait::async_trait]
    impl crate::tools::Tool for SchemaedTool {
        fn name(&self) -> &str {
            "search"
        }

        fn description(&self) -> &str {
            "requires a query argument"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            })
        }

        async fn invoke(
            &self,
            input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            crate::tools::ToolOutcome::ok(format!("searched for {}", input["query"]))
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

    /// Sleeps for `delay`, tracking how many instances are in flight at once
    /// via shared counters, then replies with `reply`.
    struct ConcurrencyTrackingTool {
        name: &'static str,
        serial: bool,
        delay: std::time::Duration,
        reply: &'static str,
        in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        max_observed: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for ConcurrencyTrackingTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "tracks how many instances run concurrently"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        fn is_serial(&self) -> bool {
            self.serial
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            use std::sync::atomic::Ordering;

            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_observed.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);

            crate::tools::ToolOutcome::ok(self.reply)
        }
    }

    #[tokio::test]
    async fn test_two_calls_to_a_serial_tool_never_overlap() {
        use std::sync::{Arc as StdArc, atomic::AtomicUsize};

        let in_flight = StdArc::new(AtomicUsize::new(0));
        let max_observed = StdArc::new(AtomicUsize::new(0));

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ConcurrencyTrackingTool {
            name: "shell",
            serial: true,
            delay: std::time::Duration::from_millis(20),
            reply: "ran",
            in_flight: in_flight.clone(),
            max_observed: max_observed.clone(),
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::tool_use("c1", "shell", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "shell", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["shell"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(
            max_observed.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "two calls to the same serial tool in one batch must never run at the same time"
        );
    }

    #[tokio::test]
    async fn test_independent_tools_run_concurrently() {
        use std::sync::{Arc as StdArc, atomic::AtomicUsize};

        let in_flight = StdArc::new(AtomicUsize::new(0));
        let max_observed = StdArc::new(AtomicUsize::new(0));

        let mut registry = ToolRegistry::new();
        for name in ["first", "second"] {
            registry.register(Arc::new(ConcurrencyTrackingTool {
                name,
                serial: false,
                delay: std::time::Duration::from_millis(20),
                reply: "ran",
                in_flight: in_flight.clone(),
                max_observed: max_observed.clone(),
            }));
        }

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
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["first", "second"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(
            max_observed.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "two independent, non-serial tools should overlap rather than run one after the other"
        );
    }

    #[tokio::test]
    async fn test_result_order_matches_call_order_despite_differing_completion_times() {
        use std::sync::{Arc as StdArc, atomic::AtomicUsize};

        let in_flight = StdArc::new(AtomicUsize::new(0));
        let max_observed = StdArc::new(AtomicUsize::new(0));

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ConcurrencyTrackingTool {
            name: "slow",
            serial: false,
            delay: std::time::Duration::from_millis(30),
            reply: "slow-result",
            in_flight: in_flight.clone(),
            max_observed: max_observed.clone(),
        }));
        registry.register(Arc::new(ConcurrencyTrackingTool {
            name: "fast",
            serial: false,
            delay: std::time::Duration::from_millis(1),
            reply: "fast-result",
            in_flight: in_flight.clone(),
            max_observed: max_observed.clone(),
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    // slow is called first but finishes last; fast is called second but
                    // finishes first - the result order must still follow the call order
                    vec![
                        ContentBlock::tool_use("c1", "slow", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "fast", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["slow", "fast"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");

        let ContentBlock::ToolResult { content: first, .. } = &last_message.content[0] else {
            panic!("expected the first result to be a tool result block");
        };
        let ContentBlock::ToolResult {
            content: second, ..
        } = &last_message.content[1]
        else {
            panic!("expected the second result to be a tool result block");
        };

        assert_eq!(
            first, "slow-result",
            "the first call's result must appear first in output order"
        );
        assert_eq!(
            second, "fast-result",
            "even though it finished before the slow call did"
        );
    }

    #[tokio::test]
    async fn test_invalid_arguments_never_reach_invoke_and_are_recoverable() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SchemaedTool));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                // missing the required "query" field
                .respond_tool_use("c1", "search", serde_json::json!({}))
                .respond_text("okay, trying differently"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["search"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("invalid arguments should be recoverable, not fatal");

        assert_eq!(outcome.text.as_deref(), Some("okay, trying differently"));

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
            panic!("expected a tool result block");
        };
        assert!(
            *is_error,
            "invalid arguments should be reported as an error result"
        );
        assert!(
            content.contains("invalid arguments"),
            "the model should see why validation failed: {content:?}"
        );
    }

    #[tokio::test]
    async fn test_valid_arguments_reach_invoke() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SchemaedTool));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "search", serde_json::json!({"query": "cats"}))
                .respond_text("found some cats"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["search"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("found some cats"));
    }

    #[tokio::test]
    async fn test_repeated_invalid_arguments_eventually_abort_the_turn() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SchemaedTool));

        // one more invalid call than the default budget's max_tool_retries (3) allows
        let mut provider_builder = MockProvider::new();
        for _ in 0..4 {
            provider_builder =
                provider_builder.respond_tool_use("c1", "search", serde_json::json!({}));
        }
        let provider: Arc<MockProvider> = Arc::new(provider_builder);

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["search"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let result = harness.run_turn(turn_request).await;

        assert!(
            result.is_err(),
            "a model that never produces valid arguments must not loop forever"
        );
    }

    #[tokio::test]
    async fn test_run_turn_truncates_gracefully_when_iteration_budget_is_reached() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "loop_tool",
            reply: "again",
        }));

        // every response asks for another tool call, forever - only the budget stops
        // this
        let mut builder = MockProvider::new();
        for _ in 0..5 {
            builder = builder.respond_tool_use("c1", "loop_tool", serde_json::json!({}));
        }
        let provider: Arc<MockProvider> = Arc::new(builder);

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["loop_tool"]);
        turn_request.ctx = ctx(RiskTier::Safe);
        turn_request.budget = Budget {
            max_iterations: Some(2),
            ..Budget::default()
        };

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("an exhausted budget should truncate gracefully, not error");

        assert_eq!(
            outcome.iterations, 2,
            "should stop exactly at the configured limit"
        );
        assert_eq!(
            provider.request_count(),
            2,
            "must not make a third round trip once the iteration budget is spent"
        );
        let text = outcome.text.unwrap();
        assert!(
            text.contains("budget"),
            "the truncation reason should be visible in the returned text, got {text:?}"
        );
    }

    #[tokio::test]
    async fn test_run_turn_truncates_gracefully_when_cost_budget_is_exceeded() {
        // claude-opus-5 is priced at $15/mtok input and $75/mtok output in
        // pricing.toml, so one million of each costs $90 - comfortably over a
        // one dollar budget in a single iteration
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond(Ok(
            crate::types::CompletionResponse::new(
                vec![
                    ContentBlock::text("partial answer"),
                    ContentBlock::tool_use("c1", "current_time", serde_json::json!({})),
                ],
                crate::types::StopReason::ToolUse,
                Usage::new(1_000_000, 1_000_000),
            ),
        )));

        let mut turn_request = request();
        turn_request.budget = Budget {
            max_cost: Some(crate::types::Cost::from_dollars(1.0)),
            ..Budget::default()
        };

        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("an exhausted cost budget should truncate gracefully, not error");

        assert_eq!(
            provider.request_count(),
            1,
            "must not make a second round trip once the cost budget is already spent"
        );
        let text = outcome.text.expect("partial text should be preserved");
        assert!(
            text.contains("partial answer"),
            "the assistant's own text should survive: {text:?}"
        );
        assert!(
            text.contains("truncated"),
            "the truncation should be visible: {text:?}"
        );
    }

    #[tokio::test]
    async fn test_run_turn_truncation_marker_names_the_limit() {
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_tool_use("c2", "current_time", serde_json::json!({})),
        );
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["current_time"]);
        turn_request.ctx = ctx(RiskTier::Safe);
        turn_request.budget = Budget {
            max_iterations: Some(1),
            ..Budget::default()
        };

        let harness = Harness::new(provider, Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should truncate gracefully");

        let text = outcome.text.expect("should have a truncation marker");
        assert!(
            text.contains("iterations"),
            "the marker should name which limit was hit, got {text:?}"
        );
    }
}
