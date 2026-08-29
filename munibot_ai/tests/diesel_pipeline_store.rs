//! Integration tests for [`DieselPipelineStore`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_session_store.rs` - see that file's module doc for the full
//! rationale. Tests share the database and isolate themselves with a
//! unique issue number per test rather than creating a database each.

use std::sync::atomic::{AtomicU64, Ordering};

use munibot_ai::pipeline::{
    ApprovePlan, CreatePlan, DieselPipelineStore, IssueAnalysis, IssueClassification,
    PipelineEvent, PipelineState, PipelineStore, RecommendedAction, ReproductionStatus,
    ResearchComplete,
};
use munibot_core::db::{DbPool, establish_pool};
use munibot_vcs::{Forge, IssueRef, RepoRef};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

fn unique_issue() -> IssueRef {
    static NEXT_ISSUE_NUMBER: AtomicU64 = AtomicU64::new(1);
    IssueRef::new(
        RepoRef::new(Forge::GitHub, "musicaloft", "munibot"),
        NEXT_ISSUE_NUMBER.fetch_add(1, Ordering::SeqCst),
    )
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
async fn test_create_pipeline_and_replay_an_empty_log_is_triaging() {
    let Some(pool) = pool().await else { return };
    let store = DieselPipelineStore::new(pool);

    let id = store
        .create_pipeline(&unique_issue())
        .await
        .expect("create failed");
    assert_eq!(
        store.replay(id).await.expect("replay failed"),
        PipelineState::Triaging
    );
}

#[tokio::test]
async fn test_appended_events_persist_and_replay_correctly() {
    let Some(pool) = pool().await else { return };
    let issue = unique_issue();
    let store = DieselPipelineStore::new(pool);
    let id = store.create_pipeline(&issue).await.expect("create failed");

    store
        .append_event(id, PipelineEvent::Triggered {
            issue: issue.clone(),
        })
        .await
        .expect("append failed");
    store
        .append_event(
            id,
            PipelineEvent::IssueAnalyzed(analysis(RecommendedAction::Proceed)),
        )
        .await
        .expect("append failed");
    store
        .append_event(
            id,
            PipelineEvent::ResearchCompleted(ResearchComplete {
                summary: "uses axum".to_string(),
                relevant_files: vec![],
            }),
        )
        .await
        .expect("append failed");
    store
        .append_event(
            id,
            PipelineEvent::PlanCreated(CreatePlan {
                summary: "add dark mode".to_string(),
                subtasks: vec![],
            }),
        )
        .await
        .expect("append failed");
    store
        .append_event(
            id,
            PipelineEvent::PlanApproved(ApprovePlan {
                strengths: "clear ordering".to_string(),
                feedback: "ship it".to_string(),
            }),
        )
        .await
        .expect("append failed");

    let events = store.events(id).await.expect("events failed");
    assert_eq!(events.len(), 5);

    assert_eq!(
        store.replay(id).await.expect("replay failed"),
        PipelineState::Scheduling
    );
}

#[tokio::test]
async fn test_events_for_two_pipelines_do_not_interfere() {
    let Some(pool) = pool().await else { return };
    let store = DieselPipelineStore::new(pool);

    let first = store
        .create_pipeline(&unique_issue())
        .await
        .expect("create failed");
    let second = store
        .create_pipeline(&unique_issue())
        .await
        .expect("create failed");

    store
        .append_event(first, PipelineEvent::Triggered {
            issue: unique_issue(),
        })
        .await
        .expect("append failed");

    assert_eq!(store.events(first).await.expect("events failed").len(), 1);
    assert!(
        store
            .events(second)
            .await
            .expect("events failed")
            .is_empty()
    );
}
