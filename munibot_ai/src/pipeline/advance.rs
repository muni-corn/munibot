//! `advance`: the pipeline's own transition function, and the
//! specification for the whole state machine.
//!
//! Every legal (state, event) pair the pipeline can ever move through is
//! exactly one match arm below. Anything not listed is illegal by
//! construction -- an executor that tries to append an event its own
//! current state doesn't allow gets an error back before that event is
//! ever persisted, rather than a nonsensical state silently entering the
//! event log.

use thiserror::Error;

use crate::pipeline::{PipelineEvent, PipelineState, RecommendedAction};

/// Why a transition was rejected.
#[derive(Error, Debug, Clone, PartialEq)]
#[error("{event} is not a legal transition from {state}")]
pub struct AdvanceError {
    /// A human-legible description of the state the pipeline was in.
    pub state: String,
    /// The event's own label (see [`PipelineEvent::label`]).
    pub event: String,
}

impl AdvanceError {
    fn new(state: &PipelineState, event: &PipelineEvent) -> Self {
        Self {
            state: format!("{state:?}"),
            event: event.label().to_string(),
        }
    }
}

/// Computes the state that results from `event` arriving while the
/// pipeline is in `state`, or rejects the transition if `event` could
/// never legally arrive there.
///
/// A pure function: no I/O, no agent invocation, nothing beyond
/// pattern-matching its own two arguments. This is deliberate -- every
/// executor decision ("is it safe to append this event") reduces to
/// calling this function once, so the whole pipeline's legal shape lives
/// in one place a reviewer can read start to finish, rather than being
/// implicit in whichever role happens to be dispatched for a given state.
pub fn advance(state: PipelineState, event: &PipelineEvent) -> Result<PipelineState, AdvanceError> {
    use PipelineEvent as E;
    use PipelineState as S;

    // a maintainer's answer resumes whatever state was waiting for it,
    // regardless of what that state was -- checked first so every other
    // arm below never has to consider AwaitingUserInput as a source state
    if let E::UserInputReceived { .. } = event {
        return match state {
            S::AwaitingUserInput { resume, .. } => Ok(*resume),
            other => Err(AdvanceError::new(&other, event)),
        };
    }

    // a failure can end the run from anywhere it hasn't already ended
    if let E::Failed { reason } = event {
        return if state.is_terminal() {
            Err(AdvanceError::new(&state, event))
        } else {
            Ok(S::Failed {
                reason: reason.clone(),
            })
        };
    }

    match (state, event) {
        (S::Triaging, E::Triggered { .. }) => Ok(S::Triaging),
        (S::Triaging, E::IssueAnalyzed(analysis)) => Ok(match analysis.recommended_action {
            RecommendedAction::Proceed => S::Researching,
            RecommendedAction::NeedsMoreInfo => S::AwaitingUserInput {
                request: crate::pipeline::InteractionRequest {
                    prompt: analysis.summary.clone(),
                },
                resume: Box::new(S::Triaging),
            },
            RecommendedAction::Skip => S::Complete,
        }),

        (S::Researching, E::ResearchCompleted(_)) => Ok(S::Planning),

        (S::Planning, E::PlanCreated(_)) => Ok(S::ReviewingPlan),
        (S::Planning, E::PlanHelpRequested(help)) => Ok(S::AwaitingUserInput {
            request: crate::pipeline::InteractionRequest {
                prompt: help.question.clone(),
            },
            resume: Box::new(S::Planning),
        }),

        (S::ReviewingPlan, E::PlanApproved(_)) => Ok(S::Scheduling),
        (S::ReviewingPlan, E::PlanChangesRequested(_)) => Ok(S::Planning),

        (S::Scheduling, E::SubtaskTestsStarted(start)) => Ok(S::TestWriting {
            subtask: start.subtask_id.clone(),
        }),
        (S::Scheduling, E::FinalReviewStarted(_)) => Ok(S::FinalReview),

        (S::TestWriting { subtask }, E::TestsSubmitted(submitted))
            if submitted.subtask_id == subtask =>
        {
            Ok(S::TestReviewing { subtask })
        }

        (S::TestReviewing { subtask }, E::TestsApproved(_)) => Ok(S::Building { subtask }),
        (S::TestReviewing { subtask }, E::TestChangesRequested(_)) => {
            Ok(S::TestWriting { subtask })
        }

        (S::Building { subtask }, E::CodeSubmitted(submitted))
            if submitted.subtask_id == subtask =>
        {
            Ok(S::ReviewingCode { subtask })
        }
        (S::Building { subtask }, E::BuildHelpRequested(help)) => Ok(S::AwaitingUserInput {
            request: crate::pipeline::InteractionRequest {
                prompt: help.question.clone(),
            },
            resume: Box::new(S::Building { subtask }),
        }),

        (S::ReviewingCode { subtask }, E::CodeApproved(_)) => Ok(S::Committing { subtask }),
        (S::ReviewingCode { subtask }, E::CodeChangesRequested(_)) => Ok(S::Building { subtask }),

        (S::Committing { .. }, E::SubtaskCommitted(_)) => Ok(S::Scheduling),

        (S::FinalReview, E::CodeChangesRequested(_)) => Ok(S::AwaitingFixSubtask),
        (S::FinalReview, E::ProjectCompleted(_)) => Ok(S::WritingPr),

        // the project manager synthesizes a fix subtask and re-enters the
        // ordinary test-and-build cycle for it -- see the fix subtask
        // synthesis commit
        (S::AwaitingFixSubtask, E::SubtaskTestsStarted(start)) => Ok(S::TestWriting {
            subtask: start.subtask_id.clone(),
        }),

        (S::WritingPr, E::PullRequestAuthored(_)) => Ok(S::Complete),

        (other, event) => Err(AdvanceError::new(&other, event)),
    }
}

