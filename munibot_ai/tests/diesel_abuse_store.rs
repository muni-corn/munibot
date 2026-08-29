//! Integration tests for [`DieselAbuseStore`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_rate_limit_store.rs` - see that file's module doc for the full
//! rationale.

use chrono::{Duration, Utc};
use munibot_ai::{
    abuse::{AbuseStore, DieselAbuseStore},
    limits::Scope,
};
use munibot_core::db::{DbPool, establish_pool};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

#[tokio::test]
async fn test_get_for_a_scope_with_no_strikes_is_none() {
    let Some(pool) = pool().await else { return };
    let store = DieselAbuseStore::new(pool);

    assert!(
        store
            .get(Scope::User(999_999))
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_record_strike_then_get_round_trips_through_the_real_stack() {
    let Some(pool) = pool().await else { return };
    let store = DieselAbuseStore::new(pool);
    let cooldown_until = Utc::now() + Duration::minutes(5);

    store
        .record_strike(
            Scope::User(1),
            1,
            cooldown_until,
            "repeated near-identical prompts",
        )
        .await
        .expect("record failed");

    let row = store
        .get(Scope::User(1))
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(row.strike_count, 1);
    assert_eq!(row.cooldown_until.timestamp(), cooldown_until.timestamp());
}

#[tokio::test]
async fn test_a_second_strike_overwrites_the_first_rather_than_duplicating_the_row() {
    let Some(pool) = pool().await else { return };
    let store = DieselAbuseStore::new(pool);
    let first_cooldown = Utc::now() + Duration::minutes(1);
    let second_cooldown = Utc::now() + Duration::minutes(10);

    store
        .record_strike(
            Scope::Guild(42),
            1,
            first_cooldown,
            "rapid persona switching",
        )
        .await
        .expect("first record failed");
    store
        .record_strike(
            Scope::Guild(42),
            2,
            second_cooldown,
            "a known prompt-injection phrasing",
        )
        .await
        .expect("second record failed");

    let row = store.get(Scope::Guild(42)).await.unwrap().unwrap();
    assert_eq!(row.strike_count, 2);
    assert_eq!(row.cooldown_until.timestamp(), second_cooldown.timestamp());
}
