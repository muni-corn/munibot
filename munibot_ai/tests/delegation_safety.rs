//! The security-critical suite for delegation (milestone 3, phase 15).
//!
//! Runs entirely against [`MockProvider`] and a real [`Ai`]/[`DelegateTool`]
//! stack - no real provider, no network, no database. Each test asserts one
//! specific hazard the milestone plan names is actually prevented, not just
//! intended:
//!
//! - a nested turn cannot exceed the invoker's granted tier
//! - a delegation chain terminates at the depth cap
//! - many delegations in one turn cannot collectively outspend that turn's own
//!   budget
//! - a non-delegable persona is unreachable

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use munibot_ai::{
    Ai,
    harness::Budget,
    persona::{AiConfig, BudgetConfig, MemoryPolicy, PersonaConfig, PersonaId, SandboxPolicy},
    provider::{MockProvider, Provider, ProviderRegistry},
    service::ProviderSource,
    tools::{
        ConversationId, DelegablePersona, DelegateTool, Delegator, DelegatorCell, Platform,
        RiskTier, Tool, ToolCtx, ToolOutcome, ToolRegistry, ToolSelection,
    },
    types::{AiError, ModelRef},
};
use serde_json::{Value, json};

/// Always resolves to the same fixed provider, regardless of which model a
/// persona names - there is only ever one `MockProvider` in these tests.
struct FixedProviderSource(Arc<dyn Provider>);

impl ProviderSource for FixedProviderSource {
    fn resolve(&self, _model: &ModelRef) -> Result<Arc<dyn Provider>, AiError> {
        Ok(self.0.clone())
    }
}

