//! Database operations for munibot's AI conversations and messages.
//!
//! Free async functions taking `&DbPool` and returning `QueryResult<T>`, in
//! their own submodule because `operations.rs` is already long enough without
//! them.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{
    DbPool,
    models::{
        AiConversation, AiMemory, AiMessage, NewAiConversation, NewAiMemory, NewAiMessage,
        NewAiToolCall, NewAiUsage,
    },
    schema::{ai_conversations, ai_memories, ai_messages, ai_tool_calls, ai_usage},
};

// mysql has no `RETURNING`, so an insert's generated id comes from a second,
// same-connection `SELECT LAST_INSERT_ID()` -- the same approach
// `operations.rs` already uses
diesel::define_sql_function!(fn last_insert_id() -> diesel::sql_types::Unsigned<diesel::sql_types::Bigint>);

/// Looks a conversation up by the scope it belongs to.
pub async fn get_conversation_by_scope(
    pool: &DbPool,
    platform: &str,
    scope_key: &str,
) -> QueryResult<Option<AiConversation>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_conversations::table
        .filter(ai_conversations::platform.eq(platform))
        .filter(ai_conversations::scope_key.eq(scope_key))
        .select(AiConversation::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Looks a conversation up by id.
pub async fn get_conversation(
    pool: &DbPool,
    conversation_id: i64,
) -> QueryResult<Option<AiConversation>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_conversations::table
        .find(conversation_id)
        .select(AiConversation::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Creates a conversation, returning the saved row.
pub async fn create_conversation(
    pool: &DbPool,
    conversation: NewAiConversation,
) -> QueryResult<AiConversation> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::insert_into(ai_conversations::table)
        .values(&conversation)
        .execute(&mut conn)
        .await?;

    let id: u64 = diesel::select(last_insert_id())
        .get_result(&mut conn)
        .await?;
    ai_conversations::table
        .find(id as i64)
        .select(AiConversation::as_select())
        .first(&mut conn)
        .await
}

/// Returns the conversation for a scope, creating one bound to `persona_id` if
/// none exists yet.
///
/// An existing conversation's `persona_id` is returned as stored rather than
/// overwritten, since changing which persona owns a conversation is an
/// explicit action, not a side effect of loading it.
pub async fn get_or_create_conversation(
    pool: &DbPool,
    platform: &str,
    scope_key: &str,
    persona_id: &str,
    owner_user_id: Option<i64>,
) -> QueryResult<AiConversation> {
    if let Some(existing) = get_conversation_by_scope(pool, platform, scope_key).await? {
        return Ok(existing);
    }

    let now = chrono::Utc::now().naive_utc();
    create_conversation(pool, NewAiConversation {
        platform: platform.to_owned(),
        scope_key: scope_key.to_owned(),
        persona_id: persona_id.to_owned(),
        owner_user_id,
        title: None,
        created_at: now,
        last_active_at: now,
    })
    .await
}

/// Every conversation a user owns, most recently active first, excluding
/// archived ones.
pub async fn list_conversations_for_user(
    pool: &DbPool,
    user_id: i64,
) -> QueryResult<Vec<AiConversation>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_conversations::table
        .filter(ai_conversations::owner_user_id.eq(user_id))
        .filter(ai_conversations::archived_at.is_null())
        .order(ai_conversations::last_active_at.desc())
        .select(AiConversation::as_select())
        .load(&mut conn)
        .await
}

