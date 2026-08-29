//! Diesel models for munibot's AI tables.
//!
//! Kept in their own module rather than alongside the rest: the AI feature adds
//! four tables here and several more across later milestones (memories, user
//! settings, attachments, rate limits, pipelines), which would push
//! `models.rs` well past the size worth reading in one sitting.

use chrono::NaiveDateTime;
use diesel::prelude::*;

use crate::db::schema::{
    ai_abuse_cooldowns, ai_attachments, ai_conversations, ai_memories, ai_messages,
    ai_pipeline_events, ai_pipelines, ai_rate_limits, ai_safety_events, ai_spend_caps,
    ai_tool_calls, ai_usage, ai_user_settings,
};

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

// ai_attachments

/// A row in the `ai_attachments` table: one uploaded image's raw bytes and
/// metadata.
///
/// `message_id` is `None` until the message that references it is actually
/// persisted - an upload always happens before `send_message`, since SSE
/// (a `GET`) cannot carry a pasted image as a query string. `data` is
/// selected separately from everything else in practice (see
/// `operations::ai::get_attachment_data`): loading every attachment's own
/// bytes just to show a thumbnail list would be wasteful.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_attachments)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiAttachment {
    pub id: i64,
    pub conversation_id: i64,
    pub message_id: Option<i64>,
    pub media_type: String,
    pub byte_size: i32,
    pub sha256: String,
    pub data: Vec<u8>,
    pub created_at: NaiveDateTime,
}

/// [`AiAttachment`] without `data` - metadata only, for listing or showing a
/// thumbnail without loading every attachment's own bytes into memory at
/// once.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_attachments)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiAttachmentMeta {
    pub id: i64,
    pub conversation_id: i64,
    pub message_id: Option<i64>,
    pub media_type: String,
    pub byte_size: i32,
    pub sha256: String,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `ai_attachments`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_attachments)]
pub struct NewAiAttachment {
    pub conversation_id: i64,
    pub media_type: String,
    pub byte_size: i32,
    pub sha256: String,
    pub data: Vec<u8>,
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

// ai_memories

/// A row in the `ai_memories` table: one fact a user has asked munibot to
/// remember.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_memories)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiMemory {
    pub id: i64,
    pub user_id: i64,
    pub key: String,
    pub value: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Insertable and upsertable shape for `ai_memories`.
///
/// Used both to insert a new memory and, via `ON DUPLICATE KEY UPDATE` on the
/// `(user_id, key)` unique index, to update an existing one's `value` in
/// place - the same upsert pattern `operations.rs:33` already documents.
#[derive(Clone, Debug, Insertable, AsChangeset)]
#[diesel(table_name = ai_memories)]
pub struct NewAiMemory {
    pub user_id: i64,
    pub key: String,
    pub value: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

// ai_user_settings

/// A row in the `ai_user_settings` table: one user's AI-related preferences.
///
/// No `Identifiable` derive: its primary key is `user_id`, not the `id` the
/// derive assumes by default, and every lookup here goes through a plain
/// `.filter(...)` rather than `.find(...)` - the same choice
/// [`crate::db::models::GuildConfig`] makes for the same reason.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = ai_user_settings)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiUserSettings {
    pub user_id: i64,
    /// Defaults to `false`: memory is opt-in, never assumed.
    pub memory_opt_in: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Insertable and upsertable shape for `ai_user_settings`.
#[derive(Clone, Debug, Insertable, AsChangeset)]
#[diesel(table_name = ai_user_settings)]
pub struct NewAiUserSettings {
    pub user_id: i64,
    pub memory_opt_in: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

// ai_rate_limits

/// A row in the `ai_rate_limits` table: one scope's counter for the current
/// window.
///
/// `scope_type` is one of `"user"`, `"guild"`, or `"global"` (see
/// `munibot_ai::limits::Scope`, which this mirrors); `scope_id` is `None`
/// for the single global row. No `Identifiable` derive: every lookup goes
/// through the `(scope_type, scope_id)` unique index via a plain
/// `.filter(...)`, the same choice
/// [`GuildConfig`](crate::db::models::GuildConfig) makes for the same reason.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = ai_rate_limits)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiRateLimit {
    pub id: i64,
    pub scope_type: String,
    pub scope_id: Option<i64>,
    pub window_start: NaiveDateTime,
    pub request_count: i32,
    pub token_count: i64,
}

/// Insertable and upsertable shape for `ai_rate_limits`.
#[derive(Clone, Debug, Insertable, AsChangeset)]
#[diesel(table_name = ai_rate_limits)]
pub struct NewAiRateLimit {
    pub scope_type: String,
    pub scope_id: Option<i64>,
    pub window_start: NaiveDateTime,
    pub request_count: i32,
    pub token_count: i64,
}

// ai_spend_caps

/// A row in the `ai_spend_caps` table: one scope's spend against a
/// configured cap for one period (e.g. `"monthly"`).
///
/// No `Identifiable` derive, for the same reason [`AiRateLimit`] has none:
/// every lookup goes through the `(scope_type, scope_id, period)` unique
/// index.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = ai_spend_caps)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiSpendCap {
    pub id: i64,
    pub scope_type: String,
    pub scope_id: Option<i64>,
    pub period: String,
    pub limit_micros: i64,
    pub current_micros: i64,
    pub reset_at: NaiveDateTime,
}

/// Insertable and upsertable shape for `ai_spend_caps`.
#[derive(Clone, Debug, Insertable, AsChangeset)]
#[diesel(table_name = ai_spend_caps)]
pub struct NewAiSpendCap {
    pub scope_type: String,
    pub scope_id: Option<i64>,
    pub period: String,
    pub limit_micros: i64,
    pub current_micros: i64,
    pub reset_at: NaiveDateTime,
}

// ai_abuse_cooldowns

/// A row in the `ai_abuse_cooldowns` table: one scope's escalating-cooldown
/// state, as tracked by `munibot_ai::abuse::AbuseDetector`.
///
/// The same `(scope_type, scope_id)` convention [`AiRateLimit`] documents;
/// no `Identifiable` derive for the same reason that type has none.
/// Deliberately carries no message content at all - `last_reason` is one of
/// a small, stable set of strings (see
/// `munibot_ai::abuse::AbuseSignal::reason`), enough to tune the detector
/// without this table becoming a second place user text ends up stored.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = ai_abuse_cooldowns)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiAbuseCooldown {
    pub id: i64,
    pub scope_type: String,
    pub scope_id: Option<i64>,
    pub strike_count: i32,
    pub cooldown_until: NaiveDateTime,
    pub last_reason: String,
    pub last_tripped_at: NaiveDateTime,
}

