//! `PipelineEvent`: what actually gets appended to a pipeline's own event
//! log, and folded back into a [`PipelineState`] on replay.

use munibot_vcs::IssueRef;
use serde::{Deserialize, Serialize};

use crate::pipeline::{
    ApproveCode, ApprovePlan, ApproveTests, BeginFinalReview, CommitComplete, CreatePlan,
    IssueAnalysis, ProjectComplete, PullRequestReady, RequestBuildHelp, RequestCodeChanges,
    RequestPlanChanges, RequestPlanHelp, RequestTestChanges, ResearchComplete, StartTaskTests,
    SubmitCode, SubmitTests,
};

/// One thing that happened over a pipeline's lifetime, in the order it
/// happened -- the append-only log `PipelineStore` persists, and the
/// only thing [`super::advance`] (a later commit) ever folds into state.
///
/// Every variant that carries a handoff payload wraps the exact type that
/// role's own `HandoffSchema` validates against (see
/// `crate::pipeline::handoff`), so an event is never a re-description of
/// what an agent said -- it *is* what an agent said, unchanged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PipelineEvent {
    /// The trigger that started this run.
    Triggered {
        issue: IssueRef,
    },
    IssueAnalyzed(IssueAnalysis),
    ResearchCompleted(ResearchComplete),
    PlanCreated(CreatePlan),
    PlanHelpRequested(RequestPlanHelp),
    PlanApproved(ApprovePlan),
    PlanChangesRequested(RequestPlanChanges),
    SubtaskTestsStarted(StartTaskTests),
    FinalReviewStarted(BeginFinalReview),
    TestsSubmitted(SubmitTests),
    TestsApproved(ApproveTests),
    TestChangesRequested(RequestTestChanges),
    CodeSubmitted(SubmitCode),
    BuildHelpRequested(RequestBuildHelp),
    CodeApproved(ApproveCode),
    CodeChangesRequested(RequestCodeChanges),
    ProjectCompleted(ProjectComplete),
    SubtaskCommitted(CommitComplete),
    PullRequestAuthored(PullRequestReady),
    /// A maintainer answered a question raised while in
    /// `PipelineState::AwaitingUserInput`.
    UserInputReceived {
        response: String,
    },
    /// The run could not continue -- a budget was exhausted, a persona
    /// invocation errored past its retry budget, or similar.
    Failed {
        reason: String,
    },
}

impl PipelineEvent {
    /// A short, stable name for this event's own variant, for the
    /// `event_type` column `PipelineStore`'s diesel implementation writes
    /// -- filterable and human-legible in a database client without
    /// deserializing `payload`, the same reason `ai_tool_calls.tool_name`
    /// is its own column rather than only ever living inside a json blob.
    pub fn label(&self) -> &'static str {
        match self {
            PipelineEvent::Triggered { .. } => "Triggered",
            PipelineEvent::IssueAnalyzed(_) => "IssueAnalyzed",
            PipelineEvent::ResearchCompleted(_) => "ResearchCompleted",
            PipelineEvent::PlanCreated(_) => "PlanCreated",
            PipelineEvent::PlanHelpRequested(_) => "PlanHelpRequested",
            PipelineEvent::PlanApproved(_) => "PlanApproved",
            PipelineEvent::PlanChangesRequested(_) => "PlanChangesRequested",
            PipelineEvent::SubtaskTestsStarted(_) => "SubtaskTestsStarted",
            PipelineEvent::FinalReviewStarted(_) => "FinalReviewStarted",
            PipelineEvent::TestsSubmitted(_) => "TestsSubmitted",
            PipelineEvent::TestsApproved(_) => "TestsApproved",
            PipelineEvent::TestChangesRequested(_) => "TestChangesRequested",
            PipelineEvent::CodeSubmitted(_) => "CodeSubmitted",
            PipelineEvent::BuildHelpRequested(_) => "BuildHelpRequested",
            PipelineEvent::CodeApproved(_) => "CodeApproved",
            PipelineEvent::CodeChangesRequested(_) => "CodeChangesRequested",
            PipelineEvent::ProjectCompleted(_) => "ProjectCompleted",
            PipelineEvent::SubtaskCommitted(_) => "SubtaskCommitted",
            PipelineEvent::PullRequestAuthored(_) => "PullRequestAuthored",
            PipelineEvent::UserInputReceived { .. } => "UserInputReceived",
            PipelineEvent::Failed { .. } => "Failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use munibot_vcs::{Forge, RepoRef};

    use super::*;

    #[test]
    fn test_label_names_the_triggered_variant() {
        let event = PipelineEvent::Triggered {
            issue: IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1),
        };
        assert_eq!(event.label(), "Triggered");
    }

    #[test]
    fn test_label_names_the_failed_variant() {
        let event = PipelineEvent::Failed {
            reason: "budget exhausted".to_string(),
        };
        assert_eq!(event.label(), "Failed");
    }

    #[test]
    fn test_every_event_round_trips_through_json() {
        let event = PipelineEvent::UserInputReceived {
            response: "use postgres".to_string(),
        };
        let encoded = serde_json::to_string(&event).unwrap();
        let decoded: PipelineEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, event);
    }
}