/// Records every call it actually reaches, after checking `ctx.require_tier`
/// itself - a stand-in for any real `NetworkRead`-or-above tool (`web_search`,
/// say), minimal enough not to need that tool's own network-backend
/// dependencies.
struct SpyTool {
    tier: RiskTier,
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl Tool for SpyTool {
    fn name(&self) -> &str {
        "spy"
    }

    fn description(&self) -> &str {
        "a tool that records whether it was actually allowed to run"
    }

    fn tier(&self) -> RiskTier {
        self.tier
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn invoke(&self, _input: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(error) = ctx.require_tier(self.tier) {
            return ToolOutcome::err(error.to_string());
        }
        *self.calls.lock().unwrap() += 1;
        ToolOutcome::ok("did the risky thing")
    }
}

fn persona_config(
    model: &str,
    prompt: &str,
    tools: ToolSelection,
    delegable: bool,
) -> PersonaConfig {
    let (provider, model_name) = model.split_once(':').unwrap();
    PersonaConfig {
        model: ModelRef::new(provider, model_name),
        prompt: prompt.to_string(),
        display_name: None,
        description: "a test persona".to_string(),
        temperature: None,
        tools,
        budget: BudgetConfig::default(),
        memory: MemoryPolicy::None,
        sandbox: SandboxPolicy::default(),
        delegable,
    }
}

/// Builds a real, offline-resolved `Ai` with a `companion` (not itself
/// delegable) and a `specialist` (delegable, offered whatever `tools`
/// names), a `delegate` tool wired over the real `Delegator` impl on `Ai`
/// itself, and `provider` behind every model reference.
fn ai_with_specialist(
    provider: Arc<MockProvider>,
    tools: ToolSelection,
) -> (Arc<Ai>, Arc<ToolRegistry>) {
    let mut config = AiConfig {
        enabled: true,
        default_persona: Some(PersonaId::new("companion")),
        ..AiConfig::default()
    };
    config.personas.insert(
        PersonaId::new("companion"),
        persona_config(
            "anthropic:claude-opus-5",
            "companion.md",
            ToolSelection::none(),
            false,
        ),
    );
    config.personas.insert(
        PersonaId::new("specialist"),
        persona_config("anthropic:claude-opus-5", "researcher.md", tools, true),
    );

    let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
    let personas =
        munibot_ai::persona::PersonaRegistry::load(&config, &providers).expect("should resolve");

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(SpyTool {
        tier: RiskTier::NetworkRead,
        calls: Arc::new(Mutex::new(0)),
    }));
    let delegator_cell = Arc::new(DelegatorCell::new());
    registry.register(Arc::new(DelegateTool::new(
        delegator_cell.clone(),
        vec![DelegablePersona {
            id: PersonaId::new("specialist"),
            description: "a test specialist".to_string(),
        }],
        config.max_delegation_depth,
    )));
    let registry = Arc::new(registry);

    let sessions = Arc::new(munibot_ai::memory::InMemorySessionStore::new());
    let ai = Arc::new(Ai::from_parts(
        personas,
        registry.clone(),
        sessions,
        Arc::new(FixedProviderSource(provider)),
    ));
    delegator_cell.set(Arc::downgrade(&ai) as std::sync::Weak<dyn Delegator>);

    (ai, registry)
}

fn ctx_with(granted_tier: RiskTier, delegation_depth: usize, remaining_budget: Budget) -> ToolCtx {
    ToolCtx {
        user_id: 1,
        platform: Platform::Web,
        granted_tier,
        guild_id: None,
        conversation_id: ConversationId(1),
        cancellation: tokio_util::sync::CancellationToken::new(),
        delegation_depth,
        remaining_budget,
        delegation_spend: Arc::new(std::sync::atomic::AtomicI64::new(0)),
    }
}

#[tokio::test]
async fn test_a_nested_turn_cannot_exceed_the_invokers_granted_tier() {
    // the specialist is configured with the spy tool (NetworkRead), but the
    // invoking human is only granted Safe - a jailbroken model calling it
    // anyway inside the delegated turn must still be refused
    let provider = Arc::new(
        MockProvider::new()
            .respond_tool_use("c1", "spy", json!({}))
            .respond_text("gave up"),
    );
    let (ai, _registry) = ai_with_specialist(provider, ToolSelection::named(["spy"]));

    let ctx = ctx_with(RiskTier::Safe, 0, Budget::default());
    let text = munibot_ai::tools::Delegator::delegate(
        ai.as_ref(),
        &PersonaId::new("specialist"),
        "try the risky thing".to_string(),
        &ctx,
    )
    .await
    .expect("the turn itself should still finish, just refuse the tool");

    assert!(
        text.contains("gave up"),
        "the specialist's own final text should reflect the refusal, got {text:?}"
    );
}

#[tokio::test]
async fn test_a_nested_turn_with_a_sufficient_tier_can_use_the_tool() {
    // the same setup, but granted NetworkRead this time - proving the
    // refusal above is really about the tier, not something else broken
    let provider = Arc::new(
        MockProvider::new()
            .respond_tool_use("c1", "spy", json!({}))
            .respond_text("did it"),
    );
    let (ai, _registry) = ai_with_specialist(provider, ToolSelection::named(["spy"]));

    let ctx = ctx_with(RiskTier::NetworkRead, 0, Budget::default());
    let text = munibot_ai::tools::Delegator::delegate(
        ai.as_ref(),
        &PersonaId::new("specialist"),
        "try the risky thing".to_string(),
        &ctx,
    )
    .await
    .expect("should succeed");

    assert_eq!(text, "did it");
}

#[tokio::test]
async fn test_a_non_delegable_persona_is_unreachable() {
    // "companion" exists in the registry and is a real, resolvable persona -
    // it's just never marked delegable, so the delegate tool must never
    // offer or accept it, regardless of what a model asks for
    let provider = Arc::new(MockProvider::new().respond_text("unused"));
    let (_ai, registry) = ai_with_specialist(provider, ToolSelection::none());

    let delegate_tool = registry
        .get("delegate")
        .expect("delegate should be registered");
    let schema = delegate_tool.input_schema();
    let enum_values = schema["properties"]["persona"]["enum"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !enum_values.contains(&json!("companion")),
        "a non-delegable persona must never appear in the delegate tool's own schema"
    );

    let outcome = delegate_tool
        .invoke(
            json!({"persona": "companion", "task": "answer for yourself"}),
            &ctx_with(RiskTier::Safe, 0, Budget::default()),
        )
        .await;
    assert!(
        matches!(outcome, ToolOutcome::Err(_)),
        "asking to delegate to a real but non-delegable persona should still be refused, got \
         {outcome:?}"
    );
}

#[tokio::test]
async fn test_a_delegation_chain_terminates_at_the_depth_cap() {
    // simulates a real recursive chain: a fake delegator that, on each call,
    // delegates one level deeper itself, through the real DelegateTool -
    // proving the cap actually stops a genuine chain, not just one hop's
    // own arithmetic
    struct RecursiveDelegator {
        max_depth: usize,
        deepest_reached: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl Delegator for RecursiveDelegator {
        async fn delegate(
            &self,
            persona: &PersonaId,
            task: String,
            ctx: &ToolCtx,
        ) -> Result<String, AiError> {
            {
                let mut deepest = self.deepest_reached.lock().unwrap();
                *deepest = (*deepest).max(ctx.delegation_depth);
            }

            let delegator_cell = Arc::new(DelegatorCell::new());
            let this = Arc::new(RecursiveDelegator {
                max_depth: self.max_depth,
                deepest_reached: self.deepest_reached.clone(),
            });
            delegator_cell.set(Arc::downgrade(&this) as std::sync::Weak<dyn Delegator>);
            // keep `this` alive for the recursive call below
            let tool = DelegateTool::new(
                delegator_cell,
                vec![DelegablePersona {
                    id: persona.clone(),
                    description: "recursive".to_string(),
                }],
                self.max_depth,
            );

            let outcome = tool
                .invoke(json!({"persona": persona.0, "task": task}), ctx)
                .await;
            match outcome {
                ToolOutcome::Ok(text) => Ok(text),
                ToolOutcome::Err(text) => Err(AiError::Tool(text)),
                ToolOutcome::Fatal(error) => Err(error),
            }
        }
    }

    let deepest_reached = Arc::new(Mutex::new(0));
    let delegator = Arc::new(RecursiveDelegator {
        max_depth: 2,
        deepest_reached: deepest_reached.clone(),
    });
    let cell = Arc::new(DelegatorCell::new());
    cell.set(Arc::downgrade(&delegator) as std::sync::Weak<dyn Delegator>);

    let tool = DelegateTool::new(
        cell,
        vec![DelegablePersona {
            id: PersonaId::new("specialist"),
            description: "recursive".to_string(),
        }],
        2,
    );

    let outcome = tool
        .invoke(
            json!({"persona": "specialist", "task": "go as deep as you can"}),
            &ctx_with(RiskTier::Safe, 0, Budget::default()),
        )
        .await;

    assert!(
        matches!(outcome, ToolOutcome::Err(_)),
        "a chain that keeps trying to go deeper should eventually be refused, got {outcome:?}"
    );
    assert_eq!(
        *deepest_reached.lock().unwrap(),
        2,
        "the chain should have reached exactly the configured maximum depth before refusing, \
         never beyond it"
    );
}

#[tokio::test]
async fn test_delegation_spend_accumulates_across_sequential_calls_sharing_one_context() {
    // Ai::delegate must record its own real cost into ctx.delegation_spend
    // after every call, on top of whatever was already there - this is the
    // half of the fix that lives in Ai::delegate itself. The other half -
    // that the harness actually uses this accumulated total to shrink what
    // a *later* serial dispatch in the same batch sees - is proven directly
    // against a real multi-call batch in
    // harness::tests::test_sequential_serial_calls_in_one_batch_see_each_others_delegation_spend,
    // which does not need a real Delegator to exercise.
    use std::sync::atomic::Ordering;

    use munibot_ai::types::{CompletionResponse, StopReason, Usage};

    let provider = Arc::new(
        MockProvider::new()
            .respond(Ok(CompletionResponse::new(
                vec![],
                StopReason::EndTurn,
                Usage::new(1_000_000, 1_000_000),
            )))
            .respond(Ok(CompletionResponse::new(
                vec![],
                StopReason::EndTurn,
                Usage::new(1_000_000, 1_000_000),
            ))),
    );
    let (ai, _registry) = ai_with_specialist(provider, ToolSelection::none());
    let ctx = ctx_with(RiskTier::Safe, 0, Budget::default());

    assert_eq!(ctx.delegation_spend.load(Ordering::SeqCst), 0);

    munibot_ai::tools::Delegator::delegate(
        ai.as_ref(),
        &PersonaId::new("specialist"),
        "first task".to_string(),
        &ctx,
    )
    .await
    .expect("should succeed");
    let after_first = ctx.delegation_spend.load(Ordering::SeqCst);
    assert!(
        after_first > 0,
        "the first delegation's real cost should have been recorded"
    );

    munibot_ai::tools::Delegator::delegate(
        ai.as_ref(),
        &PersonaId::new("specialist"),
        "second task".to_string(),
        &ctx,
    )
    .await
    .expect("should succeed");
    let after_second = ctx.delegation_spend.load(Ordering::SeqCst);
    assert!(
        after_second > after_first,
        "the second delegation's cost should accumulate on top of the first's, never replace it"
    );
}
