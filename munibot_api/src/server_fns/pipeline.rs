use dioxus::prelude::*;

// only the server actually constructs a ChatError or a PipelineEventSummary
// -- the client's own stub never runs a server function's body, so
// importing these unconditionally would warn as unused when compiling for
// web, the same reasoning `chat::stream::chat_stream` already documents
#[cfg(feature = "server")]
use crate::chat::ChatError;
#[cfg(feature = "server")]
use crate::pipeline::PipelineEventSummary;
use crate::{
    chat::ChatResult,
    pipeline::{PipelineDetail, PipelineSummary},
};

/// How often the pipeline monitor's own SSE stream re-checks every
/// pipeline's state. A live pipeline run is measured in minutes to hours,
/// not milliseconds, so this trades a little latency for not hammering
/// the database on every browser tab watching the page.
#[cfg(feature = "server")]
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// Builds one pipeline's summary from its own db row and event log,
/// folding the log through `advance` to get its current state -- exactly
/// what `PipelineStore::replay` does, called directly here since the
/// monitor page has no other reason to hold a whole `PipelineStore`.
#[cfg(feature = "server")]
fn summarize(
    row: munibot_core::db::models::AiPipeline,
    events: &[munibot_ai::pipeline::PipelineEvent],
    registry: &munibot_ai::pipeline::PipelineRegistry,
) -> ChatResult<PipelineSummary> {
    let state = events
        .iter()
        .try_fold(
            munibot_ai::pipeline::PipelineState::Triaging,
            munibot_ai::pipeline::advance,
        )
        .map_err(|error| ChatError::from(anyhow::anyhow!(error.to_string())))?;

    let elapsed_seconds = (chrono::Utc::now().naive_utc() - row.created_at).num_seconds();

    Ok(PipelineSummary {
        id: row.id,
        forge: row.forge,
        owner: row.owner,
        repo_name: row.repo_name,
        issue_number: row.issue_number,
        state: state.label().to_string(),
        subtask: state.subtask().map(|subtask| subtask.0.clone()),
        elapsed_seconds,
        running: registry.is_running(munibot_ai::pipeline::PipelineId(row.id)),
    })
}

#[cfg(feature = "server")]
async fn load_summaries(
    pool: &munibot_core::db::DbPool,
    registry: &munibot_ai::pipeline::PipelineRegistry,
) -> ChatResult<Vec<PipelineSummary>> {
    use munibot_core::db::operations::ai;

    let rows = ai::list_pipelines(pool).await?;
    let mut summaries = Vec::with_capacity(rows.len());
    for row in rows {
        let events_rows = ai::list_pipeline_events(pool, row.id).await?;
        let events: Vec<munibot_ai::pipeline::PipelineEvent> = events_rows
            .iter()
            .map(|event_row| {
                serde_json::from_str(&event_row.payload)
                    .map_err(|error| ChatError::from(anyhow::anyhow!(error.to_string())))
            })
            .collect::<ChatResult<_>>()?;
        summaries.push(summarize(row, &events, registry)?);
    }
    Ok(summaries)
}

/// Every pipeline run munibot has ever started, most recently created
/// first. Restricted to operators: which repositories munibot is
/// autonomously changing, and how, is not everyone's business.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    registry: axum::extract::Extension<std::sync::Arc<munibot_ai::pipeline::PipelineRegistry>>,
)]
pub async fn list_pipelines() -> ChatResult<Vec<PipelineSummary>> {
    crate::auth::operator::require_operator(&auth).await?;
    load_summaries(&pool, &registry).await
}

/// One pipeline's own summary plus its full event log.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    registry: axum::extract::Extension<std::sync::Arc<munibot_ai::pipeline::PipelineRegistry>>,
)]
pub async fn get_pipeline_detail(pipeline_id: i64) -> ChatResult<PipelineDetail> {
    use munibot_core::db::operations::ai;

    crate::auth::operator::require_operator(&auth).await?;

    let row = ai::get_pipeline(&pool, pipeline_id)
        .await?
        .ok_or(ChatError::PipelineNotFound)?;
    let event_rows = ai::list_pipeline_events(&pool, pipeline_id).await?;

    let events: Vec<munibot_ai::pipeline::PipelineEvent> = event_rows
        .iter()
        .map(|event_row| {
            serde_json::from_str(&event_row.payload)
                .map_err(|error| ChatError::from(anyhow::anyhow!(error.to_string())))
        })
        .collect::<ChatResult<_>>()?;

    let summary = summarize(row, &events, &registry)?;
    let event_summaries = event_rows
        .into_iter()
        .map(|event_row| PipelineEventSummary {
            seq: event_row.seq,
            event_type: event_row.event_type,
            created_at_unix: event_row.created_at.and_utc().timestamp(),
        })
        .collect();

    Ok(PipelineDetail {
        summary,
        events: event_summaries,
    })
}

/// Aborts a running pipeline: cancels its own turn and stops its
/// container. Returns whether it was actually running -- aborting one
/// that already finished, or that this process was never running in the
/// first place (queued elsewhere, or resumed by a different process), is
/// not an error, just a no-op worth reporting back.
#[server(
    auth: crate::auth::server::AuthSession,
    registry: axum::extract::Extension<std::sync::Arc<munibot_ai::pipeline::PipelineRegistry>>,
)]
pub async fn abort_pipeline_action(pipeline_id: i64) -> ChatResult<bool> {
    crate::auth::operator::require_operator(&auth).await?;
    Ok(registry
        .abort_pipeline(munibot_ai::pipeline::PipelineId(pipeline_id))
        .await)
}

/// Streams a fresh pipeline list every [`POLL_INTERVAL`] -- see
/// `chat::stream::chat_stream`'s own doc comment for why this is a
/// `#[get]` route rather than `#[server]`: SSE reconnects trivially and
/// is readable in devtools, and this page only ever needs one direction.
#[get(
    "/api/ai/pipelines/stream",
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    registry: axum::extract::Extension<std::sync::Arc<munibot_ai::pipeline::PipelineRegistry>>,
)]
pub async fn pipeline_monitor_stream()
-> ChatResult<dioxus_fullstack::ServerEvents<Vec<PipelineSummary>>> {
    crate::auth::operator::require_operator(&auth).await?;

    let pool = pool.0.clone();
    let registry = registry.0.clone();

    let stream = futures::stream::unfold(
        (pool, registry, true),
        |(pool, registry, first)| async move {
            if !first {
                tokio::time::sleep(POLL_INTERVAL).await;
            }

            let snapshot = load_summaries(&pool, &registry)
                .await
                .unwrap_or_else(|error| {
                    tracing::error!(%error, "couldn't load a pipeline monitor snapshot");
                    Vec::new()
                });

            Some((
                Ok::<Vec<PipelineSummary>, axum::BoxError>(snapshot),
                (pool, registry, false),
            ))
        },
    );

    Ok(dioxus_fullstack::ServerEvents::from_stream(stream))
}
