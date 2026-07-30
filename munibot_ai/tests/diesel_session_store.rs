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