/// Renames a conversation.
pub async fn rename_conversation(
    pool: &DbPool,
    conversation_id: i64,
    title: &str,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(ai_conversations::table.find(conversation_id))
        .set(ai_conversations::title.eq(title))
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Archives a conversation, hiding it from the listing without deleting it.
pub async fn archive_conversation(pool: &DbPool, conversation_id: i64) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(ai_conversations::table.find(conversation_id))
        .set(ai_conversations::archived_at.eq(Some(chrono::Utc::now().naive_utc())))
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Sets or replaces a conversation's summary.
pub async fn set_conversation_summary(
    pool: &DbPool,
    conversation_id: i64,
    summary: Option<&str>,
    summary_tokens: i32,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(ai_conversations::table.find(conversation_id))
        .set((
            ai_conversations::summary.eq(summary),
            ai_conversations::summary_tokens.eq(summary_tokens),
        ))
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Appends a message, assigning it the next sequence number in the
/// conversation and bumping the conversation's activity timestamp.
///
/// The sequence number is `max(seq) + 1`, read and written without a lock. Two
/// concurrent appends to the *same* conversation could therefore pick the same
/// number, in which case the unique index on `(conversation_id, seq)` rejects
/// the loser rather than silently reordering history. That is the correct
/// outcome, and contention is close to nil in practice: a conversation is
/// driven by one turn at a time.
pub async fn append_message(
    pool: &DbPool,
    conversation_id: i64,
    role: &str,
    content: &str,
    token_count: i32,
) -> QueryResult<AiMessage> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    let next_seq = ai_messages::table
        .filter(ai_messages::conversation_id.eq(conversation_id))
        .select(diesel::dsl::max(ai_messages::seq))
        .first::<Option<i32>>(&mut conn)
        .await?
        .unwrap_or(0)
        + 1;

    let now = chrono::Utc::now().naive_utc();
    diesel::insert_into(ai_messages::table)
        .values(NewAiMessage {
            conversation_id,
            seq: next_seq,
            role: role.to_owned(),
            content: content.to_owned(),
            token_count,
            created_at: now,
        })
        .execute(&mut conn)
        .await?;

    diesel::update(ai_conversations::table.find(conversation_id))
        .set(ai_conversations::last_active_at.eq(now))
        .execute(&mut conn)
        .await?;

    let id: u64 = diesel::select(last_insert_id())
        .get_result(&mut conn)
        .await?;
    ai_messages::table
        .find(id as i64)
        .select(AiMessage::as_select())
        .first(&mut conn)
        .await
}

/// A conversation's messages, oldest first.
///
/// `limit` caps how many of the *most recent* messages come back, so a long
/// conversation returns its tail rather than its head, then is reversed into
/// the oldest-first order a provider expects.
pub async fn get_messages(
    pool: &DbPool,
    conversation_id: i64,
    limit: Option<i64>,
) -> QueryResult<Vec<AiMessage>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    let Some(limit) = limit else {
        return ai_messages::table
            .filter(ai_messages::conversation_id.eq(conversation_id))
            .order(ai_messages::seq.asc())
            .select(AiMessage::as_select())
            .load(&mut conn)
            .await;
    };

    let mut newest_first = ai_messages::table
        .filter(ai_messages::conversation_id.eq(conversation_id))
        .order(ai_messages::seq.desc())
        .limit(limit)
        .select(AiMessage::as_select())
        .load(&mut conn)
        .await?;
    newest_first.reverse();
    Ok(newest_first)
}

/// Deletes a conversation's messages and clears its summary, without deleting
/// the conversation itself.
///
/// A reset conversation keeps its id, so the same scope still resolves to it -
/// it is simply empty.
pub async fn clear_conversation(pool: &DbPool, conversation_id: i64) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::delete(ai_messages::table.filter(ai_messages::conversation_id.eq(conversation_id)))
        .execute(&mut conn)
        .await?;
    diesel::update(ai_conversations::table.find(conversation_id))
        .set((
            ai_conversations::summary.eq(None::<String>),
            ai_conversations::summary_tokens.eq(0),
        ))
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Deletes every message older than the most recent `keep_recent`, and sets
/// the conversation's summary to describe what was removed.
///
/// A conversation with `keep_recent` messages or fewer is left completely
/// untouched: there is nothing to summarise, and this must never overwrite an
/// existing summary with one describing nothing.
pub async fn compact_conversation(
    pool: &DbPool,
    conversation_id: i64,
    keep_recent: i64,
    summary: &str,
    summary_tokens: i32,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    // the seq of the oldest message still worth keeping - everything at or below
    // it is what gets summarised away. absent entirely when the conversation has
    // keep_recent messages or fewer, which is exactly when nothing should happen
    let threshold: Option<i32> = ai_messages::table
        .filter(ai_messages::conversation_id.eq(conversation_id))
        .order(ai_messages::seq.desc())
        .offset(keep_recent.max(0))
        .limit(1)
        .select(ai_messages::seq)
        .first(&mut conn)
        .await
        .optional()?;

    let Some(threshold) = threshold else {
        return Ok(());
    };

    diesel::delete(
        ai_messages::table
            .filter(ai_messages::conversation_id.eq(conversation_id))
            .filter(ai_messages::seq.le(threshold)),
    )
    .execute(&mut conn)
    .await?;

    diesel::update(ai_conversations::table.find(conversation_id))
        .set((
            ai_conversations::summary.eq(summary),
            ai_conversations::summary_tokens.eq(summary_tokens),
        ))
        .execute(&mut conn)
        .await?;

    Ok(())
}

