//! Diesel models for munibot's AI tables.
//!
//! Kept in their own module rather than alongside the rest: the AI feature adds
//! four tables here and several more across later milestones (memories, user
//! settings, attachments, rate limits, pipelines), which would push
//! `models.rs` well past the size worth reading in one sitting.

use chrono::NaiveDateTime;
use diesel::prelude::*;

use crate::db::schema::{ai_conversations, ai_messages, ai_tool_calls, ai_usage};

// ai_conversations

/// A row in the `ai_conversations` table: one conversation munibot is holding,
/// on any surface.
///
/// `owner_user_id`, `title`, and `archived_at` are populated only for web
/// conversations, which belong to one person and need a name in a sidebar. A
/// Discord channel's conversation has none of the three.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_conversations)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiConversation {
    pub id: i64,
    pub platform: String,
    pub scope_key: String,
    pub persona_id: String,
    pub owner_user_id: Option<i64>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub summary_tokens: i32,
    pub archived_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub last_active_at: NaiveDateTime,
}

/// Insertable shape for `ai_conversations`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_conversations)]
pub struct NewAiConversation {
    pub platform: String,
    pub scope_key: String,
    pub persona_id: String,
    pub owner_user_id: Option<i64>,
    pub title: Option<String>,
    pub created_at: NaiveDateTime,
    pub last_active_at: NaiveDateTime,
}

// ai_messages

/// A row in the `ai_messages` table.
///
/// `content` is a JSON-encoded `Vec<ContentBlock>` rather than plain text, so
/// tool calls and their results survive a restart intact. Deserialising it is
/// the AI crate's job; this layer only moves the string.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_messages)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiMessage {
    pub id: i64,
    pub conversation_id: i64,
    pub seq: i32,
    pub role: String,
    pub content: String,
    pub token_count: i32,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `ai_messages`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_messages)]
pub struct NewAiMessage {
    pub conversation_id: i64,
    pub seq: i32,
    pub role: String,
    pub content: String,
    pub token_count: i32,
    pub created_at: NaiveDateTime,
}

// ai_usage

/// A row in the `ai_usage` table: what one completed turn cost.
///
/// Written on failure as well as success, since a turn that errored on its
/// ninth iteration still spent money.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_usage)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiUsage {
    pub id: i64,
    pub conversation_id: Option<i64>,
    pub user_id: Option<i64>,
    pub guild_id: Option<i64>,
    pub provider: String,
    pub model: String,
    pub persona_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Millionths of a dollar, as an integer so summing a month of rows does
    /// not accumulate rounding error.
    pub cost_micros: i64,
    pub iterations: i32,
    pub succeeded: bool,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `ai_usage`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_usage)]
pub struct NewAiUsage {
    pub conversation_id: Option<i64>,
    pub user_id: Option<i64>,
    pub guild_id: Option<i64>,
    pub provider: String,
    pub model: String,
    pub persona_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
    pub iterations: i32,
    pub succeeded: bool,
    pub created_at: NaiveDateTime,
}

// ai_tool_calls

/// A row in the `ai_tool_calls` table: one tool invocation, with its input and
/// output truncated to a reviewable size.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_tool_calls)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiToolCall {
    pub id: i64,
    pub conversation_id: Option<i64>,
    pub tool_name: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub duration_ms: i64,
    pub status: String,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `ai_tool_calls`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_tool_calls)]
pub struct NewAiToolCall {
    pub conversation_id: Option<i64>,
    pub tool_name: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub duration_ms: i64,
    pub status: String,
    pub created_at: NaiveDateTime,
}
