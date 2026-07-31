//! Integration tests for [`DieselMemoryOptIn`] and the full
//! [`GatedMemoryStore`] stack against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_session_store.rs` - see that file's module doc for the full
//! rationale.

use std::sync::atomic::{AtomicU32, Ordering};

use munibot_ai::memory::{
    DieselMemoryOptIn, DieselMemoryStore, GatedMemoryStore, MemoryOptIn, MemoryStore,
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
        &format!("opt-in-test-{nanos}-{n}"),
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

#[tokio::test]
async fn test_a_user_who_never_touched_the_setting_is_not_opted_in() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let opt_in = DieselMemoryOptIn::new(pool);

    assert!(!opt_in.is_opted_in(user).await.expect("should succeed"));
}

#[tokio::test]
async fn test_setting_opted_in_is_reflected_back() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let opt_in = DieselMemoryOptIn::new(pool);

    opt_in.set_opted_in(user, true).await.expect("set failed");
    assert!(opt_in.is_opted_in(user).await.unwrap());

    opt_in.set_opted_in(user, false).await.expect("set failed");
    assert!(!opt_in.is_opted_in(user).await.unwrap());
}

#[tokio::test]
async fn test_the_full_gated_stack_refuses_recording_until_opted_in() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    // a separate handle onto the same opt-in setting, standing in for whatever
    // toggles it in production (a settings server function, in milestone 2's
    // web panel) - the gated store itself has no way to flip its own gate
    let opt_in_control = DieselMemoryOptIn::new(pool.clone());
    let store = GatedMemoryStore::new(
        DieselMemoryStore::new(pool.clone()),
        DieselMemoryOptIn::new(pool),
    );

    let before = store.record(user, "favorite_color", "purple").await;
    assert!(
        before.is_err(),
        "recording before opting in should be refused"
    );

    opt_in_control.set_opted_in(user, true).await.unwrap();

    store
        .record(user, "favorite_color", "purple")
        .await
        .expect("recording after opting in should succeed");
    let memories = store.list(user).await.unwrap();
    assert_eq!(memories.len(), 1);
}

#[tokio::test]
async fn test_wipe_through_the_gated_stack_works_even_when_not_opted_in() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let opt_in_control = DieselMemoryOptIn::new(pool.clone());
    let store = GatedMemoryStore::new(
        DieselMemoryStore::new(pool.clone()),
        DieselMemoryOptIn::new(pool),
    );

    opt_in_control.set_opted_in(user, true).await.unwrap();
    store.record(user, "a", "1").await.unwrap();
    opt_in_control.set_opted_in(user, false).await.unwrap();

    // opted back out, but leftover memories from before must still be
    // deletable without opting back in first
    store
        .wipe(user)
        .await
        .expect("wipe must work regardless of opt-in status");

    // list alone would return empty while opted out regardless of whether wipe
    // did anything real, since the gate hides everything either way - opt back
    // in to prove the underlying row is actually gone, not just hidden
    opt_in_control.set_opted_in(user, true).await.unwrap();
    assert!(
        store.list(user).await.unwrap().is_empty(),
        "the memory should be truly deleted, not merely hidden by the gate"
    );
}
