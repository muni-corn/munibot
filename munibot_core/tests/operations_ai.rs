//! Integration tests for `db::operations::ai`.
//!
//! Each test gets its own isolated MySQL database via `TestDb`. MySQL must be
//! running with the devenv credentials before running these tests.

mod common;

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::Utc;
use common::TestDb;
use diesel_async::RunQueryDsl;
use munibot_core::db::{DbPool, models::NewAiConversation, operations::ai};

/// Creates a user row, since `ai_conversations.owner_user_id` is a real
/// foreign key and a web conversation cannot be created without one.
///
/// Goes through the linked-account path because that is the only user-creating
/// operation this crate exposes; the oauth fields are placeholders that no
/// assertion here depends on.
async fn a_user(pool: &DbPool) -> i64 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);

    munibot_core::db::operations::get_or_create_user_from_linked_account(
        pool,
        "discord",
        &format!("snowflake-{n}"),
        &format!("muni{n}"),
        &format!("muni{n}"),
        None,
        "unused-token",
        None,
        None,
    )
    .await
    .expect("couldn't create user")
    .id
}

/// Deletes a user directly, to exercise the migration's `ON DELETE CASCADE`.
/// There is no `delete_user` operation, and this test does not need one to
/// exist in production code.
async fn delete_user(pool: &DbPool, user_id: i64) {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    // linked_accounts.user_id has no ON DELETE CASCADE of its own, so it has to
    // go first. Worth knowing before milestone 2 phase 10 promises that deleting
    // a user erases their memories automatically -- that promise needs this
    // foreign key fixed, or an explicit ordered delete like this one.
    diesel::sql_query(format!(
        "DELETE FROM linked_accounts WHERE user_id = {user_id}"
    ))
    .execute(&mut conn)
    .await
    .expect("couldn't delete linked accounts");
    diesel::sql_query(format!("DELETE FROM users WHERE id = {user_id}"))
        .execute(&mut conn)
        .await
        .expect("couldn't delete user");
}

/// Builds the JSON a text message's content column holds. `ai_messages.content`
/// is a JSON column, so mariadb rejects anything that is not valid JSON -- a
/// bug that wrote plain text would fail loudly here rather than silently
/// corrupting a conversation's history.
fn text_content(text: &str) -> String {
    format!(r#"[{{"type":"text","text":"{text}"}}]"#)
}

fn web_conversation(owner: Option<i64>, scope_key: &str) -> NewAiConversation {
    let now = Utc::now().naive_utc();
    NewAiConversation {
        platform: "web".to_string(),
        scope_key: scope_key.to_string(),
        persona_id: "companion".to_string(),
        owner_user_id: owner,
        title: None,
        created_at: now,
        last_active_at: now,
    }
}

#[tokio::test]
async fn test_create_and_get_conversation_by_scope() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    let created = ai::create_conversation(&db.pool, web_conversation(Some(user), "c1"))
        .await
        .expect("create failed");

    let found = ai::get_conversation_by_scope(&db.pool, "web", "c1")
        .await
        .expect("lookup failed")
        .expect("should exist");
    assert_eq!(found.id, created.id);
    assert_eq!(found.persona_id, "companion");
    assert_eq!(found.owner_user_id, Some(user));
}

#[tokio::test]
async fn test_get_conversation_by_scope_missing_returns_none() {
    let db = TestDb::new().await;
    let found = ai::get_conversation_by_scope(&db.pool, "web", "nope")
        .await
        .expect("lookup failed");
    assert!(found.is_none());
}

#[tokio::test]
async fn test_get_or_create_is_idempotent_per_scope() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    let first = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", Some(user))
        .await
        .expect("first failed");
    let second = ai::get_or_create_conversation(&db.pool, "web", "c1", "researcher", Some(user))
        .await
        .expect("second failed");

    assert_eq!(
        first.id, second.id,
        "the same scope must resolve to one row"
    );
    assert_eq!(
        second.persona_id, "companion",
        "an existing conversation keeps its stored persona rather than being overwritten by a load"
    );
}

