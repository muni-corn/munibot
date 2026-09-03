//! [`SandboxLifecycle`]'s real implementation: a sandbox over the
//! pipeline's own project workspace, from that project's own image,
//! rather than [`crate::pipeline::executor::NoSandbox`]'s no-op.
//!
//! See `docs/notes/pipeline-sandbox-wiring-gap.md` for what this closes.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    persona::SandboxPolicy,
    pipeline::{ExecutorError, PipelineId, SandboxLifecycle},
    project::{ImageStatus, Project, ProjectManager, WorkspaceOwner},
    sandbox::{config::SandboxConfig, provision::ProvisionedSandbox},
    service::provision_or_degrade,
    tools::ToolRegistry,
};

/// Resolves `project`'s own image tag for `config.image`, or refuses to
/// provision at all if that project's chosen strategy resolved to an
/// image that failed to build - using the sandbox's own generically
/// configured image for a project whose devenv or Containerfile strategy
/// actually applied would silently run every sandboxed tool call against
/// the wrong environment, never a safe fallback.
///
/// `Unbuilt` keeps `config.image` as configured: exactly right for
/// [`crate::project::ImageStrategy::Base`], which never builds anything
/// at all - see that variant's own doc comment.
fn resolve_project_image(
    project: &Project,
    mut config: SandboxConfig,
) -> Result<SandboxConfig, ExecutorError> {
    match &project.image_status {
        ImageStatus::Built { tag, .. } => {
            config.image = tag.clone();
            Ok(config)
        }
        ImageStatus::Failed { reason } => Err(ExecutorError::Sandbox(format!(
            "{}'s own image failed to build :< {reason}",
            project.slug
        ))),
        ImageStatus::Unbuilt => Ok(config),
    }
}

/// A [`SandboxLifecycle`] over one pipeline run's own project: resolves
/// the workspace for `pipeline_id` (a dedicated worktree on `branch`, via
/// [`ProjectManager::workspace_for`]), provisions a sandbox over it from
/// the project's own image, and hands back the resulting tool registry.
///
/// Provisions eagerly, at [`SandboxPolicy::Required`] - unlike a chat
/// persona's own `Optional` sandbox (which degrades and lets the model
/// find out on first use), a pipeline role that reaches
/// `Executor::run`'s sandboxed states cannot do its job at all without
/// one, so a broken image or an unreachable podman must fail the run
/// immediately, not silently produce handoffs a `write`/`bash` tool call
/// was never actually able to back up.
///
/// [`Self::teardown`] drops the underlying session, but never the
/// **worktree** itself - that is `ProjectManager`'s own concern (see
/// `crate::project::collect_unused_workspaces`), and leaving it on disk
/// is the entire point of a pipeline's workspace being persistent: a
/// failed run stays inspectable, and a resumed one finds its own work
/// still there.
pub struct ProjectSandboxLifecycle {
    project_manager: Arc<ProjectManager>,
    project: Project,
    branch: String,
    pipeline_id: PipelineId,
    sandbox_config: SandboxConfig,
    base_tools: Arc<ToolRegistry>,
    provisioned: Mutex<Option<ProvisionedSandbox>>,
}

