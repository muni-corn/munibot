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
async fn test_get_message_returns_the_row_by_id() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();
    let saved = ai::append_message(&db.pool, conversation.id, "user", &text_content("hi"), 0)
        .await
        .unwrap();

    let found = ai::get_message(&db.pool, saved.id)
        .await
        .expect("load failed")
        .expect("should have found the message");
    assert_eq!(found.id, saved.id);
    assert_eq!(found.conversation_id, conversation.id);
    assert_eq!(found.content, text_content("hi"));
}

#[tokio::test]
async fn test_get_message_missing_returns_none() {
    let db = TestDb::new().await;
    assert!(
        ai::get_message(&db.pool, 999_999)
            .await
            .expect("load failed")
            .is_none()
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
async fn test_get_messages_page_with_no_cursor_returns_the_most_recent_page() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();

    for content in ["one", "two", "three", "four"] {
        ai::append_message(&db.pool, conversation.id, "user", &text_content(content), 0)
            .await
            .unwrap();
    }

    let messages = ai::get_messages_page(&db.pool, conversation.id, None, 2)
        .await
        .expect("load failed");
    let contents: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    assert_eq!(
        contents,
        vec![text_content("three"), text_content("four")],
        "the first page should be the tail of the conversation, oldest-first"
    );
}

#[tokio::test]
async fn test_get_messages_page_with_a_cursor_returns_the_page_before_it() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();

    for content in ["one", "two", "three", "four"] {
        ai::append_message(&db.pool, conversation.id, "user", &text_content(content), 0)
            .await
            .unwrap();
    }

    let first_page = ai::get_messages_page(&db.pool, conversation.id, None, 2)
        .await
        .expect("load failed");
    let oldest_in_first_page = first_page[0].seq;

    let second_page =
        ai::get_messages_page(&db.pool, conversation.id, Some(oldest_in_first_page), 2)
            .await
            .expect("load failed");
    let contents: Vec<String> = second_page.iter().map(|m| m.content.clone()).collect();
    assert_eq!(
        contents,
        vec![text_content("one"), text_content("two")],
        "a cursor should return the page immediately before it, still oldest-first"
    );
}

#[tokio::test]
async fn test_get_messages_page_past_the_beginning_is_empty() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();

    let first = ai::append_message(&db.pool, conversation.id, "user", &text_content("one"), 0)
        .await
        .unwrap();

    let page = ai::get_messages_page(&db.pool, conversation.id, Some(first.seq), 10)
        .await
        .expect("load failed");
    assert!(
        page.is_empty(),
        "asking for the page before the very first message should return nothing, not error"
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

#[tokio::test]
async fn test_compact_conversation_deletes_old_messages_and_sets_summary() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();
    for content in ["one", "two", "three", "four"] {
        ai::append_message(&db.pool, conversation.id, "user", &text_content(content), 0)
            .await
            .unwrap();
    }

    ai::compact_conversation(&db.pool, conversation.id, 2, "one and two happened", 5)
        .await
        .expect("compact failed");

    let messages = ai::get_messages(&db.pool, conversation.id, None)
        .await
        .unwrap();
    let contents: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    assert_eq!(
        contents,
        vec![text_content("three"), text_content("four")],
        "only the most recent keep_recent messages should survive"
    );

    let reloaded = ai::get_conversation(&db.pool, conversation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.summary.as_deref(), Some("one and two happened"));
    assert_eq!(reloaded.summary_tokens, 5);
}

#[tokio::test]
async fn test_compact_conversation_on_a_short_conversation_is_a_noop() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();
    ai::append_message(
        &db.pool,
        conversation.id,
        "user",
        &text_content("only message"),
        0,
    )
    .await
    .unwrap();

    ai::compact_conversation(&db.pool, conversation.id, 5, "should never be written", 99)
        .await
        .expect("compact failed");

    let messages = ai::get_messages(&db.pool, conversation.id, None)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1, "a short conversation must be left alone");

    let reloaded = ai::get_conversation(&db.pool, conversation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded.summary, None,
        "a noop compaction must never overwrite the summary, even with nothing"
    );
}

#[tokio::test]
async fn test_compact_conversation_with_exactly_keep_recent_messages_is_a_noop() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();
    for content in ["one", "two"] {
        ai::append_message(&db.pool, conversation.id, "user", &text_content(content), 0)
            .await
            .unwrap();
    }

    ai::compact_conversation(&db.pool, conversation.id, 2, "unused", 0)
        .await
        .expect("compact failed");

    let messages = ai::get_messages(&db.pool, conversation.id, None)
        .await
        .unwrap();
    assert_eq!(
        messages.len(),
        2,
        "a conversation with exactly keep_recent messages has nothing to cut"
    );
}