#[tokio::test]
async fn test_the_same_scope_key_on_two_platforms_is_two_conversations() {
    let db = TestDb::new().await;

    let web = ai::get_or_create_conversation(&db.pool, "web", "shared", "companion", None)
        .await
        .expect("web failed");
    let discord = ai::get_or_create_conversation(&db.pool, "discord", "shared", "companion", None)
        .await
        .expect("discord failed");

    assert_ne!(web.id, discord.id);
}

#[tokio::test]
async fn test_append_message_assigns_increasing_sequence_numbers() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();

    let first = ai::append_message(&db.pool, conversation.id, "user", "[]", 3)
        .await
        .expect("first append failed");
    let second = ai::append_message(&db.pool, conversation.id, "assistant", "[]", 5)
        .await
        .expect("second append failed");

    assert_eq!(first.seq, 1);
    assert_eq!(second.seq, 2);
}

#[tokio::test]
async fn test_sequence_numbers_are_per_conversation() {
    let db = TestDb::new().await;
    let a = ai::get_or_create_conversation(&db.pool, "web", "a", "companion", None)
        .await
        .unwrap();
    let b = ai::get_or_create_conversation(&db.pool, "web", "b", "companion", None)
        .await
        .unwrap();

    ai::append_message(&db.pool, a.id, "user", "[]", 0)
        .await
        .unwrap();
    let b_first = ai::append_message(&db.pool, b.id, "user", "[]", 0)
        .await
        .unwrap();

    assert_eq!(
        b_first.seq, 1,
        "a second conversation starts its own numbering rather than continuing the first's"
    );
}

#[tokio::test]
async fn test_get_messages_returns_oldest_first() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();

    for content in ["one", "two", "three"] {
        ai::append_message(&db.pool, conversation.id, "user", &text_content(content), 0)
            .await
            .unwrap();
    }

    let messages = ai::get_messages(&db.pool, conversation.id, None)
        .await
        .expect("load failed");
    let contents: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    assert_eq!(contents, vec![
        text_content("one"),
        text_content("two"),
        text_content("three")
    ]);
}

#[tokio::test]
async fn test_get_messages_limit_keeps_the_most_recent_still_oldest_first() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();

    for content in ["one", "two", "three", "four"] {
        ai::append_message(&db.pool, conversation.id, "user", &text_content(content), 0)
            .await
            .unwrap();
    }

    let messages = ai::get_messages(&db.pool, conversation.id, Some(2))
        .await
        .expect("load failed");
    let contents: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    assert_eq!(
        contents,
        vec![text_content("three"), text_content("four")],
        "a limit should return the tail of the conversation, still in oldest-first order"
    );
}

#[tokio::test]
async fn test_appending_bumps_last_active_at() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();
    let before = conversation.last_active_at;

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    ai::append_message(&db.pool, conversation.id, "user", "[]", 0)
        .await
        .unwrap();

    let after = ai::get_conversation(&db.pool, conversation.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        after.last_active_at > before,
        "appending should bump activity, so the sidebar orders by real recency"
    );
}

#[tokio::test]
async fn test_clear_conversation_empties_it_without_deleting_it() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();
    ai::append_message(&db.pool, conversation.id, "user", "[]", 0)
        .await
        .unwrap();
    ai::set_conversation_summary(&db.pool, conversation.id, Some("we talked about cats"), 7)
        .await
        .unwrap();

    ai::clear_conversation(&db.pool, conversation.id)
        .await
        .expect("clear failed");

    let messages = ai::get_messages(&db.pool, conversation.id, None)
        .await
        .unwrap();
    assert!(messages.is_empty());

    let reloaded = ai::get_conversation(&db.pool, conversation.id)
        .await
        .unwrap()
        .expect("the conversation itself must survive a clear");
    assert_eq!(reloaded.summary, None);
    assert_eq!(reloaded.summary_tokens, 0);
}

#[tokio::test]
async fn test_clearing_then_appending_restarts_numbering() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();
    ai::append_message(&db.pool, conversation.id, "user", "[]", 0)
        .await
        .unwrap();
    ai::clear_conversation(&db.pool, conversation.id)
        .await
        .unwrap();

    let fresh = ai::append_message(&db.pool, conversation.id, "user", "[]", 0)
        .await
        .unwrap();
    assert_eq!(fresh.seq, 1);
}