/// Insertable and upsertable shape for `ai_abuse_cooldowns`.
#[derive(Clone, Debug, Insertable, AsChangeset)]
#[diesel(table_name = ai_abuse_cooldowns)]
pub struct NewAiAbuseCooldown {
    pub scope_type: String,
    pub scope_id: Option<i64>,
    pub strike_count: i32,
    pub cooldown_until: NaiveDateTime,
    pub last_reason: String,
    pub last_tripped_at: NaiveDateTime,
}

// ai_safety_events

/// A row in the `ai_safety_events` table: one trip of a rate limit, a spend
/// cap, a moderation check, or a crisis classifier - see
/// `munibot_ai::safety::SafetyEvent`, which this mirrors.
///
/// Deliberately carries no raw content: `content_hash` is a one-way digest,
/// the same reasoning [`AiAbuseCooldown`]'s own doc comment documents for
/// `last_reason`.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = ai_safety_events)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiSafetyEvent {
    pub id: i64,
    pub event_type: String,
    pub scope_type: String,
    pub scope_id: Option<i64>,
    pub reason: String,
    pub content_hash: Option<String>,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `ai_safety_events`. No `AsChangeset`: safety events
/// are append-only, never updated in place, unlike every other table in
/// this module.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_safety_events)]
pub struct NewAiSafetyEvent {
    pub event_type: String,
    pub scope_type: String,
    pub scope_id: Option<i64>,
    pub reason: String,
    pub content_hash: Option<String>,
    pub created_at: NaiveDateTime,
}

// ai_pipelines

/// A row in the `ai_pipelines` table: one autonomous pipeline run,
/// identified by the issue that triggered it.
///
/// Deliberately holds only identity, never a mutable "current state"
/// column -- `munibot_ai::pipeline::PipelineState` is always a fold over
/// `ai_pipeline_events` (see [`AiPipelineEvent`]), recomputed by replay, so
/// there is nothing here for a crash mid-run to leave inconsistent.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_pipelines)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiPipeline {
    pub id: i64,
    pub forge: String,
    pub owner: String,
    pub repo_name: String,
    pub issue_number: u64,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `ai_pipelines`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_pipelines)]
pub struct NewAiPipeline {
    pub forge: String,
    pub owner: String,
    pub repo_name: String,
    pub issue_number: u64,
    pub created_at: NaiveDateTime,
}

// ai_pipeline_events

/// A row in the `ai_pipeline_events` table: one event in a pipeline's own
/// append-only log.
///
/// `payload` is a json-encoded event, the same json-encoded-text
/// convention `ai_messages.content` already uses rather than a native
/// json column. The unique index on `(pipeline_id, seq)` is what makes
/// this log append-only and gap-free -- enforced by the database itself,
/// not merely by application code that could race.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_pipeline_events)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiPipelineEvent {
    pub id: i64,
    pub pipeline_id: i64,
    pub seq: i32,
    pub event_type: String,
    pub payload: String,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `ai_pipeline_events`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_pipeline_events)]
pub struct NewAiPipelineEvent {
    pub pipeline_id: i64,
    pub seq: i32,
    pub event_type: String,
    pub payload: String,
    pub created_at: NaiveDateTime,
}