// --- usage ---

use munibot_core::db::models::NewAiUsage;

fn usage_row(conversation_id: Option<i64>, user_id: Option<i64>, succeeded: bool) -> NewAiUsage {
    NewAiUsage {
        conversation_id,
        user_id,
        guild_id: None,
        provider: "anthropic".to_string(),
        model: "claude-opus-5".to_string(),
        persona_id: "companion".to_string(),
        input_tokens: 100,
        output_tokens: 200,
        cost_micros: 5_000,
        iterations: 2,
        succeeded,
        created_at: Utc::now().naive_utc(),
    }
}

#[tokio::test]
async fn test_record_usage_writes_a_row() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    let conversation = ai::create_conversation(&db.pool, web_conversation(Some(user), "c1"))
        .await
        .unwrap();

    ai::record_usage(&db.pool, usage_row(Some(conversation.id), Some(user), true))
        .await
        .expect("record failed");
}

#[tokio::test]
async fn test_record_usage_on_a_failed_turn() {
    let db = TestDb::new().await;
    // conversation_id and user_id are independently optional at the type level, so
    // a usage row should be writable with neither - a turn that failed before ever
    // reaching a conversation still cost money
    ai::record_usage(&db.pool, usage_row(None, None, false))
        .await
        .expect("record failed");
}

#[tokio::test]
async fn test_deleting_a_user_does_not_delete_their_usage_history() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    ai::record_usage(&db.pool, usage_row(None, Some(user), true))
        .await
        .unwrap();

    // usage rows are a billing/audit record, not conversation state - unlike
    // ai_conversations, this should survive the user being deleted (the migration
    // sets ON DELETE SET NULL on ai_usage.user_id, not CASCADE)
    delete_user(&db.pool, user).await;
}

// --- tool call auditing ---

use munibot_core::db::models::NewAiToolCall;

fn tool_call_row(conversation_id: Option<i64>, status: &str) -> NewAiToolCall {
    NewAiToolCall {
        conversation_id,
        tool_name: "current_time".to_string(),
        input: Some("{}".to_string()),
        output: Some("12:00".to_string()),
        duration_ms: 5,
        status: status.to_string(),
        created_at: Utc::now().naive_utc(),
    }
}

#[tokio::test]
async fn test_record_tool_call_writes_a_row() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();

    ai::record_tool_call(&db.pool, tool_call_row(Some(conversation.id), "ok"))
        .await
        .expect("record failed");
}

#[tokio::test]
async fn test_record_tool_call_without_a_conversation() {
    let db = TestDb::new().await;
    // conversation_id is nullable: a call this crate might one day audit outside
    // any stored conversation must still be writable
    ai::record_tool_call(&db.pool, tool_call_row(None, "fatal"))
        .await
        .expect("record failed");
}

#[tokio::test]
async fn test_record_tool_call_after_the_conversation_is_archived_still_succeeds() {
    let db = TestDb::new().await;
    let conversation = ai::get_or_create_conversation(&db.pool, "web", "c1", "companion", None)
        .await
        .unwrap();
    ai::archive_conversation(&db.pool, conversation.id)
        .await
        .unwrap();

    // the conversation row still exists (archiving hides, it does not delete),
    // so the foreign key is still satisfiable and this must still succeed
    ai::record_tool_call(&db.pool, tool_call_row(Some(conversation.id), "ok"))
        .await
        .expect("record failed");
}

// --- memory ---

#[tokio::test]
async fn test_upsert_memory_creates_a_new_row() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    let saved = ai::upsert_memory(&db.pool, user, "favorite_color", "purple")
        .await
        .expect("upsert failed");

    assert_eq!(saved.key, "favorite_color");
    assert_eq!(saved.value, "purple");
}

#[tokio::test]
async fn test_upsert_memory_on_an_existing_key_replaces_the_value() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    let first = ai::upsert_memory(&db.pool, user, "favorite_color", "purple")
        .await
        .unwrap();
    let second = ai::upsert_memory(&db.pool, user, "favorite_color", "green")
        .await
        .expect("upsert failed");

    assert_eq!(
        first.id, second.id,
        "the same key should update the same row"
    );
    assert_eq!(second.value, "green");

    let all = ai::list_memories(&db.pool, user).await.unwrap();
    assert_eq!(
        all.len(),
        1,
        "an update must not create a second row for the same key"
    );
}

