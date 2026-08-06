//! Integration tests for `db::operations::ai::limits`.
//!
//! Each test gets its own isolated MySQL database via `TestDb`. MySQL must be
//! running with the devenv credentials before running these tests.

mod common;

use chrono::{Duration, SubsecRound, Utc};
use common::TestDb;
use munibot_core::db::{models::NewAiSpendCap, operations::ai};

#[tokio::test]
async fn test_get_rate_limit_missing_returns_none() {
    let db = TestDb::new().await;
    assert!(
        ai::get_rate_limit(&db.pool, "user", Some(1))
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_reset_rate_limit_window_creates_and_replaces_a_row() {
    let db = TestDb::new().await;
    let now = Utc::now().naive_utc().trunc_subsecs(0);

    ai::reset_rate_limit_window(&db.pool, "user", Some(1), now, 1, 100)
        .await
        .expect("reset failed");

    let row = ai::get_rate_limit(&db.pool, "user", Some(1))
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(row.request_count, 1);
    assert_eq!(row.token_count, 100);

    let later = now + Duration::minutes(5);
    ai::reset_rate_limit_window(&db.pool, "user", Some(1), later, 0, 0)
        .await
        .expect("reset failed");

    let row = ai::get_rate_limit(&db.pool, "user", Some(1))
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(
        row.request_count, 0,
        "resetting the window should replace the old counts, not add to them"
    );
    assert_eq!(row.window_start, later);
}

#[tokio::test]
async fn test_increment_rate_limit_adds_to_existing_counts() {
    let db = TestDb::new().await;
    let now = Utc::now().naive_utc().trunc_subsecs(0);
    ai::reset_rate_limit_window(&db.pool, "user", Some(1), now, 1, 100)
        .await
        .expect("reset failed");

    ai::increment_rate_limit(&db.pool, "user", Some(1), 1, 50)
        .await
        .expect("increment failed");
    ai::increment_rate_limit(&db.pool, "user", Some(1), 1, 50)
        .await
        .expect("increment failed");

    let row = ai::get_rate_limit(&db.pool, "user", Some(1))
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(row.request_count, 3, "1 from the reset plus 2 increments");
    assert_eq!(row.token_count, 200, "100 from the reset plus 2 * 50");
}

#[tokio::test]
async fn test_rate_limit_scopes_are_independent() {
    let db = TestDb::new().await;
    let now = Utc::now().naive_utc().trunc_subsecs(0);

    ai::reset_rate_limit_window(&db.pool, "user", Some(1), now, 5, 500)
        .await
        .unwrap();
    ai::reset_rate_limit_window(&db.pool, "user", Some(2), now, 1, 10)
        .await
        .unwrap();
    ai::reset_rate_limit_window(&db.pool, "global", None, now, 9, 900)
        .await
        .unwrap();

    let user_one = ai::get_rate_limit(&db.pool, "user", Some(1))
        .await
        .unwrap()
        .unwrap();
    let user_two = ai::get_rate_limit(&db.pool, "user", Some(2))
        .await
        .unwrap()
        .unwrap();
    let global = ai::get_rate_limit(&db.pool, "global", None)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(user_one.request_count, 5);
    assert_eq!(user_two.request_count, 1);
    assert_eq!(global.request_count, 9);
}

#[tokio::test]
async fn test_get_spend_cap_missing_returns_none() {
    let db = TestDb::new().await;
    assert!(
        ai::get_spend_cap(&db.pool, "user", Some(1), "monthly")
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_upsert_spend_cap_creates_and_replaces_a_row() {
    let db = TestDb::new().await;
    let now = Utc::now().naive_utc().trunc_subsecs(0);
    let reset_at = now + Duration::days(30);

    ai::upsert_spend_cap(&db.pool, NewAiSpendCap {
        scope_type: "user".to_string(),
        scope_id: Some(1),
        period: "monthly".to_string(),
        limit_micros: 5_000_000,
        current_micros: 0,
        reset_at,
    })
    .await
    .expect("upsert failed");

    let row = ai::get_spend_cap(&db.pool, "user", Some(1), "monthly")
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(row.limit_micros, 5_000_000);
    assert_eq!(row.current_micros, 0);

    let new_reset_at = reset_at + Duration::days(30);
    ai::upsert_spend_cap(&db.pool, NewAiSpendCap {
        scope_type: "user".to_string(),
        scope_id: Some(1),
        period: "monthly".to_string(),
        limit_micros: 5_000_000,
        current_micros: 0,
        reset_at: new_reset_at,
    })
    .await
    .expect("upsert failed");

    let row = ai::get_spend_cap(&db.pool, "user", Some(1), "monthly")
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(
        row.reset_at, new_reset_at,
        "rolling a cap over should replace reset_at, not leave the old one"
    );
}

#[tokio::test]
async fn test_increment_spend_adds_to_the_current_period() {
    let db = TestDb::new().await;
    let now = Utc::now().naive_utc().trunc_subsecs(0);
    ai::upsert_spend_cap(&db.pool, NewAiSpendCap {
        scope_type: "user".to_string(),
        scope_id: Some(1),
        period: "monthly".to_string(),
        limit_micros: 5_000_000,
        current_micros: 0,
        reset_at: now + Duration::days(30),
    })
    .await
    .unwrap();

    ai::increment_spend(&db.pool, "user", Some(1), "monthly", 1_200)
        .await
        .expect("increment failed");
    ai::increment_spend(&db.pool, "user", Some(1), "monthly", 800)
        .await
        .expect("increment failed");

    let row = ai::get_spend_cap(&db.pool, "user", Some(1), "monthly")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.current_micros, 2_000);
}

#[tokio::test]
async fn test_spend_cap_periods_are_independent_for_the_same_scope() {
    let db = TestDb::new().await;
    let now = Utc::now().naive_utc().trunc_subsecs(0);

    ai::upsert_spend_cap(&db.pool, NewAiSpendCap {
        scope_type: "user".to_string(),
        scope_id: Some(1),
        period: "daily".to_string(),
        limit_micros: 100_000,
        current_micros: 0,
        reset_at: now + Duration::days(1),
    })
    .await
    .unwrap();
    ai::upsert_spend_cap(&db.pool, NewAiSpendCap {
        scope_type: "user".to_string(),
        scope_id: Some(1),
        period: "monthly".to_string(),
        limit_micros: 5_000_000,
        current_micros: 0,
        reset_at: now + Duration::days(30),
    })
    .await
    .unwrap();

    ai::increment_spend(&db.pool, "user", Some(1), "daily", 500)
        .await
        .unwrap();

    let daily = ai::get_spend_cap(&db.pool, "user", Some(1), "daily")
        .await
        .unwrap()
        .unwrap();
    let monthly = ai::get_spend_cap(&db.pool, "user", Some(1), "monthly")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(daily.current_micros, 500);
    assert_eq!(
        monthly.current_micros, 0,
        "incrementing one period must not leak into another for the same scope"
    );
}
