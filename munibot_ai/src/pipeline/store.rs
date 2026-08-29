//! `PipelineStore`: persisting a pipeline's own append-only event log, and
//! folding it back into a [`PipelineState`] on replay.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

use async_trait::async_trait;
use munibot_core::db::{DbPool, operations::ai as core_ai};
use munibot_vcs::IssueRef;
use thiserror::Error;

use crate::pipeline::{
    InteractionRequest, PipelineEvent, PipelineId, PipelineState, RecommendedAction,
};

/// Why a [`PipelineStore`] operation failed.
#[derive(Error, Debug)]
pub enum PipelineStoreError {
    #[error("pipeline {0:?} was not found")]
    NotFound(PipelineId),
    #[error("couldn't (de)serialize a pipeline event: {0}")]
    Serialization(String),
    #[error("pipeline store operation failed: {0}")]
    Other(String),
}

/// Folds one event onto the state it's assumed to have already arrived
/// on, producing the state that comes next.
///
/// Trusts the event log rather than validating it: every event this
/// function ever sees was itself produced by an executor that already
/// checked a transition was legal before appending it (a later commit adds
/// that check as its own, separate concern -- see
/// `docs/plans/ai/milestone-5-autonomous.md`'s pipeline advance commit).
/// This function's own job is only to compute the resulting state, not to
/// police how it got there.
fn fold_one(state: PipelineState, event: &PipelineEvent) -> PipelineState {
    use PipelineEvent as E;
    use PipelineState as S;

    match event {
        E::Triggered { .. } => S::Triaging,
        E::IssueAnalyzed(analysis) => match analysis.recommended_action {
            RecommendedAction::Proceed => S::Researching,
            RecommendedAction::NeedsMoreInfo => S::AwaitingUserInput {
                request: InteractionRequest {
                    prompt: analysis.summary.clone(),
                },
                resume: Box::new(S::Triaging),
            },
            RecommendedAction::Skip => S::Complete,
        },
        E::ResearchCompleted(_) => S::Planning,
        E::PlanCreated(_) => S::ReviewingPlan,
        E::PlanHelpRequested(help) => S::AwaitingUserInput {
            request: InteractionRequest {
                prompt: help.question.clone(),
            },
            resume: Box::new(S::Planning),
        },
        E::PlanApproved(_) => S::Scheduling,
        E::PlanChangesRequested(_) => S::Planning,
        E::SubtaskTestsStarted(start) => S::TestWriting {
            subtask: start.subtask_id.clone(),
        },
        E::FinalReviewStarted(_) => S::FinalReview,
        E::TestsSubmitted(submitted) => S::TestReviewing {
            subtask: submitted.subtask_id.clone(),
        },
        E::TestsApproved(_) => match state {
            S::TestReviewing { subtask } => S::Building { subtask },
            other => other,
        },
        E::TestChangesRequested(_) => match state {
            S::TestReviewing { subtask } => S::TestWriting { subtask },
            other => other,
        },
        E::CodeSubmitted(submitted) => S::ReviewingCode {
            subtask: submitted.subtask_id.clone(),
        },
        E::BuildHelpRequested(help) => S::AwaitingUserInput {
            request: InteractionRequest {
                prompt: help.question.clone(),
            },
            resume: Box::new(state),
        },
        E::CodeApproved(_) => match state {
            S::ReviewingCode { subtask } => S::Committing { subtask },
            other => other,
        },
        E::CodeChangesRequested(_) => match state {
            S::ReviewingCode { subtask } => S::Building { subtask },
            S::FinalReview => S::AwaitingFixSubtask,
            other => other,
        },
        E::ProjectCompleted(_) => S::WritingPr,
        E::SubtaskCommitted(_) => S::Scheduling,
        E::PullRequestAuthored(_) => S::Complete,
        E::UserInputReceived { .. } => match state {
            S::AwaitingUserInput { resume, .. } => *resume,
            other => other,
        },
        E::Failed { reason } => S::Failed {
            reason: reason.clone(),
        },
    }
}

/// Folds a whole event log into the state it resolves to, starting from
/// `PipelineState::Triaging` -- an empty log resolves there too, since
/// that's the state a run is in before its own `Triggered` event even
/// arrives.
fn fold(events: &[PipelineEvent]) -> PipelineState {
    events.iter().fold(PipelineState::Triaging, fold_one)
}

