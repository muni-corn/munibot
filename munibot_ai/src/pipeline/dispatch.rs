//! `AgentDispatcher`: running one pipeline role's turn and getting its
//! handoff back, over the same harness every other delegable persona uses.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, atomic::AtomicI64},
};

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    harness::{Harness, TurnRequest},
    persona::PersonaRegistry,
    pipeline::{AgentRole, persona_for},
    service::ProviderSource,
    tools::{ConversationId, Platform, RiskTier, ToolCtx, ToolRegistry},
    types::{AiError, Cost, History, Message, Usage},
};

/// Why running one agent turn failed.
#[derive(Error, Debug)]
pub enum DispatchError {
    #[error("no persona is configured for {0:?}")]
    PersonaUnavailable(AgentRole),
    #[error("couldn't render {0:?}'s own system prompt: {1}")]
    Prompt(AgentRole, AiError),
    #[error("couldn't resolve a provider for {0:?}: {1}")]
    Provider(AgentRole, AiError),
    #[error("{0:?}'s own turn failed: {1}")]
    Turn(AgentRole, AiError),
    #[error("{0:?} ended its turn with plain text instead of calling handoff")]
    NoHandoff(AgentRole),
}

/// Everything one agent invocation needs beyond which role it's running
/// as: the task brief (the turn's entire input, the same
/// "no invoking history" boundary `Delegator::delegate` already draws),
/// the identifiers a nested tool call gets audited and cancelled under,
/// and, once a sandbox exists for this run, the tool registry a sandboxed
/// role should use instead of whatever base registry the dispatcher would
/// otherwise reach for.
pub struct AgentContext {
    pub task: String,
    pub conversation_id: ConversationId,
    pub cancellation: CancellationToken,
    /// The executor's own sandbox lifecycle (see `crate::pipeline::executor`)
    /// owns provisioning a sandbox and layering its tools on -- this is
    /// how that reaches a dispatcher that otherwise has no idea a sandbox
    /// exists, without the dispatcher having to manage sandbox state
    /// itself.
    pub tools: Option<Arc<ToolRegistry>>,
}

impl AgentContext {
    pub fn new(task: impl Into<String>, conversation_id: ConversationId) -> Self {
        Self {
            task: task.into(),
            conversation_id,
            cancellation: CancellationToken::new(),
            tools: None,
        }
    }

    /// Uses `tools` instead of the dispatcher's own base registry for this
    /// one invocation.
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }
}

/// What one completed agent turn produced.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentOutput {
    /// The validated handoff payload -- always present: every pipeline
    /// persona is run with a handoff schema, so a turn that ended with
    /// plain text instead is [`DispatchError::NoHandoff`], not a
    /// differently-shaped success.
    pub handoff: Value,
    pub usage: Usage,
    pub cost: Cost,
}

/// Runs one pipeline role's turn, returning its handoff.
///
/// A trait rather than depending on [`HarnessDispatcher`] directly, so the
/// executor (a later commit) is testable with no model calls at all -
/// see [`MockAgentDispatcher`].
#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    async fn invoke_agent(
        &self,
        role: AgentRole,
        context: AgentContext,
    ) -> Result<AgentOutput, DispatchError>;
}

/// The variables every pipeline persona's own prompt template needs --
/// the same `{{user_name}}`/`{{platform}}` framing every other persona
/// uses (see `docs/notes/persona-prompt-porting.md`). `{{platform}}` comes
/// from `Platform::Pipeline`'s own `Display`, the same as any other
/// persona; `{{user_name}}` has no chat user to draw from, so it names
/// the repository's maintainers instead, since that's who the pipeline is
/// ultimately acting on behalf of.
fn prompt_variables() -> HashMap<String, String> {
    HashMap::from([
        (
            "user_name".to_string(),
            "the repository's maintainers".to_string(),
        ),
        ("platform".to_string(), Platform::Pipeline.to_string()),
    ])
}

/// Runs a pipeline role's turn for real, over [`Harness::run_turn`].
pub struct HarnessDispatcher {
    providers: Arc<dyn ProviderSource>,
    personas: Arc<PersonaRegistry>,
    tools: Arc<ToolRegistry>,
}

impl HarnessDispatcher {
    pub fn new(
        providers: Arc<dyn ProviderSource>,
        personas: Arc<PersonaRegistry>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            providers,
            personas,
            tools,
        }
    }
}

#[async_trait]
impl AgentDispatcher for HarnessDispatcher {
    async fn invoke_agent(
        &self,
        role: AgentRole,
        context: AgentContext,
    ) -> Result<AgentOutput, DispatchError> {
        let persona =
            persona_for(role, &self.personas).ok_or(DispatchError::PersonaUnavailable(role))?;
        let provider = self
            .providers
            .resolve(&persona.model)
            .map_err(|error| DispatchError::Provider(role, error))?;
        let system = persona
            .system_prompt
            .render(&prompt_variables())
            .map_err(|error| DispatchError::Prompt(role, error))?;

        let mut history = History::new();
        history.push(Message::user(context.task));

        let ctx = ToolCtx {
            // no human user initiated this turn -- the pipeline itself did
            user_id: 0,
            platform: Platform::Pipeline,
            // an autonomous role is trusted for whatever its own tool
            // selection allows, never narrowed by a caller's own
            // permissions the way a chat invocation is
            granted_tier: RiskTier::Sandbox,
            guild_id: None,
            conversation_id: context.conversation_id,
            cancellation: context.cancellation,
            delegation_depth: 0,
            remaining_budget: persona.budget.clone(),
            delegation_spend: Arc::new(AtomicI64::new(0)),
        };

        let mut request = TurnRequest::new(persona.model.clone(), history, ctx)
            .with_system(system)
            .with_tools(persona.tools.clone())
            .with_params(persona.params.clone())
            .with_budget(persona.budget.clone());
        if let Some(handoff) = persona.handoff.clone() {
            request = request.with_handoff(handoff);
        }

        let tools = context.tools.unwrap_or_else(|| self.tools.clone());
        let harness = Harness::new(provider, tools);
        let outcome = harness
            .run_turn(request)
            .await
            .map_err(|error| DispatchError::Turn(role, error))?;

        let handoff = outcome.handoff.ok_or(DispatchError::NoHandoff(role))?;

        Ok(AgentOutput {
            handoff,
            usage: outcome.usage,
            cost: outcome.cost,
        })
    }
}

