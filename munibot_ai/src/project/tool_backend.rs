//! Adapts [`ProjectManager`] to the narrower `ProjectRegistrar` interface
//! the `manage_project` tool needs.
//!
//! `ai::tools` cannot name [`ProjectManager`] directly - it sits below
//! `ai::project` in this crate's dependency graph (see `project.rs`'s own
//! doc comment) - so this bridge lives here, on the side that is already
//! allowed to depend on both.

use std::sync::Arc;

use munibot_vcs::{Forge, RepoRef};

use crate::{
    project::{ImageStatus, ProjectManager, RegisterProjectSpec},
    sandbox::config::SandboxNetworkConfig,
    tools::{ProjectRegistrar, RegisterProjectArgs},
    types::AiError,
};

/// Bridges [`ProjectManager::register`] to `crate::tools::ProjectRegistrar`.
pub struct ProjectManagerRegistrar {
    manager: Arc<ProjectManager>,
}

impl ProjectManagerRegistrar {
    pub fn new(manager: Arc<ProjectManager>) -> Self {
        Self { manager }
    }
}

#[async_trait::async_trait]
impl ProjectRegistrar for ProjectManagerRegistrar {
    async fn register_project(&self, args: RegisterProjectArgs<'_>) -> Result<String, AiError> {
        let project = self
            .manager
            .register(RegisterProjectSpec {
                munibot_user_id: args.munibot_user_id as i64,
                slug: args.slug.to_string(),
                display_name: args.display_name.to_string(),
                repo: RepoRef::new(Forge::GitHub, args.repo_owner, args.repo_name),
                clone_url: args.clone_url.to_string(),
                default_branch: args.default_branch.to_string(),
                // closed by default, matching SandboxConfig's own default -
                // a tool-driven registration has no operator sitting at a
                // form to pick a wider policy the way the projects page does
                network_policy: SandboxNetworkConfig::None,
                trigger: None,
                token: None,
                installation_id: None,
            })
            .await
            .map_err(|error| AiError::Other(error.to_string()))?;

        Ok(match &project.image_status {
            ImageStatus::Unbuilt => format!(
                "registered {:?} - resolved to the {} image strategy, nothing to build",
                project.slug, project.image_strategy
            ),
            ImageStatus::Built { tag, .. } => format!(
                "registered {:?} - built {tag:?} via the {} strategy",
                project.slug, project.image_strategy
            ),
            ImageStatus::Failed { reason } => format!(
                "registered {:?}, but the {} image build failed :< {reason}",
                project.slug, project.image_strategy
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{InMemoryProjectStore, ProjectsSettings};

    async fn run(args: &[&str]) {
        let status = tokio::process::Command::new("git")
            .args(args)
            .status()
            .await
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    async fn local_upstream() -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let bare = std::env::temp_dir().join(format!("munibot_ai_tool_backend_upstream_{unique}"));
        let work = std::env::temp_dir().join(format!("munibot_ai_tool_backend_work_{unique}"));
        std::fs::create_dir_all(&work).unwrap();

        run(&[
            "init",
            "--bare",
            "--initial-branch",
            "main",
            &bare.display().to_string(),
        ])
        .await;
        let work_str = work.display().to_string();
        run(&["-C", &work_str, "init", "--initial-branch", "main"]).await;
        run(&["-C", &work_str, "config", "user.email", "muni@example.com"]).await;
        run(&["-C", &work_str, "config", "user.name", "muni"]).await;
        run(&["-C", &work_str, "config", "commit.gpgsign", "false"]).await;
        std::fs::write(work.join("README.md"), "hello").unwrap();
        run(&["-C", &work_str, "add", "."]).await;
        run(&["-C", &work_str, "commit", "-m", "initial commit"]).await;
        run(&["-C", &work_str, "push", &bare.display().to_string(), "main"]).await;
        std::fs::remove_dir_all(&work).ok();
        bare
    }

    #[tokio::test]
    async fn test_registers_and_summarizes_a_base_strategy_project() {
        let bare = local_upstream().await;
        let root = std::env::temp_dir().join(format!(
            "munibot_ai_tool_backend_root_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Arc::new(InMemoryProjectStore::new());
        let settings = ProjectsSettings {
            root: root.clone(),
            ..ProjectsSettings::default()
        };
        let manager = Arc::new(ProjectManager::new(store, settings));
        let registrar = ProjectManagerRegistrar::new(manager);

        let clone_url = format!("file://{}", bare.display());
        let summary = registrar
            .register_project(crate::tools::RegisterProjectArgs {
                munibot_user_id: 1,
                slug: "cocoa",
                display_name: "Cocoa",
                repo_owner: "muni-corn",
                repo_name: "cocoa",
                clone_url: &clone_url,
                default_branch: "main",
            })
            .await
            .expect("should register");

        assert!(summary.contains("cocoa"));
        assert!(summary.contains("base"));

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&bare).ok();
    }

    #[tokio::test]
    async fn test_a_registration_failure_surfaces_as_an_error_not_a_panic() {
        let store = Arc::new(InMemoryProjectStore::new());
        let settings = ProjectsSettings {
            root: std::env::temp_dir().join(format!(
                "munibot_ai_tool_backend_root_fail_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
            ..ProjectsSettings::default()
        };
        let manager = Arc::new(ProjectManager::new(store, settings));
        let registrar = ProjectManagerRegistrar::new(manager);

        let error = registrar
            .register_project(crate::tools::RegisterProjectArgs {
                munibot_user_id: 1,
                slug: "cocoa",
                display_name: "Cocoa",
                repo_owner: "muni-corn",
                repo_name: "cocoa",
                clone_url: "file:///definitely/does/not/exist",
                default_branch: "main",
            })
            .await
            .expect_err("cloning a nonexistent repository should fail");
        assert!(!error.to_string().is_empty());
    }
}
