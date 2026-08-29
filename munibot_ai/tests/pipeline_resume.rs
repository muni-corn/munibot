//! Integration test: a pipeline resumes correctly from a brand new
//! `DieselPipelineStore`/`DbPool` handle, standing in for a real process
//! restart -- the payoff for event sourcing. Skipped entirely unless
//! `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching `diesel_pipeline_store.rs`.

use std::sync::{Arc, atomic::AtomicU64};

use munibot_ai::{
    pipeline::{
        AgentOutput, DieselPipelineStore, DispatchError, Executor, ExecutorOutcome, IssueAnalysis,
        IssueClassification, MockAgentDispatcher, NoSandbox, PipelineEvent, PipelineState,
        PipelineStore, RecommendedAction, ReproductionStatus, ResearchComplete,
    },
    tools::ToolRegistry,
    types::{Cost, Usage},
};
use munibot_core::db::{DbPool, establish_pool};
use munibot_vcs::{Forge, IssueRef, RepoRef};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

fn unique_issue() -> IssueRef {
    static NEXT_ISSUE_NUMBER: AtomicU64 = AtomicU64::new(100_000);
    IssueRef::new(
        RepoRef::new(Forge::GitHub, "musicaloft", "munibot"),
        NEXT_ISSUE_NUMBER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
    )
}

fn ok_output(handoff: serde_json::Value) -> Result<AgentOutput, DispatchError> {
    Ok(AgentOutput {
        handoff,
        usage: Usage::default(),
        cost: Cost::ZERO,
    })
}

#[tokio::test]
async fn test_a_pipeline_resumes_correctly_from_a_brand_new_store_handle() {
    let Some(first_pool) = pool().await else {
        return;
    };
    let issue = unique_issue();

    // "before the crash": create the pipeline and get it partway, through
    // a store backed by its own connection pool
    let store_before_restart = DieselPipelineStore::new(first_pool);
    let id = store_before_restart.create_pipeline(&issue).await.unwrap();
    store_before_restart
        .append_event(id, PipelineEvent::Triggered {
            issue: issue.clone(),
        })
        .await
        .unwrap();
    store_before_restart
        .append_event(
            id,
            PipelineEvent::IssueAnalyzed(IssueAnalysis {
                classification: IssueClassification::Bug,
                reproduction_status: ReproductionStatus::Reproduced,
                summary: "crashes on startup".to_string(),
                reproduction_details: "ran with no config".to_string(),
                recommended_action: RecommendedAction::Proceed,
                relevant_files: vec![],
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        store_before_restart.replay(id).await.unwrap(),
        PipelineState::Researching
    );

    // "the process is killed here" -- everything above is dropped,
    // nothing about it carries forward except what is in the database
    drop(store_before_restart);

    // "the process restarts": an entirely new pool, an entirely new
    // store, an entirely new executor and dispatcher
    let Some(second_pool) = pool().await else {
        return;
    };
    let store_after_restart: Arc<dyn PipelineStore> =
        Arc::new(DieselPipelineStore::new(second_pool));

    let dispatcher = Arc::new(
        MockAgentDispatcher::new()
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
    let sandbox = Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())));
    let executor = Executor::new(store_after_restart.clone(), dispatcher, sandbox);

    let outcome = executor.run(id).await.expect("resumed run should succeed");
    assert!(
        matches!(
            outcome,
            ExecutorOutcome::Paused(PipelineState::AwaitingUserInput { .. })
        ),
        "the resumed run should continue past Researching, not restart from Triaging"
    );

    let events = store_after_restart.events(id).await.unwrap();
    assert_eq!(
        events.len(),
        4,
        "the two events from before the restart, plus ResearchCompleted and PlanHelpRequested"
    );
}