impl ProjectSandboxLifecycle {
    pub fn new(
        project_manager: Arc<ProjectManager>,
        project: Project,
        branch: String,
        pipeline_id: PipelineId,
        sandbox_config: SandboxConfig,
        base_tools: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            project_manager,
            project,
            branch,
            pipeline_id,
            sandbox_config,
            base_tools,
            provisioned: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl SandboxLifecycle for ProjectSandboxLifecycle {
    async fn provision(&self) -> Result<Arc<ToolRegistry>, ExecutorError> {
        let workspace = self
            .project_manager
            .workspace_for(
                &self.project,
                &self.branch,
                WorkspaceOwner::Pipeline(self.pipeline_id.0),
            )
            .await
            .map_err(|error| ExecutorError::Sandbox(error.to_string()))?;

        let config = resolve_project_image(&self.project, self.sandbox_config.clone())?;

        let provisioned = provision_or_degrade(
            SandboxPolicy::Required,
            config,
            Some(workspace.path),
            &self.base_tools,
        )
        .await
        .map_err(|error| ExecutorError::Sandbox(error.to_string()))?
        .expect("SandboxPolicy::Required always provisions something");

        let tools = provisioned.tools.clone();
        *self.provisioned.lock().await = Some(provisioned);
        Ok(tools)
    }

    async fn teardown(&self) {
        self.provisioned.lock().await.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectId;

    fn base_project() -> Project {
        Project {
            id: ProjectId(1),
            slug: "cocoa".to_string(),
            display_name: "Cocoa".to_string(),
            repo: munibot_vcs::RepoRef::new(munibot_vcs::Forge::GitHub, "muni-corn", "cocoa"),
            clone_url: "https://example.invalid/cocoa.git".to_string(),
            default_branch: "main".to_string(),
            local_path: std::path::PathBuf::from("/tmp/does-not-matter"),
            image_strategy: crate::project::ImageStrategy::Base,
            image_status: ImageStatus::Unbuilt,
            network_policy: crate::sandbox::config::SandboxNetworkConfig::None,
            trigger: None,
            installation_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_an_unbuilt_base_strategy_project_keeps_the_configured_default_image() {
        let project = base_project();
        let config = resolve_project_image(&project, SandboxConfig {
            image: "munibot-sandbox:latest".to_string(),
            ..SandboxConfig::default()
        })
        .expect("should succeed");
        assert_eq!(config.image, "munibot-sandbox:latest");
    }

    #[test]
    fn test_a_built_project_image_overrides_the_configured_default() {
        let mut project = base_project();
        project.image_status = ImageStatus::Built {
            tag: "munibot-project-cocoa:abc123".to_string(),
            hash: "abc123".to_string(),
            at: chrono::Utc::now(),
        };

        let config =
            resolve_project_image(&project, SandboxConfig::default()).expect("should succeed");
        assert_eq!(config.image, "munibot-project-cocoa:abc123");
    }

    #[test]
    fn test_a_failed_project_image_refuses_to_provision_at_all() {
        let mut project = base_project();
        project.image_status = ImageStatus::Failed {
            reason: "nix evaluation failed".to_string(),
        };

        let error = resolve_project_image(&project, SandboxConfig::default())
            .expect_err("should refuse rather than fall back to the generic image");
        assert!(
            matches!(error, ExecutorError::Sandbox(reason) if reason.contains("nix evaluation failed"))
        );
    }

    /// The lifecycle mechanics, against a real `ProjectManager` (an
    /// in-memory store and a local git upstream, no network) but a
    /// sandbox image that doesn't exist - enough to prove `provision`
    /// resolves a real, dedicated `pipeline-<id>` workspace and surfaces
    /// a provisioning failure as `ExecutorError::Sandbox`, without
    /// needing a real working tool agent inside a real container at
    /// all. A full round trip against a real image is commit 243's own
    /// job - see `docs/plans/ai/milestone-7-projects.md`'s phase 27.
    #[tokio::test]
    async fn test_provision_resolves_a_dedicated_pipeline_workspace_before_failing_to_connect() {
        use crate::project::test_support::{TestUpstream, manager_with_root, minimal_spec};

        let upstream = TestUpstream::new(&[("README.md", "hello")]).await;
        let root = std::env::temp_dir().join(format!(
            "munibot_ai_pipeline_sandbox_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (manager, _store) = manager_with_root(root.clone());
        let project = manager
            .register(minimal_spec(&upstream, 1, "cocoa"))
            .await
            .expect("should register");

        let lifecycle = ProjectSandboxLifecycle::new(
            Arc::new(manager),
            project.clone(),
            "munibot/issue-17".to_string(),
            PipelineId(17),
            SandboxConfig {
                image: "definitely-does-not-exist-anywhere:latest".to_string(),
                ..SandboxConfig::default()
            },
            Arc::new(ToolRegistry::new()),
        );

        let Err(error) = lifecycle.provision().await else {
            panic!("an image that doesn't exist should fail to provision");
        };
        assert!(matches!(error, ExecutorError::Sandbox(_)));

        assert!(
            project.local_path.join("worktrees/pipeline-17").is_dir(),
            "the dedicated pipeline workspace should exist even though provisioning the sandbox \
             over it failed"
        );

        // teardown on a lifecycle that never successfully provisioned is
        // a safe no-op, never a panic
        lifecycle.teardown().await;

        std::fs::remove_dir_all(&root).ok();
    }
}
