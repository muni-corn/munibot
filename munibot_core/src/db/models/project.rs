//! Diesel models for `ai_projects` and `ai_workspaces`.
//!
//! Kept in their own module rather than growing `models/ai.rs` (already at
//! the file-size limit `AGENTS.md` calls for) or `models.rs` itself.

use chrono::NaiveDateTime;
use diesel::prelude::*;

use crate::db::schema::{ai_projects, ai_workspaces};

// ai_projects

/// A row in the `ai_projects` table: one project munibot has cloned - a git
/// repository, one directory on disk, one container image.
///
/// `image_tag`, `image_source_hash`, `image_built_at`, and `image_error`
/// together are the persisted form of `munibot_ai::project::ImageStatus` -
/// see the migration's own comment for the exact correspondence.
/// `trigger_mode`/`trigger_label` are a flattened projection of
/// `munibot_vcs::TriggerMode`, not that type's own serialization -
/// `TriggerMode::Any` has no representation in two flat columns, so a
/// project configured with one is a decision for the operations layer
/// (commit 209), not this model.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_projects)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiProject {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    pub forge: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub clone_url: String,
    pub default_branch: String,
    pub local_path: String,
    pub image_strategy: String,
    pub image_tag: Option<String>,
    pub image_source_hash: Option<String>,
    pub image_built_at: Option<NaiveDateTime>,
    pub image_error: Option<String>,
    pub network_policy: String,
    pub trigger_mode: Option<String>,
    pub trigger_label: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Insertable and upsertable shape for `ai_projects` - `upsert_project`
/// (see `operations::ai::project`) uses this both as the row to insert and,
/// on a `slug` collision, the changeset to update with via
/// `on_conflict(...).do_update()`, never `replace_into` - see
/// `operations.rs`'s own note on why `replace_into` is a trap for a table
/// with more than one concern's worth of columns.
#[derive(Clone, Debug, Insertable, AsChangeset)]
#[diesel(table_name = ai_projects)]
pub struct NewAiProject {
    pub slug: String,
    pub display_name: String,
    pub forge: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub clone_url: String,
    pub default_branch: String,
    pub local_path: String,
    pub image_strategy: String,
    pub image_tag: Option<String>,
    pub image_source_hash: Option<String>,
    pub image_built_at: Option<NaiveDateTime>,
    pub image_error: Option<String>,
    pub network_policy: String,
    pub trigger_mode: Option<String>,
    pub trigger_label: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Changeset for updating an existing `ai_projects` row - every field the
/// project manager (commit 220) or the projects page (commit 224) might
/// change after registration: image build results, network policy, and
/// trigger configuration. Identity fields (`slug`, `repo`, `clone_url`,
/// `local_path`) are deliberately absent - a project's identity and its
/// on-disk location are set once at registration and never edited.
#[derive(Clone, Debug, AsChangeset)]
#[diesel(table_name = ai_projects)]
pub struct UpdateAiProject {
    pub display_name: Option<String>,
    pub default_branch: Option<String>,
    pub image_strategy: Option<String>,
    pub image_tag: Option<Option<String>>,
    pub image_source_hash: Option<Option<String>>,
    pub image_built_at: Option<Option<NaiveDateTime>>,
    pub image_error: Option<Option<String>>,
    pub network_policy: Option<String>,
    pub trigger_mode: Option<Option<String>>,
    pub trigger_label: Option<Option<String>>,
    pub updated_at: NaiveDateTime,
}

// ai_workspaces

/// A row in the `ai_workspaces` table: one `git worktree` of a project, on
/// its own branch. Persistent - it survives a run, unlike the ephemeral
/// sandbox container over it.
#[derive(Clone, Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = ai_workspaces)]
#[diesel(check_for_backend(diesel::mysql::Mysql))]
pub struct AiWorkspace {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub branch: String,
    pub path: String,
    pub pipeline_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub created_at: NaiveDateTime,
    pub last_used_at: NaiveDateTime,
}

/// Insertable shape for `ai_workspaces`.
#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = ai_workspaces)]
pub struct NewAiWorkspace {
    pub project_id: i64,
    pub name: String,
    pub branch: String,
    pub path: String,
    pub pipeline_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub created_at: NaiveDateTime,
    pub last_used_at: NaiveDateTime,
}

/// Changeset for touching a workspace's `last_used_at` - the only mutation
/// a live session performs on an existing row; everything else about a
/// workspace is set once at creation.
#[derive(Clone, Debug, AsChangeset)]
#[diesel(table_name = ai_workspaces)]
pub struct TouchAiWorkspace {
    pub last_used_at: NaiveDateTime,
}
