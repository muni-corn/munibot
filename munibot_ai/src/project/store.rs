//! `ProjectStore`: persisting projects and their workspaces. Mirrors
//! `crate::pipeline::store::PipelineStore`'s shape - a trait, an in-memory
//! fake for tests, and a diesel implementation backed by `ai_projects`/
//! `ai_workspaces`.

use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Mutex};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use munibot_core::db::{
    DbPool,
    models::{AiProject, AiWorkspace, NewAiProject, NewAiWorkspace, UpdateAiProject},
    operations::ai as core_ai,
};
use munibot_vcs::{Forge, RepoRef, TriggerMode};

use crate::{
    project::{
        ImageStatus, ImageStrategy, NewProject, NewWorkspace, Project, ProjectError, ProjectId,
        Workspace, WorkspaceId,
    },
    sandbox::config::SandboxNetworkConfig,
};

/// Persists projects and their workspaces.
#[async_trait]
pub trait ProjectStore: Send + Sync {
    /// Inserts a new project, or - on a slug collision - updates the
    /// existing row with `spec`'s own fields. See
    /// `munibot_core::db::operations::ai::upsert_project` for why this is
    /// idempotent rather than erroring on a repeated registration.
    async fn upsert_project(&self, spec: &NewProject) -> Result<Project, ProjectError>;

    /// Looks a project up by id.
    async fn get_project(&self, id: ProjectId) -> Result<Project, ProjectError>;

    /// Looks a project up by its unique slug.
    async fn get_project_by_slug(&self, slug: &str) -> Result<Option<Project>, ProjectError>;

    /// Every registered project.
    async fn list_projects(&self) -> Result<Vec<Project>, ProjectError>;

    /// Records the result of an image build attempt.
    async fn update_image_status(
        &self,
        id: ProjectId,
        strategy: ImageStrategy,
        status: ImageStatus,
    ) -> Result<Project, ProjectError>;

    /// Removes a project and every workspace under it.
    async fn delete_project(&self, id: ProjectId) -> Result<(), ProjectError>;

    /// Creates a new workspace under a project.
    async fn create_workspace(&self, spec: &NewWorkspace) -> Result<Workspace, ProjectError>;

    /// Looks a workspace up by id.
    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace, ProjectError>;

    /// Looks a workspace up by its name within one project.
    async fn get_workspace_by_name(
        &self,
        project_id: ProjectId,
        name: &str,
    ) -> Result<Option<Workspace>, ProjectError>;

    /// Every workspace under one project.
    async fn list_workspaces_for_project(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<Workspace>, ProjectError>;

    /// Marks a workspace as just used - what a session records every time
    /// it actually touches a workspace, and what the collection sweep
    /// (a later commit) compares `workspace_ttl` against.
    async fn touch_workspace(&self, id: WorkspaceId) -> Result<(), ProjectError>;

    /// Removes a workspace.
    async fn delete_workspace(&self, id: WorkspaceId) -> Result<(), ProjectError>;
}

/// An in-memory [`ProjectStore`], for tests - see [`DieselProjectStore`]
/// for the production implementation.
#[derive(Default)]
pub struct InMemoryProjectStore {
    next_project_id: Mutex<i64>,
    next_workspace_id: Mutex<i64>,
    projects: Mutex<HashMap<ProjectId, Project>>,
    workspaces: Mutex<HashMap<WorkspaceId, Workspace>>,
}

impl InMemoryProjectStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Backdates a workspace's `last_used_at` directly - there is no
    /// production path that would ever do this (a real session only ever
    /// moves it forward, via [`ProjectStore::touch_workspace`]), so this
    /// exists purely for tests that need to simulate a workspace old
    /// enough for the collection sweep (`crate::project::collect`) to
    /// actually act on.
    #[cfg(test)]
    pub(crate) fn force_last_used_at_for_test(&self, id: WorkspaceId, at: chrono::DateTime<Utc>) {
        if let Some(workspace) = self
            .workspaces
            .lock()
            .expect("workspace store lock poisoned")
            .get_mut(&id)
        {
            workspace.last_used_at = at;
        }
    }
}

