use chrono::NaiveDateTime;
use diesel::prelude::*;

use crate::db::schema::{
    autodelete_timers, community_links, guild_configs, guild_payouts, guild_wallets, quotes,
};

// guild_configs

/// A row in the `guild_configs` table.
#[derive(Clone, Debug, Queryable, Insertable, AsChangeset, Selectable)]
#[diesel(table_name = guild_configs)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct GuildConfig {
    pub guild_id: i64,
    pub logging_channel: Option<i64>,
}

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
