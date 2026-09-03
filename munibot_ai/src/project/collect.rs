//! Collecting workspaces that have gone unused - the price of choosing
//! persistent workspaces over ephemeral ones.

use std::time::Duration;

use chrono::Utc;

use crate::project::{
    Project, ProjectError, ProjectStore, WorkspaceId, bare_repo_path, prune_worktrees,
    remove_worktree,
};

/// Removes every workspace under `project` that has gone unused for at
/// least `ttl`, skipping any a running pipeline still holds.
///
/// `is_pipeline_running` is a plain closure rather than
/// `crate::pipeline::PipelineRegistry` itself: `ai::project` sits below
/// `ai::pipeline` in the dependency graph (see this crate's own module
/// doc comment on `project.rs`), so it cannot name that module's types.
/// The caller (wherever this sweep is actually scheduled) supplies
/// `|pipeline_id| pipeline_registry.is_running(PipelineId(pipeline_id))`.
///
/// For each workspace collected: removes the worktree, prunes git's own
/// administrative record, and deletes the row - in that exact order, so
/// a crash partway through leaves something
/// [`crate::project::reconcile_workspaces`] can clean up rather than an
/// orphan (a worktree directory gone but the row still there, or a git
/// administrative record still pointing at a directory that no longer
/// exists).
pub async fn collect_unused_workspaces(
    store: &dyn ProjectStore,
    project: &Project,
    ttl: Duration,
    is_pipeline_running: impl Fn(i64) -> bool,
) -> Result<Vec<WorkspaceId>, ProjectError> {
    let now = Utc::now();
    let ttl = chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::MAX);
    let bare_repo = bare_repo_path(&project.local_path);

    let mut collected = Vec::new();
    for workspace in store.list_workspaces_for_project(project.id).await? {
        if let Some(pipeline_id) = workspace.pipeline_id
            && is_pipeline_running(pipeline_id)
        {
            continue;
        }

        if now.signed_duration_since(workspace.last_used_at) < ttl {
            continue;
        }

        remove_worktree(&bare_repo, &workspace.path).await?;
        prune_worktrees(&bare_repo).await?;
        store.delete_workspace(workspace.id).await?;
        collected.push(workspace.id);
    }

    Ok(collected)
}

#[cfg(test)]
mod tests {
    use munibot_vcs::{Forge, RepoRef};

    use super::*;
    use crate::project::{
        ImageStrategy, InMemoryProjectStore, NewProject, NewWorkspace, create_worktree,
    };

    struct TestRepo {
        project_root: std::path::PathBuf,
        bare_repo: std::path::PathBuf,
    }

    impl TestRepo {
        async fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let upstream_bare =
                std::env::temp_dir().join(format!("munibot_ai_collect_upstream_{unique}"));
            let work_dir = std::env::temp_dir().join(format!("munibot_ai_collect_work_{unique}"));
            std::fs::create_dir_all(&work_dir).unwrap();

            run(&[
                "init",
                "--bare",
                "--initial-branch",
                "main",
                &upstream_bare.display().to_string(),
            ])
            .await;
            let work_dir_str = work_dir.display().to_string();
            run(&["-C", &work_dir_str, "init", "--initial-branch", "main"]).await;
            run(&[
                "-C",
                &work_dir_str,
                "config",
                "user.email",
                "muni@example.com",
            ])
            .await;
            run(&["-C", &work_dir_str, "config", "user.name", "muni"]).await;
            run(&["-C", &work_dir_str, "config", "commit.gpgsign", "false"]).await;
            std::fs::write(work_dir.join("README.md"), "hello").unwrap();
            run(&["-C", &work_dir_str, "add", "."]).await;
            run(&["-C", &work_dir_str, "commit", "-m", "initial commit"]).await;
            run(&[
                "-C",
                &work_dir_str,
                "push",
                &upstream_bare.display().to_string(),
                "main",
            ])
            .await;
            std::fs::remove_dir_all(&work_dir).ok();

            let project_root =
                std::env::temp_dir().join(format!("munibot_ai_collect_project_{unique}"));
            crate::project::clone_or_fetch_project(
                &project_root,
                &format!("file://{}", upstream_bare.display()),
                None,
            )
            .await
            .unwrap();
            std::fs::remove_dir_all(&upstream_bare).ok();

            let bare_repo = bare_repo_path(&project_root);
            Self {
                project_root,
                bare_repo,
            }
        }