#[cfg(test)]
mod tests {
    use munibot_vcs::{Forge, IssueRef, RepoRef};

    use super::*;
    use crate::pipeline::{
        ApproveCode, ApprovePlan, ApproveTests, BeginFinalReview, CommitComplete, CreatePlan,
        InteractionRequest, IssueAnalysis, IssueClassification, ProjectComplete, PullRequestReady,
        ReproductionStatus, RequestBuildHelp, RequestCodeChanges, RequestPlanChanges,
        RequestPlanHelp, RequestTestChanges, ResearchComplete, StartTaskTests, SubmitCode,
        SubmitTests, SubtaskId,
    };

    fn subtask() -> SubtaskId {
        SubtaskId("task-1".to_string())
    }

    fn analysis(action: RecommendedAction) -> IssueAnalysis {
        IssueAnalysis {
            classification: IssueClassification::Bug,
            reproduction_status: ReproductionStatus::Reproduced,
            summary: "crashes on startup".to_string(),
            reproduction_details: "ran with no config".to_string(),
            recommended_action: action,
            relevant_files: vec![],
        }
    }

    // -- every legal transition, one test each --

    #[test]
    fn test_triggered_from_triaging_stays_triaging() {
        let issue = IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1);
        assert_eq!(
            advance(PipelineState::Triaging, &PipelineEvent::Triggered { issue }).unwrap(),
            PipelineState::Triaging
        );
    }

    #[test]
    fn test_issue_analyzed_proceed_moves_to_researching() {
        let event = PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::Proceed));
        assert_eq!(
            advance(PipelineState::Triaging, &event).unwrap(),
            PipelineState::Researching
        );
    }

    #[test]
    fn test_issue_analyzed_needs_more_info_awaits_user_input() {
        let event = PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::NeedsMoreInfo));
        let result = advance(PipelineState::Triaging, &event).unwrap();
        assert!(matches!(result, PipelineState::AwaitingUserInput { .. }));
    }

    #[test]
    fn test_issue_analyzed_skip_completes() {
        let event = PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::Skip));
        assert_eq!(
            advance(PipelineState::Triaging, &event).unwrap(),
            PipelineState::Complete
        );
    }

    #[test]
    fn test_user_input_received_resumes_whatever_state_was_waiting() {
        let state = PipelineState::AwaitingUserInput {
            request: InteractionRequest {
                prompt: "?".to_string(),
            },
            resume: Box::new(PipelineState::Planning),
        };
        let event = PipelineEvent::UserInputReceived {
            response: "yes".to_string(),
        };
        assert_eq!(advance(state, &event).unwrap(), PipelineState::Planning);
    }

    #[test]
    fn test_research_completed_moves_to_planning() {
        let event = PipelineEvent::ResearchCompleted(ResearchComplete {
            summary: "uses axum".to_string(),
            relevant_files: vec![],
        });
        assert_eq!(
            advance(PipelineState::Researching, &event).unwrap(),
            PipelineState::Planning
        );
    }

    #[test]
    fn test_plan_created_moves_to_reviewing_plan() {
        let event = PipelineEvent::PlanCreated(CreatePlan {
            summary: "add dark mode".to_string(),
            subtasks: vec![],
        });
        assert_eq!(
            advance(PipelineState::Planning, &event).unwrap(),
            PipelineState::ReviewingPlan
        );
    }

    #[test]
    fn test_plan_help_requested_awaits_user_input() {
        let event = PipelineEvent::PlanHelpRequested(RequestPlanHelp {
            question: "redis or in-memory?".to_string(),
        });
        assert!(matches!(
            advance(PipelineState::Planning, &event).unwrap(),
            PipelineState::AwaitingUserInput { .. }
        ));
    }

    #[test]
    fn test_plan_approved_moves_to_scheduling() {
        let event = PipelineEvent::PlanApproved(ApprovePlan {
            strengths: "clear ordering".to_string(),
            feedback: "ship it".to_string(),
        });
        assert_eq!(
            advance(PipelineState::ReviewingPlan, &event).unwrap(),
            PipelineState::Scheduling
        );
    }

    #[test]
    fn test_plan_changes_requested_returns_to_planning() {
        let event = PipelineEvent::PlanChangesRequested(RequestPlanChanges {
            feedback: "subtask 3 depends on subtask 5".to_string(),
        });
        assert_eq!(
            advance(PipelineState::ReviewingPlan, &event).unwrap(),
            PipelineState::Planning
        );
    }

    #[test]
    fn test_subtask_tests_started_from_scheduling_moves_to_test_writing() {
        let event = PipelineEvent::SubtaskTestsStarted(StartTaskTests {
            subtask_id: subtask(),
        });
        assert_eq!(
            advance(PipelineState::Scheduling, &event).unwrap(),
            PipelineState::TestWriting { subtask: subtask() }
        );
    }

    #[test]
    fn test_final_review_started_from_scheduling_moves_to_final_review() {
        let event = PipelineEvent::FinalReviewStarted(BeginFinalReview {});
        assert_eq!(
            advance(PipelineState::Scheduling, &event).unwrap(),
            PipelineState::FinalReview
        );
    }

    #[test]
    fn test_tests_submitted_for_the_right_subtask_moves_to_test_reviewing() {
        let event = PipelineEvent::TestsSubmitted(SubmitTests {
            subtask_id: subtask(),
            summary: "tests for the toggle".to_string(),
            assumptions: "assumes a theme field".to_string(),
        });
        let state = PipelineState::TestWriting { subtask: subtask() };
        assert_eq!(
            advance(state, &event).unwrap(),
            PipelineState::TestReviewing { subtask: subtask() }
        );
    }

    #[test]
    fn test_tests_submitted_for_the_wrong_subtask_is_rejected() {
        let event = PipelineEvent::TestsSubmitted(SubmitTests {
            subtask_id: SubtaskId("task-2".to_string()),
            summary: "tests for a different subtask".to_string(),
            assumptions: "assumes nothing".to_string(),
        });
        let state = PipelineState::TestWriting { subtask: subtask() };
        assert!(advance(state, &event).is_err());
    }

    #[test]
    fn test_tests_approved_moves_to_building() {
        let event = PipelineEvent::TestsApproved(ApproveTests {
            feedback: "good coverage".to_string(),
        });
        let state = PipelineState::TestReviewing { subtask: subtask() };
        assert_eq!(advance(state, &event).unwrap(), PipelineState::Building {
            subtask: subtask()
        });
    }

    #[test]
    fn test_test_changes_requested_returns_to_test_writing() {
        let event = PipelineEvent::TestChangesRequested(RequestTestChanges {
            feedback: "missing an edge case".to_string(),
        });
        let state = PipelineState::TestReviewing { subtask: subtask() };
        assert_eq!(
            advance(state, &event).unwrap(),
            PipelineState::TestWriting { subtask: subtask() }
        );
    }

    #[test]
    fn test_code_submitted_for_the_right_subtask_moves_to_reviewing_code() {
        let event = PipelineEvent::CodeSubmitted(SubmitCode {
            subtask_id: subtask(),
            summary: "implemented the toggle".to_string(),
        });
        let state = PipelineState::Building { subtask: subtask() };
        assert_eq!(
            advance(state, &event).unwrap(),
            PipelineState::ReviewingCode { subtask: subtask() }
        );
    }

    #[test]
    fn test_build_help_requested_awaits_user_input() {
        let event = PipelineEvent::BuildHelpRequested(RequestBuildHelp {
            question: "which css variable holds the accent color?".to_string(),
        });
        let state = PipelineState::Building { subtask: subtask() };
        assert!(matches!(
            advance(state, &event).unwrap(),
            PipelineState::AwaitingUserInput { .. }
        ));
    }

    #[test]
    fn test_code_approved_moves_to_committing() {
        let event = PipelineEvent::CodeApproved(ApproveCode {
            feedback: "clean".to_string(),
        });
        let state = PipelineState::ReviewingCode { subtask: subtask() };
        assert_eq!(advance(state, &event).unwrap(), PipelineState::Committing {
            subtask: subtask()
        });
    }

    #[test]
    fn test_code_changes_requested_from_reviewing_code_returns_to_building() {
        let event = PipelineEvent::CodeChangesRequested(RequestCodeChanges {
            feedback: "missing error handling".to_string(),
        });
        let state = PipelineState::ReviewingCode { subtask: subtask() };
        assert_eq!(advance(state, &event).unwrap(), PipelineState::Building {
            subtask: subtask()
        });
    }

    #[test]
    fn test_subtask_committed_moves_to_scheduling() {
        let event = PipelineEvent::SubtaskCommitted(CommitComplete {
            subtask_id: subtask(),
            commit_sha: "abc123".to_string(),
        });
        let state = PipelineState::Committing { subtask: subtask() };
        assert_eq!(advance(state, &event).unwrap(), PipelineState::Scheduling);
    }

    #[test]
    fn test_code_changes_requested_from_final_review_moves_to_awaiting_fix_subtask() {
        let event = PipelineEvent::CodeChangesRequested(RequestCodeChanges {
            feedback: "subtask 2 broke subtask 4's tests".to_string(),
        });
        assert_eq!(
            advance(PipelineState::FinalReview, &event).unwrap(),
            PipelineState::AwaitingFixSubtask
        );
    }

    #[test]
    fn test_project_completed_moves_to_writing_pr() {
        let event = PipelineEvent::ProjectCompleted(ProjectComplete {});
        assert_eq!(
            advance(PipelineState::FinalReview, &event).unwrap(),
            PipelineState::WritingPr
        );
    }

    #[test]
    fn test_subtask_tests_started_from_awaiting_fix_subtask_re_enters_test_writing() {
        let event = PipelineEvent::SubtaskTestsStarted(StartTaskTests {
            subtask_id: subtask(),
        });
        assert_eq!(
            advance(PipelineState::AwaitingFixSubtask, &event).unwrap(),
            PipelineState::TestWriting { subtask: subtask() }
        );
    }

    #[test]
    fn test_pull_request_authored_completes() {
        let event = PipelineEvent::PullRequestAuthored(PullRequestReady {
            title: "add dark mode".to_string(),
            body: "closes #42".to_string(),
        });
        assert_eq!(
            advance(PipelineState::WritingPr, &event).unwrap(),
            PipelineState::Complete
        );
    }

    #[test]
    fn test_failed_ends_the_run_from_a_non_terminal_state() {
        let event = PipelineEvent::Failed {
            reason: "budget exhausted".to_string(),
        };
        assert_eq!(
            advance(PipelineState::Researching, &event).unwrap(),
            PipelineState::Failed {
                reason: "budget exhausted".to_string()
            }
        );
    }

    // -- illegal transitions --

    #[test]
    fn test_failed_is_rejected_from_an_already_terminal_state() {
        let event = PipelineEvent::Failed {
            reason: "again?".to_string(),
        };
        assert!(advance(PipelineState::Complete, &event).is_err());
    }

    #[test]
    fn test_user_input_received_is_rejected_when_nothing_was_waiting() {
        let event = PipelineEvent::UserInputReceived {
            response: "unsolicited".to_string(),
        };
        assert!(advance(PipelineState::Triaging, &event).is_err());
    }

    #[test]
    fn test_approve_tests_is_rejected_while_triaging() {
        let event = PipelineEvent::TestsApproved(ApproveTests {
            feedback: "good".to_string(),
        });
        assert!(advance(PipelineState::Triaging, &event).is_err());
    }

    #[test]
    fn test_plan_created_is_rejected_while_reviewing_plan() {
        // the architect can't submit a second plan while one is already
        // under review
        let event = PipelineEvent::PlanCreated(CreatePlan {
            summary: "a second plan".to_string(),
            subtasks: vec![],
        });
        assert!(advance(PipelineState::ReviewingPlan, &event).is_err());
    }

    #[test]
    fn test_commit_complete_is_rejected_before_code_is_approved() {
        let event = PipelineEvent::SubtaskCommitted(CommitComplete {
            subtask_id: subtask(),
            commit_sha: "abc123".to_string(),
        });
        let state = PipelineState::ReviewingCode { subtask: subtask() };
        assert!(advance(state, &event).is_err());
    }

    #[test]
    fn test_project_completed_is_rejected_before_final_review() {
        let event = PipelineEvent::ProjectCompleted(ProjectComplete {});
        assert!(advance(PipelineState::Scheduling, &event).is_err());
    }

    #[test]
    fn test_triggered_is_rejected_once_the_pipeline_has_moved_on() {
        let issue = IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1);
        let event = PipelineEvent::Triggered { issue };
        assert!(advance(PipelineState::Researching, &event).is_err());
    }

    #[test]
    fn test_pull_request_authored_is_rejected_before_writing_pr() {
        let event = PipelineEvent::PullRequestAuthored(PullRequestReady {
            title: "t".to_string(),
            body: "b".to_string(),
        });
        assert!(advance(PipelineState::FinalReview, &event).is_err());
    }

    #[test]
    fn test_error_message_names_both_the_state_and_the_event() {
        let event = PipelineEvent::ProjectCompleted(ProjectComplete {});
        let error = advance(PipelineState::Scheduling, &event).expect_err("should be illegal");
        assert!(error.event.contains("ProjectCompleted"));
        assert!(error.state.contains("Scheduling"));
    }
}
