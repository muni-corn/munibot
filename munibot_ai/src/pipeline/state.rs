//! The pipeline's own state machine's states -- see [`super::advance`] (a
//! later commit) for the pure transition function that moves between them.

use serde::{Deserialize, Serialize};

/// A stable identifier for one pipeline run, assigned by the pipeline
/// store when the run is created.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PipelineId(pub i64);

/// A stable identifier for one subtask within a pipeline's own plan,
/// assigned by the software architect when the plan is created -- see
/// `crate::pipeline::plan::Subtask`, which this identifies.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct SubtaskId(pub String);

/// One question the pipeline needs a human to answer before it can
/// continue -- what [`PipelineState::AwaitingUserInput`] carries, and the
/// payload every `InteractionAdapter` (a later commit) is given to ask.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InteractionRequest {
    /// The question itself, already written in whatever role is asking
    /// it's own voice -- an interaction adapter shows this verbatim,
    /// rather than composing its own wording around a structured reason
    /// code.
    pub prompt: String,
}

/// Every state one pipeline run can be in, from the moment an issue event
/// triggers it to the moment it opens a pull request (or gives up).
///
/// Mirrors the pipeline diagram in
/// `docs/plans/ai/milestone-5-autonomous.md` exactly: every box in that
/// diagram is one variant here, and every arrow out of a box is a
/// transition [`super::advance`] (a later commit) validates.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PipelineState {
    Triaging,
    Researching,
    Planning,
    ReviewingPlan,
    TestWriting { subtask: SubtaskId },
    TestReviewing { subtask: SubtaskId },
    Building { subtask: SubtaskId },
    ReviewingCode { subtask: SubtaskId },
    Committing { subtask: SubtaskId },
    FinalReview,
    AwaitingFixSubtask,
    WritingPr,
    AwaitingUserInput { request: InteractionRequest },
    Complete,
    Failed { reason: String },
}

impl PipelineState {
    /// Whether this state ends the run -- no further agent invocation or
    /// event should ever move a pipeline out of a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, PipelineState::Complete | PipelineState::Failed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_id_serializes_as_a_bare_number() {
        let encoded = serde_json::to_value(PipelineId(42)).unwrap();
        assert_eq!(encoded, serde_json::json!(42));
    }

    #[test]
    fn test_subtask_id_serializes_as_a_bare_string() {
        let encoded = serde_json::to_value(SubtaskId("task-1".to_string())).unwrap();
        assert_eq!(encoded, serde_json::json!("task-1"));
    }

    #[test]
    fn test_complete_is_terminal() {
        assert!(PipelineState::Complete.is_terminal());
    }

    #[test]
    fn test_failed_is_terminal() {
        assert!(
            PipelineState::Failed {
                reason: "budget exhausted".to_string()
            }
            .is_terminal()
        );
    }

    #[test]
    fn test_triaging_is_not_terminal() {
        assert!(!PipelineState::Triaging.is_terminal());
    }

    #[test]
    fn test_awaiting_user_input_is_not_terminal() {
        let state = PipelineState::AwaitingUserInput {
            request: InteractionRequest {
                prompt: "which database should this use?".to_string(),
            },
        };
        assert!(!state.is_terminal());
    }

    #[test]
    fn test_a_subtask_scoped_state_round_trips_through_json() {
        let state = PipelineState::Building {
            subtask: SubtaskId("task-1".to_string()),
        };
        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: PipelineState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, state);
    }

    #[test]
    fn test_every_state_is_distinguishable_from_every_other() {
        // a cheap guard against two variants accidentally becoming
        // structurally identical (and so indistinguishable after a
        // round trip) as this enum grows
        let subtask = SubtaskId("task-1".to_string());
        let states = vec![
            PipelineState::Triaging,
            PipelineState::Researching,
            PipelineState::Planning,
            PipelineState::ReviewingPlan,
            PipelineState::TestWriting {
                subtask: subtask.clone(),
            },
            PipelineState::TestReviewing {
                subtask: subtask.clone(),
            },
            PipelineState::Building {
                subtask: subtask.clone(),
            },
            PipelineState::ReviewingCode {
                subtask: subtask.clone(),
            },
            PipelineState::Committing {
                subtask: subtask.clone(),
            },
            PipelineState::FinalReview,
            PipelineState::AwaitingFixSubtask,
            PipelineState::WritingPr,
            PipelineState::AwaitingUserInput {
                request: InteractionRequest {
                    prompt: "?".to_string(),
                },
            },
            PipelineState::Complete,
            PipelineState::Failed {
                reason: "?".to_string(),
            },
        ];

        for (i, a) in states.iter().enumerate() {
            for (j, b) in states.iter().enumerate() {
                assert_eq!(
                    i == j,
                    a == b,
                    "states at {i} and {j} should only be equal to themselves"
                );
            }
        }
    }
}