        fn worktree_path(&self, name: &str) -> std::path::PathBuf {
            self.project_root.join("worktrees").join(name)
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.project_root).ok();
        }
    }

    async fn run(args: &[&str]) {
        let status = tokio::process::Command::new("git")
            .args(args)
            .status()
            .await
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    async fn sample_project(store: &InMemoryProjectStore, repo: &TestRepo) -> Project {
        store
            .upsert_project(&NewProject {
                slug: "cocoa".to_string(),
                display_name: "Cocoa".to_string(),
                repo: RepoRef::new(Forge::GitHub, "muni-corn", "cocoa"),
                clone_url: "https://github.com/muni-corn/cocoa.git".to_string(),
                default_branch: "main".to_string(),
                local_path: repo.project_root.clone(),
                image_strategy: ImageStrategy::Base,
                network_policy: crate::sandbox::config::SandboxNetworkConfig::None,
                trigger: None,
                installation_id: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_an_old_scratch_workspace_past_its_ttl_is_collected() {
        let repo = TestRepo::new().await;
        let store = InMemoryProjectStore::new();
        let project = sample_project(&store, &repo).await;

        let path = repo.worktree_path("scratch");
        create_worktree(&repo.bare_repo, &path, "munibot/scratch", "main")
            .await
            .unwrap();
        let workspace = store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name: "scratch".to_string(),
                branch: "munibot/scratch".to_string(),
                path: path.clone(),
                pipeline_id: None,
                conversation_id: None,
            })
            .await
            .unwrap();
        // backdate it well past any reasonable ttl
        force_last_used_at(
            &store,
            workspace.id,
            Utc::now() - chrono::Duration::days(30),
        )
        .await;

        let collected =
            collect_unused_workspaces(&store, &project, Duration::from_secs(3600), |_| false)
                .await
                .expect("should collect");

        assert_eq!(collected, vec![workspace.id]);
        assert!(!path.exists(), "the worktree directory should be removed");
        assert!(store.get_workspace(workspace.id).await.is_err());
    }

    #[tokio::test]
    async fn test_a_recently_used_workspace_is_not_collected() {
        let repo = TestRepo::new().await;
        let store = InMemoryProjectStore::new();
        let project = sample_project(&store, &repo).await;

        let path = repo.worktree_path("scratch");
        create_worktree(&repo.bare_repo, &path, "munibot/scratch", "main")
            .await
            .unwrap();
        store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name: "scratch".to_string(),
                branch: "munibot/scratch".to_string(),
                path: path.clone(),
                pipeline_id: None,
                conversation_id: None,
            })
            .await
            .unwrap();

        let collected =
            collect_unused_workspaces(&store, &project, Duration::from_secs(3600), |_| false)
                .await
                .expect("should succeed");

        assert!(collected.is_empty());
        assert!(
            path.is_dir(),
            "a freshly used workspace must survive the sweep"
        );
    }

    #[tokio::test]
    async fn test_an_old_workspace_with_a_running_pipeline_is_never_collected() {
        let repo = TestRepo::new().await;
        let store = InMemoryProjectStore::new();
        let project = sample_project(&store, &repo).await;

        let path = repo.worktree_path("pipeline-42");
        create_worktree(&repo.bare_repo, &path, "munibot/issue-42", "main")
            .await
            .unwrap();
        let workspace = store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name: "pipeline-42".to_string(),
                branch: "munibot/issue-42".to_string(),
                path: path.clone(),
                pipeline_id: Some(42),
                conversation_id: None,
            })
            .await
            .unwrap();
        force_last_used_at(
            &store,
            workspace.id,
            Utc::now() - chrono::Duration::days(30),
        )
        .await;

        let collected =
            collect_unused_workspaces(&store, &project, Duration::from_secs(3600), |id| id == 42)
                .await
                .expect("should succeed");

        assert!(
            collected.is_empty(),
            "a workspace whose pipeline is still running must never be collected, no matter how \
             old"
        );
        assert!(path.is_dir());
    }

    /// `InMemoryProjectStore` has no direct setter for `last_used_at` -
    /// this reaches through `touch_workspace` and a manual override for
    /// the handful of collection tests above that need to simulate
    /// something old enough to actually collect.
    async fn force_last_used_at(
        store: &InMemoryProjectStore,
        id: WorkspaceId,
        at: chrono::DateTime<Utc>,
    ) {
        store.force_last_used_at_for_test(id, at);
    }
}
