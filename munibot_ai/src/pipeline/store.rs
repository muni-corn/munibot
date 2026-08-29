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
use munibot_vcs::{Forge, IssueRef, RepoRef};
use thiserror::Error;

use super::advance::advance;
use crate::pipeline::{PipelineEvent, PipelineId, PipelineState};

/// Why a [`PipelineStore`] operation failed.
#[derive(Error, Debug)]
pub enum PipelineStoreError {
    #[error("pipeline {0:?} was not found")]
    NotFound(PipelineId),
    #[error("couldn't (de)serialize a pipeline event: {0}")]
    Serialization(String),
    /// `replay` folded a persisted event log through `advance` and hit an
    /// illegal transition -- every event in a real log was itself
    /// appended after `advance` already accepted it, so this means the
    /// log itself was corrupted or written by something that bypassed
    /// that check, not an ordinary runtime condition.
    #[error("pipeline event log is not a legal history: {0}")]
    InvalidHistory(String),
    #[error("pipeline store operation failed: {0}")]
    Other(String),
}

/// Folds a whole event log into the state it resolves to, starting from
/// `PipelineState::Triaging` -- an empty log resolves there too, since
/// that's the state a run is in before its own `Triggered` event even
/// arrives.
fn fold(events: &[PipelineEvent]) -> Result<PipelineState, PipelineStoreError> {
    events
        .iter()
        .try_fold(PipelineState::Triaging, |state, event| {
            advance(state, event)
                .map_err(|error| PipelineStoreError::InvalidHistory(error.to_string()))
        })
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
        fold(&self.events(pipeline_id).await?)
    }

    /// The issue a pipeline was created for -- what a resumed run needs to
    /// rebuild the forge-specific dispatcher and sandbox its own executor
    /// requires, since neither is itself part of the persisted event log.
    async fn issue_for(&self, pipeline_id: PipelineId) -> Result<IssueRef, PipelineStoreError>;

    /// Every pipeline this store has ever created, in no particular
    /// order -- what resuming after a restart starts from.
    async fn all_pipeline_ids(&self) -> Result<Vec<PipelineId>, PipelineStoreError>;

    /// Every pipeline whose own event log has not yet resolved to a
    /// terminal state -- what actually needs resuming after a restart.
    /// A default method: replaying each candidate is enough to answer
    /// this generically, so no implementation needs its own version.
    async fn non_terminal_pipeline_ids(&self) -> Result<Vec<PipelineId>, PipelineStoreError> {
        let mut non_terminal = Vec::new();
        for pipeline_id in self.all_pipeline_ids().await? {
            if !self.replay(pipeline_id).await?.is_terminal() {
                non_terminal.push(pipeline_id);
            }
        }
        Ok(non_terminal)
    }
}

/// An in-memory [`PipelineStore`], for tests -- see
/// `DieselPipelineStore` for the production implementation backed by
/// `ai_pipelines`/`ai_pipeline_events`.
#[derive(Default)]
pub struct InMemoryPipelineStore {
    next_id: AtomicI64,
    pipelines: Mutex<HashMap<PipelineId, (IssueRef, Vec<PipelineEvent>)>>,
}

impl InMemoryPipelineStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PipelineStore for InMemoryPipelineStore {
    async fn create_pipeline(&self, issue: &IssueRef) -> Result<PipelineId, PipelineStoreError> {
        let id = PipelineId(self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
        self.pipelines
            .lock()
            .expect("pipeline store lock poisoned")
            .insert(id, (issue.clone(), Vec::new()));
        Ok(id)
    }

    async fn append_event(
        &self,
        pipeline_id: PipelineId,
        event: PipelineEvent,
    ) -> Result<(), PipelineStoreError> {
        let mut pipelines = self.pipelines.lock().expect("pipeline store lock poisoned");
        let (_, events) = pipelines
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
            .map(|(_, events)| events.clone())
            .ok_or(PipelineStoreError::NotFound(pipeline_id))
    }

    async fn issue_for(&self, pipeline_id: PipelineId) -> Result<IssueRef, PipelineStoreError> {
        self.pipelines
            .lock()
            .expect("pipeline store lock poisoned")
            .get(&pipeline_id)
            .map(|(issue, _)| issue.clone())
            .ok_or(PipelineStoreError::NotFound(pipeline_id))
    }

    async fn all_pipeline_ids(&self) -> Result<Vec<PipelineId>, PipelineStoreError> {
        Ok(self
            .pipelines
            .lock()
            .expect("pipeline store lock poisoned")
            .keys()
            .copied()
            .collect())
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

    async fn issue_for(&self, pipeline_id: PipelineId) -> Result<IssueRef, PipelineStoreError> {
        let row = core_ai::get_pipeline(&self.pool, pipeline_id.0)
            .await
            .map_err(|error| PipelineStoreError::Other(error.to_string()))?
            .ok_or(PipelineStoreError::NotFound(pipeline_id))?;

        let forge = match row.forge.as_str() {
            "github" => Forge::GitHub,
            other => {
                return Err(PipelineStoreError::Other(format!(
                    "unknown forge {other:?} stored for pipeline {pipeline_id:?}"
                )));
            }
        };

        Ok(IssueRef::new(
            RepoRef::new(forge, row.owner, row.repo_name),
            row.issue_number,
        ))
    }

    async fn all_pipeline_ids(&self) -> Result<Vec<PipelineId>, PipelineStoreError> {
        Ok(core_ai::list_pipeline_ids(&self.pool)
            .await
            .map_err(|error| PipelineStoreError::Other(error.to_string()))?
            .into_iter()
            .map(PipelineId)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use munibot_vcs::{Forge, RepoRef};

    use super::*;
    use crate::pipeline::{
        ApprovePlan, CreatePlan, IssueAnalysis, IssueClassification, RecommendedAction,
        ReproductionStatus,
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

    #[tokio::test]
    async fn test_issue_for_returns_the_issue_a_pipeline_was_created_for() {
        let store = InMemoryPipelineStore::new();
        let id = store.create_pipeline(&issue()).await.unwrap();
        assert_eq!(store.issue_for(id).await.unwrap(), issue());
    }

    #[tokio::test]
    async fn test_issue_for_an_unknown_pipeline_is_not_found() {
        let store = InMemoryPipelineStore::new();
        let error = store
            .issue_for(PipelineId(999))
            .await
            .expect_err("should not find a pipeline that was never created");
        assert!(matches!(error, PipelineStoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_all_pipeline_ids_lists_every_created_pipeline() {
        let store = InMemoryPipelineStore::new();
        let first = store.create_pipeline(&issue()).await.unwrap();
        let second = store.create_pipeline(&issue()).await.unwrap();

        let mut ids = store.all_pipeline_ids().await.unwrap();
        ids.sort_by_key(|id| id.0);
        assert_eq!(ids, vec![first, second]);
    }

    #[tokio::test]
    async fn test_non_terminal_pipeline_ids_excludes_completed_runs() {
        let store = InMemoryPipelineStore::new();
        let completed = store.create_pipeline(&issue()).await.unwrap();
        store
            .append_event(completed, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();
        store
            .append_event(
                completed,
                PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::Skip)),
            )
            .await
            .unwrap();

        let still_running = store.create_pipeline(&issue()).await.unwrap();
        store
            .append_event(still_running, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();

        let non_terminal = store.non_terminal_pipeline_ids().await.unwrap();
        assert_eq!(non_terminal, vec![still_running]);
    }
}
