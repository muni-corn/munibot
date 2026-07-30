//! Integration tests for [`DieselUsageRecorder`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_session_store.rs` - see that file's module doc for the full
//! rationale.

use munibot_ai::{
    types::{AiError, Cost, Usage},
    usage::{DieselUsageRecorder, UsageRecord, UsageRecorder},
};
use munibot_core::db::{DbPool, establish_pool};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

fn record(succeeded: bool) -> UsageRecord {
    UsageRecord {
        conversation_id: None,
        user_id: None,
        guild_id: None,
        provider: "anthropic".to_string(),
        model: "claude-opus-5".to_string(),
        persona_id: "companion".to_string(),
        usage: Usage::new(100, 200),
        cost: Cost::from_micros(5_000),
        iterations: 2,
        succeeded,
    }
}

#[tokio::test]
async fn test_recording_a_successful_turn_succeeds() {
    let Some(pool) = pool().await else { return };
    let recorder = DieselUsageRecorder::new(pool);

    recorder
        .record(record(true))
        .await
        .expect("recording should succeed");
}

#[tokio::test]
async fn test_recording_a_failed_turn_succeeds_too() {
    let Some(pool) = pool().await else { return };
    let recorder = DieselUsageRecorder::new(pool);

    // the whole point of this table: a failed turn still gets a row, since it
    // still cost money
    recorder
        .record(record(false))
        .await
        .expect("recording a failure should still succeed");
}

#[tokio::test]
async fn test_recorder_is_usable_as_a_trait_object() {
    let Some(pool) = pool().await else { return };
    let recorder: std::sync::Arc<dyn UsageRecorder> =
        std::sync::Arc::new(DieselUsageRecorder::new(pool));

    let result: Result<(), AiError> = recorder.record(record(true)).await;
    assert!(result.is_ok());
}
