use chrono::NaiveDateTime;
use diesel::prelude::*;

use crate::db::schema::{
    autodelete_timers, community_links, email_signin_tokens, guild_configs, guild_payouts,
    guild_wallets, linked_accounts, quotes, user_permissions, users,
};

pub mod ai;

pub use ai::{
    AiAbuseCooldown, AiAttachment, AiAttachmentMeta, AiChannelAllowlistEntry, AiConversation,
    AiMemory, AiMessage, AiPipeline, AiPipelineEvent, AiRateLimit, AiSafetyEvent, AiSpendCap,
    AiToolCall, AiUsage, AiUserSettings, NewAiAbuseCooldown, NewAiAttachment,
    NewAiChannelAllowlistEntry, NewAiConversation, NewAiMemory, NewAiMessage, NewAiPipeline,
    NewAiPipelineEvent, NewAiRateLimit, NewAiSafetyEvent, NewAiSpendCap, NewAiToolCall, NewAiUsage,
    NewAiUserSettings,
};

// guild_configs

/// A row in the `guild_configs` table.
///
/// Two settings' worth of columns share this one row (logging and, as of
/// milestone 6, ai) - see `operations::set_guild_logging_channel` and
/// `operations::set_guild_ai_settings`'s own doc comments for why every
/// write must go through one of those rather than constructing this
/// struct directly and calling `operations::upsert_guild_config` with it.
#[derive(Clone, Debug, Queryable, Insertable, AsChangeset, Selectable)]
#[diesel(table_name = guild_configs)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct GuildConfig {
    pub guild_id: i64,
    pub logging_channel: Option<i64>,
    /// Whether munibot's discord ai surface (mentions, replies, dms) is
    /// enabled at all for this guild. Defaults to `false`: a guild that
    /// existed before this column did must not wake up ai-enabled just
    /// because a row for it already existed.
    pub ai_enabled: bool,
    /// A persona id, overriding the service-wide default for this guild
    /// specifically. `None` falls back to whatever `Ai::default_persona_id`
    /// resolves to.
    pub ai_default_persona: Option<String>,
    /// `"all"` (every channel) or `"allowlist"` (only channels in
    /// `ai_channel_allowlist`) - a plain string rather than a Rust enum at
    /// this layer, the same stringly-typed convention
    /// `munibot_ai::limits::Scope::scope_type` already uses for a database
    /// column with a small, fixed set of values.
    pub ai_channel_mode: String,
}

/// The channel mode a guild's ai settings use before anyone has ever saved
/// them - matching this column's own `DEFAULT 'all'` in the migration:
/// every channel, since a guild's ai surface enabling at all is already a
/// deliberate, opt-in decision, and an allowlist on top of that opt-in is
/// a second decision no one has made yet.
pub const DEFAULT_AI_CHANNEL_MODE: &str = "all";

// autodelete_timers

/// A row in the `autodelete_timers` table.
#[derive(Clone, Debug, Queryable, Insertable, Selectable)]
#[diesel(table_name = autodelete_timers)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AutoDeleteTimerRow {
    pub channel_id: i64,
    pub guild_id: i64,
    pub duration_secs: i64,
    pub last_cleaned: NaiveDateTime,
    pub last_message_id_cleaned: i64,
    pub mode: String,
}

/// Changeset for updating an existing `autodelete_timers` row.
#[derive(Clone, Debug, AsChangeset)]
#[diesel(table_name = autodelete_timers)]
pub struct UpdateAutoDeleteTimer {
    pub duration_secs: Option<i64>,
    pub last_cleaned: Option<NaiveDateTime>,
    pub last_message_id_cleaned: Option<i64>,
    pub mode: Option<String>,
}

// guild_wallets

/// A row in the `guild_wallets` table.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = guild_wallets)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct GuildWallet {
    pub id: i64,
    pub guild_id: i64,
    pub user_id: i64,
    pub balance: u64,
}

/// Insertable shape for `guild_wallets`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = guild_wallets)]
pub struct NewGuildWallet {
    pub guild_id: i64,
    pub user_id: i64,
    pub balance: u64,
}

// guild_payouts

/// A row in the `guild_payouts` table.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = guild_payouts)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct GuildPayout {
    pub id: i64,
    pub guild_id: i64,
    pub user_id: i64,
    pub balance: u64,
    pub last_payout: NaiveDateTime,
}

/// Insertable shape for `guild_payouts`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = guild_payouts)]
pub struct NewGuildPayout {
    pub guild_id: i64,
    pub user_id: i64,
    pub balance: u64,
    pub last_payout: NaiveDateTime,
}

// community_links

/// A row in the `community_links` table.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = community_links)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct CommunityLink {
    pub id: i64,
    pub twitch_streamer_id: Option<String>,
    pub discord_guild_id: Option<i64>,
}

/// Insertable shape for `community_links`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = community_links)]
pub struct NewCommunityLink {
    pub twitch_streamer_id: Option<String>,
    pub discord_guild_id: Option<i64>,
}

// quotes

/// A row in the `quotes` table.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = quotes)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct Quote {
    pub id: i64,
    pub community_id: i64,
    pub sequential_id: i32,
    pub created_at: NaiveDateTime,
    pub quote: String,
    pub invoker: String,
    pub stream_category: String,
    pub stream_title: String,
}

/// Insertable shape for `quotes` (without auto-increment `id` and
/// caller-computed `sequential_id`).
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = quotes)]
pub struct NewQuote {
    pub community_id: i64,
    pub sequential_id: i32,
    pub created_at: NaiveDateTime,
    pub quote: String,
    pub invoker: String,
    pub stream_category: String,
    pub stream_title: String,
}

// users

/// A row in the `users` table. Represents a single munibot account, which may
/// have one or more `LinkedAccount`s (discord, and eventually twitch/github).
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = users)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct User {
    pub id: i64,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `users`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = users)]
pub struct NewUser {
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub created_at: NaiveDateTime,
}

// linked_accounts

/// A row in the `linked_accounts` table: one external provider account
/// (identified by `provider` + `provider_user_id`) linked to a munibot user.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = linked_accounts)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct LinkedAccount {
    pub id: i64,
    pub user_id: i64,
    pub provider: String,
    pub provider_user_id: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Insertable shape for `linked_accounts`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = linked_accounts)]
pub struct NewLinkedAccount {
    pub user_id: i64,
    pub provider: String,
    pub provider_user_id: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

// email_signin_tokens

/// A row in the `email_signin_tokens` table: one outstanding magic-link
/// sign-in request. `token_hash` is a SHA-256 hex digest of the actual
/// token mailed to the address, never the token itself - see the
/// migration's own comment for why.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = email_signin_tokens)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct EmailSigninToken {
    pub id: i64,
    pub email: String,
    pub token_hash: String,
    pub expires_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

/// Insertable and upsertable shape for `email_signin_tokens`.
#[derive(Clone, Debug, Insertable, AsChangeset)]
#[diesel(table_name = email_signin_tokens)]
pub struct NewEmailSigninToken {
    pub email: String,
    pub token_hash: String,
    pub expires_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

// user_permissions

/// A row in the `user_permissions` table: one permission (the snake_case
/// string form of `crate::permission::Permission`) granted to a munibot
/// user.
#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = user_permissions)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct UserPermission {
    pub id: i64,
    pub user_id: i64,
    pub permission: String,
    pub created_at: NaiveDateTime,
}

/// Insertable shape for `user_permissions`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = user_permissions)]
pub struct NewUserPermission {
    pub user_id: i64,
    pub permission: String,
    pub created_at: NaiveDateTime,
}