#[async_trait]
impl ProjectStore for InMemoryProjectStore {
    async fn upsert_project(&self, spec: &NewProject) -> Result<Project, ProjectError> {
        let mut projects = self.projects.lock().expect("project store lock poisoned");

        if let Some(existing) = projects.values_mut().find(|p| p.slug == spec.slug) {
            existing.display_name = spec.display_name.clone();
            existing.repo = spec.repo.clone();
            existing.clone_url = spec.clone_url.clone();
            existing.default_branch = spec.default_branch.clone();
            existing.local_path = spec.local_path.clone();
            existing.image_strategy = spec.image_strategy;
            existing.network_policy = spec.network_policy;
            existing.trigger = spec.trigger.clone();
            existing.updated_at = Utc::now();
            return Ok(existing.clone());
        }

        let mut next_id = self.next_project_id.lock().expect("lock poisoned");
        *next_id += 1;
        let now = Utc::now();
        let project = Project {
            id: ProjectId(*next_id),
            slug: spec.slug.clone(),
            display_name: spec.display_name.clone(),
            repo: spec.repo.clone(),
            clone_url: spec.clone_url.clone(),
            default_branch: spec.default_branch.clone(),
            local_path: spec.local_path.clone(),
            image_strategy: spec.image_strategy,
            image_status: ImageStatus::Unbuilt,
            network_policy: spec.network_policy,
            trigger: spec.trigger.clone(),
            created_at: now,
            updated_at: now,
        };
        projects.insert(project.id, project.clone());
        Ok(project)
    }

    async fn get_project(&self, id: ProjectId) -> Result<Project, ProjectError> {
        self.projects
            .lock()
            .expect("project store lock poisoned")
            .get(&id)
            .cloned()
            .ok_or(ProjectError::NotFound(id))
    }

    async fn get_project_by_slug(&self, slug: &str) -> Result<Option<Project>, ProjectError> {
        Ok(self
            .projects
            .lock()
            .expect("project store lock poisoned")
            .values()
            .find(|p| p.slug == slug)
            .cloned())
    }

    async fn list_projects(&self) -> Result<Vec<Project>, ProjectError> {
        Ok(self
            .projects
            .lock()
            .expect("project store lock poisoned")
            .values()
            .cloned()
            .collect())
    }

    async fn update_image_status(
        &self,
        id: ProjectId,
        strategy: ImageStrategy,
        status: ImageStatus,
    ) -> Result<Project, ProjectError> {
        let mut projects = self.projects.lock().expect("project store lock poisoned");
        let project = projects.get_mut(&id).ok_or(ProjectError::NotFound(id))?;
        project.image_strategy = strategy;
        project.image_status = status;
        project.updated_at = Utc::now();
        Ok(project.clone())
    }

    async fn delete_project(&self, id: ProjectId) -> Result<(), ProjectError> {
        self.projects
            .lock()
            .expect("project store lock poisoned")
            .remove(&id);
        self.workspaces
            .lock()
            .expect("workspace store lock poisoned")
            .retain(|_, workspace| workspace.project_id != id);
        Ok(())
    }

    async fn create_workspace(&self, spec: &NewWorkspace) -> Result<Workspace, ProjectError> {
        let mut workspaces = self
            .workspaces
            .lock()
            .expect("workspace store lock poisoned");
        if workspaces
            .values()
            .any(|w| w.project_id == spec.project_id && w.name == spec.name)
        {
            return Err(ProjectError::Other(format!(
                "a workspace named {:?} already exists under project {:?}",
                spec.name, spec.project_id
            )));
        }

        let mut next_id = self.next_workspace_id.lock().expect("lock poisoned");
        *next_id += 1;
        let now = Utc::now();
        let workspace = Workspace {
            id: WorkspaceId(*next_id),
            project_id: spec.project_id,
            name: spec.name.clone(),
            branch: spec.branch.clone(),
            path: spec.path.clone(),
            pipeline_id: spec.pipeline_id,
            conversation_id: spec.conversation_id,
            created_at: now,
            last_used_at: now,
        };
        workspaces.insert(workspace.id, workspace.clone());
        Ok(workspace)
    }

    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace, ProjectError> {
        self.workspaces
            .lock()
            .expect("workspace store lock poisoned")
            .get(&id)
            .cloned()
            .ok_or_else(|| ProjectError::Other(format!("workspace {id:?} wasn't found :<")))
    }

