//! Database operations for `ai_projects` and `ai_workspaces`.
//!
//! Free async functions taking `&DbPool` and returning `QueryResult<T>`,
//! matching every other submodule here. `munibot_ai::project` (a later
//! commit) is the layer above this that actually resolves an image
//! strategy, clones a repository, and manages worktrees - this module only
//! ever moves rows.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{
    DbPool,
    models::{
        AiProject, AiWorkspace, NewAiProject, NewAiWorkspace, TouchAiWorkspace, UpdateAiProject,
    },
    schema::{ai_projects, ai_workspaces},
};

diesel::define_sql_function!(fn last_insert_id() -> diesel::sql_types::Unsigned<diesel::sql_types::Bigint>);

// ai_projects

/// Inserts a new project, or - on a `slug` collision - updates the
/// existing row with every field `project` carries.
/// `on_conflict(...).do_update()`, never `replace_into`: a whole-row `REPLACE
/// INTO` would silently reset columns this call doesn't know about back to
/// their defaults on a concurrent registration race, the same trap
/// `operations.rs`'s own note documents for `guild_configs`.
///
/// Registration (a later commit) calls this so that registering the same
/// repository twice - a retried webhook delivery, an operator resubmitting
/// the same form - is idempotent rather than erroring on a duplicate slug.
pub async fn upsert_project(pool: &DbPool, project: &NewAiProject) -> QueryResult<AiProject> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    diesel::insert_into(ai_projects::table)
        .values(project)
        .on_conflict(diesel::dsl::DuplicatedKeys)
        .do_update()
        .set(project)
        .execute(&mut conn)
        .await?;

    ai_projects::table
        .filter(ai_projects::slug.eq(&project.slug))
        .select(AiProject::as_select())
        .first(&mut conn)
        .await
}

/// Looks a project up by id.
pub async fn get_project(pool: &DbPool, project_id: i64) -> QueryResult<Option<AiProject>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_projects::table
        .find(project_id)
        .select(AiProject::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Looks a project up by its unique slug - what a webhook delivery or the
/// projects page resolves a human-chosen name through.
pub async fn get_project_by_slug(pool: &DbPool, slug: &str) -> QueryResult<Option<AiProject>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_projects::table
        .filter(ai_projects::slug.eq(slug))
        .select(AiProject::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Every registered project, most recently updated first - what the
/// projects page lists.
pub async fn list_projects(pool: &DbPool) -> QueryResult<Vec<AiProject>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_projects::table
        .order(ai_projects::updated_at.desc())
        .select(AiProject::as_select())
        .load(&mut conn)
        .await
}

/// Applies a partial update - an image build result, a network or trigger
/// change - without touching identity fields. See [`UpdateAiProject`] for
/// exactly what can change after registration.
pub async fn update_project(
    pool: &DbPool,
    project_id: i64,
    changes: &UpdateAiProject,
) -> QueryResult<AiProject> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(ai_projects::table.find(project_id))
        .set(changes)
        .execute(&mut conn)
        .await?;
    ai_projects::table
        .find(project_id)
        .select(AiProject::as_select())
        .first(&mut conn)
        .await
}

/// Removes a project and, via `ai_workspaces`' own `ON DELETE CASCADE`,
/// every workspace under it. Removing the rows here is the easy part -
/// the caller (the project manager, a later commit) is responsible for
/// removing the on-disk clone and worktrees first, and for refusing this
/// entirely while a pipeline is running against the project.
pub async fn delete_project(pool: &DbPool, project_id: i64) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::delete(ai_projects::table.find(project_id))
        .execute(&mut conn)
        .await?;
    Ok(())
}

// ai_workspaces

/// Creates a new workspace row under a project. Fails on a duplicate
/// `(project_id, name)` - unlike [`upsert_project`], a workspace's own
/// creation is not upserted: the caller (worktree management, a later
/// commit) checks whether one already exists first, since a name
/// collision there means reusing the existing worktree, not overwriting
/// its row.
pub async fn create_workspace(
    pool: &DbPool,
    workspace: &NewAiWorkspace,
) -> QueryResult<AiWorkspace> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    diesel::insert_into(ai_workspaces::table)
        .values(workspace)
        .execute(&mut conn)
        .await?;

    let id: u64 = diesel::select(last_insert_id())
        .get_result(&mut conn)
        .await?;
    ai_workspaces::table
        .find(id as i64)
        .select(AiWorkspace::as_select())
        .first(&mut conn)
        .await
}

/// Looks a workspace up by id.
pub async fn get_workspace(pool: &DbPool, workspace_id: i64) -> QueryResult<Option<AiWorkspace>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_workspaces::table
        .find(workspace_id)
        .select(AiWorkspace::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Looks a workspace up by its name within one project - what
/// `workspace_for` (a later commit) checks before deciding whether to
/// create a fresh worktree or reuse an existing one.
pub async fn get_workspace_by_name(
    pool: &DbPool,
    project_id: i64,
    name: &str,
) -> QueryResult<Option<AiWorkspace>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_workspaces::table
        .filter(ai_workspaces::project_id.eq(project_id))
        .filter(ai_workspaces::name.eq(name))
        .select(AiWorkspace::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Every workspace under one project - what a project's detail view
/// lists, and what `prune` (a later commit) reconciles against what is
/// actually on disk.
pub async fn list_workspaces_for_project(
    pool: &DbPool,
    project_id: i64,
) -> QueryResult<Vec<AiWorkspace>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_workspaces::table
        .filter(ai_workspaces::project_id.eq(project_id))
        .order(ai_workspaces::id.asc())
        .select(AiWorkspace::as_select())
        .load(&mut conn)
        .await
}

/// Updates a workspace's `last_used_at` - the only mutation a live session
/// performs on an existing row, and what the workspace-collection sweep
/// (a later commit) compares `workspace_ttl` against.
pub async fn touch_workspace(
    pool: &DbPool,
    workspace_id: i64,
    now: chrono::NaiveDateTime,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(ai_workspaces::table.find(workspace_id))
        .set(&TouchAiWorkspace { last_used_at: now })
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// Removes a workspace row. The caller is responsible for removing the
/// worktree and its administrative record on disk first - see
/// `docs/plans/ai/milestone-7-projects.md`'s ordering note on why a crash
/// mid-collection must leave something `prune` can reconcile rather than
/// an orphan.
pub async fn delete_workspace(pool: &DbPool, workspace_id: i64) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::delete(ai_workspaces::table.find(workspace_id))
        .execute(&mut conn)
        .await?;
    Ok(())
}