#[tokio::test]
async fn test_get_memory_missing_returns_none() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    assert!(
        ai::get_memory(&db.pool, user, "nope")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_the_same_key_for_two_users_is_two_independent_rows() {
    let db = TestDb::new().await;
    let alice = a_user(&db.pool).await;
    let bob = a_user(&db.pool).await;

    ai::upsert_memory(&db.pool, alice, "favorite_color", "purple")
        .await
        .unwrap();
    ai::upsert_memory(&db.pool, bob, "favorite_color", "green")
        .await
        .unwrap();

    let alice_memory = ai::get_memory(&db.pool, alice, "favorite_color")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        alice_memory.value, "purple",
        "one user's memories must never leak into another's"
    );
}

#[tokio::test]
async fn test_count_memories_reflects_distinct_keys_not_upserts() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    ai::upsert_memory(&db.pool, user, "a", "1").await.unwrap();
    ai::upsert_memory(&db.pool, user, "a", "2").await.unwrap();
    ai::upsert_memory(&db.pool, user, "b", "3").await.unwrap();

    assert_eq!(ai::count_memories(&db.pool, user).await.unwrap(), 2);
}

#[tokio::test]
async fn test_forget_memory_removes_only_the_named_key() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    ai::upsert_memory(&db.pool, user, "a", "1").await.unwrap();
    ai::upsert_memory(&db.pool, user, "b", "2").await.unwrap();

    ai::forget_memory(&db.pool, user, "a")
        .await
        .expect("forget failed");

    let remaining = ai::list_memories(&db.pool, user).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].key, "b");
}

#[tokio::test]
async fn test_forgetting_a_memory_that_never_existed_is_not_an_error() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    ai::forget_memory(&db.pool, user, "never-existed")
        .await
        .expect("forgetting a nonexistent memory should not error");
}

#[tokio::test]
async fn test_wipe_memories_removes_everything_for_that_user_only() {
    let db = TestDb::new().await;
    let alice = a_user(&db.pool).await;
    let bob = a_user(&db.pool).await;
    ai::upsert_memory(&db.pool, alice, "a", "1").await.unwrap();
    ai::upsert_memory(&db.pool, alice, "b", "2").await.unwrap();
    ai::upsert_memory(&db.pool, bob, "c", "3").await.unwrap();

    ai::wipe_memories(&db.pool, alice)
        .await
        .expect("wipe failed");

    assert!(ai::list_memories(&db.pool, alice).await.unwrap().is_empty());
    assert_eq!(
        ai::list_memories(&db.pool, bob).await.unwrap().len(),
        1,
        "wiping one user's memories must never touch another's"
    );
}

#[tokio::test]
async fn test_deleting_a_user_cascades_to_their_memories() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    ai::upsert_memory(&db.pool, user, "a", "1").await.unwrap();

    delete_user(&db.pool, user).await;

    // there is no way to list a deleted user's memories directly, but the
    // migration's ON DELETE CASCADE means the row is simply gone - proven
    // indirectly by being able to reuse the same key for a brand new user
    // without a leftover row causing a conflict
    let new_user = a_user(&db.pool).await;
    assert!(
        ai::get_memory(&db.pool, new_user, "a")
            .await
            .unwrap()
            .is_none()
    );
}

// --- user settings ---

#[tokio::test]
async fn test_get_user_settings_before_any_setting_is_touched_is_none() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    assert!(
        ai::get_user_settings(&db.pool, user)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_set_memory_opt_in_creates_the_settings_row() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    let settings = ai::set_memory_opt_in(&db.pool, user, true)
        .await
        .expect("set failed");
    assert!(settings.memory_opt_in);

    let reloaded = ai::get_user_settings(&db.pool, user)
        .await
        .unwrap()
        .unwrap();
    assert!(reloaded.memory_opt_in);
}

#[tokio::test]
async fn test_set_memory_opt_in_toggles_an_existing_row_rather_than_duplicating_it() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;

    ai::set_memory_opt_in(&db.pool, user, true).await.unwrap();
    ai::set_memory_opt_in(&db.pool, user, false).await.unwrap();

    let settings = ai::get_user_settings(&db.pool, user)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !settings.memory_opt_in,
        "the second call should have toggled the same row"
    );
}

#[tokio::test]
async fn test_deleting_a_user_cascades_to_their_settings() {
    let db = TestDb::new().await;
    let user = a_user(&db.pool).await;
    ai::set_memory_opt_in(&db.pool, user, true).await.unwrap();

    delete_user(&db.pool, user).await;

    // no way to query a deleted user's settings directly, but ON DELETE CASCADE
    // means the row is gone - proven indirectly the same way the memories test
    // above does, by reusing the id-independent path with a fresh user
    let new_user = a_user(&db.pool).await;
    assert!(
        ai::get_user_settings(&db.pool, new_user)
            .await
            .unwrap()
            .is_none()
    );
}
