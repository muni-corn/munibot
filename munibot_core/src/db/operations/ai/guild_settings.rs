//! Database operations for a guild's ai channel allowlist.
//!
//! Split out the same way `limits.rs`/`abuse.rs`/`safety.rs` are: a
//! genuinely separate concern from conversation storage, with no policy of
//! its own here - whether a guild's `ai_channel_mode` actually consults
//! this list at all is `munibot_discord`'s own concern, not this module's.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{DbPool, models::NewAiChannelAllowlistEntry, schema::ai_channel_allowlist};

/// Every channel id a guild has allowed, for its own settings page and for
/// `munibot_discord`'s own allowlist check.
pub async fn list_ai_channel_allowlist(pool: &DbPool, guild_id: i64) -> QueryResult<Vec<i64>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_channel_allowlist::table
        .filter(ai_channel_allowlist::guild_id.eq(guild_id))
        .select(ai_channel_allowlist::channel_id)
        .load(&mut conn)
        .await
}

/// Replaces a guild's entire channel allowlist with `channel_ids` -
/// deletes every existing entry for the guild, then inserts the new set.
/// A full replace rather than a diff: the settings page this backs saves
/// its whole multi-select at once, and there is no meaningful "partial"
/// allowlist save to support instead.
pub async fn set_ai_channel_allowlist(
    pool: &DbPool,
    guild_id: i64,
    channel_ids: &[i64],
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    diesel::delete(ai_channel_allowlist::table.filter(ai_channel_allowlist::guild_id.eq(guild_id)))
        .execute(&mut conn)
        .await?;

    if channel_ids.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now().naive_utc();
    let rows: Vec<NewAiChannelAllowlistEntry> = channel_ids
        .iter()
        .map(|&channel_id| NewAiChannelAllowlistEntry {
            guild_id,
            channel_id,
            created_at: now,
        })
        .collect();

    diesel::insert_into(ai_channel_allowlist::table)
        .values(&rows)
        .execute(&mut conn)
        .await?;

    Ok(())
}
