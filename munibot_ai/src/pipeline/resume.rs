//! Resuming every non-terminal pipeline after a restart -- the payoff for
//! event sourcing: recovery is just the same replay every other read of a
//! pipeline's state already does, so there is no separate "was this run
//! interrupted" bookkeeping to get wrong.

use std::sync::Arc;

use crate::pipeline::{
    Executor, ExecutorError, ExecutorOutcome, PipelineId, PipelineStore, PipelineStoreError,
};

/// Why resuming one pipeline failed, distinct from an ordinary
/// `ExecutorError`: `Lookup` covers everything that goes wrong before an
/// `Executor` for it could even be built.
#[derive(Debug)]
pub enum ResumeError {
    Lookup(PipelineStoreError),
    Executor(ExecutorError),
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResumeError::Lookup(error) => {
                write!(f, "couldn't look up a pipeline to resume: {error}")
            }
            ResumeError::Executor(error) => write!(f, "couldn't resume a pipeline: {error}"),
        }
    }
}

impl std::error::Error for ResumeError {}

/// Loads every non-terminal pipeline from `store` and runs each one
/// forward with an `Executor` `build_executor` provides, re-provisioning
/// whatever sandbox its own current state needs (see `Executor::run`'s
/// own provisioning check, which triggers on any state past `Triaging`
/// regardless of whether this particular call ever passes through
/// `Researching`).
///
/// `build_executor` is a factory rather than a single shared `Executor`,
/// since each pipeline may need its own forge-specific dispatcher and
/// sandbox depending on which repository and issue it belongs to --
/// `PipelineStore::issue_for` is what a real factory uses to tell them
/// apart. Returns one result per pipeline that needed resuming, in no
/// particular order.
pub async fn resume_all(
    store: Arc<dyn PipelineStore>,
    build_executor: impl Fn(PipelineId) -> Executor,
) -> Vec<(PipelineId, Result<ExecutorOutcome, ResumeError>)> {
    let pipeline_ids = match store.non_terminal_pipeline_ids().await {
        Ok(ids) => ids,
        Err(error) => {
            // nothing to resume from is itself unrecoverable here -- there
            // is no pipeline id to report a per-pipeline failure against,
            // so this surfaces as a single, empty result set with the
            // failure logged by the caller instead
            tracing::error!(%error, "couldn't list non-terminal pipelines to resume");
            return Vec::new();
        }
    };

    let mut results = Vec::with_capacity(pipeline_ids.len());
    for pipeline_id in pipeline_ids {
        let executor = build_executor(pipeline_id);
        let outcome = executor
            .run(pipeline_id)
            .await
            .map_err(ResumeError::Executor);
        results.push((pipeline_id, outcome));
    }
    results
}

#[cfg(test)]
mod tests {
    use munibot_vcs::{Forge, IssueRef, RepoRef};

    use super::*;
    use crate::{
        pipeline::{
            InMemoryPipelineStore, IssueAnalysis, IssueClassification, MockAgentDispatcher,
            NoSandbox, PipelineEvent, PipelineState, RecommendedAction, ReproductionStatus,
        },
        tools::ToolRegistry,
    };

    fn issue() -> IssueRef {
        IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1)
    }

    fn ok_output(
        handoff: serde_json::Value,
    ) -> Result<crate::pipeline::AgentOutput, crate::pipeline::DispatchError> {
        Ok(crate::pipeline::AgentOutput {
            handoff,
            usage: crate::types::Usage::default(),
            cost: crate::types::Cost::ZERO,
        })
    }

    #[tokio::test]
    async fn test_resume_all_skips_pipelines_already_terminal() {
        let store: Arc<dyn PipelineStore> = Arc::new(InMemoryPipelineStore::new());
        let id = store.create_pipeline(&issue()).await.unwrap();
        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();
        store
            .append_event(
                id,
                PipelineEvent::IssueAnalyzed(IssueAnalysis {
                    classification: IssueClassification::NotActionable,
                    reproduction_status: ReproductionStatus::NotApplicable,
                    summary: "spam".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::Skip,
                    relevant_files: vec![],
                }),
            )
            .await
            .unwrap();

        // a dispatcher with no scripted responses at all -- if resume_all
        // tried to run this already-terminal pipeline, invoking it would
        // panic
        let results = resume_all(store.clone(), |_id| {
            Executor::new(
                store.clone(),
                Arc::new(MockAgentDispatcher::new()),
                Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new()))),
            )
        })
        .await;

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_resume_all_runs_every_non_terminal_pipeline_to_completion() {
        let store: Arc<dyn PipelineStore> = Arc::new(InMemoryPipelineStore::new());
        let id = store.create_pipeline(&issue()).await.unwrap();
        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();

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

        let results = resume_all(store.clone(), |_id| {
            Executor::new(
                store.clone(),
                dispatcher.clone(),
                Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new()))),
            )
        })
        .await;

        assert_eq!(results.len(), 1);
        let (resumed_id, outcome) = &results[0];
        assert_eq!(*resumed_id, id);
        assert_eq!(
            outcome.as_ref().unwrap(),
            &ExecutorOutcome::Finished(PipelineState::Complete)
        );
    }

    #[tokio::test]
    async fn test_resuming_after_a_fresh_store_handle_continues_where_it_left_off() {
        // simulates "a new process started": a brand new store handle
        // (here, the same in-memory store shared behind an Arc, since
        // that is the whole point of an Arc -- the real analogue is
        // DieselPipelineStore, where a fresh handle is just a new
        // connection to the same, already-persisted rows) and a brand
        // new Executor built fresh rather than reused
        let store: Arc<dyn PipelineStore> = Arc::new(InMemoryPipelineStore::new());
        let id = store.create_pipeline(&issue()).await.unwrap();
        store
            .append_event(id, PipelineEvent::Triggered { issue: issue() })
            .await
            .unwrap();
        store
            .append_event(
                id,
                PipelineEvent::IssueAnalyzed(IssueAnalysis {
                    classification: IssueClassification::Bug,
                    reproduction_status: ReproductionStatus::Reproduced,
                    summary: "crashes".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::Proceed,
                    relevant_files: vec![],
                }),
            )
            .await
            .unwrap();
        assert_eq!(store.replay(id).await.unwrap(), PipelineState::Researching);

        // "restart": nothing about the executor below carries any memory
        // of the two events already appended above -- only the store does
        let dispatcher = Arc::new(
            MockAgentDispatcher::new()
                .respond(ok_output(
                    serde_json::to_value(crate::pipeline::ResearchComplete {
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
        let results = resume_all(store.clone(), |_id| {
            Executor::new(
                store.clone(),
                dispatcher.clone(),
                Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new()))),
            )
        })
        .await;

        assert_eq!(results.len(), 1);
        assert!(matches!(
            results[0].1.as_ref().unwrap(),
            ExecutorOutcome::Paused(PipelineState::AwaitingUserInput { .. })
        ));
    }
}
