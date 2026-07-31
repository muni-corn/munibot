//! Integration tests for [`DieselMemoryStore`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_session_store.rs` - see that file's module doc for the full
//! rationale.

use std::sync::atomic::{AtomicU32, Ordering};

use munibot_ai::memory::{DieselMemoryStore, MemoryStore};
use munibot_core::db::{DbPool, establish_pool};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

/// Creates a user row, since `ai_memories.user_id` is a real foreign key.
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
        &format!("memory-test-{nanos}-{n}"),
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

/// Skips the test when no database is configured, otherwise returns a user
/// created against `pool` and a [`DieselMemoryStore`] over a clone of it.
macro_rules! store_and_user {
    () => {{
        let Some(pool) = pool().await else { return };
        let user = a_user(&pool).await;
        (DieselMemoryStore::new(pool), user)
    }};
}

#[tokio::test]
async fn test_a_recorded_memory_appears_in_the_list() {
    let (store, user) = store_and_user!();
    store
        .record(user, "favorite_color", "purple")
        .await
        .expect("record failed");

    let memories = store.list(user).await.expect("list failed");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].key, "favorite_color");
    assert_eq!(memories[0].value, "purple");
}

#[tokio::test]
async fn test_recording_the_same_key_again_replaces_the_value() {
    let (store, user) = store_and_user!();
    store
        .record(user, "favorite_color", "purple")
        .await
        .unwrap();
    store.record(user, "favorite_color", "green").await.unwrap();

    let memories = store.list(user).await.unwrap();
    assert_eq!(
        memories.len(),
        1,
        "an update must not create a second memory"
    );
    assert_eq!(memories[0].value, "green");
}

#[tokio::test]
async fn test_forget_removes_only_the_named_key() {
    let (store, user) = store_and_user!();
    store.record(user, "a", "1").await.unwrap();
    store.record(user, "b", "2").await.unwrap();

    store.forget(user, "a").await.expect("forget failed");

    let memories = store.list(user).await.unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].key, "b");
}

#[tokio::test]
async fn test_forgetting_a_memory_that_never_existed_is_not_an_error() {
    let (store, user) = store_and_user!();
    store
        .forget(user, "never-existed")
        .await
        .expect("forgetting a nonexistent memory should not error");
}

#[tokio::test]
async fn test_wipe_removes_everything_for_that_user_only() {
    let Some(pool) = pool().await else { return };
    let alice = a_user(&pool).await;
    let bob = a_user(&pool).await;
    let store = DieselMemoryStore::new(pool);

    store.record(alice, "a", "1").await.unwrap();
    store.record(bob, "b", "2").await.unwrap();

    store.wipe(alice).await.expect("wipe failed");

    assert!(store.list(alice).await.unwrap().is_empty());
    assert_eq!(
        store.list(bob).await.unwrap().len(),
        1,
        "wiping one user's memories must never touch another's"
    );
}

#[tokio::test]
async fn test_recording_past_the_cap_refuses_a_genuinely_new_key() {
    let (store, user) = store_and_user!();

    for n in 0..100 {
        store
            .record(user, &format!("key-{n}"), "value")
            .await
            .unwrap_or_else(|error| panic!("record {n} should have succeeded: {error}"));
    }

    let result = store.record(user, "key-100", "one too many").await;
    assert!(result.is_err(), "the 101st distinct key should be refused");
}

#[tokio::test]
async fn test_updating_an_existing_key_is_never_capped() {
    let (store, user) = store_and_user!();

    for n in 0..100 {
        store
            .record(user, &format!("key-{n}"), "value")
            .await
            .unwrap();
    }

    // updating one of the 100 already-recorded keys must still work, since it
    // does not grow the total count past the cap
    store
        .record(user, "key-0", "updated value")
        .await
        .expect("updating an existing key must never be capped");
}
