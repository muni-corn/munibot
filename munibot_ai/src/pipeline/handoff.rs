//! The twelve pipeline agent roles, and the payload each one's `handoff`
//! tool call must produce.
//!
//! Every type here derives `JsonSchema` so a later commit can turn it
//! straight into a [`crate::harness::HandoffSchema`] with
//! `schemars::schema_for!`, matching how every other tool in this crate
//! already declares its own arguments (see
//! `crate::types::ToolSchema::from_schemars`) -- the schema and the type a
//! response deserializes into can never drift apart, because they are the
//! same type.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::pipeline::SubtaskId;

/// Which of the twelve pipeline roles a turn is being run as.
///
/// See the pipeline diagram in `docs/plans/ai/milestone-5-autonomous.md`
/// for how these roles hand off to one another; each variant here is one
/// box in that diagram.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    IssueAnalyst,
    CodebaseResearcher,
    SoftwareArchitect,
    ArchitectureReviewer,
    ProjectManager,
    TestEngineer,
    TestReviewer,
    Builder,
    CodeReviewer,
    FinalCodeReviewer,
    CommitCrafter,
    PrAuthor,
}

/// How urgently, and how confidently, the issue analyst believes work
/// should proceed.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedAction {
    /// Enough is known to start the codebase researcher.
    Proceed,
    /// The issue is too ambiguous to act on without a clarifying question.
    NeedsMoreInfo,
    /// Not actionable -- spam, a duplicate, or out of scope.
    Skip,
}

/// What kind of issue this is, as the issue analyst classifies it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueClassification {
    Bug,
    Feature,
    Question,
    NotActionable,
}

/// Whether the issue analyst could reproduce a reported bug.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReproductionStatus {
    Reproduced,
    NotReproducible,
    NoStepsProvided,
    /// The issue isn't a bug report, so reproduction doesn't apply.
    NotApplicable,
}

/// [`AgentRole::IssueAnalyst`]'s handoff: a classification, a reproduction
/// attempt, and a recommendation for what should happen next.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct IssueAnalysis {
    pub classification: IssueClassification,
    pub reproduction_status: ReproductionStatus,
    pub summary: String,
    pub reproduction_details: String,
    pub recommended_action: RecommendedAction,
    pub relevant_files: Vec<String>,
}

/// [`AgentRole::CodebaseResearcher`]'s handoff: what the software architect
/// needs to know about the checked-out repository to write a plan.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ResearchComplete {
    pub summary: String,
    pub relevant_files: Vec<String>,
}

/// One subtask as the software architect drafts it in [`CreatePlan`] --
/// distinct from `crate::pipeline::plan::Subtask` (a later commit), which
/// additionally tracks status through the build-and-review cycle. This is
/// only ever the model's own output; the executor is what turns it into a
/// tracked subtask.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SubtaskDraft {
    pub id: SubtaskId,
    pub title: String,
    pub description: String,
    pub instructions: String,
    pub commit_message: String,
    pub files_affected: Vec<String>,
    pub dependencies: Vec<SubtaskId>,
}

/// A completed plan, ready for review.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct CreatePlan {
    pub summary: String,
    pub subtasks: Vec<SubtaskDraft>,
}

/// A question that needs a human's judgement before a plan can be written.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct RequestPlanHelp {
    pub question: String,
}

/// [`AgentRole::SoftwareArchitect`]'s handoff: either a completed plan, or
/// a question the architect cannot resolve alone.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "action")]
pub enum SoftwareArchitectHandoff {
    CreatePlan(CreatePlan),
    RequestPlanHelp(RequestPlanHelp),
}

/// The architecture reviewer's approval, including what the plan does
/// well -- `strengths` is required, not optional: a review that only ever
/// lists problems trains the architect to defend against criticism rather
/// than to build on what already works.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ApprovePlan {
    pub strengths: String,
    pub feedback: String,
}

/// The architecture reviewer's rejection, with what must change.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct RequestPlanChanges {
    pub feedback: String,
}

/// [`AgentRole::ArchitectureReviewer`]'s handoff: approve the plan, or send
/// it back to the architect with feedback.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "action")]
pub enum ArchitectureReviewerHandoff {
    ApprovePlan(ApprovePlan),
    RequestPlanChanges(RequestPlanChanges),
}

/// Which subtask the project manager has decided to start next.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StartTaskTests {
    pub subtask_id: SubtaskId,
}

/// Every subtask is committed; time to review the whole project together.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct BeginFinalReview {}

/// A subtask the project manager synthesizes in response to the final
/// reviewer's own feedback -- carries a full draft, not just an id,
/// since this subtask does not exist anywhere in the plan yet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct FixSubtask {
    /// The final reviewer's own feedback this subtask exists to address.
    pub review_feedback: String,
    /// Which already-committed subtask the reviewer's feedback was about.
    pub parent_subtask_id: SubtaskId,
    pub subtask: SubtaskDraft,
}

