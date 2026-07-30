//! Integration tests for [`DieselToolAuditor`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_session_store.rs` - see that file's module doc for the full
//! rationale.

use std::time::Duration;

use munibot_ai::audit::{DieselToolAuditor, ToolAuditor, ToolCallRecord, ToolCallStatus};
use munibot_core::db::{DbPool, establish_pool};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

fn record(status: ToolCallStatus) -> ToolCallRecord {
    ToolCallRecord {
        conversation_id: None,
        tool_name: "current_time".to_string(),
        input: "{}".to_string(),
        output: "12:00".to_string(),
        duration: Duration::from_millis(5),
        status,
    }
}

#[tokio::test]
async fn test_recording_a_successful_call() {
    let Some(pool) = pool().await else { return };
    let auditor = DieselToolAuditor::new(pool);

    // record() returns () rather than a Result, so this test's only assertion is
    // that it does not panic - a database failure inside the auditor is logged
    // and swallowed, never propagated
    auditor.record(record(ToolCallStatus::Ok)).await;
}

#[tokio::test]
async fn test_recording_a_failed_call_also_succeeds() {
    let Some(pool) = pool().await else { return };
    let auditor = DieselToolAuditor::new(pool);

    auditor.record(record(ToolCallStatus::Fatal)).await;
}

#[tokio::test]
async fn test_auditor_is_usable_as_a_trait_object() {
    let Some(pool) = pool().await else { return };
    let auditor: std::sync::Arc<dyn ToolAuditor> =
        std::sync::Arc::new(DieselToolAuditor::new(pool));

    auditor.record(record(ToolCallStatus::Err)).await;
}
