//! Integration tests for [`DieselRateLimitStore`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_memory_opt_in.rs` - see that file's module doc for the full
//! rationale.

use chrono::Utc;
use munibot_ai::limits::{DieselRateLimitStore, RateLimitStore, Scope};
use munibot_core::db::{DbPool, establish_pool};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

#[tokio::test]
async fn test_get_window_for_an_unstarted_scope_is_none() {
    let Some(pool) = pool().await else { return };
    let store = DieselRateLimitStore::new(pool);

    assert!(
        store
            .get_window(Scope::User(999_999))
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_reset_then_get_round_trips_through_the_real_stack() {
    let Some(pool) = pool().await else { return };
    let store = DieselRateLimitStore::new(pool);
    let now = Utc::now();

    store
        .reset_window(Scope::Global, now, 3, 300)
        .await
        .expect("reset failed");

    let window = store
        .get_window(Scope::Global)
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(window.request_count, 3);
    assert_eq!(window.token_count, 300);
}

#[tokio::test]
async fn test_increment_adds_to_an_existing_window() {
    let Some(pool) = pool().await else { return };
    let store = DieselRateLimitStore::new(pool);
    let now = Utc::now();

    store
        .reset_window(Scope::Guild(1), now, 1, 100)
        .await
        .expect("reset failed");
    store
        .increment(Scope::Guild(1), 1, 50)
        .await
        .expect("increment failed");

    let window = store.get_window(Scope::Guild(1)).await.unwrap().unwrap();
    assert_eq!(window.request_count, 2);
    assert_eq!(window.token_count, 150);
}
