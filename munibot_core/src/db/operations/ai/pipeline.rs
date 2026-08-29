//! Database operations for `ai_pipelines` and `ai_pipeline_events`.
//!
//! Free async functions taking `&DbPool` and returning `QueryResult<T>`,
//! matching every other submodule here. `munibot_ai::pipeline::store` is
//! the layer above this that actually folds an event log into a
//! `PipelineState` -- this module only ever moves rows.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{
    DbPool,
    models::{AiPipeline, AiPipelineEvent, NewAiPipeline, NewAiPipelineEvent},
    schema::{ai_pipeline_events, ai_pipelines},
};

diesel::define_sql_function!(fn last_insert_id() -> diesel::sql_types::Unsigned<diesel::sql_types::Bigint>);

/// Creates a new pipeline run row, identifying the issue that triggered
/// it.
pub async fn create_pipeline(
    pool: &DbPool,
    forge: &str,
    owner: &str,
    repo_name: &str,
    issue_number: u64,
) -> QueryResult<AiPipeline> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    diesel::insert_into(ai_pipelines::table)
        .values(NewAiPipeline {
            forge: forge.to_owned(),
            owner: owner.to_owned(),
            repo_name: repo_name.to_owned(),
            issue_number,
            created_at: chrono::Utc::now().naive_utc(),
        })
        .execute(&mut conn)
        .await?;

    let id: u64 = diesel::select(last_insert_id())
        .get_result(&mut conn)
        .await?;
    ai_pipelines::table
        .find(id as i64)
        .select(AiPipeline::as_select())
        .first(&mut conn)
        .await
}

/// Looks a pipeline up by id.
pub async fn get_pipeline(pool: &DbPool, pipeline_id: i64) -> QueryResult<Option<AiPipeline>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_pipelines::table
        .find(pipeline_id)
        .select(AiPipeline::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Every pipeline ever created, in no particular order -- what resuming
/// after a restart starts from.
pub async fn list_pipeline_ids(pool: &DbPool) -> QueryResult<Vec<i64>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_pipelines::table
        .select(ai_pipelines::id)
        .load(&mut conn)
        .await
}

/// Appends one event, assigning it the next sequence number in this
/// pipeline's own log.
///
/// The sequence number is `max(seq) + 1`, read and written without a lock
/// -- the same pattern `append_message` already uses for
/// `ai_messages.seq`, and the same reasoning applies: two concurrent
/// appends to the *same* pipeline could pick the same number, in which
/// case the unique index on `(pipeline_id, seq)` rejects the loser rather
/// than silently interleaving history. A pipeline is driven by one
/// executor loop at a time, so contention is close to nil in practice.
pub async fn append_pipeline_event(
    pool: &DbPool,
    pipeline_id: i64,
    event_type: &str,
    payload: &str,
) -> QueryResult<AiPipelineEvent> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    let next_seq = ai_pipeline_events::table
        .filter(ai_pipeline_events::pipeline_id.eq(pipeline_id))
        .select(diesel::dsl::max(ai_pipeline_events::seq))
        .first::<Option<i32>>(&mut conn)
        .await?
        .unwrap_or(-1)
        + 1;

    diesel::insert_into(ai_pipeline_events::table)
        .values(NewAiPipelineEvent {
            pipeline_id,
            seq: next_seq,
            event_type: event_type.to_owned(),
            payload: payload.to_owned(),
            created_at: chrono::Utc::now().naive_utc(),
        })
        .execute(&mut conn)
        .await?;

    let id: u64 = diesel::select(last_insert_id())
        .get_result(&mut conn)
        .await?;
    ai_pipeline_events::table
        .find(id as i64)
        .select(AiPipelineEvent::as_select())
        .first(&mut conn)
        .await
}

/// Every event in one pipeline's own log, in the order they were appended
/// -- `replay` folds exactly this sequence into a `PipelineState`.
pub async fn list_pipeline_events(
    pool: &DbPool,
    pipeline_id: i64,
) -> QueryResult<Vec<AiPipelineEvent>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_pipeline_events::table
        .filter(ai_pipeline_events::pipeline_id.eq(pipeline_id))
        .order(ai_pipeline_events::seq.asc())
        .select(AiPipelineEvent::as_select())
        .load(&mut conn)
        .await
}
