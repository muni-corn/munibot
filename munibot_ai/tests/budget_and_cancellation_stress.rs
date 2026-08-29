//! Budget and cancellation stress suite (milestone 6, phase 23).
//!
//! Runs entirely against [`MockProvider`] and, for the cancellation tests,
//! a deliberately slow-but-real tool future - no network, no real
//! provider, no real sandbox container. Every test here scripts a
//! provider that would loop *forever* left unchecked (it always responds
//! with another tool call, or always calls a permanently-invalid one, or
//! never resolves at all) and proves one specific budget dimension - or
//! cancellation itself - is what actually stops it, not the script running
//! out (every "infinite" script here is scripted far past what any of
//! these tests should ever need).
//!
//! These are exactly the failure modes that only show up in production: a
//! persona whose budget was misconfigured, a provider that hangs instead
//! of erroring, a cancelled request whose tool call keeps running in the
//! background anyway. Nothing here depends on timing being *fast* to pass
//! (a real bug would hang or panic, not merely run slow), only on it being
//! deterministic - the same reasoning `injection_resistance.rs` documents
//! for testing against a scripted provider instead of a real one.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use munibot_ai::{
    harness::{Budget, Harness, TurnRequest},
    provider::{MockProvider, Provider},
    tools::{ConversationId, Platform, RiskTier, Tool, ToolCtx, ToolOutcome, ToolRegistry},
    types::{
        AiError, CompletionRequest, CompletionResponse, ContentBlock, History, Message, ModelRef,
        StopReason, Usage,
    },
};
use serde_json::{Value, json};

/// How many scripted responses is "far more than any budget in these
/// tests should ever allow through" - large enough that a test failing
/// because the *script* ran out (a test bug) is easy to tell apart from
/// the budget genuinely never tripping (the real bug this suite exists to
/// catch).
const FAR_MORE_THAN_ANY_BUDGET_ALLOWS: usize = 500;

fn ctx() -> ToolCtx {
    ToolCtx {
        user_id: 1,
        platform: Platform::Web,
        granted_tier: RiskTier::Safe,
        guild_id: None,
        conversation_id: ConversationId(1),
        cancellation: tokio_util::sync::CancellationToken::new(),
        delegation_depth: 0,
        remaining_budget: Budget::default(),
        delegation_spend: Arc::new(std::sync::atomic::AtomicI64::new(0)),
    }
}

fn request(budget: Budget) -> TurnRequest {
    TurnRequest::new(
        ModelRef::new("anthropic", "claude-opus-5"),
        History::from(vec![Message::user("go")]),
        ctx(),
    )
    .with_budget(budget)
}

fn unlimited() -> Budget {
    Budget {
        max_iterations: None,
        max_input_tokens: None,
        max_output_tokens: None,
        max_wall_clock: None,
        max_cost: None,
        max_tool_retries: None,
    }
}

/// A no-op tool that always succeeds, so an "infinite tool-calling loop"
/// script has something real to dispatch each iteration rather than
/// tripping on an unknown-tool refusal instead of the budget under test.
struct NoopTool;

#[async_trait::async_trait]
impl Tool for NoopTool {
    fn name(&self) -> &str {
        "noop"
    }

    fn description(&self) -> &str {
        "does nothing"
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Safe
    }

    fn input_schema(&self) -> Value {
        // `additionalProperties: false` matters here: without it, JSON
        // Schema allows extra properties by default, which would make
        // `test_max_tool_retries_terminates_a_persistently_invalid_tool_call_loop`'s
        // own "invalid" call schema-valid after all
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    async fn invoke(&self, _input: Value, _ctx: &ToolCtx) -> ToolOutcome {
        ToolOutcome::ok("done")
    }
}

/// A provider that always responds with another `noop` tool call, an
/// arbitrarily large but finite number of times - "an otherwise-infinite
/// loop" without needing an actually-unbounded script.
fn endless_tool_calling_provider() -> Arc<MockProvider> {
    let mut provider = MockProvider::new();
    for i in 0..FAR_MORE_THAN_ANY_BUDGET_ALLOWS {
        provider = provider.respond_tool_use(format!("c{i}"), "noop", json!({}));
    }
    Arc::new(provider)
}

fn registry_with_noop() -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(NoopTool));
    Arc::new(registry)
}

