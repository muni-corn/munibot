//! Database operations for munibot's ai safety event auditing.
//!
//! Split out the same way `limits.rs`/`abuse.rs` are: a genuinely separate
//! concern, with no policy of its own here - what counts as an event and
//! what it means lives in `munibot_ai::safety`.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{
    DbPool,
    models::{AiSafetyEvent, NewAiSafetyEvent},
    schema::ai_safety_events,
};

/// Records one safety event. Append-only - see [`NewAiSafetyEvent`]'s own
/// doc comment for why there is no corresponding update or upsert.
pub async fn record_safety_event(pool: &DbPool, event: NewAiSafetyEvent) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::insert_into(ai_safety_events::table)
        .values(&event)
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Lists the most recent safety events, newest first, for an operator
/// dashboard. `limit` is required rather than defaulting internally - the
/// caller (a paginated GUI list) is what actually knows a sensible page
/// size.
pub async fn list_safety_events(pool: &DbPool, limit: i64) -> QueryResult<Vec<AiSafetyEvent>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_safety_events::table
        .order(ai_safety_events::created_at.desc())
        .limit(limit)
        .select(AiSafetyEvent::as_select())
        .load(&mut conn)
        .await
}
