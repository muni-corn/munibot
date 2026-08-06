//! Integration tests for [`DieselSpendCapStore`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_rate_limit_store.rs` - see that file's module doc for the full
//! rationale.

use chrono::Utc;
use munibot_ai::limits::{DieselSpendCapStore, Scope, SpendCapStore};
use munibot_core::db::{DbPool, establish_pool};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

#[tokio::test]
async fn test_get_cap_for_a_never_created_scope_is_none() {
    let Some(pool) = pool().await else { return };
    let store = DieselSpendCapStore::new(pool);

    assert!(
        store
            .get_cap(Scope::User(999_999), "monthly")
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_upsert_then_get_round_trips_through_the_real_stack() {
    let Some(pool) = pool().await else { return };
    let store = DieselSpendCapStore::new(pool);
    let reset_at = Utc::now();

    store
        .upsert_cap(Scope::Global, "monthly", 10_000, 2_500, reset_at)
        .await
        .expect("upsert failed");

    let cap = store
        .get_cap(Scope::Global, "monthly")
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(cap.limit_micros, 10_000);
    assert_eq!(cap.current_micros, 2_500);
}

#[tokio::test]
async fn test_increment_spend_adds_to_an_existing_cap() {
    let Some(pool) = pool().await else { return };
    let store = DieselSpendCapStore::new(pool);
    let reset_at = Utc::now();

    store
        .upsert_cap(Scope::User(1), "monthly", 10_000, 1_000, reset_at)
        .await
        .expect("upsert failed");
    store
        .increment_spend(Scope::User(1), "monthly", 500)
        .await
        .expect("increment failed");

    let cap = store
        .get_cap(Scope::User(1), "monthly")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cap.current_micros, 1_500);
}

#[tokio::test]
async fn test_upsert_over_an_existing_cap_replaces_it_wholesale() {
    let Some(pool) = pool().await else { return };
    let store = DieselSpendCapStore::new(pool);
    let reset_at = Utc::now();

    store
        .upsert_cap(Scope::User(2), "monthly", 10_000, 9_000, reset_at)
        .await
        .expect("first upsert failed");
    store
        .upsert_cap(Scope::User(2), "monthly", 20_000, 0, reset_at)
        .await
        .expect("second upsert failed");

    let cap = store
        .get_cap(Scope::User(2), "monthly")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cap.limit_micros, 20_000);
    assert_eq!(cap.current_micros, 0);
}
