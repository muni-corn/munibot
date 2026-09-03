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
    audit::ToolAuditor,
    harness::{Harness, TurnRequest},
    persona::PersonaRegistry,
    pipeline::{AgentRole, PipelineId, persona_for},
    service::ProviderSource,
    tools::{Platform, RiskTier, ToolCtx, ToolRegistry},
    types::{AiError, Cost, History, Message, Usage},
    usage::{UsageRecord, UsageRecorder},
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
///
/// Carries `pipeline_id`, never a `ConversationId` - a pipeline run has
/// no stored conversation behind it at all. Before
/// `docs/plans/ai/milestone-7-projects.md` commit 230, this field held a
/// fabricated `ConversationId(pipeline_id.0)` instead, which collided
/// with a *real* conversation id the moment anything (memory, the
/// `todo_write` tool) recorded under it - the two id spaces are not the
/// same space, and pretending they were was the bug, not a convenience.
pub struct AgentContext {
    pub task: String,
    pub pipeline_id: PipelineId,
    pub cancellation: CancellationToken,
    /// The executor's own sandbox lifecycle (see `crate::pipeline::executor`)
    /// owns provisioning a sandbox and layering its tools on -- this is
    /// how that reaches a dispatcher that otherwise has no idea a sandbox
    /// exists, without the dispatcher having to manage sandbox state
    /// itself.
    pub tools: Option<Arc<ToolRegistry>>,
}

impl AgentContext {
    pub fn new(task: impl Into<String>, pipeline_id: PipelineId) -> Self {
        Self {
            task: task.into(),
            pipeline_id,
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
    /// `None` until [`Self::with_auditor`] enables it - matching every
    /// other optional-recorder builder in this crate (see
    /// `crate::service::Ai::with_tool_auditor`'s own doc comment).
    tool_auditor: Option<Arc<dyn ToolAuditor>>,
    /// `None` until [`Self::with_usage_recorder`] enables it.
    usage_recorder: Option<Arc<dyn UsageRecorder>>,
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
            tool_auditor: None,
            usage_recorder: None,
        }
    }

    /// Enables auditing every tool call a pipeline turn makes, the same
    /// as [`crate::service::Ai::with_tool_auditor`] does for a chat turn.
    pub fn with_auditor(mut self, auditor: Arc<dyn ToolAuditor>) -> Self {
        self.tool_auditor = Some(auditor);
        self
    }

    /// Enables recording a [`UsageRecord`] after every pipeline turn, on
    /// failure as well as success - the same as
    /// [`crate::service::Ai::with_usage_recorder`] does for a chat turn.
    pub fn with_usage_recorder(mut self, recorder: Arc<dyn UsageRecorder>) -> Self {
        self.usage_recorder = Some(recorder);
        self
    }
}