/// [`AgentRole::ProjectManager`]'s handoff: start the next subtask, begin
/// the final review once every subtask is committed, or -- only when
/// invoked from `AwaitingFixSubtask` -- synthesize a fix subtask from the
/// final reviewer's own feedback.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "action")]
pub enum ProjectManagerHandoff {
    StartTaskTests(StartTaskTests),
    BeginFinalReview(BeginFinalReview),
    FixSubtask(FixSubtask),
}

/// [`AgentRole::TestEngineer`]'s handoff: tests written and confirmed to
/// fail for the right reason, ready for review.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SubmitTests {
    pub subtask_id: SubtaskId,
    pub summary: String,
    /// What the test engineer assumed about the implementation that
    /// doesn't exist yet -- the field the code reviewer's own prompt must
    /// call `assumptions`, not `implementation_issues` (a defect ported
    /// prompts fixed rather than inherited, see
    /// `docs/plans/ai/milestone-5-autonomous.md`).
    pub assumptions: String,
}

/// The test reviewer's approval.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ApproveTests {
    pub feedback: String,
}

/// The test reviewer's rejection, with what must change.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct RequestTestChanges {
    pub feedback: String,
}

/// [`AgentRole::TestReviewer`]'s handoff: approve the tests as a
/// specification, or send them back with feedback.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "action")]
pub enum TestReviewerHandoff {
    ApproveTests(ApproveTests),
    RequestTestChanges(RequestTestChanges),
}

/// [`AgentRole::Builder`]'s handoff: the subtask implemented against its
/// approved tests, ready for code review.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SubmitCode {
    pub subtask_id: SubtaskId,
    pub summary: String,
}

/// A question that needs a human's judgement before the builder can
/// continue.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct RequestBuildHelp {
    pub question: String,
}

/// [`AgentRole::Builder`]'s handoff: submitted code, or a question the
/// builder cannot resolve alone.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "action")]
pub enum BuilderHandoff {
    SubmitCode(SubmitCode),
    RequestBuildHelp(RequestBuildHelp),
}

/// The code reviewer's approval.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ApproveCode {
    pub feedback: String,
}

/// A rejection with what must change -- shared between
/// [`AgentRole::CodeReviewer`] (one subtask) and
/// [`AgentRole::FinalCodeReviewer`] (the whole project): both reviews are
/// the same judgement call at a different scope, so they share the same
/// rejection shape.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct RequestCodeChanges {
    pub feedback: String,
}

/// [`AgentRole::CodeReviewer`]'s handoff: approve one subtask's code, or
/// send it back with feedback.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "action")]
pub enum CodeReviewerHandoff {
    ApproveCode(ApproveCode),
    RequestCodeChanges(RequestCodeChanges),
}

/// Every subtask holds up together; the project is ready for a pull
/// request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ProjectComplete {}

/// [`AgentRole::FinalCodeReviewer`]'s handoff: the whole project approved
/// together, or changes requested against one subtask.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "action")]
pub enum FinalCodeReviewerHandoff {
    RequestCodeChanges(RequestCodeChanges),
    ProjectComplete(ProjectComplete),
}

/// [`AgentRole::CommitCrafter`]'s handoff: one subtask's approved changes,
/// committed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct CommitComplete {
    pub subtask_id: SubtaskId,
    pub commit_sha: String,
}

