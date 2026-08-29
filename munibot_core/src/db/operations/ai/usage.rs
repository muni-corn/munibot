//! Aggregate usage queries over `ai_usage`.
//!
//! Summed in SQL via `SUM`/`COUNT`, never pulled into rust and summed there -
//! this table only grows, and a usage summary that scans every row itself
//! gets slow within a month of any real traffic.

use chrono::{Duration, NaiveDate, Utc};
use diesel::{
    prelude::*,
    sql_types::{BigInt, Date, Nullable},
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

/// Assembles one grouped aggregate row into a `(key, UsageTotals)` pair -
/// shared by every `sum_usage_by_*` query below, which all differ only in
/// what they group by.
fn totals_row<K>(
    key: K,
    cost_micros: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    turn_count: i64,
) -> (K, UsageTotals) {
    (key, UsageTotals {
        cost_micros: cost_micros.unwrap_or(0),
        input_tokens: input_tokens.unwrap_or(0),
        output_tokens: output_tokens.unwrap_or(0),
        turn_count,
    })
}

/// Global totals grouped by persona, sorted highest-cost first - not sorted
/// in SQL (ordering by a raw aggregate expression is fragile across diesel
/// versions), since the result set here is bounded by the number of
/// distinct personas ever used, never by how many rows `ai_usage` itself
/// has grown to.
#[allow(clippy::type_complexity)]
pub async fn sum_usage_by_persona(pool: &DbPool) -> QueryResult<Vec<(String, UsageTotals)>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let rows: Vec<(String, Option<i64>, Option<i64>, Option<i64>, i64)> = ai_usage::table
        .group_by(ai_usage::persona_id)
        .select((
            ai_usage::persona_id,
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(cost_micros)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(input_tokens)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(output_tokens)"),
            diesel::dsl::count_star(),
        ))
        .load(&mut conn)
        .await?;

    let mut totals: Vec<_> = rows
        .into_iter()
        .map(|(persona_id, cost, input, output, turns)| {
            totals_row(persona_id, cost, input, output, turns)
        })
        .collect();
    totals.sort_by_key(|entry| std::cmp::Reverse(entry.1.cost_micros));
    Ok(totals)
}

/// Global totals grouped by `(provider, model)`, sorted highest-cost first -
/// the same reasoning `sum_usage_by_persona` documents for sorting in rust
/// rather than sql.
#[allow(clippy::type_complexity)]
pub async fn sum_usage_by_model(
    pool: &DbPool,
) -> QueryResult<Vec<((String, String), UsageTotals)>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let rows: Vec<(String, String, Option<i64>, Option<i64>, Option<i64>, i64)> = ai_usage::table
        .group_by((ai_usage::provider, ai_usage::model))
        .select((
            ai_usage::provider,
            ai_usage::model,
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(cost_micros)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(input_tokens)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(output_tokens)"),
            diesel::dsl::count_star(),
        ))
        .load(&mut conn)
        .await?;

    let mut totals: Vec<_> = rows
        .into_iter()
        .map(|(provider, model, cost, input, output, turns)| {
            totals_row((provider, model), cost, input, output, turns)
        })
        .collect();
    totals.sort_by_key(|entry| std::cmp::Reverse(entry.1.cost_micros));
    Ok(totals)
}

/// Global totals grouped by user, sorted highest-cost first - `None` for a
/// turn recorded with no user at all (there is no such case today, but
/// `ai_usage.user_id` is nullable, so this stays exhaustive rather than
/// silently dropping a row a future caller adds).
#[allow(clippy::type_complexity)]
pub async fn sum_usage_by_user(pool: &DbPool) -> QueryResult<Vec<(Option<i64>, UsageTotals)>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let rows: Vec<(Option<i64>, Option<i64>, Option<i64>, Option<i64>, i64)> = ai_usage::table
        .group_by(ai_usage::user_id)
        .select((
            ai_usage::user_id,
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(cost_micros)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(input_tokens)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(output_tokens)"),
            diesel::dsl::count_star(),
        ))
        .load(&mut conn)
        .await?;

    let mut totals: Vec<_> = rows
        .into_iter()
        .map(|(user_id, cost, input, output, turns)| {
            totals_row(user_id, cost, input, output, turns)
        })
        .collect();
    totals.sort_by_key(|entry| std::cmp::Reverse(entry.1.cost_micros));
    Ok(totals)
}

/// Global totals per day over the last `days` days, oldest first - "spend
/// over time" for the operator dashboard's own chart. A day with no usage
/// at all is simply absent rather than a zeroed row; the caller fills any
/// gap it cares about rendering.
#[allow(clippy::type_complexity)]
pub async fn sum_usage_daily(
    pool: &DbPool,
    days: i64,
) -> QueryResult<Vec<(NaiveDate, UsageTotals)>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    let since = (Utc::now() - Duration::days(days)).naive_utc();

    let rows: Vec<(NaiveDate, Option<i64>, Option<i64>, Option<i64>, i64)> = ai_usage::table
        .filter(ai_usage::created_at.ge(since))
        .group_by(diesel::dsl::sql::<Date>("DATE(created_at)"))
        .select((
            diesel::dsl::sql::<Date>("DATE(created_at)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(cost_micros)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(input_tokens)"),
            diesel::dsl::sql::<Nullable<BigInt>>("SUM(output_tokens)"),
            diesel::dsl::count_star(),
        ))
        .load(&mut conn)
        .await?;

    let mut totals: Vec<_> = rows
        .into_iter()
        .map(|(date, cost, input, output, turns)| totals_row(date, cost, input, output, turns))
        .collect();
    totals.sort_by_key(|(date, _)| *date);
    Ok(totals)
}
