//! The autonomous development pipeline: triage, research, planning, review,
//! implementation, and a pull request, orchestrated over the same harness
//! and personas every other delegable role already uses.
//!
//! See `docs/plans/ai/milestone-5-autonomous.md` for the full design. This
//! module reaches `munibot_vcs::IssueSource` and
//! `munibot_vcs::PullRequestTarget` through that crate directly, never
//! `munibot_github` -- the pipeline is forge-agnostic by construction, the
//! same way the harness itself is provider-agnostic.

mod advance;
mod branch;
mod concurrency;
mod dispatch;
mod event;
mod executor;
mod github_interaction;
mod handoff;
mod handoff_schema;
mod interaction;
mod plan;
mod state;
mod store;
mod web_chat_interaction;

pub use advance::{AdvanceError, advance};
pub use branch::resolve_branch_name;
pub use concurrency::{ConcurrencyConfig, PipelineRegistry};
pub use dispatch::{
    AgentContext, AgentDispatcher, AgentOutput, DispatchError, HarnessDispatcher,
    MockAgentDispatcher,
};
pub use event::PipelineEvent;
pub use executor::{
    Executor, ExecutorError, ExecutorOutcome, NoSandbox, SandboxLifecycle, role_for_state,
};
pub use github_interaction::GitHubCommentAdapter;
pub use handoff::{
    AgentRole, ApproveCode, ApprovePlan, ApproveTests, ArchitectureReviewerHandoff,
    BeginFinalReview, BuilderHandoff, CodeReviewerHandoff, CommitComplete, CreatePlan,
    FinalCodeReviewerHandoff, FixSubtask, IssueAnalysis, IssueClassification, ProjectComplete,
    ProjectManagerHandoff, PullRequestReady, RecommendedAction, ReproductionStatus,
    RequestBuildHelp, RequestCodeChanges, RequestPlanChanges, RequestPlanHelp, RequestTestChanges,
    ResearchComplete, SoftwareArchitectHandoff, StartTaskTests, SubmitCode, SubmitTests,
    SubtaskDraft, TestReviewerHandoff,
};
pub use handoff_schema::{handoff_schema_for, persona_for, persona_id_for};
pub use interaction::{
    InteractionAdapter, InteractionError, InteractionResponse, MockInteractionAdapter,
};
pub use plan::{Plan, Subtask, SubtaskStatus};
pub use state::{InteractionRequest, PipelineId, PipelineState, SubtaskId};
pub use store::{DieselPipelineStore, InMemoryPipelineStore, PipelineStore, PipelineStoreError};
pub use web_chat_interaction::{PendingReplyRegistry, WebChatAdapter};