#[async_trait]
impl AgentDispatcher for HarnessDispatcher {
    #[tracing::instrument(
        skip_all,
        fields(role = ?role, pipeline_id = context.pipeline_id.0)
    )]
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

        let pipeline_id = context.pipeline_id;
        let ctx = ToolCtx {
            // no human user initiated this turn -- the pipeline itself did
            user_id: 0,
            platform: Platform::Pipeline,
            // an autonomous role is trusted for whatever its own tool
            // selection allows, never narrowed by a caller's own
            // permissions the way a chat invocation is
            granted_tier: RiskTier::Sandbox,
            guild_id: None,
            // no stored conversation behind a pipeline turn at all -- see
            // AgentContext's own doc comment on why this is pipeline_id,
            // never a fabricated ConversationId standing in for it
            conversation_id: None,
            pipeline_id: Some(pipeline_id.0),
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
        let mut harness = Harness::new(provider, tools);
        if let Some(auditor) = &self.tool_auditor {
            harness = harness.with_auditor(auditor.clone());
        }

        let (result, turn_usage) = harness.run_turn_recording_usage(request).await;

        if let Some(recorder) = &self.usage_recorder {
            let record = UsageRecord {
                conversation_id: None,
                user_id: None,
                guild_id: None,
                provider: persona.model.provider().to_string(),
                model: persona.model.model().to_string(),
                persona_id: persona.id.0.clone(),
                usage: turn_usage.usage,
                cost: turn_usage.cost,
                iterations: turn_usage.iterations,
                succeeded: result.is_ok(),
                pipeline_id: Some(pipeline_id.0),
            };
            if let Err(error) = recorder.record(record).await {
                tracing::warn!(%error, pipeline_id = pipeline_id.0, "failed to record pipeline usage");
            }
        }

        let outcome = result.map_err(|error| DispatchError::Turn(role, error))?;
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
    use std::collections::HashMap;

    use super::*;
    use crate::{
        persona::{AiConfig, BudgetConfig, MemoryPolicy, PersonaConfig, PersonaId, SandboxPolicy},
        pipeline::{IssueAnalysis, IssueClassification, RecommendedAction, ReproductionStatus},
        provider::{MockProvider, Provider, ProviderRegistry},
        tools::ToolSelection,
        types::ModelRef,
        usage::UsageRecorder,
    };

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
                AgentContext::new("write a plan", PipelineId(1)),
            )
            .await
            .unwrap();
        assert_eq!(first.handoff["action"], "CreatePlan");

        let second = dispatcher
            .invoke_agent(
                AgentRole::ArchitectureReviewer,
                AgentContext::new("review the plan", PipelineId(1)),
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
                AgentContext::new("write a plan", PipelineId(1)),
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
                AgentContext::new("implement it", PipelineId(1)),
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
                AgentContext::new("implement it", PipelineId(1)),
            )
            .await
            .ok();
    }

    #[test]
    fn test_agent_context_new_starts_with_a_fresh_cancellation_token() {
        let context = AgentContext::new("do something", PipelineId(1));
        assert!(!context.cancellation.is_cancelled());
    }

    /// A [`ProviderSource`] that always returns the same fixed provider,
    /// the same small test double `service.rs`'s own test module keeps.
    struct FixedProviderSource(Arc<dyn Provider>);

    impl ProviderSource for FixedProviderSource {
        fn resolve(&self, _model: &ModelRef) -> Result<Arc<dyn Provider>, AiError> {
            Ok(self.0.clone())
        }
    }

    /// A [`UsageRecorder`] that keeps everything it was asked to record.
    #[derive(Default)]
    struct FakeUsageRecorder {
        records: Mutex<Vec<UsageRecord>>,
    }

    impl FakeUsageRecorder {
        fn records(&self) -> Vec<UsageRecord> {
            self.records.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl UsageRecorder for FakeUsageRecorder {
        async fn record(&self, record: UsageRecord) -> Result<(), AiError> {
            self.records.lock().unwrap().push(record);
            Ok(())
        }
    }

    /// A real, offline-resolved [`PersonaRegistry`] with one persona at
    /// `issue-analyst`'s own conventional id and embedded prompt - enough
    /// for [`persona_for`] to resolve [`AgentRole::IssueAnalyst`] against
    /// it exactly as `HarnessDispatcher` does in production.
    fn issue_analyst_personas() -> Arc<PersonaRegistry> {
        let mut config = AiConfig {
            enabled: true,
            default_persona: None,
            default_model: None,
            prompt_dir: None,
            crisis_resources: Vec::new(),
            rate_limits: crate::persona::config::RateLimitConfig::default(),
            spend_caps: crate::persona::config::SpendCapConfig::default(),
            abuse: crate::persona::config::AbuseConfig::default(),
            sandbox: crate::sandbox::config::SandboxSettings::default(),
            projects: crate::project::ProjectsSettings::default(),
            max_delegation_depth: 2,
            personas: HashMap::new(),
        };
        config
            .personas
            .insert(PersonaId::new("issue-analyst"), PersonaConfig {
                model: Some(ModelRef::new("anthropic", "claude-opus-5")),
                prompt: "issue-analyst.md".to_string(),
                display_name: Some("Issue Analyst".to_string()),
                description: "triages an incoming issue".to_string(),
                temperature: None,
                tools: ToolSelection::none(),
                budget: BudgetConfig::default(),
                memory: MemoryPolicy::None,
                sandbox: SandboxPolicy::default(),
                delegable: false,
                moderation_fail_closed: None,
            });
        let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
        Arc::new(PersonaRegistry::load(&config, &providers).expect("should resolve"))
    }

    fn analysis() -> IssueAnalysis {
        IssueAnalysis {
            classification: IssueClassification::Bug,
            reproduction_status: ReproductionStatus::Reproduced,
            summary: "crashes on startup".to_string(),
            reproduction_details: "ran with no config".to_string(),
            recommended_action: RecommendedAction::Proceed,
            relevant_files: vec![],
        }
    }

    /// The whole point of commit 230: a real pipeline turn's usage is
    /// recorded under the pipeline's own id, never a conversation id at
    /// all - not the fabricated one this same commit deletes, and not a
    /// real one either, since a pipeline turn has no conversation behind
    /// it.
    #[tokio::test]
    async fn test_harness_dispatcher_records_usage_under_the_pipelines_own_id() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new().respond_tool_use(
            "c1",
            "handoff",
            serde_json::to_value(analysis()).unwrap(),
        ));
        let recorder = Arc::new(FakeUsageRecorder::default());

        let dispatcher = HarnessDispatcher::new(
            Arc::new(FixedProviderSource(provider)),
            issue_analyst_personas(),
            Arc::new(ToolRegistry::new()),
        )
        .with_usage_recorder(recorder.clone());

        let output = dispatcher
            .invoke_agent(
                AgentRole::IssueAnalyst,
                AgentContext::new("investigate the crash", PipelineId(99)),
            )
            .await
            .expect("should succeed");
        assert_eq!(output.handoff["summary"], "crashes on startup");

        let records = recorder.records();
        assert_eq!(records.len(), 1, "exactly one turn should be recorded");
        assert_eq!(records[0].pipeline_id, Some(99));
        assert_eq!(
            records[0].conversation_id, None,
            "a pipeline turn has no conversation behind it at all"
        );
        assert!(records[0].succeeded);
    }

    /// Usage is recorded on a failed turn too, matching
    /// `crate::service::Ai`'s own chat-path philosophy: a turn that
    /// errors partway through still spent money on the iterations it
    /// completed before failing.
    #[tokio::test]
    async fn test_harness_dispatcher_records_usage_even_when_the_turn_fails() {
        let provider: Arc<dyn Provider> = Arc::new(
            MockProvider::new().respond_error(AiError::Provider("the model is down".to_string())),
        );
        let recorder = Arc::new(FakeUsageRecorder::default());

        let dispatcher = HarnessDispatcher::new(
            Arc::new(FixedProviderSource(provider)),
            issue_analyst_personas(),
            Arc::new(ToolRegistry::new()),
        )
        .with_usage_recorder(recorder.clone());

        let error = dispatcher
            .invoke_agent(
                AgentRole::IssueAnalyst,
                AgentContext::new("investigate the crash", PipelineId(7)),
            )
            .await
            .expect_err("a provider error should fail the turn");
        assert!(matches!(
            error,
            DispatchError::Turn(AgentRole::IssueAnalyst, _)
        ));

        let records = recorder.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].pipeline_id, Some(7));
        assert!(
            !records[0].succeeded,
            "a failed turn is still recorded, just marked unsuccessful"
        );
    }
}
