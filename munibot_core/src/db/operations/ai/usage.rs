//! Aggregate usage queries over `ai_usage`.
//!
//! Summed in SQL via `SUM`/`COUNT`, never pulled into rust and summed there -
//! this table only grows, and a usage summary that scans every row itself
//! gets slow within a month of any real traffic.

use diesel::{
    prelude::*,
    sql_types::{BigInt, Nullable},
};
use diesel_async::RunQueryDsl;

use crate::db::{DbPool, schema::ai_usage};

// diesel's `sum()` promotes a MySQL `BIGINT` column to `Nullable<Numeric>`
// (decimal), which would drag in the `bigdecimal` crate for one query. A raw,
// hardcoded (never interpolated) `SUM(...)` cast back to a plain,
// possibly-null bigint sidesteps that instead - still query-built, just with
// an explicit result type.

/// Aggregate totals across some slice of `ai_usage` - either one user's own
/// history, or the whole table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageTotals {
    pub cost_micros: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub turn_count: i64,
}

/// Every turn ever recorded for one user, successful or not.
///
/// Unfiltered by `succeeded`: a turn that failed part way through still
/// spent real tokens, the same reasoning `record_usage`'s own doc comment
/// gives for writing a row either way.
pub async fn sum_usage_for_user(pool: &DbPool, user_id: i64) -> QueryResult<UsageTotals> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let (cost_micros, input_tokens, output_tokens, turn_count): (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        i64,
    ) = ai_usage::table
        .filter(ai_usage::user_id.eq(user_id))
        .select((
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(cost_micros)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(input_tokens)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(output_tokens)"),
            diesel::dsl::count_star(),
        ))
        .first(&mut conn)
        .await?;

    Ok(UsageTotals {
        cost_micros: cost_micros.unwrap_or(0),
        input_tokens: input_tokens.unwrap_or(0),
        output_tokens: output_tokens.unwrap_or(0),
        turn_count,
    })
}

/// Every turn ever recorded across every user - the same totals
/// `sum_usage_for_user` returns, unfiltered by who ran the turn.
pub async fn sum_usage_global(pool: &DbPool) -> QueryResult<UsageTotals> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let (cost_micros, input_tokens, output_tokens, turn_count): (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        i64,
    ) = ai_usage::table
        .select((
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(cost_micros)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(input_tokens)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(output_tokens)"),
            diesel::dsl::count_star(),
        ))
        .first(&mut conn)
        .await?;

    Ok(UsageTotals {
        cost_micros: cost_micros.unwrap_or(0),
        input_tokens: input_tokens.unwrap_or(0),
        output_tokens: output_tokens.unwrap_or(0),
        turn_count,
    })
}
