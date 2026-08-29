//! The executor: the loop that actually drives a pipeline forward.
//!
//! Dispatch the agent for the current state, append the resulting event,
//! advance, persist, repeat until the run reaches a terminal state or has
//! nothing left to dispatch. Every iteration is durable -- the state
//! before and after each one is always a replay over
//! `PipelineStore::events`, never held only in memory -- so a crash
//! resumes a run rather than restarting it (see the resume-after-restart
//! commit for the part that actually reloads a run at startup).

use std::sync::Arc;

use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    pipeline::{
        AgentContext, AgentDispatcher, AgentRole, DispatchError, InteractionAdapter,
        InteractionError, PipelineEvent, PipelineId, PipelineState, PipelineStore,
        PipelineStoreError, SubtaskId, advance,
    },
    tools::{ConversationId, ToolRegistry},
};

/// Why one executor run stopped short of finishing normally.
#[derive(Error, Debug)]
pub enum ExecutorError {
    #[error(transparent)]
    Store(#[from] PipelineStoreError),
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    #[error("couldn't parse {0:?}'s own handoff payload: {1}")]
    Handoff(AgentRole, serde_json::Error),
    #[error("{0:?}'s own handoff named an action this role never produces")]
    UnexpectedAction(AgentRole),
    #[error("{role:?}'s own handoff produced an event {advance} would reject: {reason}")]
    IllegalTransition {
        role: AgentRole,
        advance: &'static str,
        reason: String,
    },
    #[error("couldn't provision a sandbox: {0}")]
    Sandbox(String),
    #[error(transparent)]
    Interaction(#[from] InteractionError),
    #[error("the run was aborted")]
    Cancelled,
}

/// Where a run stopped.
#[derive(Debug, PartialEq)]
pub enum ExecutorOutcome {
    /// Reached a terminal state (`Complete` or `Failed`).
    Finished(PipelineState),
    /// Stopped with more work ahead, but nothing to dispatch right now --
    /// in practice, always `AwaitingUserInput`.
    Paused(PipelineState),
}

/// Provisions and tears down the one sandbox a run's hands-on roles share.
///
/// A trait for the same reason [`AgentDispatcher`] is: tests substitute a
/// mock that never touches podman, and the real implementation (wrapping
/// `crate::sandbox::provision_if_needed` and a repository checkout) is a
/// documented, separate concern -- see
/// `docs/notes/pipeline-sandbox-wiring-gap.md` for exactly what is not
/// yet wired end to end.
#[async_trait::async_trait]
pub trait SandboxLifecycle: Send + Sync {
    /// Provisions a sandbox, returning the tool registry a sandboxed
    /// role's turn should run with instead of whatever base registry the
    /// dispatcher would otherwise reach for.
    async fn provision(&self) -> Result<Arc<ToolRegistry>, ExecutorError>;

    /// Tears down whatever `provision` set up. A no-op if `provision` was
    /// never called.
    async fn teardown(&self);
}

/// A [`SandboxLifecycle`] that never actually provisions anything --
/// every role runs against whichever base registry the dispatcher itself
/// holds.
///
/// The right choice for a role sequence that never needs one (an
/// `IssueAnalyst` skip, say), and for any test that only cares about the
/// executor's own loop mechanics.
pub struct NoSandbox {
    tools: Arc<ToolRegistry>,
}

impl NoSandbox {
    pub fn new(tools: Arc<ToolRegistry>) -> Self {
        Self { tools }
    }
}

#[async_trait::async_trait]
impl SandboxLifecycle for NoSandbox {
    async fn provision(&self) -> Result<Arc<ToolRegistry>, ExecutorError> {
        Ok(self.tools.clone())
    }

    async fn teardown(&self) {}
}

/// Which role should be dispatched while a run is in `state`, or `None`
/// when there is nothing to dispatch -- a terminal state, or
/// `AwaitingUserInput`, which waits on a human rather than an agent.
///
/// A pure function: this mapping is the entire "who does what" table for
/// the executor's own loop, readable start to finish in one place rather
/// than implicit in a chain of `if let` checks.
pub fn role_for_state(state: &PipelineState) -> Option<AgentRole> {
    use PipelineState as S;
    match state {
        S::Triaging => Some(AgentRole::IssueAnalyst),
        S::Researching => Some(AgentRole::CodebaseResearcher),
        S::Planning => Some(AgentRole::SoftwareArchitect),
        S::ReviewingPlan => Some(AgentRole::ArchitectureReviewer),
        S::Scheduling => Some(AgentRole::ProjectManager),
        S::TestWriting { .. } => Some(AgentRole::TestEngineer),
        S::TestReviewing { .. } => Some(AgentRole::TestReviewer),
        S::Building { .. } => Some(AgentRole::Builder),
        S::ReviewingCode { .. } => Some(AgentRole::CodeReviewer),
        S::Committing { .. } => Some(AgentRole::CommitCrafter),
        S::FinalReview => Some(AgentRole::FinalCodeReviewer),
        // the project manager synthesizes a fix subtask here -- see the
        // fix subtask synthesis commit
        S::AwaitingFixSubtask => Some(AgentRole::ProjectManager),
        S::WritingPr => Some(AgentRole::PrAuthor),
        S::AwaitingUserInput { .. } | S::Complete | S::Failed { .. } => None,
    }
}

/// Parses `role`'s own raw handoff `Value` into the [`PipelineEvent`] it
/// describes.
///
/// The inverse of `handoff_schema_for`: that function tells the model
/// what shape to produce, this one turns the shape it actually produced
/// into the pipeline's own event vocabulary.
fn event_from_handoff(role: AgentRole, handoff: Value) -> Result<PipelineEvent, ExecutorError> {
    use crate::pipeline::{
        ArchitectureReviewerHandoff, BuilderHandoff, CodeReviewerHandoff, FinalCodeReviewerHandoff,
        ProjectManagerHandoff, SoftwareArchitectHandoff, TestReviewerHandoff,
    };

    fn parse<T: serde::de::DeserializeOwned>(
        role: AgentRole,
        handoff: Value,
    ) -> Result<T, ExecutorError> {
        serde_json::from_value(handoff).map_err(|error| ExecutorError::Handoff(role, error))
    }

    Ok(match role {
        AgentRole::IssueAnalyst => PipelineEvent::IssueAnalyzed(parse(role, handoff)?),
        AgentRole::CodebaseResearcher => PipelineEvent::ResearchCompleted(parse(role, handoff)?),
        AgentRole::SoftwareArchitect => match parse(role, handoff)? {
            SoftwareArchitectHandoff::CreatePlan(create) => PipelineEvent::PlanCreated(create),
            SoftwareArchitectHandoff::RequestPlanHelp(help) => {
                PipelineEvent::PlanHelpRequested(help)
            }
        },
        AgentRole::ArchitectureReviewer => match parse(role, handoff)? {
            ArchitectureReviewerHandoff::ApprovePlan(approve) => {
                PipelineEvent::PlanApproved(approve)
            }
            ArchitectureReviewerHandoff::RequestPlanChanges(request) => {
                PipelineEvent::PlanChangesRequested(request)
            }
        },
        AgentRole::ProjectManager => match parse(role, handoff)? {
            ProjectManagerHandoff::StartTaskTests(start) => {
                PipelineEvent::SubtaskTestsStarted(start)
            }
            ProjectManagerHandoff::BeginFinalReview(begin) => {
                PipelineEvent::FinalReviewStarted(begin)
            }
            ProjectManagerHandoff::FixSubtask(fix) => PipelineEvent::FixSubtaskSynthesized(fix),
        },
        AgentRole::TestEngineer => PipelineEvent::TestsSubmitted(parse(role, handoff)?),
        AgentRole::TestReviewer => match parse(role, handoff)? {
            TestReviewerHandoff::ApproveTests(approve) => PipelineEvent::TestsApproved(approve),
            TestReviewerHandoff::RequestTestChanges(request) => {
                PipelineEvent::TestChangesRequested(request)
            }
        },
        AgentRole::Builder => match parse(role, handoff)? {
            BuilderHandoff::SubmitCode(submit) => PipelineEvent::CodeSubmitted(submit),
            BuilderHandoff::RequestBuildHelp(help) => PipelineEvent::BuildHelpRequested(help),
        },
        AgentRole::CodeReviewer => match parse(role, handoff)? {
            CodeReviewerHandoff::ApproveCode(approve) => PipelineEvent::CodeApproved(approve),
            CodeReviewerHandoff::RequestCodeChanges(request) => {
                PipelineEvent::CodeChangesRequested(request)
            }
        },
        AgentRole::FinalCodeReviewer => match parse(role, handoff)? {
            FinalCodeReviewerHandoff::RequestCodeChanges(request) => {
                PipelineEvent::CodeChangesRequested(request)
            }
            FinalCodeReviewerHandoff::ProjectComplete(complete) => {
                PipelineEvent::ProjectCompleted(complete)
            }
        },
        AgentRole::CommitCrafter => PipelineEvent::SubtaskCommitted(parse(role, handoff)?),
        AgentRole::PrAuthor => PipelineEvent::PullRequestAuthored(parse(role, handoff)?),
    })
}

/// The task brief `role`'s own turn should run with, given everything
/// that has happened in this run so far.
///
/// Deliberately simple: enough context for the role's own prompt (which
/// already carries the judgement and standards for its job) to act on,
/// referencing the most recent event actually relevant to it rather than
/// replaying the whole log.
fn task_brief(role: AgentRole, state: &PipelineState, events: &[PipelineEvent]) -> String {
    fn last<'a, T>(
        events: &'a [PipelineEvent],
        matcher: impl Fn(&'a PipelineEvent) -> Option<T>,
    ) -> Option<T> {
        events.iter().rev().find_map(matcher)
    }

    match role {
        AgentRole::IssueAnalyst => {
            let issue = last(events, |event| match event {
                PipelineEvent::Triggered { issue } => Some(issue.clone()),
                _ => None,
            });
            match issue {
                Some(issue) => format!("Triage {issue}."),
                None => "Triage this issue.".to_string(),
            }
        }
        AgentRole::CodebaseResearcher => {
            let summary = last(events, |event| match event {
                PipelineEvent::IssueAnalyzed(analysis) => Some(analysis.summary.clone()),
                _ => None,
            })
            .unwrap_or_default();
            format!("Research the codebase for this issue: {summary}")
        }
        AgentRole::SoftwareArchitect => {
            let summary = last(events, |event| match event {
                PipelineEvent::ResearchCompleted(research) => Some(research.summary.clone()),
                PipelineEvent::PlanChangesRequested(request) => Some(format!(
                    "the architecture reviewer requested changes: {}",
                    request.feedback
                )),
                _ => None,
            })
            .unwrap_or_default();
            format!("Write a plan. Context: {summary}")
        }
        AgentRole::ArchitectureReviewer => {
            let summary = last(events, |event| match event {
                PipelineEvent::PlanCreated(plan) => Some(plan.summary.clone()),
                _ => None,
            })
            .unwrap_or_default();
            format!("Review this plan: {summary}")
        }
        AgentRole::ProjectManager => {
            if matches!(state, PipelineState::AwaitingFixSubtask) {
                let feedback = last(events, |event| match event {
                    PipelineEvent::CodeChangesRequested(request) => Some(request.feedback.clone()),
                    _ => None,
                })
                .unwrap_or_default();
                format!(
                    "The final reviewer requested changes: {feedback}. Synthesize a fix subtask \
                     for it."
                )
            } else {
                "Decide what to work on next.".to_string()
            }
        }
        AgentRole::TestEngineer => {
            let subtask = subtask_of(state);
            format!("Write tests for subtask {subtask:?}.")
        }
        AgentRole::TestReviewer => {
            let summary = last(events, |event| match event {
                PipelineEvent::TestsSubmitted(submitted) => Some(submitted.summary.clone()),
                _ => None,
            })
            .unwrap_or_default();
            format!("Review these tests: {summary}")
        }
        AgentRole::Builder => {
            let subtask = subtask_of(state);
            format!("Implement subtask {subtask:?} against its approved tests.")
        }
        AgentRole::CodeReviewer => {
            let summary = last(events, |event| match event {
                PipelineEvent::CodeSubmitted(submitted) => Some(submitted.summary.clone()),
                _ => None,
            })
            .unwrap_or_default();
            format!("Review this implementation: {summary}")
        }
        AgentRole::FinalCodeReviewer => {
            "Review every change across every subtask against the original plan.".to_string()
        }
        AgentRole::CommitCrafter => {
            let subtask = subtask_of(state);
            format!("Commit the approved changes for subtask {subtask:?}.")
        }
        AgentRole::PrAuthor => "Write the pull request title and body.".to_string(),
    }
}

/// The subtask a subtask-scoped state names, if `state` is one.
fn subtask_of(state: &PipelineState) -> Option<&SubtaskId> {
    use PipelineState as S;
    match state {
        S::TestWriting { subtask }
        | S::TestReviewing { subtask }
        | S::Building { subtask }
        | S::ReviewingCode { subtask }
        | S::Committing { subtask } => Some(subtask),
        _ => None,
    }
}

/// Drives one pipeline run forward.
pub struct Executor {
    store: Arc<dyn PipelineStore>,
    dispatcher: Arc<dyn AgentDispatcher>,
    sandbox: Arc<dyn SandboxLifecycle>,
    /// Checked once per loop iteration, and threaded into every
    /// `AgentContext` so an in-flight turn's own tool calls see it too --
    /// what `PipelineRegistry::abort_pipeline` cancels to stop this run.
    /// A fresh, never-cancelled token by default; `with_cancellation`
    /// overrides it with one a registry actually holds onto.
    cancellation: CancellationToken,
}

impl Executor {
    pub fn new(
        store: Arc<dyn PipelineStore>,
        dispatcher: Arc<dyn AgentDispatcher>,
        sandbox: Arc<dyn SandboxLifecycle>,
    ) -> Self {
        Self {
            store,
            dispatcher,
            sandbox,
            cancellation: CancellationToken::new(),
        }
    }

    /// Runs with `cancellation` instead of a fresh, never-cancelled token
    /// -- what lets an external caller (in practice,
    /// `PipelineRegistry::abort_pipeline`) actually stop this run.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Runs `pipeline_id` forward from wherever its own event log
    /// currently resolves to, until it reaches a terminal state, has
    /// nothing left to dispatch, or is cancelled.
    ///
    /// Every iteration replays the store rather than tracking state in a
    /// local variable across iterations -- more calls than strictly
    /// necessary, but it means this function has exactly one source of
    /// truth for "what state is this run in", the same one a restart
    /// would use to resume it.
    pub async fn run(&self, pipeline_id: PipelineId) -> Result<ExecutorOutcome, ExecutorError> {
        let mut sandbox_tools: Option<Arc<ToolRegistry>> = None;

        loop {
            if self.cancellation.is_cancelled() {
                self.sandbox.teardown().await;
                return Err(ExecutorError::Cancelled);
            }

            let state = self.store.replay(pipeline_id).await?;

            if state.is_terminal() {
                self.sandbox.teardown().await;
                return Ok(ExecutorOutcome::Finished(state));
            }

            // provisioned the first time a run reaches any state past
            // Triaging (in the ordinary flow, that is Researching) --
            // phrased this way rather than "exactly Researching" so that
            // resuming a run that had already moved past it re-provisions
            // immediately, instead of never provisioning at all because
            // this particular call to run() never actually passes through
            // Researching itself
            if !matches!(state, PipelineState::Triaging) && sandbox_tools.is_none() {
                sandbox_tools = Some(self.sandbox.provision().await?);
            }

            let Some(role) = role_for_state(&state) else {
                self.sandbox.teardown().await;
                return Ok(ExecutorOutcome::Paused(state));
            };

            let events_so_far = self.store.events(pipeline_id).await?;
            let task = task_brief(role, &state, &events_so_far);
            let mut context = AgentContext::new(task, ConversationId(pipeline_id.0.unsigned_abs()));
            context.cancellation = self.cancellation.clone();
            if let Some(tools) = &sandbox_tools {
                context = context.with_tools(tools.clone());
            }

            let output = self.dispatcher.invoke_agent(role, context).await?;
            let event = event_from_handoff(role, output.handoff)?;

            // validated before persisting -- append_event enforces nothing
            // of its own, so a bad event must never reach the log at all
            advance(state, &event).map_err(|error| ExecutorError::IllegalTransition {
                role,
                advance: "advance",
                reason: error.to_string(),
            })?;

            self.store.append_event(pipeline_id, event).await?;
        }
    }

    /// Runs `pipeline_id` all the way to a terminal state, resolving every
    /// pause along the way through `adapter` -- what a caller that
    /// actually wants a finished run (rather than "one leg of it, maybe
    /// paused") should call.
    ///
    /// Each pause is a fresh `run` after the previous one already
    /// persisted `UserInputReceived`, so a crash between an answer
    /// arriving and the next `run` starting resumes exactly where the
    /// event log says it should, the same as any other iteration.
    pub async fn run_with_interaction(
        &self,
        pipeline_id: PipelineId,
        adapter: &dyn InteractionAdapter,
    ) -> Result<PipelineState, ExecutorError> {
        loop {
            match self.run(pipeline_id).await? {
                ExecutorOutcome::Finished(state) => return Ok(state),
                ExecutorOutcome::Paused(PipelineState::AwaitingUserInput { request, .. }) => {
                    let answer = adapter.request_input(pipeline_id, &request).await?;
                    self.store
                        .append_event(pipeline_id, PipelineEvent::UserInputReceived {
                            response: answer.response,
                        })
                        .await?;
                }
                // role_for_state only ever returns None for AwaitingUserInput
                // or a terminal state, and terminal states are handled by
                // the Finished arm above -- reachable only if that
                // invariant is ever broken, so surfaced rather than looped
                // on forever
                ExecutorOutcome::Paused(other) => return Ok(other),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use munibot_vcs::{Forge, IssueRef, RepoRef};

    use super::*;
    use crate::{
        pipeline::{
            AgentOutput, ApprovePlan, BeginFinalReview, CreatePlan, DispatchError,
            InMemoryPipelineStore, IssueAnalysis, IssueClassification, MockAgentDispatcher,
            PullRequestReady, RecommendedAction, ReproductionStatus, ResearchComplete,
        },
        types::{Cost, Usage},
    };

    fn issue() -> IssueRef {
        IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1)
    }

    fn ok_output(handoff: Value) -> Result<AgentOutput, DispatchError> {
        Ok(AgentOutput {
            handoff,
            usage: Usage::default(),
            cost: Cost::ZERO,
        })
    }

    async fn new_pipeline(store: &InMemoryPipelineStore) -> PipelineId {
        let id = store.create_pipeline(&issue()).await.unwrap();
        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();
        id
    }

    #[test]
    fn test_role_for_state_maps_every_non_terminal_state_to_a_role() {
        assert_eq!(
            role_for_state(&PipelineState::Triaging),
            Some(AgentRole::IssueAnalyst)
        );
        assert_eq!(
            role_for_state(&PipelineState::Researching),
            Some(AgentRole::CodebaseResearcher)
        );
        assert_eq!(
            role_for_state(&PipelineState::Scheduling),
            Some(AgentRole::ProjectManager)
        );
        assert_eq!(
            role_for_state(&PipelineState::AwaitingFixSubtask),
            Some(AgentRole::ProjectManager)
        );
        assert_eq!(
            role_for_state(&PipelineState::WritingPr),
            Some(AgentRole::PrAuthor)
        );
    }

    #[test]
    fn test_role_for_state_has_nothing_to_dispatch_when_terminal_or_awaiting_input() {
        assert_eq!(role_for_state(&PipelineState::Complete), None);
        assert_eq!(
            role_for_state(&PipelineState::Failed {
                reason: "?".to_string()
            }),
            None
        );
        assert_eq!(
            role_for_state(&PipelineState::AwaitingUserInput {
                request: crate::pipeline::InteractionRequest {
                    prompt: "?".to_string()
                },
                resume: Box::new(PipelineState::Triaging),
            }),
            None
        );
    }

    #[tokio::test]
    async fn test_executor_runs_a_skipped_issue_straight_to_complete() {
        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        let dispatcher = Arc::new(
            MockAgentDispatcher::new().respond(ok_output(
                serde_json::to_value(IssueAnalysis {
                    classification: IssueClassification::NotActionable,
                    reproduction_status: ReproductionStatus::NotApplicable,
                    summary: "spam".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::Skip,
                    relevant_files: vec![],
                })
                .unwrap(),
            )),
        );
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let executor = Executor::new(store.clone(), dispatcher, sandbox);

        let outcome = executor.run(id).await.unwrap();
        assert_eq!(outcome, ExecutorOutcome::Finished(PipelineState::Complete));
    }

    #[tokio::test]
    async fn test_executor_pauses_on_needs_more_info() {
        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        let dispatcher = Arc::new(
            MockAgentDispatcher::new().respond(ok_output(
                serde_json::to_value(IssueAnalysis {
                    classification: IssueClassification::Bug,
                    reproduction_status: ReproductionStatus::NoStepsProvided,
                    summary: "not enough detail".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::NeedsMoreInfo,
                    relevant_files: vec![],
                })
                .unwrap(),
            )),
        );
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let executor = Executor::new(store.clone(), dispatcher, sandbox);

        let outcome = executor.run(id).await.unwrap();
        assert!(matches!(
            outcome,
            ExecutorOutcome::Paused(PipelineState::AwaitingUserInput { .. })
        ));
    }

    #[tokio::test]
    async fn test_executor_provisions_a_sandbox_exactly_once_entering_researching() {
        struct CountingSandbox {
            provisions: std::sync::atomic::AtomicUsize,
            teardowns: std::sync::atomic::AtomicUsize,
        }

        #[async_trait::async_trait]
        impl SandboxLifecycle for CountingSandbox {
            async fn provision(&self) -> Result<Arc<ToolRegistry>, ExecutorError> {
                self.provisions
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(Arc::new(ToolRegistry::new()))
            }

            async fn teardown(&self) {
                self.teardowns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        let dispatcher = Arc::new(
            MockAgentDispatcher::new()
                .respond(ok_output(
                    serde_json::to_value(IssueAnalysis {
                        classification: IssueClassification::Bug,
                        reproduction_status: ReproductionStatus::Reproduced,
                        summary: "crashes".to_string(),
                        reproduction_details: String::new(),
                        recommended_action: RecommendedAction::Proceed,
                        relevant_files: vec![],
                    })
                    .unwrap(),
                ))
                .respond(ok_output(
                    serde_json::to_value(ResearchComplete {
                        summary: "uses axum".to_string(),
                        relevant_files: vec![],
                    })
                    .unwrap(),
                ))
                .respond(ok_output(serde_json::json!({
                    "action": "RequestPlanHelp",
                    "question": "redis or in-memory?",
                }))),
        );

        let sandbox = Arc::new(CountingSandbox {
            provisions: std::sync::atomic::AtomicUsize::new(0),
            teardowns: std::sync::atomic::AtomicUsize::new(0),
        });
        let executor = Executor::new(store.clone(), dispatcher, sandbox.clone());

        let outcome = executor.run(id).await.unwrap();
        assert!(matches!(
            outcome,
            ExecutorOutcome::Paused(PipelineState::AwaitingUserInput { .. })
        ));
        assert_eq!(
            sandbox.provisions.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            sandbox.teardowns.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn test_executor_tears_down_the_sandbox_on_reaching_a_terminal_state() {
        struct CountingSandbox {
            teardowns: std::sync::atomic::AtomicUsize,
        }

        #[async_trait::async_trait]
        impl SandboxLifecycle for CountingSandbox {
            async fn provision(&self) -> Result<Arc<ToolRegistry>, ExecutorError> {
                Ok(Arc::new(ToolRegistry::new()))
            }

            async fn teardown(&self) {
                self.teardowns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        let dispatcher = Arc::new(
            MockAgentDispatcher::new().respond(ok_output(
                serde_json::to_value(IssueAnalysis {
                    classification: IssueClassification::NotActionable,
                    reproduction_status: ReproductionStatus::NotApplicable,
                    summary: "spam".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::Skip,
                    relevant_files: vec![],
                })
                .unwrap(),
            )),
        );

        let sandbox = Arc::new(CountingSandbox {
            teardowns: std::sync::atomic::AtomicUsize::new(0),
        });
        let executor = Executor::new(store.clone(), dispatcher, sandbox.clone());

        executor.run(id).await.unwrap();
        assert_eq!(
            sandbox.teardowns.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn test_executor_persists_every_event_along_the_way() {
        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        let dispatcher = Arc::new(
            MockAgentDispatcher::new().respond(ok_output(
                serde_json::to_value(IssueAnalysis {
                    classification: IssueClassification::NotActionable,
                    reproduction_status: ReproductionStatus::NotApplicable,
                    summary: "spam".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::Skip,
                    relevant_files: vec![],
                })
                .unwrap(),
            )),
        );
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let executor = Executor::new(store.clone(), dispatcher, sandbox);

        executor.run(id).await.unwrap();

        let events = store.events(id).await.unwrap();
        assert_eq!(events.len(), 2, "Triggered plus IssueAnalyzed");
        assert_eq!(events[1].label(), "IssueAnalyzed");
    }

    #[tokio::test]
    async fn test_executor_rejects_an_illegal_handoff_without_persisting_it() {
        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        // an issue analyst that, implausibly, hands back a CommitComplete
        // -- a shape event_from_handoff will never produce for this role,
        // but a malformed/malicious model output could still send the
        // wrong json shape for its own role's schema
        let dispatcher = Arc::new(MockAgentDispatcher::new().respond(ok_output(
            serde_json::json!({"not": "a valid issue analysis"}),
        )));
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let executor = Executor::new(store.clone(), dispatcher, sandbox);

        let error = executor
            .run(id)
            .await
            .expect_err("should reject the malformed handoff");
        assert!(matches!(
            error,
            ExecutorError::Handoff(AgentRole::IssueAnalyst, _)
        ));

        let events = store.events(id).await.unwrap();
        assert_eq!(
            events.len(),
            1,
            "only the original Triggered event, nothing bad persisted"
        );
    }

    #[tokio::test]
    async fn test_executor_runs_the_final_review_to_writing_pr_and_completes() {
        let store = Arc::new(InMemoryPipelineStore::new());
        let id = store.create_pipeline(&issue()).await.unwrap();
        // fast-forward straight to FinalReview via direct event appends,
        // proving the executor resumes correctly from any legal state
        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();

        // walk through every intervening legal transition so replay stays
        // a legal history
        store
            .append_event(
                id,
                PipelineEvent::IssueAnalyzed(IssueAnalysis {
                    classification: IssueClassification::Bug,
                    reproduction_status: ReproductionStatus::Reproduced,
                    summary: "s".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::Proceed,
                    relevant_files: vec![],
                }),
            )
            .await
            .unwrap();
        store
            .append_event(
                id,
                PipelineEvent::ResearchCompleted(ResearchComplete {
                    summary: "s".to_string(),
                    relevant_files: vec![],
                }),
            )
            .await
            .unwrap();
        store
            .append_event(
                id,
                PipelineEvent::PlanCreated(CreatePlan {
                    summary: "s".to_string(),
                    subtasks: vec![],
                }),
            )
            .await
            .unwrap();
        store
            .append_event(
                id,
                PipelineEvent::PlanApproved(ApprovePlan {
                    strengths: "s".to_string(),
                    feedback: "f".to_string(),
                }),
            )
            .await
            .unwrap();
        store
            .append_event(id, PipelineEvent::FinalReviewStarted(BeginFinalReview {}))
            .await
            .unwrap();

        let dispatcher = Arc::new(
            MockAgentDispatcher::new()
                .respond(ok_output(serde_json::json!({"action": "ProjectComplete"})))
                .respond(ok_output(
                    serde_json::to_value(PullRequestReady {
                        title: "t".to_string(),
                        body: "b".to_string(),
                    })
                    .unwrap(),
                )),
        );
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let executor = Executor::new(store.clone(), dispatcher, sandbox);

        let outcome = executor.run(id).await.unwrap();
        assert_eq!(outcome, ExecutorOutcome::Finished(PipelineState::Complete));
    }

    #[tokio::test]
    async fn test_run_with_interaction_resolves_a_pause_and_keeps_going_to_completion() {
        use crate::pipeline::{InteractionResponse, MockInteractionAdapter};

        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        let dispatcher = Arc::new(
            MockAgentDispatcher::new()
                .respond(ok_output(
                    serde_json::to_value(IssueAnalysis {
                        classification: IssueClassification::Bug,
                        reproduction_status: ReproductionStatus::NoStepsProvided,
                        summary: "not enough detail".to_string(),
                        reproduction_details: String::new(),
                        recommended_action: RecommendedAction::NeedsMoreInfo,
                        relevant_files: vec![],
                    })
                    .unwrap(),
                ))
                .respond(ok_output(
                    serde_json::to_value(IssueAnalysis {
                        classification: IssueClassification::NotActionable,
                        reproduction_status: ReproductionStatus::NotApplicable,
                        summary: "actually just spam".to_string(),
                        reproduction_details: String::new(),
                        recommended_action: RecommendedAction::Skip,
                        relevant_files: vec![],
                    })
                    .unwrap(),
                )),
        );
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let executor = Executor::new(store.clone(), dispatcher, sandbox);

        let adapter = MockInteractionAdapter::new().answer(Ok(InteractionResponse::new(
            "no steps, it's just spam actually",
        )));

        let final_state = executor.run_with_interaction(id, &adapter).await.unwrap();
        assert_eq!(final_state, PipelineState::Complete);
        assert_eq!(
            adapter.requests().len(),
            1,
            "should have asked exactly once"
        );
    }

    #[tokio::test]
    async fn test_run_with_interaction_resolves_multiple_pauses_in_sequence() {
        use crate::pipeline::{InteractionResponse, MockInteractionAdapter};

        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        // pauses on NeedsMoreInfo twice before finally proceeding
        let dispatcher = Arc::new(
            MockAgentDispatcher::new()
                .respond(ok_output(
                    serde_json::to_value(IssueAnalysis {
                        classification: IssueClassification::Bug,
                        reproduction_status: ReproductionStatus::NoStepsProvided,
                        summary: "first question".to_string(),
                        reproduction_details: String::new(),
                        recommended_action: RecommendedAction::NeedsMoreInfo,
                        relevant_files: vec![],
                    })
                    .unwrap(),
                ))
                .respond(ok_output(
                    serde_json::to_value(IssueAnalysis {
                        classification: IssueClassification::Bug,
                        reproduction_status: ReproductionStatus::NoStepsProvided,
                        summary: "second question".to_string(),
                        reproduction_details: String::new(),
                        recommended_action: RecommendedAction::NeedsMoreInfo,
                        relevant_files: vec![],
                    })
                    .unwrap(),
                ))
                .respond(ok_output(
                    serde_json::to_value(IssueAnalysis {
                        classification: IssueClassification::NotActionable,
                        reproduction_status: ReproductionStatus::NotApplicable,
                        summary: "done".to_string(),
                        reproduction_details: String::new(),
                        recommended_action: RecommendedAction::Skip,
                        relevant_files: vec![],
                    })
                    .unwrap(),
                )),
        );
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let executor = Executor::new(store.clone(), dispatcher, sandbox);

        let adapter = MockInteractionAdapter::new()
            .answer(Ok(InteractionResponse::new("first answer")))
            .answer(Ok(InteractionResponse::new("second answer")));

        let final_state = executor.run_with_interaction(id, &adapter).await.unwrap();
        assert_eq!(final_state, PipelineState::Complete);
        assert_eq!(adapter.requests().len(), 2);
    }

    #[tokio::test]
    async fn test_executor_re_enters_test_writing_after_a_synthesized_fix_subtask() {
        use crate::pipeline::{RequestCodeChanges, SubmitTests, SubtaskId};

        let store = Arc::new(InMemoryPipelineStore::new());
        let id = store.create_pipeline(&issue()).await.unwrap();
        for event in [
            PipelineEvent::Triggered { issue: issue() },
            PipelineEvent::IssueAnalyzed(IssueAnalysis {
                classification: IssueClassification::Bug,
                reproduction_status: ReproductionStatus::Reproduced,
                summary: "s".to_string(),
                reproduction_details: String::new(),
                recommended_action: RecommendedAction::Proceed,
                relevant_files: vec![],
            }),
            PipelineEvent::ResearchCompleted(ResearchComplete {
                summary: "s".to_string(),
                relevant_files: vec![],
            }),
            PipelineEvent::PlanCreated(CreatePlan {
                summary: "s".to_string(),
                subtasks: vec![],
            }),
            PipelineEvent::PlanApproved(ApprovePlan {
                strengths: "s".to_string(),
                feedback: "f".to_string(),
            }),
            PipelineEvent::FinalReviewStarted(BeginFinalReview {}),
            PipelineEvent::CodeChangesRequested(RequestCodeChanges {
                feedback: "subtask 2 broke subtask 4's tests".to_string(),
            }),
        ] {
            store.append_event(id, event).await.unwrap();
        }
        assert_eq!(
            store.replay(id).await.unwrap(),
            PipelineState::AwaitingFixSubtask
        );

        let dispatcher = Arc::new(
            MockAgentDispatcher::new()
                .respond(ok_output(serde_json::json!({
                    "action": "FixSubtask",
                    "review_feedback": "subtask 2 broke subtask 4's tests",
                    "parent_subtask_id": "task-2",
                    "subtask": {
                        "id": "task-5",
                        "title": "fix the regression",
                        "description": "d",
                        "instructions": "i",
                        "commit_message": "fix: repair the regression",
                        "files_affected": [],
                        "dependencies": [],
                    },
                })))
                .respond(ok_output(
                    serde_json::to_value(SubmitTests {
                        subtask_id: SubtaskId("task-5".to_string()),
                        summary: "tests for the fix".to_string(),
                        assumptions: "none".to_string(),
                    })
                    .unwrap(),
                ))
                .respond(ok_output(
                    serde_json::json!({"action": "ApproveTests", "feedback": "good"}),
                ))
                .respond(ok_output(serde_json::json!({
                    "action": "RequestBuildHelp",
                    "question": "which module owns this regression?",
                }))),
        );
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let executor = Executor::new(store.clone(), dispatcher, sandbox);

        let outcome = executor.run(id).await.unwrap();
        assert!(matches!(
            outcome,
            ExecutorOutcome::Paused(PipelineState::AwaitingUserInput { .. })
        ));

        let events = store.events(id).await.unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.label() == "FixSubtaskSynthesized"),
            "the fix subtask synthesis event should have been persisted"
        );
    }

    #[tokio::test]
    async fn test_a_cancelled_token_stops_the_run_before_the_next_dispatch() {
        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        // no scripted response at all -- if the executor dispatched
        // anything, this would panic instead of returning Cancelled
        let dispatcher = Arc::new(MockAgentDispatcher::new());
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let executor =
            Executor::new(store.clone(), dispatcher, sandbox).with_cancellation(cancellation);

        let error = executor
            .run(id)
            .await
            .expect_err("a pre-cancelled run should stop");
        assert!(matches!(error, ExecutorError::Cancelled));
    }

    #[tokio::test]
    async fn test_cancellation_is_checked_before_dispatching_not_only_at_startup() {
        let store = Arc::new(InMemoryPipelineStore::new());
        let id = new_pipeline(&store).await;

        // no scripted response -- a real dispatch attempt would panic,
        // proving the cancellation check runs before ever reaching it
        let dispatcher = Arc::new(MockAgentDispatcher::new());
        let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
        let cancellation = CancellationToken::new();
        let executor = Executor::new(store.clone(), dispatcher, sandbox)
            .with_cancellation(cancellation.clone());

        cancellation.cancel();
        let error = executor
            .run(id)
            .await
            .expect_err("should stop once cancelled");
        assert!(matches!(error, ExecutorError::Cancelled));

        let events = store.events(id).await.unwrap();
        assert_eq!(
            events.len(),
            1,
            "only the original Triggered event, nothing dispatched"
        );
    }
}
