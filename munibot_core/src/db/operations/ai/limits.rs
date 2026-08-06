//! Database operations for munibot's AI rate limits and spend caps.
//!
//! Split out from the rest of `ai.rs` rather than added to it: that file was
//! already long enough, and this is a genuinely separate concern (cost
//! control, not conversation storage). Every function here is a bare CRUD
//! primitive with no policy of its own - what counts as a scope, what the
//! configured limits are, and what to do once one is hit all live in
//! `munibot_ai`'s own rate limiter and spend cap types.

use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{
    DbPool,
    models::{AiRateLimit, AiSpendCap, NewAiRateLimit, NewAiSpendCap},
    schema::{ai_rate_limits, ai_spend_caps},
};

// ai_rate_limits

/// Looks a scope's current rate limit window up, if one has ever been
/// started for it.
pub async fn get_rate_limit(
    pool: &DbPool,
    scope_type: &str,
    scope_id: Option<i64>,
) -> QueryResult<Option<AiRateLimit>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let mut query = ai_rate_limits::table
        .filter(ai_rate_limits::scope_type.eq(scope_type))
        .into_boxed();
    query = match scope_id {
        Some(id) => query.filter(ai_rate_limits::scope_id.eq(id)),
        None => query.filter(ai_rate_limits::scope_id.is_null()),
    };
    query
        .select(AiRateLimit::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Starts a fresh window for a scope, replacing whatever window (if any)
/// existed before - used once the previous window has expired.
///
/// Uses `INSERT ... ON DUPLICATE KEY UPDATE` on the `(scope_type, scope_id)`
/// unique index, the same reasoning `upsert_memory` documents: a `REPLACE
/// INTO` would delete and reinsert the row, which is unnecessary churn here
/// too.
pub async fn reset_rate_limit_window(
    pool: &DbPool,
    scope_type: &str,
    scope_id: Option<i64>,
    window_start: NaiveDateTime,
    request_count: i32,
    token_count: i64,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::insert_into(ai_rate_limits::table)
        .values(NewAiRateLimit {
            scope_type: scope_type.to_string(),
            scope_id,
            window_start,
            request_count,
            token_count,
        })
        .on_conflict(diesel::dsl::DuplicatedKeys)
        .do_update()
        .set((
            ai_rate_limits::window_start.eq(window_start),
            ai_rate_limits::request_count.eq(request_count),
            ai_rate_limits::token_count.eq(token_count),
        ))
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Adds to a scope's counters within its current window.
///
/// A raw `SET count = count + ?` update rather than a read-modify-write, so
/// two concurrent turns for the same scope both land instead of one
/// silently overwriting the other.
pub async fn increment_rate_limit(
    pool: &DbPool,
    scope_type: &str,
    scope_id: Option<i64>,
    requests: i32,
    tokens: i64,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    match scope_id {
        Some(id) => {
            diesel::update(
                ai_rate_limits::table
                    .filter(ai_rate_limits::scope_type.eq(scope_type))
                    .filter(ai_rate_limits::scope_id.eq(id)),
            )
            .set((
                ai_rate_limits::request_count.eq(ai_rate_limits::request_count + requests),
                ai_rate_limits::token_count.eq(ai_rate_limits::token_count + tokens),
            ))
            .execute(&mut conn)
            .await?;
        }
        None => {
            diesel::update(
                ai_rate_limits::table
                    .filter(ai_rate_limits::scope_type.eq(scope_type))
                    .filter(ai_rate_limits::scope_id.is_null()),
            )
            .set((
                ai_rate_limits::request_count.eq(ai_rate_limits::request_count + requests),
                ai_rate_limits::token_count.eq(ai_rate_limits::token_count + tokens),
            ))
            .execute(&mut conn)
            .await?;
        }
    }
    Ok(())
}

// ai_spend_caps

/// Looks a scope's spend cap up for one period, if one has ever been
/// created for it.
pub async fn get_spend_cap(
    pool: &DbPool,
    scope_type: &str,
    scope_id: Option<i64>,
    period: &str,
) -> QueryResult<Option<AiSpendCap>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let mut query = ai_spend_caps::table
        .filter(ai_spend_caps::scope_type.eq(scope_type))
        .filter(ai_spend_caps::period.eq(period))
        .into_boxed();
    query = match scope_id {
        Some(id) => query.filter(ai_spend_caps::scope_id.eq(id)),
        None => query.filter(ai_spend_caps::scope_id.is_null()),
    };
    query
        .select(AiSpendCap::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Creates a scope's spend cap for a period, or replaces it wholesale - used
/// both the first time a scope is checked and to roll a cap over once
/// `reset_at` has passed.
///
/// The same `INSERT ... ON DUPLICATE KEY UPDATE` reasoning as
/// `reset_rate_limit_window`, on the `(scope_type, scope_id, period)`
/// unique index.
pub async fn upsert_spend_cap(pool: &DbPool, cap: NewAiSpendCap) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::insert_into(ai_spend_caps::table)
        .values(&cap)
        .on_conflict(diesel::dsl::DuplicatedKeys)
        .do_update()
        .set((
            ai_spend_caps::limit_micros.eq(cap.limit_micros),
            ai_spend_caps::current_micros.eq(cap.current_micros),
            ai_spend_caps::reset_at.eq(cap.reset_at),
        ))
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Adds to a scope's spend within its current period.
///
/// The same raw `SET current_micros = current_micros + ?` reasoning as
/// `increment_rate_limit`: two concurrent turns for the same scope must
/// both be counted, not race each other over one read-modify-write.
pub async fn increment_spend(
    pool: &DbPool,
    scope_type: &str,
    scope_id: Option<i64>,
    period: &str,
    micros: i64,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    match scope_id {
        Some(id) => {
            diesel::update(
                ai_spend_caps::table
                    .filter(ai_spend_caps::scope_type.eq(scope_type))
                    .filter(ai_spend_caps::scope_id.eq(id))
                    .filter(ai_spend_caps::period.eq(period)),
            )
            .set(ai_spend_caps::current_micros.eq(ai_spend_caps::current_micros + micros))
            .execute(&mut conn)
            .await?;
        }
        None => {
            diesel::update(
                ai_spend_caps::table
                    .filter(ai_spend_caps::scope_type.eq(scope_type))
                    .filter(ai_spend_caps::scope_id.is_null())
                    .filter(ai_spend_caps::period.eq(period)),
            )
            .set(ai_spend_caps::current_micros.eq(ai_spend_caps::current_micros + micros))
            .execute(&mut conn)
            .await?;
        }
    }
    Ok(())
}