/// [`AgentRole::PrAuthor`]'s handoff: a pull request title and body,
/// written from the real diff and commit history.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PullRequestReady {
    pub title: String,
    pub body: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_agent_role_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&AgentRole::FinalCodeReviewer).unwrap();
        assert_eq!(encoded, "\"final_code_reviewer\"");
    }

    #[test]
    fn test_every_agent_role_round_trips_through_json() {
        let roles = [
            AgentRole::IssueAnalyst,
            AgentRole::CodebaseResearcher,
            AgentRole::SoftwareArchitect,
            AgentRole::ArchitectureReviewer,
            AgentRole::ProjectManager,
            AgentRole::TestEngineer,
            AgentRole::TestReviewer,
            AgentRole::Builder,
            AgentRole::CodeReviewer,
            AgentRole::FinalCodeReviewer,
            AgentRole::CommitCrafter,
            AgentRole::PrAuthor,
        ];
        assert_eq!(roles.len(), 12, "there are exactly twelve pipeline roles");

        for role in roles {
            let encoded = serde_json::to_string(&role).unwrap();
            let decoded: AgentRole = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, role);
        }
    }

    #[test]
    fn test_software_architect_handoff_tags_create_plan_by_action() {
        let payload = json!({
            "action": "CreatePlan",
            "summary": "add a dark mode toggle",
            "subtasks": [],
        });
        let handoff: SoftwareArchitectHandoff = serde_json::from_value(payload).unwrap();
        assert!(matches!(handoff, SoftwareArchitectHandoff::CreatePlan(_)));
    }

    #[test]
    fn test_software_architect_handoff_tags_request_plan_help_by_action() {
        let payload = json!({
            "action": "RequestPlanHelp",
            "question": "should this use redis or an in-memory cache?",
        });
        let handoff: SoftwareArchitectHandoff = serde_json::from_value(payload).unwrap();
        assert!(matches!(
            handoff,
            SoftwareArchitectHandoff::RequestPlanHelp(_)
        ));
    }

    #[test]
    fn test_architecture_reviewer_handoff_requires_strengths_on_approval() {
        let payload = json!({
            "action": "ApprovePlan",
            "feedback": "looks solid",
        });
        let error = serde_json::from_value::<ArchitectureReviewerHandoff>(payload)
            .expect_err("strengths should be required, not defaulted");
        assert!(error.to_string().contains("strengths"));
    }

    #[test]
    fn test_architecture_reviewer_handoff_round_trips_a_rejection() {
        let handoff = ArchitectureReviewerHandoff::RequestPlanChanges(RequestPlanChanges {
            feedback: "subtask 3 depends on subtask 5, which comes after it".to_string(),
        });
        let encoded = serde_json::to_value(&handoff).unwrap();
        let decoded: ArchitectureReviewerHandoff = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, handoff);
    }

    #[test]
    fn test_project_manager_handoff_tags_start_task_tests_by_action() {
        let payload = json!({
            "action": "StartTaskTests",
            "subtask_id": "task-1",
        });
        let handoff: ProjectManagerHandoff = serde_json::from_value(payload).unwrap();
        match handoff {
            ProjectManagerHandoff::StartTaskTests(start) => {
                assert_eq!(start.subtask_id, SubtaskId("task-1".to_string()));
            }
            other => panic!("expected StartTaskTests, got {other:?}"),
        }
    }

    #[test]
    fn test_project_manager_handoff_tags_begin_final_review_by_action() {
        let payload = json!({ "action": "BeginFinalReview" });
        let handoff: ProjectManagerHandoff = serde_json::from_value(payload).unwrap();
        assert!(matches!(
            handoff,
            ProjectManagerHandoff::BeginFinalReview(_)
        ));
    }

    #[test]
    fn test_project_manager_handoff_tags_fix_subtask_by_action() {
        let payload = json!({
            "action": "FixSubtask",
            "review_feedback": "subtask 2 broke subtask 4's tests",
            "parent_subtask_id": "task-2",
            "subtask": {
                "id": "task-5",
                "title": "fix the regression in subtask 4",
                "description": "d",
                "instructions": "i",
                "commit_message": "fix: repair subtask 4's regression",
                "files_affected": [],
                "dependencies": [],
            },
        });
        let handoff: ProjectManagerHandoff = serde_json::from_value(payload).unwrap();
        match handoff {
            ProjectManagerHandoff::FixSubtask(fix) => {
                assert_eq!(fix.parent_subtask_id, SubtaskId("task-2".to_string()));
                assert_eq!(fix.subtask.id, SubtaskId("task-5".to_string()));
            }
            other => panic!("expected FixSubtask, got {other:?}"),
        }
    }

    #[test]
    fn test_submit_tests_has_an_assumptions_field_not_implementation_issues() {
        let payload = json!({
            "subtask_id": "task-1",
            "summary": "tests for the dark mode toggle",
            "assumptions": "assumes a `theme` field exists on user settings",
        });
        let submitted: SubmitTests = serde_json::from_value(payload).unwrap();
        assert_eq!(
            submitted.assumptions,
            "assumes a `theme` field exists on user settings"
        );
    }

    #[test]
    fn test_test_reviewer_handoff_tags_by_action() {
        let approve = json!({ "action": "ApproveTests", "feedback": "good coverage" });
        let handoff: TestReviewerHandoff = serde_json::from_value(approve).unwrap();
        assert!(matches!(handoff, TestReviewerHandoff::ApproveTests(_)));

        let reject = json!({ "action": "RequestTestChanges", "feedback": "missing an edge case" });
        let handoff: TestReviewerHandoff = serde_json::from_value(reject).unwrap();
        assert!(matches!(
            handoff,
            TestReviewerHandoff::RequestTestChanges(_)
        ));
    }

    #[test]
    fn test_builder_handoff_tags_by_action() {
        let submit = json!({
            "action": "SubmitCode",
            "subtask_id": "task-1",
            "summary": "implemented the toggle",
        });
        let handoff: BuilderHandoff = serde_json::from_value(submit).unwrap();
        assert!(matches!(handoff, BuilderHandoff::SubmitCode(_)));

        let help = json!({
            "action": "RequestBuildHelp",
            "question": "which css variable holds the accent color?",
        });
        let handoff: BuilderHandoff = serde_json::from_value(help).unwrap();
        assert!(matches!(handoff, BuilderHandoff::RequestBuildHelp(_)));
    }

    #[test]
    fn test_code_reviewer_handoff_tags_by_action() {
        let approve = json!({ "action": "ApproveCode", "feedback": "clean" });
        let handoff: CodeReviewerHandoff = serde_json::from_value(approve).unwrap();
        assert!(matches!(handoff, CodeReviewerHandoff::ApproveCode(_)));

        let reject =
            json!({ "action": "RequestCodeChanges", "feedback": "missing error handling" });
        let handoff: CodeReviewerHandoff = serde_json::from_value(reject).unwrap();
        assert!(matches!(
            handoff,
            CodeReviewerHandoff::RequestCodeChanges(_)
        ));
    }

    #[test]
    fn test_final_code_reviewer_handoff_tags_by_action() {
        let complete = json!({ "action": "ProjectComplete" });
        let handoff: FinalCodeReviewerHandoff = serde_json::from_value(complete).unwrap();
        assert!(matches!(
            handoff,
            FinalCodeReviewerHandoff::ProjectComplete(_)
        ));

        let reject = json!({ "action": "RequestCodeChanges", "feedback": "subtask 2 broke subtask 4's tests" });
        let handoff: FinalCodeReviewerHandoff = serde_json::from_value(reject).unwrap();
        assert!(matches!(
            handoff,
            FinalCodeReviewerHandoff::RequestCodeChanges(_)
        ));
    }

    #[test]
    fn test_commit_complete_round_trips() {
        let commit = CommitComplete {
            subtask_id: SubtaskId("task-1".to_string()),
            commit_sha: "abc123".to_string(),
        };
        let encoded = serde_json::to_value(&commit).unwrap();
        let decoded: CommitComplete = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, commit);
    }

    #[test]
    fn test_pull_request_ready_round_trips() {
        let pr = PullRequestReady {
            title: "add a dark mode toggle".to_string(),
            body: "closes #42".to_string(),
        };
        let encoded = serde_json::to_value(&pr).unwrap();
        let decoded: PullRequestReady = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, pr);
    }

    #[test]
    fn test_issue_analysis_round_trips_through_json() {
        let analysis = IssueAnalysis {
            classification: IssueClassification::Bug,
            reproduction_status: ReproductionStatus::Reproduced,
            summary: "crashes on startup with a null config".to_string(),
            reproduction_details: "ran `munibot` with no config.toml present".to_string(),
            recommended_action: RecommendedAction::Proceed,
            relevant_files: vec!["munibot_core/src/config.rs".to_string()],
        };
        let encoded = serde_json::to_value(&analysis).unwrap();
        let decoded: IssueAnalysis = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, analysis);
    }

    #[test]
    fn test_every_plain_struct_handoff_produces_an_object_json_schema() {
        // a cheap guard that every plain-struct payload here actually
        // derives JsonSchema correctly, ahead of a later commit wiring
        // schema_for! into a real HandoffSchema per role
        macro_rules! assert_object_schema {
            ($ty:ty) => {
                let schema = schemars::schema_for!($ty).to_value();
                assert_eq!(
                    schema["type"], "object",
                    concat!(stringify!($ty), " should be an object schema")
                );
            };
        }

        assert_object_schema!(IssueAnalysis);
        assert_object_schema!(ResearchComplete);
        assert_object_schema!(SubmitTests);
        assert_object_schema!(CommitComplete);
        assert_object_schema!(PullRequestReady);
    }

    #[test]
    fn test_every_action_tagged_handoff_produces_a_one_of_json_schema() {
        // the multi-action handoffs are internally-tagged enums, which
        // schemars represents as `oneOf` rather than a bare object schema
        macro_rules! assert_one_of_schema {
            ($ty:ty) => {
                let schema = schemars::schema_for!($ty).to_value();
                assert!(
                    schema["oneOf"].is_array(),
                    "{} should schema as oneOf its possible actions",
                    stringify!($ty)
                );
            };
        }

        assert_one_of_schema!(SoftwareArchitectHandoff);
        assert_one_of_schema!(ArchitectureReviewerHandoff);
        assert_one_of_schema!(ProjectManagerHandoff);
        assert_one_of_schema!(TestReviewerHandoff);
        assert_one_of_schema!(BuilderHandoff);
        assert_one_of_schema!(CodeReviewerHandoff);
        assert_one_of_schema!(FinalCodeReviewerHandoff);
    }
}