async fn run_with_timeout(
    harness: &Harness,
    turn_request: TurnRequest,
) -> munibot_ai::harness::TurnOutcome {
    tokio::time::timeout(Duration::from_secs(10), harness.run_turn(turn_request))
        .await
        .expect("a tripped budget must stop the loop well within this generous timeout")
        .expect("a budget limit must truncate gracefully, never fail the turn outright")
}

#[tokio::test]
async fn test_max_iterations_terminates_an_otherwise_infinite_tool_calling_loop() {
    let harness = Harness::new(endless_tool_calling_provider(), registry_with_noop());
    let mut turn_request = request(Budget {
        max_iterations: Some(3),
        ..unlimited()
    });
    turn_request.tools = munibot_ai::tools::ToolSelection::named(["noop"]);

    let outcome = run_with_timeout(&harness, turn_request).await;
    assert!(
        outcome.text.unwrap_or_default().contains("iterations"),
        "the truncation should name iterations as why it stopped"
    );
}

#[tokio::test]
async fn test_max_cost_terminates_an_otherwise_infinite_tool_calling_loop() {
    let mut provider = MockProvider::new();
    for i in 0..FAR_MORE_THAN_ANY_BUDGET_ALLOWS {
        provider = provider.respond(Ok(CompletionResponse::new(
            vec![ContentBlock::tool_use(format!("c{i}"), "noop", json!({}))],
            StopReason::ToolUse,
            Usage::new(1_000_000, 1_000_000),
        )));
    }

    let harness = Harness::new(Arc::new(provider), registry_with_noop());
    let mut turn_request = request(Budget {
        max_cost: Some(munibot_ai::types::Cost::from_dollars(0.01)),
        ..unlimited()
    });
    turn_request.tools = munibot_ai::tools::ToolSelection::named(["noop"]);

    let outcome = run_with_timeout(&harness, turn_request).await;
    assert!(outcome.text.unwrap_or_default().contains("cost"));
}

#[tokio::test]
async fn test_max_input_tokens_terminates_an_otherwise_infinite_tool_calling_loop() {
    let mut provider = MockProvider::new();
    for i in 0..FAR_MORE_THAN_ANY_BUDGET_ALLOWS {
        provider = provider.respond(Ok(CompletionResponse::new(
            vec![ContentBlock::tool_use(format!("c{i}"), "noop", json!({}))],
            StopReason::ToolUse,
            Usage::new(1_000_000, 0),
        )));
    }

    let harness = Harness::new(Arc::new(provider), registry_with_noop());
    let mut turn_request = request(Budget {
        max_input_tokens: Some(100),
        ..unlimited()
    });
    turn_request.tools = munibot_ai::tools::ToolSelection::named(["noop"]);

    let outcome = run_with_timeout(&harness, turn_request).await;
    assert!(outcome.text.unwrap_or_default().contains("input tokens"));
}

#[tokio::test]
async fn test_max_output_tokens_terminates_an_otherwise_infinite_tool_calling_loop() {
    let mut provider = MockProvider::new();
    for i in 0..FAR_MORE_THAN_ANY_BUDGET_ALLOWS {
        provider = provider.respond(Ok(CompletionResponse::new(
            vec![ContentBlock::tool_use(format!("c{i}"), "noop", json!({}))],
            StopReason::ToolUse,
            Usage::new(0, 1_000_000),
        )));
    }

    let harness = Harness::new(Arc::new(provider), registry_with_noop());
    let mut turn_request = request(Budget {
        max_output_tokens: Some(100),
        ..unlimited()
    });
    turn_request.tools = munibot_ai::tools::ToolSelection::named(["noop"]);

    let outcome = run_with_timeout(&harness, turn_request).await;
    assert!(outcome.text.unwrap_or_default().contains("output tokens"));
}

#[tokio::test]
async fn test_max_wall_clock_terminates_an_otherwise_infinite_but_fast_loop() {
    // every individual call in this script resolves instantly - proving
    // this is the *cumulative*, between-iterations wall clock check, not
    // the single-hung-call deadline `race_provider` itself adds
    let harness = Harness::new(endless_tool_calling_provider(), registry_with_noop());
    let mut turn_request = request(Budget {
        max_wall_clock: Some(Duration::from_millis(50)),
        ..unlimited()
    });
    turn_request.tools = munibot_ai::tools::ToolSelection::named(["noop"]);

    let outcome = run_with_timeout(&harness, turn_request).await;
    assert!(outcome.text.unwrap_or_default().contains("wall clock"));
}

