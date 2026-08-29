//! Database operations for munibot's AI abuse-detection cooldowns.
//!
//! Split out the same way `limits.rs` is: a genuinely separate concern (an
//! escalating cooldown for abusive *behaviour*, not cost) with no policy of
//! its own here - what counts as a strike and how long a cooldown lasts
//! both live in `munibot_ai::abuse::AbuseDetector`.

use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{
    DbPool,
    models::{AiAbuseCooldown, NewAiAbuseCooldown},
    schema::ai_abuse_cooldowns,
};

/// Looks a scope's current cooldown state up, if it has ever tripped
/// before.
pub async fn get_abuse_cooldown(
    pool: &DbPool,
    scope_type: &str,
    scope_id: Option<i64>,
) -> QueryResult<Option<AiAbuseCooldown>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let mut query = ai_abuse_cooldowns::table
        .filter(ai_abuse_cooldowns::scope_type.eq(scope_type))
        .into_boxed();
    query = match scope_id {
        Some(id) => query.filter(ai_abuse_cooldowns::scope_id.eq(id)),
        None => query.filter(ai_abuse_cooldowns::scope_id.is_null()),
    };
    query
        .select(AiAbuseCooldown::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Records a fresh strike for a scope: its new strike count, how long it is
/// now cooling down for, and why.
///
/// Uses `INSERT ... ON DUPLICATE KEY UPDATE` on the `(scope_type, scope_id)`
/// unique index, the same `upsert_memory`/`reset_rate_limit_window`
/// reasoning: a `REPLACE INTO` would delete and reinsert the row for no
/// benefit here either.
pub async fn upsert_abuse_cooldown(
    pool: &DbPool,
    scope_type: &str,
    scope_id: Option<i64>,
    strike_count: i32,
    cooldown_until: NaiveDateTime,
    last_tripped_at: NaiveDateTime,
    last_reason: &str,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::insert_into(ai_abuse_cooldowns::table)
        .values(NewAiAbuseCooldown {
            scope_type: scope_type.to_string(),
            scope_id,
            strike_count,
            cooldown_until,
            last_reason: last_reason.to_string(),
            last_tripped_at,
        })
        .on_conflict(diesel::dsl::DuplicatedKeys)
        .do_update()
        .set((
            ai_abuse_cooldowns::strike_count.eq(strike_count),
            ai_abuse_cooldowns::cooldown_until.eq(cooldown_until),
            ai_abuse_cooldowns::last_reason.eq(last_reason),
            ai_abuse_cooldowns::last_tripped_at.eq(last_tripped_at),
        ))
        .execute(&mut conn)
        .await?;
    Ok(())
}
