//! Integration tests for [`DieselSessionStore`] against a real database.
//!
//! These are the only tests in this crate that touch a database, and they are
//! **skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set**, so
//! `cargo test` stays green and offline on a machine with no MySQL running.
//! Every other test in `munibot_ai` uses `InMemorySessionStore` and needs
//! nothing external.
//!
//! Run them with, for example:
//!
//! ```text
//! MUNIBOT_AI_TEST_DATABASE_URL="mysql://munibot:...@127.0.0.1:3307/munibot" \
//!   cargo test -p munibot_ai --test diesel_session_store
//! ```
//!
//! The target database must already have the workspace migrations applied.
//! Tests share it and isolate themselves with unique scope keys rather than
//! creating a database each, which keeps this file free of migration and
//! database-management machinery.

use std::sync::atomic::{AtomicU32, Ordering};

use munibot_ai::{
    memory::{ConversationScope, DieselSessionStore, SessionStore},
    tools::Platform,
    types::{ContentBlock, Message, Role},
};
use munibot_core::db::{DbPool, establish_pool};

/// Connects to the test database, or returns `None` when none is configured.
async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    // establish_pool reads DATABASE_URL specifically, so mirror it across rather
    // than duplicating its pool construction
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

/// A scope key no other test run will collide with.
fn unique_scope() -> ConversationScope {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    ConversationScope::new(Platform::Web, format!("test-{nanos}-{n}"))
}

/// Skips the test body when no database is configured.
macro_rules! store {
    () => {
        match pool().await {
            Some(pool) => DieselSessionStore::new(pool),
            None => return,
        }
    };
}

#[tokio::test]
async fn test_load_or_create_is_idempotent_and_survives_a_second_store() {
    let store = store!();
    let scope = unique_scope();

    let first = store.load_or_create(&scope, "companion").await.unwrap();
    let second = store.load_or_create(&scope, "companion").await.unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.persona_id, "companion");
    assert_eq!(second.scope, scope);
}

#[tokio::test]
async fn test_appended_messages_come_back_in_order() {
    let store = store!();
    let scope = unique_scope();
    let conversation = store.load_or_create(&scope, "companion").await.unwrap();

    store
        .append(conversation.id, Message::user("first"))
        .await
        .unwrap();
    store
        .append(conversation.id, Message::assistant("second"))
        .await
        .unwrap();

    let history = store.history(conversation.id, None).await.unwrap();
    let texts: Vec<String> = history.iter().map(Message::text).collect();
    assert_eq!(texts, vec!["first".to_string(), "second".to_string()]);
    assert_eq!(history.iter().next().unwrap().role, Role::User);
}

#[tokio::test]
async fn test_a_tool_call_round_trips_through_the_database() {
    let store = store!();
    let scope = unique_scope();
    let conversation = store.load_or_create(&scope, "companion").await.unwrap();

    // the reason content is a JSON column rather than text: a tool call and its
    // result have to survive a restart intact, not be flattened to prose
    let call = Message::new(Role::Assistant, vec![ContentBlock::tool_use(
        "c1",
        "web_search",
        serde_json::json!({"query": "cats"}),
    )]);
    let result = Message::tool_results(vec![ContentBlock::tool_result("c1", "found some cats")]);

    store.append(conversation.id, call.clone()).await.unwrap();
    store.append(conversation.id, result.clone()).await.unwrap();

    let history = store.history(conversation.id, None).await.unwrap();
    let messages: Vec<&Message> = history.iter().collect();
    assert_eq!(messages[0], &call, "the tool call must survive verbatim");
    assert_eq!(messages[1], &result, "so must its result");
}