#[tokio::test]
async fn test_max_tool_retries_terminates_a_persistently_invalid_tool_call_loop() {
    // "noop" takes no arguments, so a call naming an argument it never
    // declared fails schema validation every single time - a model stuck
    // making the same mistake, never a real, dispatchable call at all
    let mut provider = MockProvider::new();
    for i in 0..FAR_MORE_THAN_ANY_BUDGET_ALLOWS {
        provider =
            provider.respond_tool_use(format!("c{i}"), "noop", json!({"unexpected": "field"}));
    }

    let harness = Harness::new(Arc::new(provider), registry_with_noop());
    let mut turn_request = request(Budget {
        max_tool_retries: Some(2),
        ..unlimited()
    });
    turn_request.tools = munibot_ai::tools::ToolSelection::named(["noop"]);

    let result = tokio::time::timeout(Duration::from_secs(10), harness.run_turn(turn_request))
        .await
        .expect("a tripped retry budget must stop the loop well within this generous timeout");

    assert!(
        matches!(result, Err(AiError::SchemaViolation(_))),
        "giving up after too many invalid tool calls should be a schema violation, got {result:?}"
    );
}

/// A tool that increments `live` on entry and, via an RAII guard whose
/// `Drop` impl decrements it again, only ever goes back down once its own
/// future actually finishes *or is dropped* - proving genuine cleanup, not
/// just that the harness's own await returned promptly while this tool
/// silently kept running in the background.
struct LiveTrackingTool {
    delay: Duration,
    live: Arc<AtomicUsize>,
}

struct LiveGuard(Arc<AtomicUsize>);

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl Tool for LiveTrackingTool {
    fn name(&self) -> &str {
        "slow"
    }

    fn description(&self) -> &str {
        "sleeps, tracking whether it is still alive"
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Safe
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn invoke(&self, _input: Value, _ctx: &ToolCtx) -> ToolOutcome {
        self.live.fetch_add(1, Ordering::SeqCst);
        let _guard = LiveGuard(self.live.clone());
        tokio::time::sleep(self.delay).await;
        ToolOutcome::ok("finished")
    }
}

#[tokio::test]
async fn test_cancellation_actually_drops_a_running_tools_own_future() {
    let live = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(LiveTrackingTool {
        delay: Duration::from_secs(3600),
        live: live.clone(),
    }));

    let provider = Arc::new(MockProvider::new().respond_tool_use("c1", "slow", json!({})));
    let harness = Harness::new(provider, Arc::new(registry));

    let mut turn_request = request(Budget::default());
    turn_request.tools = munibot_ai::tools::ToolSelection::named(["slow"]);
    let cancellation = turn_request.ctx.cancellation.clone();

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancellation.cancel();
    });

    let result = tokio::time::timeout(Duration::from_secs(5), harness.run_turn(turn_request))
        .await
        .expect("the turn must return well before the tool's own hour-long delay");
    assert!(matches!(result, Err(AiError::Cancelled)));

    assert_eq!(
        live.load(Ordering::SeqCst),
        0,
        "the tool's own future must have actually been dropped by cancellation, not merely \
         abandoned to keep running in the background - a real orphaned task (or, for a real tool, \
         an orphaned sandbox container) would leave this counter at 1"
    );
}

/// A provider whose `complete` never resolves at all - proven bounded by
/// the wall clock budget alone in `harness.rs`'s own
/// `test_a_hanging_provider_is_bounded_by_the_wall_clock_budget_*`; this
/// test instead proves the same thing from cancellation's own side: a
/// hung provider call is *also* interruptible by cancellation, not only by
/// its budget running out, and interrupting it never leaves anything
/// running behind it either (there is nothing to drop on the provider
/// side, but the turn itself must still exit promptly).
struct NeverRespondingProvider;

#[async_trait::async_trait]
impl Provider for NeverRespondingProvider {
    fn name(&self) -> &str {
        "never-responds"
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AiError> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn test_cancelling_a_turn_stuck_on_a_hanging_provider_call_still_exits_promptly() {
    let harness = Harness::new(
        Arc::new(NeverRespondingProvider),
        Arc::new(ToolRegistry::new()),
    );

    let turn_request = request(unlimited()); // no wall clock budget - only cancellation can end this
    let cancellation = turn_request.ctx.cancellation.clone();

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancellation.cancel();
    });

    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(5), harness.run_turn(turn_request))
        .await
        .expect(
            "cancellation must interrupt a hanging provider call even with no wall clock budget \
             at all",
        );

    assert!(matches!(result, Err(AiError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(1));
}