    async fn get_workspace_by_name(
        &self,
        project_id: ProjectId,
        name: &str,
    ) -> Result<Option<Workspace>, ProjectError> {
        Ok(self
            .workspaces
            .lock()
            .expect("workspace store lock poisoned")
            .values()
            .find(|w| w.project_id == project_id && w.name == name)
            .cloned())
    }

    async fn list_workspaces_for_project(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<Workspace>, ProjectError> {
        Ok(self
            .workspaces
            .lock()
            .expect("workspace store lock poisoned")
            .values()
            .filter(|w| w.project_id == project_id)
            .cloned()
            .collect())
    }

    async fn touch_workspace(&self, id: WorkspaceId) -> Result<(), ProjectError> {
        let mut workspaces = self
            .workspaces
            .lock()
            .expect("workspace store lock poisoned");
        let workspace = workspaces
            .get_mut(&id)
            .ok_or_else(|| ProjectError::Other(format!("workspace {id:?} wasn't found :<")))?;
        workspace.last_used_at = Utc::now();
        Ok(())
    }

    async fn delete_workspace(&self, id: WorkspaceId) -> Result<(), ProjectError> {
        self.workspaces
            .lock()
            .expect("workspace store lock poisoned")
            .remove(&id);
        Ok(())
    }
}

/// The production [`ProjectStore`], backed by `ai_projects` and
/// `ai_workspaces`.
pub struct DieselProjectStore {
    pool: DbPool,
}

impl DieselProjectStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

/// The trigger discriminant string stored in `ai_projects.trigger_mode` -
/// only the flat variants of `TriggerMode` are representable in two
/// columns; `TriggerMode::Any` has no flat form, so persisting one is a
/// caller error rather than silently dropped.
fn trigger_to_columns(
    trigger: &Option<TriggerMode>,
) -> Result<(Option<String>, Option<String>), ProjectError> {
    match trigger {
        None => Ok((None, None)),
        Some(TriggerMode::AllIssues) => Ok((Some("all_issues".to_string()), None)),
        Some(TriggerMode::Label(label)) => Ok((Some("label".to_string()), Some(label.clone()))),
        Some(TriggerMode::Keyword(keyword)) => {
            Ok((Some("keyword".to_string()), Some(keyword.clone())))
        }
        Some(TriggerMode::Any(_)) => Err(ProjectError::Other(
            "TriggerMode::Any isn't representable in ai_projects' flat trigger columns yet"
                .to_string(),
        )),
    }
}

fn columns_to_trigger(
    mode: Option<&str>,
    label: Option<&str>,
) -> Result<Option<TriggerMode>, ProjectError> {
    match mode {
        None => Ok(None),
        Some("all_issues") => Ok(Some(TriggerMode::AllIssues)),
        Some("label") => Ok(Some(TriggerMode::Label(
            label
                .ok_or_else(|| {
                    ProjectError::Other("trigger_mode 'label' with no trigger_label".to_string())
                })?
                .to_string(),
        ))),
        Some("keyword") => Ok(Some(TriggerMode::Keyword(
            label
                .ok_or_else(|| {
                    ProjectError::Other("trigger_mode 'keyword' with no trigger_label".to_string())
                })?
                .to_string(),
        ))),
        Some(other) => Err(ProjectError::Other(format!(
            "unknown trigger_mode {other:?} stored on a project row"
        ))),
    }
}

fn image_status_to_columns(
    status: &ImageStatus,
) -> (
    Option<String>,
    Option<String>,
    Option<NaiveDateTime>,
    Option<String>,
) {
    match status {
        ImageStatus::Unbuilt => (None, None, None, None),
        ImageStatus::Built { tag, hash, at } => (
            Some(tag.clone()),
            Some(hash.clone()),
            Some(at.naive_utc()),
            None,
        ),
        ImageStatus::Failed { reason } => (None, None, None, Some(reason.clone())),
    }
}

fn columns_to_image_status(
    tag: Option<String>,
    hash: Option<String>,
    built_at: Option<NaiveDateTime>,
    error: Option<String>,
) -> Result<ImageStatus, ProjectError> {
    match (tag, hash, built_at, error) {
        (None, None, None, None) => Ok(ImageStatus::Unbuilt),
        (Some(tag), Some(hash), Some(at), None) => Ok(ImageStatus::Built {
            tag,
            hash,
            at: DateTime::<Utc>::from_naive_utc_and_offset(at, Utc),
        }),
        (None, None, None, Some(reason)) => Ok(ImageStatus::Failed { reason }),
        other => Err(ProjectError::Other(format!(
            "a project row's image status columns are not a legal combination: {other:?}"
        ))),
    }
}

fn row_to_project(row: AiProject) -> Result<Project, ProjectError> {
    let forge = match row.forge.as_str() {
        "github" => Forge::GitHub,
        other => {
            return Err(ProjectError::Other(format!(
                "unknown forge {other:?} stored for project {:?}",
                row.id
            )));
        }
    };
    let image_strategy = ImageStrategy::from_str(&row.image_strategy).map_err(|_| {
        ProjectError::Other(format!(
            "unknown image_strategy {:?} stored for project {:?}",
            row.image_strategy, row.id
        ))
    })?;
    let network_policy = SandboxNetworkConfig::from_str(&row.network_policy).map_err(|_| {
        ProjectError::Other(format!(
            "unknown network_policy {:?} stored for project {:?}",
            row.network_policy, row.id
        ))
    })?;
    let image_status = columns_to_image_status(
        row.image_tag,
        row.image_source_hash,
        row.image_built_at,
        row.image_error,
    )?;
    let trigger = columns_to_trigger(row.trigger_mode.as_deref(), row.trigger_label.as_deref())?;

    Ok(Project {
        id: ProjectId(row.id),
        slug: row.slug,
        display_name: row.display_name,
        repo: RepoRef::new(forge, row.repo_owner, row.repo_name),
        clone_url: row.clone_url,
        default_branch: row.default_branch,
        local_path: PathBuf::from(row.local_path),
        image_strategy,
        image_status,
        network_policy,
        trigger,
        created_at: DateTime::<Utc>::from_naive_utc_and_offset(row.created_at, Utc),
        updated_at: DateTime::<Utc>::from_naive_utc_and_offset(row.updated_at, Utc),
    })
}

fn row_to_workspace(row: AiWorkspace) -> Workspace {
    Workspace {
        id: WorkspaceId(row.id),
        project_id: ProjectId(row.project_id),
        name: row.name,
        branch: row.branch,
        path: PathBuf::from(row.path),
        pipeline_id: row.pipeline_id,
        conversation_id: row.conversation_id,
        created_at: DateTime::<Utc>::from_naive_utc_and_offset(row.created_at, Utc),
        last_used_at: DateTime::<Utc>::from_naive_utc_and_offset(row.last_used_at, Utc),
    }
}

#[async_trait]
impl ProjectStore for DieselProjectStore {
    async fn upsert_project(&self, spec: &NewProject) -> Result<Project, ProjectError> {
        let now = Utc::now().naive_utc();
        let (trigger_mode, trigger_label) = trigger_to_columns(&spec.trigger)?;

        let row = core_ai::upsert_project(&self.pool, &NewAiProject {
            slug: spec.slug.clone(),
            display_name: spec.display_name.clone(),
            forge: spec.repo.forge.to_string(),
            repo_owner: spec.repo.owner.clone(),
            repo_name: spec.repo.name.clone(),
            clone_url: spec.clone_url.clone(),
            default_branch: spec.default_branch.clone(),
            local_path: spec.local_path.to_string_lossy().into_owned(),
            image_strategy: spec.image_strategy.to_string(),
            image_tag: None,
            image_source_hash: None,
            image_built_at: None,
            image_error: None,
            network_policy: spec.network_policy.to_string(),
            trigger_mode,
            trigger_label,
            created_at: now,
            updated_at: now,
        })
        .await
        .map_err(|error| ProjectError::Other(error.to_string()))?;

        row_to_project(row)
    }