#[tokio::test]
async fn test_history_limit_returns_the_most_recent_still_oldest_first() {
    let store = store!();
    let scope = unique_scope();
    let conversation = store.load_or_create(&scope, "companion").await.unwrap();

    for text in ["one", "two", "three"] {
        store
            .append(conversation.id, Message::user(text))
            .await
            .unwrap();
    }

    let history = store.history(conversation.id, Some(2)).await.unwrap();
    let texts: Vec<String> = history.iter().map(Message::text).collect();
    assert_eq!(texts, vec!["two".to_string(), "three".to_string()]);
}

#[tokio::test]
async fn test_summary_is_stored_and_reloaded() {
    let store = store!();
    let scope = unique_scope();
    let conversation = store.load_or_create(&scope, "companion").await.unwrap();

    store
        .set_summary(conversation.id, "we talked about cats".to_string())
        .await
        .unwrap();

    let reloaded = store.load_or_create(&scope, "companion").await.unwrap();
    assert_eq!(reloaded.summary.as_deref(), Some("we talked about cats"));
}

#[tokio::test]
async fn test_a_new_conversation_has_no_title_until_one_is_set() {
    let store = store!();
    let scope = unique_scope();
    let conversation = store.load_or_create(&scope, "companion").await.unwrap();
    assert_eq!(conversation.title, None);
}

#[tokio::test]
async fn test_title_is_stored_and_reloaded() {
    let store = store!();
    let scope = unique_scope();
    let conversation = store.load_or_create(&scope, "companion").await.unwrap();

    store
        .set_title(conversation.id, "weekend plans".to_string())
        .await
        .unwrap();

    let reloaded = store.load_or_create(&scope, "companion").await.unwrap();
    assert_eq!(reloaded.title.as_deref(), Some("weekend plans"));
}

#[tokio::test]
async fn test_clear_empties_history_but_keeps_the_conversation() {
    let store = store!();
    let scope = unique_scope();
    let conversation = store.load_or_create(&scope, "companion").await.unwrap();
    store
        .append(conversation.id, Message::user("hello"))
        .await
        .unwrap();
    store
        .set_summary(conversation.id, "a summary".to_string())
        .await
        .unwrap();

    store.clear(conversation.id).await.unwrap();

    assert!(
        store
            .history(conversation.id, None)
            .await
            .unwrap()
            .is_empty()
    );

    let reloaded = store.load_or_create(&scope, "companion").await.unwrap();
    assert_eq!(
        reloaded.id, conversation.id,
        "a reset conversation keeps its id, so the same scope still resolves to it"
    );
    assert_eq!(reloaded.summary, None);
}

#[tokio::test]
async fn test_history_survives_a_fresh_store_over_the_same_database() {
    let Some(pool) = pool().await else { return };
    let scope = unique_scope();

    let conversation = {
        let store = DieselSessionStore::new(pool.clone());
        let conversation = store.load_or_create(&scope, "companion").await.unwrap();
        store
            .append(conversation.id, Message::user("remember this"))
            .await
            .unwrap();
        conversation
    };

    // a second store standing in for a restarted process
    let store = DieselSessionStore::new(pool);
    let history = store.history(conversation.id, None).await.unwrap();
    let texts: Vec<String> = history.iter().map(Message::text).collect();
    assert_eq!(
        texts,
        vec!["remember this".to_string()],
        "this is the whole point of the phase: a conversation outlives the process"
    );
}

// --- conversation directory ---
//
// These need a real user row, because ai_conversations.owner_user_id is a
// foreign key. They go through the linked-account path, which is the only
// user-creating operation munibot_core exposes.

use munibot_ai::memory::ConversationDirectory;

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
        &format!("ai-test-{nanos}-{n}"),
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
async fn test_created_conversations_appear_in_the_owners_listing() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let store = DieselSessionStore::new(pool);

    let created = store.create_for_user(user, "companion").await.unwrap();

    let listed = store.list_for_user(user).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert_eq!(listed[0].persona_id, "companion");
    assert!(listed[0].title.is_none());
}

