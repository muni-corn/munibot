//! Wire types for the pipeline monitor page.
//!
//! Distinct from `munibot_ai::pipeline`'s own types: those model the
//! pipeline's real state machine and event log in full, including data
//! (a `SubtaskDraft`, a diff's own text) nobody should ever have to
//! serialize to a browser tab. These are a deliberately thin, read-only
//! summary of them.

use serde::{Deserialize, Serialize};

/// One pipeline run, summarized for the monitor page's list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineSummary {
    pub id: i64,
    pub forge: String,
    pub owner: String,
    pub repo_name: String,
    pub issue_number: u64,
    /// A human-legible label for the current `PipelineState`, e.g.
    /// `"building"` or `"awaiting_user_input"`.
    pub state: String,
    /// The subtask id a subtask-scoped state names, if the current state
    /// is one.
    pub subtask: Option<String>,
    pub elapsed_seconds: i64,
    /// Whether this process is actually running this pipeline right now
    /// -- distinct from "not yet terminal", since a non-terminal pipeline
    /// nobody is currently driving forward (queued, or paused waiting on
    /// a human) is not the same as one actively spending tokens.
    pub running: bool,
}

/// One event in a pipeline's own log, summarized for the detail view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineEventSummary {
    pub seq: i32,
    pub event_type: String,
    pub created_at_unix: i64,
}

/// A pipeline's own summary, plus its full event log -- what the
/// per-pipeline detail view shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineDetail {
    pub summary: PipelineSummary,
    pub events: Vec<PipelineEventSummary>,
}