/// Writes one row recording what a turn cost.
///
/// Called on failure as well as success: a turn that errored on its ninth
/// iteration still spent the first eight, and a usage table that only
/// records successes understates spend exactly when something is going
/// wrong.
pub async fn record_usage(pool: &DbPool, usage: NewAiUsage) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::insert_into(ai_usage::table)
        .values(&usage)
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Writes one row auditing a finished tool call.
///
/// The only way to debug a bad tool loop after the fact, and what a chat
/// surface's tool activity display reads back for a past conversation.
pub async fn record_tool_call(pool: &DbPool, tool_call: NewAiToolCall) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::insert_into(ai_tool_calls::table)
        .values(&tool_call)
        .execute(&mut conn)
        .await?;
    Ok(())
}

// ai_memories

/// Looks up one specific memory by key.
pub async fn get_memory(pool: &DbPool, user_id: i64, key: &str) -> QueryResult<Option<AiMemory>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_memories::table
        .filter(ai_memories::user_id.eq(user_id))
        .filter(ai_memories::key.eq(key))
        .select(AiMemory::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Every memory a user has recorded, in no particular guaranteed order.
pub async fn list_memories(pool: &DbPool, user_id: i64) -> QueryResult<Vec<AiMemory>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_memories::table
        .filter(ai_memories::user_id.eq(user_id))
        .select(AiMemory::as_select())
        .load(&mut conn)
        .await
}

/// How many memories a user currently has recorded.
///
/// The caller enforcing a per-user cap - not this function - decides what to
/// do with the count; this is a plain read with no policy attached.
pub async fn count_memories(pool: &DbPool, user_id: i64) -> QueryResult<i64> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_memories::table
        .filter(ai_memories::user_id.eq(user_id))
        .count()
        .get_result(&mut conn)
        .await
}

/// Records a fact under `key`, replacing any existing value for that key.
///
/// Uses MySQL's `INSERT ... ON DUPLICATE KEY UPDATE` on the `(user_id, key)`
/// unique index, the same reasoning `upsert_guild_config` documents: a
/// `REPLACE INTO` would delete and reinsert the row, losing `created_at` in
/// the process. A bare CRUD primitive with no cap enforcement of its own -
/// see `crate::memory::DieselMemoryStore` in `munibot_ai` for that.
pub async fn upsert_memory(
    pool: &DbPool,
    user_id: i64,
    key: &str,
    value: &str,
) -> QueryResult<AiMemory> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let now = chrono::Utc::now().naive_utc();

    diesel::insert_into(ai_memories::table)
        .values(NewAiMemory {
            user_id,
            key: key.to_string(),
            value: value.to_string(),
            created_at: now,
            updated_at: now,
        })
        .on_conflict(diesel::dsl::DuplicatedKeys)
        .do_update()
        .set((
            ai_memories::value.eq(value),
            ai_memories::updated_at.eq(now),
        ))
        .execute(&mut conn)
        .await?;

    ai_memories::table
        .filter(ai_memories::user_id.eq(user_id))
        .filter(ai_memories::key.eq(key))
        .select(AiMemory::as_select())
        .first(&mut conn)
        .await
}

/// Forgets one specific memory. Not an error if it never existed.
pub async fn forget_memory(pool: &DbPool, user_id: i64, key: &str) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::delete(
        ai_memories::table
            .filter(ai_memories::user_id.eq(user_id))
            .filter(ai_memories::key.eq(key)),
    )
    .execute(&mut conn)
    .await?;
    Ok(())
}

/// Forgets everything a user has ever recorded.
pub async fn wipe_memories(pool: &DbPool, user_id: i64) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::delete(ai_memories::table.filter(ai_memories::user_id.eq(user_id)))
        .execute(&mut conn)
        .await?;
    Ok(())
}