#[tokio::test]
async fn test_a_listing_never_shows_another_persons_conversations() {
    let Some(pool) = pool().await else { return };
    let mine = a_user(&pool).await;
    let theirs = a_user(&pool).await;
    let store = DieselSessionStore::new(pool);

    store.create_for_user(mine, "companion").await.unwrap();
    store.create_for_user(theirs, "companion").await.unwrap();

    let listed = store.list_for_user(mine).await.unwrap();
    assert_eq!(
        listed.len(),
        1,
        "ownership is the whole point of this listing"
    );
}

#[tokio::test]
async fn test_channel_scoped_conversations_never_appear_in_a_listing() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let store = DieselSessionStore::new(pool);
    store.create_for_user(user, "companion").await.unwrap();

    // a discord channel's conversation belongs to a place, not a person
    store
        .load_or_create(&unique_scope(), "companion")
        .await
        .unwrap();

    assert_eq!(store.list_for_user(user).await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_two_new_conversations_get_distinct_scope_keys() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let store = DieselSessionStore::new(pool);

    let first = store.create_for_user(user, "companion").await.unwrap();
    let second = store.create_for_user(user, "companion").await.unwrap();

    assert_ne!(first.id, second.id);
    assert_ne!(
        first.scope.scope_key, second.scope.scope_key,
        "a generated scope key must not collide, or two conversations become one"
    );
}

#[tokio::test]
async fn test_renaming_shows_up_in_the_listing() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let store = DieselSessionStore::new(pool);
    let conversation = store.create_for_user(user, "companion").await.unwrap();

    store.rename(conversation.id, "about cats").await.unwrap();

    let listed = store.list_for_user(user).await.unwrap();
    assert_eq!(listed[0].title.as_deref(), Some("about cats"));
}

#[tokio::test]
async fn test_archiving_hides_a_conversation_without_losing_its_history() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let store = DieselSessionStore::new(pool);
    let conversation = store.create_for_user(user, "companion").await.unwrap();
    store
        .append(conversation.id, Message::user("hello"))
        .await
        .unwrap();

    store.archive(conversation.id).await.unwrap();

    assert!(store.list_for_user(user).await.unwrap().is_empty());
    assert_eq!(
        store.history(conversation.id, None).await.unwrap().len(),
        1,
        "archiving hides a conversation; it must not destroy what was said in it"
    );
}

#[tokio::test]
async fn test_listing_is_most_recently_active_first() {
    let Some(pool) = pool().await else { return };
    let user = a_user(&pool).await;
    let store = DieselSessionStore::new(pool);

    let older = store.create_for_user(user, "companion").await.unwrap();
    let newer = store.create_for_user(user, "companion").await.unwrap();

    // appending bumps last_active_at, so the older conversation should overtake
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    store
        .append(older.id, Message::user("still talking"))
        .await
        .unwrap();

    let listed = store.list_for_user(user).await.unwrap();
    let ids: Vec<_> = listed.iter().map(|e| e.id).collect();
    assert_eq!(
        ids,
        vec![older.id, newer.id],
        "a sidebar orders by real recency, not creation order"
    );
}

#[tokio::test]
async fn test_diesel_store_compact_replaces_old_messages_with_a_summary() {
    let store = store!();
    let scope = unique_scope();
    let conversation = store.load_or_create(&scope, "companion").await.unwrap();

    for text in ["one", "two", "three", "four"] {
        store
            .append(conversation.id, Message::user(text))
            .await
            .unwrap();
    }

    store
        .compact(conversation.id, 2, "one and two happened".to_string())
        .await
        .expect("compact failed");

    let history = store.history(conversation.id, None).await.unwrap();
    let texts: Vec<String> = history.iter().map(Message::text).collect();
    assert_eq!(texts, vec!["three".to_string(), "four".to_string()]);

    let reloaded = store.load_or_create(&scope, "companion").await.unwrap();
    assert_eq!(reloaded.summary.as_deref(), Some("one and two happened"));
}
