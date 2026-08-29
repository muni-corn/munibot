//! Integration tests for [`DieselSafetyEventAuditor`] against a real
//! database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_rate_limit_store.rs` - see that file's module doc for the full
//! rationale.

use munibot_ai::{
    limits::Scope,
    safety::{DieselSafetyEventAuditor, SafetyEvent, SafetyEventAuditor, SafetyEventType},
};
use munibot_core::db::{
    DbPool, establish_pool,
    operations::ai::{list_safety_events, record_safety_event},
};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

#[tokio::test]
async fn test_recording_an_event_then_listing_it_round_trips_through_the_real_stack() {
    let Some(pool) = pool().await else { return };
    let auditor = DieselSafetyEventAuditor::new(pool.clone());

    auditor
        .record(
            SafetyEvent::new(
                SafetyEventType::RateLimit,
                Scope::User(123_456_789),
                "too many requests",
            )
            .with_content("please slow down please slow down please slow down"),
        )
        .await;

    let events = list_safety_events(&pool, 50).await.expect("query failed");
    let recorded = events
        .iter()
        .find(|event| event.reason == "too many requests")
        .expect("the recorded event should be listed");

    assert_eq!(recorded.event_type, "rate_limit");
    assert_eq!(recorded.scope_type, "user");
    assert_eq!(recorded.scope_id, Some(123_456_789));
    assert!(
        recorded.content_hash.is_some(),
        "content should be hashed, not stored raw"
    );
}

#[tokio::test]
async fn test_an_event_with_no_content_stores_no_hash() {
    let Some(pool) = pool().await else { return };

    record_safety_event(&pool, munibot_core::db::models::NewAiSafetyEvent {
        event_type: "spend_cap".to_string(),
        scope_type: "global".to_string(),
        scope_id: None,
        reason: "test-only: global spend cap reached".to_string(),
        content_hash: None,
        created_at: chrono::Utc::now().naive_utc(),
    })
    .await
    .expect("insert failed");

    let events = list_safety_events(&pool, 50).await.expect("query failed");
    let recorded = events
        .iter()
        .find(|event| event.reason == "test-only: global spend cap reached")
        .expect("the recorded event should be listed");

    assert!(recorded.content_hash.is_none());
}