/// A scripted [`AgentDispatcher`], for testing the executor with no model
/// calls at all.
///
/// Every invocation is recorded (role and task), so a test can assert on
/// what the executor actually asked for, the same reasoning
/// [`crate::provider::mock::MockProvider`] already applies one layer
/// down.
#[derive(Default)]
pub struct MockAgentDispatcher {
    responses: Mutex<VecDeque<Result<AgentOutput, DispatchError>>>,
    calls: Mutex<Vec<(AgentRole, String)>>,
}

impl MockAgentDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues the next response `invoke_agent` returns, regardless of
    /// which role asks -- tests script one role at a time, in the order
    /// the executor is expected to call them.
    pub fn respond(self, output: Result<AgentOutput, DispatchError>) -> Self {
        self.responses
            .lock()
            .expect("mock lock poisoned")
            .push_back(output);
        self
    }

    /// Every `(role, task)` pair `invoke_agent` was actually called with,
    /// in call order.
    pub fn calls(&self) -> Vec<(AgentRole, String)> {
        self.calls.lock().expect("mock lock poisoned").clone()
    }
}

#[async_trait]
impl AgentDispatcher for MockAgentDispatcher {
    async fn invoke_agent(
        &self,
        role: AgentRole,
        context: AgentContext,
    ) -> Result<AgentOutput, DispatchError> {
        self.calls
            .lock()
            .expect("mock lock poisoned")
            .push((role, context.task));

        self.responses
            .lock()
            .expect("mock lock poisoned")
            .pop_front()
            .unwrap_or_else(|| {
                panic!("MockAgentDispatcher ran out of scripted responses for {role:?}")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(handoff: Value) -> AgentOutput {
        AgentOutput {
            handoff,
            usage: Usage::default(),
            cost: Cost::ZERO,
        }
    }

    #[tokio::test]
    async fn test_mock_dispatcher_returns_scripted_responses_in_order() {
        let dispatcher = MockAgentDispatcher::new()
            .respond(Ok(output(serde_json::json!({"action": "CreatePlan"}))))
            .respond(Ok(output(serde_json::json!({"action": "ApprovePlan"}))));

        let first = dispatcher
            .invoke_agent(
                AgentRole::SoftwareArchitect,
                AgentContext::new("write a plan", ConversationId(1)),
            )
            .await
            .unwrap();
        assert_eq!(first.handoff["action"], "CreatePlan");

        let second = dispatcher
            .invoke_agent(
                AgentRole::ArchitectureReviewer,
                AgentContext::new("review the plan", ConversationId(1)),
            )
            .await
            .unwrap();
        assert_eq!(second.handoff["action"], "ApprovePlan");
    }

    #[tokio::test]
    async fn test_mock_dispatcher_records_every_call() {
        let dispatcher = MockAgentDispatcher::new()
            .respond(Ok(output(serde_json::json!({"action": "CreatePlan"}))));

        dispatcher
            .invoke_agent(
                AgentRole::SoftwareArchitect,
                AgentContext::new("write a plan", ConversationId(1)),
            )
            .await
            .unwrap();

        let calls = dispatcher.calls();
        assert_eq!(calls, vec![(
            AgentRole::SoftwareArchitect,
            "write a plan".to_string()
        )]);
    }

    #[tokio::test]
    async fn test_mock_dispatcher_can_script_a_failure() {
        let dispatcher =
            MockAgentDispatcher::new().respond(Err(DispatchError::NoHandoff(AgentRole::Builder)));

        let error = dispatcher
            .invoke_agent(
                AgentRole::Builder,
                AgentContext::new("implement it", ConversationId(1)),
            )
            .await
            .expect_err("should surface the scripted error");
        assert!(matches!(
            error,
            DispatchError::NoHandoff(AgentRole::Builder)
        ));
    }

    #[tokio::test]
    #[should_panic(expected = "ran out of scripted responses")]
    async fn test_mock_dispatcher_panics_when_it_runs_out_of_scripted_responses() {
        let dispatcher = MockAgentDispatcher::new();
        dispatcher
            .invoke_agent(
                AgentRole::Builder,
                AgentContext::new("implement it", ConversationId(1)),
            )
            .await
            .ok();
    }

    #[test]
    fn test_agent_context_new_starts_with_a_fresh_cancellation_token() {
        let context = AgentContext::new("do something", ConversationId(1));
        assert!(!context.cancellation.is_cancelled());
    }
}