    async fn get_project(&self, id: ProjectId) -> Result<Project, ProjectError> {
        let row = core_ai::get_project(&self.pool, id.0)
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))?
            .ok_or(ProjectError::NotFound(id))?;
        row_to_project(row)
    }

    async fn get_project_by_slug(&self, slug: &str) -> Result<Option<Project>, ProjectError> {
        let Some(row) = core_ai::get_project_by_slug(&self.pool, slug)
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))?
        else {
            return Ok(None);
        };
        row_to_project(row).map(Some)
    }

    async fn list_projects(&self) -> Result<Vec<Project>, ProjectError> {
        core_ai::list_projects(&self.pool)
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))?
            .into_iter()
            .map(row_to_project)
            .collect()
    }

    async fn update_image_status(
        &self,
        id: ProjectId,
        strategy: ImageStrategy,
        status: ImageStatus,
    ) -> Result<Project, ProjectError> {
        let (image_tag, image_source_hash, image_built_at, image_error) =
            image_status_to_columns(&status);

        let row = core_ai::update_project(&self.pool, id.0, &UpdateAiProject {
            display_name: None,
            default_branch: None,
            image_strategy: Some(strategy.to_string()),
            image_tag: Some(image_tag),
            image_source_hash: Some(image_source_hash),
            image_built_at: Some(image_built_at),
            image_error: Some(image_error),
            network_policy: None,
            trigger_mode: None,
            trigger_label: None,
            updated_at: Utc::now().naive_utc(),
        })
        .await
        .map_err(|error| ProjectError::Other(error.to_string()))?;

        row_to_project(row)
    }

    async fn delete_project(&self, id: ProjectId) -> Result<(), ProjectError> {
        core_ai::delete_project(&self.pool, id.0)
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))
    }

    async fn create_workspace(&self, spec: &NewWorkspace) -> Result<Workspace, ProjectError> {
        let now = Utc::now().naive_utc();
        let row = core_ai::create_workspace(&self.pool, &NewAiWorkspace {
            project_id: spec.project_id.0,
            name: spec.name.clone(),
            branch: spec.branch.clone(),
            path: spec.path.to_string_lossy().into_owned(),
            pipeline_id: spec.pipeline_id,
            conversation_id: spec.conversation_id,
            created_at: now,
            last_used_at: now,
        })
        .await
        .map_err(|error| ProjectError::Other(error.to_string()))?;

        Ok(row_to_workspace(row))
    }

    async fn get_workspace(&self, id: WorkspaceId) -> Result<Workspace, ProjectError> {
        let row = core_ai::get_workspace(&self.pool, id.0)
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))?
            .ok_or_else(|| ProjectError::Other(format!("workspace {id:?} wasn't found :<")))?;
        Ok(row_to_workspace(row))
    }

    async fn get_workspace_by_name(
        &self,
        project_id: ProjectId,
        name: &str,
    ) -> Result<Option<Workspace>, ProjectError> {
        let row = core_ai::get_workspace_by_name(&self.pool, project_id.0, name)
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))?;
        Ok(row.map(row_to_workspace))
    }

    async fn list_workspaces_for_project(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<Workspace>, ProjectError> {
        let rows = core_ai::list_workspaces_for_project(&self.pool, project_id.0)
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))?;
        Ok(rows.into_iter().map(row_to_workspace).collect())
    }

    async fn touch_workspace(&self, id: WorkspaceId) -> Result<(), ProjectError> {
        core_ai::touch_workspace(&self.pool, id.0, Utc::now().naive_utc())
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))
    }

    async fn delete_workspace(&self, id: WorkspaceId) -> Result<(), ProjectError> {
        core_ai::delete_workspace(&self.pool, id.0)
            .await
            .map_err(|error| ProjectError::Other(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(slug: &str) -> NewProject {
        NewProject {
            slug: slug.to_string(),
            display_name: "Cocoa".to_string(),
            repo: RepoRef::new(Forge::GitHub, "muni-corn", "cocoa"),
            clone_url: "https://github.com/muni-corn/cocoa.git".to_string(),
            default_branch: "main".to_string(),
            local_path: PathBuf::from("/var/lib/munibot/projects/1/cocoa"),
            image_strategy: ImageStrategy::Base,
            network_policy: SandboxNetworkConfig::None,
            trigger: None,
        }
    }

    #[tokio::test]
    async fn test_upsert_project_creates_a_new_row() {
        let store = InMemoryProjectStore::new();
        let created = store.upsert_project(&spec("cocoa")).await.unwrap();
        assert_eq!(created.slug, "cocoa");
        assert_eq!(created.image_status, ImageStatus::Unbuilt);
    }

    #[tokio::test]
    async fn test_upsert_project_on_a_slug_collision_updates_the_same_row() {
        let store = InMemoryProjectStore::new();
        let first = store.upsert_project(&spec("cocoa")).await.unwrap();

        let mut second = spec("cocoa");
        second.display_name = "Cocoa (renamed)".to_string();
        let updated = store.upsert_project(&second).await.unwrap();

        assert_eq!(updated.id, first.id);
        assert_eq!(updated.display_name, "Cocoa (renamed)");
        assert_eq!(store.list_projects().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_get_project_missing_is_not_found() {
        let store = InMemoryProjectStore::new();
        let error = store.get_project(ProjectId(999)).await.unwrap_err();
        assert!(matches!(error, ProjectError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_get_project_by_slug_missing_returns_none() {
        let store = InMemoryProjectStore::new();
        assert!(
            store
                .get_project_by_slug("does-not-exist")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_update_image_status_persists_a_built_status() {
        let store = InMemoryProjectStore::new();
        let project = store.upsert_project(&spec("cocoa")).await.unwrap();

        let updated = store
            .update_image_status(project.id, ImageStrategy::Devenv, ImageStatus::Built {
                tag: "munibot-project-cocoa:abc123".to_string(),
                hash: "abc123".to_string(),
                at: Utc::now(),
            })
            .await
            .unwrap();

        assert_eq!(updated.image_strategy, ImageStrategy::Devenv);
        assert!(matches!(updated.image_status, ImageStatus::Built { .. }));
    }

    #[tokio::test]
    async fn test_delete_project_cascades_to_its_workspaces() {
        let store = InMemoryProjectStore::new();
        let project = store.upsert_project(&spec("cocoa")).await.unwrap();
        let workspace = store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name: "scratch".to_string(),
                branch: "munibot/scratch".to_string(),
                path: PathBuf::from("/var/lib/munibot/projects/1/cocoa/worktrees/scratch"),
                pipeline_id: None,
                conversation_id: None,
            })
            .await
            .unwrap();

        store.delete_project(project.id).await.unwrap();

        let error = store.get_workspace(workspace.id).await.unwrap_err();
        assert!(matches!(error, ProjectError::Other(_)));
    }

    #[tokio::test]
    async fn test_create_workspace_rejects_a_duplicate_name_within_the_same_project() {
        let store = InMemoryProjectStore::new();
        let project = store.upsert_project(&spec("cocoa")).await.unwrap();
        let workspace_spec = NewWorkspace {
            project_id: project.id,
            name: "scratch".to_string(),
            branch: "munibot/scratch".to_string(),
            path: PathBuf::from("/var/lib/munibot/projects/1/cocoa/worktrees/scratch"),
            pipeline_id: None,
            conversation_id: None,
        };
        store.create_workspace(&workspace_spec).await.unwrap();

        let error = store
            .create_workspace(&workspace_spec)
            .await
            .expect_err("a duplicate name within the same project should be rejected");
        assert!(matches!(error, ProjectError::Other(_)));
    }

    #[tokio::test]
    async fn test_get_workspace_by_name_finds_a_created_row() {
        let store = InMemoryProjectStore::new();
        let project = store.upsert_project(&spec("cocoa")).await.unwrap();
        store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name: "pipeline-142".to_string(),
                branch: "munibot/issue-17".to_string(),
                path: PathBuf::from("/var/lib/munibot/projects/1/cocoa/worktrees/pipeline-142"),
                pipeline_id: Some(142),
                conversation_id: None,
            })
            .await
            .unwrap();

        let found = store
            .get_workspace_by_name(project.id, "pipeline-142")
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(found.pipeline_id, Some(142));
    }

    #[tokio::test]
    async fn test_list_workspaces_for_project_only_lists_that_projects_workspaces() {
        let store = InMemoryProjectStore::new();
        let cocoa = store.upsert_project(&spec("cocoa")).await.unwrap();
        let munibot = store.upsert_project(&spec("munibot")).await.unwrap();

        store
            .create_workspace(&NewWorkspace {
                project_id: cocoa.id,
                name: "scratch".to_string(),
                branch: "munibot/scratch".to_string(),
                path: PathBuf::from("/scratch"),
                pipeline_id: None,
                conversation_id: None,
            })
            .await
            .unwrap();
        store
            .create_workspace(&NewWorkspace {
                project_id: munibot.id,
                name: "scratch".to_string(),
                branch: "munibot/scratch".to_string(),
                path: PathBuf::from("/scratch"),
                pipeline_id: None,
                conversation_id: None,
            })
            .await
            .unwrap();

        let cocoa_workspaces = store.list_workspaces_for_project(cocoa.id).await.unwrap();
        assert_eq!(cocoa_workspaces.len(), 1);
        assert_eq!(cocoa_workspaces[0].project_id, cocoa.id);
    }

    #[tokio::test]
    async fn test_touch_workspace_updates_last_used_at() {
        let store = InMemoryProjectStore::new();
        let project = store.upsert_project(&spec("cocoa")).await.unwrap();
        let workspace = store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name: "scratch".to_string(),
                branch: "munibot/scratch".to_string(),
                path: PathBuf::from("/scratch"),
                pipeline_id: None,
                conversation_id: None,
            })
            .await
            .unwrap();
        let before = workspace.last_used_at;

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        store.touch_workspace(workspace.id).await.unwrap();

        let after = store.get_workspace(workspace.id).await.unwrap();
        assert!(after.last_used_at > before);
    }

    #[tokio::test]
    async fn test_trigger_round_trips_through_columns() {
        for trigger in [
            None,
            Some(TriggerMode::AllIssues),
            Some(TriggerMode::Label("ai-pipeline".to_string())),
            Some(TriggerMode::Keyword("crash".to_string())),
        ] {
            let (mode, label) = trigger_to_columns(&trigger).unwrap();
            let parsed = columns_to_trigger(mode.as_deref(), label.as_deref()).unwrap();
            assert_eq!(trigger, parsed);
        }
    }

    #[test]
    fn test_trigger_any_is_rejected_rather_than_silently_dropped() {
        let trigger = Some(TriggerMode::Any(vec![TriggerMode::AllIssues]));
        let error = trigger_to_columns(&trigger).unwrap_err();
        assert!(matches!(error, ProjectError::Other(_)));
    }

    #[test]
    fn test_image_status_round_trips_through_columns() {
        let now = Utc::now();
        for status in [
            ImageStatus::Unbuilt,
            ImageStatus::Built {
                tag: "t".to_string(),
                hash: "h".to_string(),
                at: DateTime::<Utc>::from_naive_utc_and_offset(now.naive_utc(), Utc),
            },
            ImageStatus::Failed {
                reason: "r".to_string(),
            },
        ] {
            let (tag, hash, at, error) = image_status_to_columns(&status);
            let parsed = columns_to_image_status(tag, hash, at, error).unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn test_image_status_columns_reject_an_illegal_combination() {
        let error = columns_to_image_status(Some("tag".to_string()), None, None, None)
            .expect_err("a tag with no hash/built_at is not a legal Built status");
        assert!(matches!(error, ProjectError::Other(_)));
    }
}
