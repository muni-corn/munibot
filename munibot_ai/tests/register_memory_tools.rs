//! Integration tests for [`register_memory_tools`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_memory_opt_in.rs` - see that file's module doc for the full
//! rationale.

use std::sync::atomic::{AtomicU32, Ordering};

use munibot_ai::{
    memory::register_memory_tools,
    tools::{ConversationId, Platform, RiskTier, ToolCtx, ToolOutcome, ToolRegistry},
};
use munibot_core::db::{DbPool, establish_pool};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

async fn a_user(pool: &DbPool) -> u64 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    munibot_core::db::operations::get_or_create_user_from_linked_account(
        pool,
        "discord",
        &format!("register-memory-tools-test-{nanos}-{n}"),
        "muni",
        "muni",
        None,
        "unused-token",
        None,
        None,
    )
    .await
    .expect("couldn't create a user")
    .id as u64
}

fn ctx(user_id: u64) -> ToolCtx {
    ToolCtx {
        user_id,
        platform: Platform::Web,
        granted_tier: RiskTier::Safe,
        guild_id: None,
        conversation_id: ConversationId(1),
        cancellation: tokio_util::sync::CancellationToken::new(),
        delegation_depth: 0,
        remaining_budget: munibot_ai::harness::Budget::default(),
        delegation_spend: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
    }
}

#[tokio::test]
async fn test_remember_and_forget_are_registered_under_their_own_names() {
    let Some(pool) = pool().await else { return };
    let mut registry = ToolRegistry::new();

    register_memory_tools(&mut registry, pool);

    assert!(registry.get("remember").is_some());
    assert!(registry.get("forget").is_some());
}

#[tokio::test]
async fn test_remember_refuses_until_the_user_has_opted_in() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let mut registry = ToolRegistry::new();
    register_memory_tools(&mut registry, pool);

    let remember = registry.get("remember").expect("should be registered");
    let outcome = remember
        .invoke(
            serde_json::json!({"key": "favorite_color", "value": "cyan"}),
            &ctx(user),
        )
        .await;

    assert!(
        matches!(outcome, ToolOutcome::Err(_)),
        "remembering before opting in should be a recoverable refusal, got {outcome:?}"
    );
}

#[tokio::test]
async fn test_remember_then_forget_round_trips_through_the_real_stack() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    munibot_core::db::operations::ai::set_memory_opt_in(&pool, user as i64, true)
        .await
        .expect("couldn't opt the test user in");

    let mut registry = ToolRegistry::new();
    register_memory_tools(&mut registry, pool.clone());
    let remember = registry.get("remember").expect("should be registered");
    let forget = registry.get("forget").expect("should be registered");

    let outcome = remember
        .invoke(
            serde_json::json!({"key": "favorite_color", "value": "cyan"}),
            &ctx(user),
        )
        .await;
    assert!(
        matches!(outcome, ToolOutcome::Ok(_)),
        "remembering after opting in should succeed, got {outcome:?}"
    );

    let stored = munibot_core::db::operations::ai::list_memories(&pool, user as i64)
        .await
        .expect("load failed");
    assert_eq!(
        stored.len(),
        1,
        "the memory should really be in the database"
    );

    let outcome = forget
        .invoke(serde_json::json!({"key": "favorite_color"}), &ctx(user))
        .await;
    assert!(
        matches!(outcome, ToolOutcome::Ok(_)),
        "forgetting should succeed, got {outcome:?}"
    );

    let stored = munibot_core::db::operations::ai::list_memories(&pool, user as i64)
        .await
        .expect("load failed");
    assert!(stored.is_empty(), "the memory should really be gone");
}