#[tokio::test]
async fn test_set_and_clear_conversation_summary() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();

    ai::set_conversation_summary(&db.pool, conversation.id, Some("a summary"), 4)
        .await
        .unwrap();
    let with_summary = ai::get_conversation(&db.pool, conversation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(with_summary.summary.as_deref(), Some("a summary"));
    assert_eq!(with_summary.summary_tokens, 4);

    ai::set_conversation_summary(&db.pool, conversation.id, None, 0)
        .await
        .unwrap();
    let without = ai::get_conversation(&db.pool, conversation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(without.summary, None);
}

#[tokio::test]
async fn test_list_conversations_for_user_is_most_recent_first() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    let older = ai::create_conversation(&db.pool, web_conversation(Some(user), "older"))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let newer = ai::create_conversation(&db.pool, web_conversation(Some(user), "newer"))
        .await
        .unwrap();

    let listed = ai::list_conversations_for_user(&db.pool, user)
        .await
        .expect("list failed");
    let ids: Vec<i64> = listed.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![newer.id, older.id]);
}

#[tokio::test]
async fn test_list_conversations_excludes_other_users() {
    let db = TestDb::new().await;
    let mine = a_user(&db.pool).await;
    let theirs = a_user(&db.pool).await;

    ai::create_conversation(&db.pool, web_conversation(Some(mine), "mine"))
        .await
        .unwrap();
    ai::create_conversation(&db.pool, web_conversation(Some(theirs), "theirs"))
        .await
        .unwrap();

    let listed = ai::list_conversations_for_user(&db.pool, mine)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].scope_key, "mine");
}

#[tokio::test]
async fn test_list_conversations_excludes_channel_scoped_ones() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    ai::create_conversation(&db.pool, web_conversation(Some(user), "mine"))
        .await
        .unwrap();
    // a discord channel's conversation has no owner, and must never appear in
    // anyone's personal sidebar
    ai::get_or_create_conversation(&db.pool, "discord", "channel-1", "companion", None)
        .await
        .unwrap();

    let listed = ai::list_conversations_for_user(&db.pool, user)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
}

#[tokio::test]
async fn test_rename_conversation() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    let conversation = ai::create_conversation(&db.pool, web_conversation(Some(user), "c1"))
        .await
        .unwrap();

    ai::rename_conversation(&db.pool, conversation.id, "about cats")
        .await
        .expect("rename failed");

    let reloaded = ai::get_conversation(&db.pool, conversation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.title.as_deref(), Some("about cats"));
}

#[tokio::test]
async fn test_archived_conversations_drop_out_of_the_listing() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    let conversation = ai::create_conversation(&db.pool, web_conversation(Some(user), "c1"))
        .await
        .unwrap();

    ai::archive_conversation(&db.pool, conversation.id)
        .await
        .expect("archive failed");

    let listed = ai::list_conversations_for_user(&db.pool, user)
        .await
        .unwrap();
    assert!(listed.is_empty());

    let still_there = ai::get_conversation(&db.pool, conversation.id)
        .await
        .unwrap();
    assert!(
        still_there.is_some(),
        "archiving hides a conversation, it does not delete it"
    );
}

#[tokio::test]
async fn test_deleting_a_user_cascades_to_their_conversations_and_messages() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    let conversation = ai::create_conversation(&db.pool, web_conversation(Some(user), "c1"))
        .await
        .unwrap();
    ai::append_message(&db.pool, conversation.id, "user", "[]", 0)
        .await
        .unwrap();

    delete_user(&db.pool, user).await;

    assert!(
        ai::get_conversation(&db.pool, conversation.id)
            .await
            .unwrap()
            .is_none(),
        "deleting a user must take their conversations with them, which is the whole point of the \
         ON DELETE CASCADE in the migration"
    );
    assert!(
        ai::get_messages(&db.pool, conversation.id, None)
            .await
            .unwrap()
            .is_empty()
    );
}