/// Persists one pipeline's own append-only event log.
///
/// `replay` has one, shared meaning regardless of implementation: fold
/// every event `events` returns, in order, into the [`PipelineState`] that
/// results. Recovery from a crash is exactly this replay, never a
/// mutated column read back -- see `ai_pipelines`' own migration.
#[async_trait]
pub trait PipelineStore: Send + Sync {
    /// Creates a new pipeline run for `issue`, returning its id.
    async fn create_pipeline(&self, issue: &IssueRef) -> Result<PipelineId, PipelineStoreError>;

    /// Appends one event to `pipeline_id`'s own log.
    async fn append_event(
        &self,
        pipeline_id: PipelineId,
        event: PipelineEvent,
    ) -> Result<(), PipelineStoreError>;

    /// Every event in `pipeline_id`'s own log, in the order they were
    /// appended.
    async fn events(
        &self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PipelineEvent>, PipelineStoreError>;

    /// The state `pipeline_id`'s own event log currently resolves to.
    async fn replay(&self, pipeline_id: PipelineId) -> Result<PipelineState, PipelineStoreError> {
        Ok(fold(&self.events(pipeline_id).await?))
    }
}

/// An in-memory [`PipelineStore`], for tests -- see
/// `DieselPipelineStore` for the production implementation backed by
/// `ai_pipelines`/`ai_pipeline_events`.
#[derive(Default)]
pub struct InMemoryPipelineStore {
    next_id: AtomicI64,
    pipelines: Mutex<HashMap<PipelineId, Vec<PipelineEvent>>>,
}

impl InMemoryPipelineStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PipelineStore for InMemoryPipelineStore {
    async fn create_pipeline(&self, _issue: &IssueRef) -> Result<PipelineId, PipelineStoreError> {
        let id = PipelineId(self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
        self.pipelines
            .lock()
            .expect("pipeline store lock poisoned")
            .insert(id, Vec::new());
        Ok(id)
    }

    async fn append_event(
        &self,
        pipeline_id: PipelineId,
        event: PipelineEvent,
    ) -> Result<(), PipelineStoreError> {
        let mut pipelines = self.pipelines.lock().expect("pipeline store lock poisoned");
        let events = pipelines
            .get_mut(&pipeline_id)
            .ok_or(PipelineStoreError::NotFound(pipeline_id))?;
        events.push(event);
        Ok(())
    }

    async fn events(
        &self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PipelineEvent>, PipelineStoreError> {
        self.pipelines
            .lock()
            .expect("pipeline store lock poisoned")
            .get(&pipeline_id)
            .cloned()
            .ok_or(PipelineStoreError::NotFound(pipeline_id))
    }
}

/// The production [`PipelineStore`], backed by `ai_pipelines` and
/// `ai_pipeline_events`.
pub struct DieselPipelineStore {
    pool: DbPool,
}

impl DieselPipelineStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PipelineStore for DieselPipelineStore {
    async fn create_pipeline(&self, issue: &IssueRef) -> Result<PipelineId, PipelineStoreError> {
        let row = core_ai::create_pipeline(
            &self.pool,
            &issue.repo.forge.to_string(),
            &issue.repo.owner,
            &issue.repo.name,
            issue.number,
        )
        .await
        .map_err(|error| PipelineStoreError::Other(error.to_string()))?;

        Ok(PipelineId(row.id))
    }

    async fn append_event(
        &self,
        pipeline_id: PipelineId,
        event: PipelineEvent,
    ) -> Result<(), PipelineStoreError> {
        let payload = serde_json::to_string(&event)
            .map_err(|error| PipelineStoreError::Serialization(error.to_string()))?;

        core_ai::append_pipeline_event(&self.pool, pipeline_id.0, event.label(), &payload)
            .await
            .map_err(|error| PipelineStoreError::Other(error.to_string()))?;
        Ok(())
    }

    async fn events(
        &self,
        pipeline_id: PipelineId,
    ) -> Result<Vec<PipelineEvent>, PipelineStoreError> {
        let rows = core_ai::list_pipeline_events(&self.pool, pipeline_id.0)
            .await
            .map_err(|error| PipelineStoreError::Other(error.to_string()))?;

        rows.iter()
            .map(|row| {
                serde_json::from_str(&row.payload)
                    .map_err(|error| PipelineStoreError::Serialization(error.to_string()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use munibot_vcs::{Forge, RepoRef};

    use super::*;
    use crate::pipeline::{
        ApprovePlan, CreatePlan, IssueAnalysis, IssueClassification, ReproductionStatus,
    };

    fn issue() -> IssueRef {
        IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1)
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

    #[tokio::test]
    async fn test_create_pipeline_returns_an_id() {
        let store = InMemoryPipelineStore::new();
        let id = store.create_pipeline(&issue()).await.unwrap();
        assert_eq!(id, PipelineId(1));
    }

    #[tokio::test]
    async fn test_two_pipelines_get_distinct_ids() {
        let store = InMemoryPipelineStore::new();
        let first = store.create_pipeline(&issue()).await.unwrap();
        let second = store.create_pipeline(&issue()).await.unwrap();
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn test_appending_to_an_unknown_pipeline_is_not_found() {
        let store = InMemoryPipelineStore::new();
        let error = store
            .append_event(PipelineId(999), PipelineEvent::Failed {
                reason: "?".to_string(),
            })
            .await
            .expect_err("should not find a pipeline that was never created");
        assert!(matches!(error, PipelineStoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_events_returns_them_in_append_order() {
        let store = InMemoryPipelineStore::new();
        let id = store.create_pipeline(&issue()).await.unwrap();

        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();
        store
            .append_event(
                id,
                PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::Proceed)),
            )
            .await
            .unwrap();

        let events = store.events(id).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].label(), "Triggered");
        assert_eq!(events[1].label(), "IssueAnalyzed");
    }

    #[tokio::test]
    async fn test_replay_of_an_empty_log_is_triaging() {
        let store = InMemoryPipelineStore::new();
        let id = store.create_pipeline(&issue()).await.unwrap();
        assert_eq!(store.replay(id).await.unwrap(), PipelineState::Triaging);
    }

    #[tokio::test]
    async fn test_replay_follows_the_happy_path_through_research_and_planning() {
        let store = InMemoryPipelineStore::new();
        let id = store.create_pipeline(&issue()).await.unwrap();

        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();
        assert_eq!(store.replay(id).await.unwrap(), PipelineState::Triaging);

        store
            .append_event(
                id,
                PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::Proceed)),
            )
            .await
            .unwrap();
        assert_eq!(store.replay(id).await.unwrap(), PipelineState::Researching);

        store
            .append_event(
                id,
                PipelineEvent::ResearchCompleted(crate::pipeline::ResearchComplete {
                    summary: "uses axum".to_string(),
                    relevant_files: vec![],
                }),
            )
            .await
            .unwrap();
        assert_eq!(store.replay(id).await.unwrap(), PipelineState::Planning);

        store
            .append_event(
                id,
                PipelineEvent::PlanCreated(CreatePlan {
                    summary: "add dark mode".to_string(),
                    subtasks: vec![],
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            store.replay(id).await.unwrap(),
            PipelineState::ReviewingPlan
        );

        store
            .append_event(
                id,
                PipelineEvent::PlanApproved(ApprovePlan {
                    strengths: "clear ordering".to_string(),
                    feedback: "looks good".to_string(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(store.replay(id).await.unwrap(), PipelineState::Scheduling);
    }

    #[tokio::test]
    async fn test_replay_pauses_on_needs_more_info_and_resumes_to_triaging() {
        let store = InMemoryPipelineStore::new();
        let id = store.create_pipeline(&issue()).await.unwrap();

        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();
        store
            .append_event(
                id,
                PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::NeedsMoreInfo)),
            )
            .await
            .unwrap();

        let state = store.replay(id).await.unwrap();
        assert!(matches!(state, PipelineState::AwaitingUserInput { .. }));

        store
            .append_event(id, PipelineEvent::UserInputReceived {
                response: "yes, it happens every time".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(store.replay(id).await.unwrap(), PipelineState::Triaging);
    }

    #[tokio::test]
    async fn test_replay_of_a_skipped_issue_is_complete() {
        let store = InMemoryPipelineStore::new();
        let id = store.create_pipeline(&issue()).await.unwrap();

        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();
        store
            .append_event(
                id,
                PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::Skip)),
            )
            .await
            .unwrap();

        assert_eq!(store.replay(id).await.unwrap(), PipelineState::Complete);
    }

    #[tokio::test]
    async fn test_replaying_an_unknown_pipeline_is_not_found() {
        let store = InMemoryPipelineStore::new();
        let error = store
            .replay(PipelineId(999))
            .await
            .expect_err("should not find a pipeline that was never created");
        assert!(matches!(error, PipelineStoreError::NotFound(_)));
    }
}
