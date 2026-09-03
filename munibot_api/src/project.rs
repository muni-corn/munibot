//! Wire types for the projects page.
//!
//! Distinct from `munibot_ai::project`'s own domain types, the same
//! reasoning `munibot_api::pipeline`'s own module doc comment gives for
//! `PipelineSummary`: a deliberately thin, read-only projection, never
//! the real thing.

use serde::{Deserialize, Serialize};

/// One registered project, summarized for the projects page's list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    /// `"owner/name"` - `munibot_vcs::RepoRef`'s own `Display` form.
    pub repo: String,
    /// The strum snake-case string form, e.g. `"containerfile"`.
    pub image_strategy: String,
    /// `"unbuilt"` | `"built"` | `"failed"` -
    /// `munibot_ai::project::ImageStatus::label`'s own form.
    pub image_status: String,
    pub image_tag: Option<String>,
    pub image_built_at_unix: Option<i64>,
    pub image_error: Option<String>,
    /// `"none"` | `"full"`.
    pub network_policy: String,
    /// A real, actionable sentence per unmet strategy, in resolution
    /// order - what the projects page shows verbatim so "why didn't
    /// munibot use my devenv" never requires reading a log. Empty when
    /// the first strategy tried (`Containerfile`) already applied.
    pub skip_reasons: Vec<String>,
    /// A human-readable description of this project's trigger
    /// configuration, e.g. `"every issue"` or `"labeled \"ai-pipeline\""` -
    /// `None` when nothing is configured yet.
    pub trigger: Option<String>,
    pub workspace_count: usize,
    pub created_at_unix: i64,
    pub updated_at_unix: i64,
}

/// One workspace, summarized for a project's detail view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub id: i64,
    pub name: String,
    pub branch: String,
    pub pipeline_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub last_used_at_unix: i64,
}

/// A project's own summary, plus every workspace under it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectDetail {
    pub summary: ProjectSummary,
    pub workspaces: Vec<WorkspaceSummary>,
}

/// Everything the projects page's register form collects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegisterProjectForm {
    pub slug: String,
    pub display_name: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub clone_url: String,
    pub default_branch: String,
}

/// One registered project, projected just far enough for the chat page's
/// project picker to list and label it - never the full `ProjectSummary`
/// `list_projects` returns, which is operator-only. Binding one's own
/// conversation to an already-registered project is not an operator
/// action (see `set_conversation_project`'s own doc comment), so listing
/// which projects exist to bind to cannot be either.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectPickerEntry {
    pub id: i64,
    pub display_name: String,
    /// `"unbuilt"` | `"built"` | `"failed"` - the same form
    /// `ProjectSummary::image_status` uses, shown next to a project's
    /// name so picking one that isn't ready yet is a visible choice, not
    /// a silent surprise on the next `coder` turn.
    pub image_status: String,
}
