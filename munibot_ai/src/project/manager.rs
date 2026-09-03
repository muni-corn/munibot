//! `ProjectManager`: the facade tying together the project store, git
//! plumbing, strategy detection, image building, and workspace sessions -
//! the only thing anything outside `ai::project` should need to hold.
//!
//! Everything this coordinates is a component built by an earlier commit
//! in `docs/plans/ai/milestone-7-projects.md`'s phase 25; the work always
//! belongs in the module that owns it, never inlined here - see this
//! type's own size for why.

use std::{path::PathBuf, sync::Arc};

use munibot_vcs::{RepoRef, TriggerMode};

use crate::{
    project::{
        ImageBuilder, ImageStatus, NewProject, NewWorkspace, Project, ProjectError, ProjectStore,
        ProjectsSettings, Workspace, bare_repo_path, clone_or_fetch_project, create_worktree,
        detect_image_strategy,
    },
    sandbox::config::SandboxNetworkConfig,
};

/// The default worktree every project gets at registration, on its own
/// default branch - what strategy detection and the first image build
/// inspect, and a project's own "nothing else asked for a workspace yet"
/// fallback.
const DEFAULT_WORKSPACE_NAME: &str = "default";

/// Everything [`ProjectManager::register`] needs that isn't computed
/// internally - deliberately not [`NewProject`] itself: `local_path` and
/// `image_strategy` are this call's own job to determine, not the
/// caller's.
pub struct RegisterProjectSpec {
    /// Whose projects directory this belongs under -
    /// `<ai.projects.root>/<munibot_user_id>/<slug>/`.
    pub munibot_user_id: i64,
    pub slug: String,
    pub display_name: String,
    pub repo: RepoRef,
    pub clone_url: String,
    pub default_branch: String,
    pub network_policy: SandboxNetworkConfig,
    pub trigger: Option<TriggerMode>,
    /// A forge access token, if cloning needs one - never logged, never
    /// placed in a URL or argv. See `crate::project::clone`'s own doc
    /// comment.
    pub token: Option<String>,
}

/// Which workspace a caller wants: `workspace_for` derives a stable name
/// from this, so a repeated call for the same pipeline or conversation
/// always resolves to the exact same on-disk worktree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceOwner {
    Pipeline(i64),
    Conversation(i64),
    /// No pipeline or conversation owns this - a project's shared default
    /// workspace, or an ad-hoc inspection.
    Scratch,
}

impl WorkspaceOwner {
    fn workspace_name(&self) -> String {
        match self {
            WorkspaceOwner::Pipeline(id) => format!("pipeline-{id}"),
            WorkspaceOwner::Conversation(id) => format!("chat-{id}"),
            WorkspaceOwner::Scratch => DEFAULT_WORKSPACE_NAME.to_string(),
        }
    }

    fn ids(&self) -> (Option<i64>, Option<i64>) {
        match self {
            WorkspaceOwner::Pipeline(id) => (Some(*id), None),
            WorkspaceOwner::Conversation(id) => (None, Some(*id)),
            WorkspaceOwner::Scratch => (None, None),
        }
    }
}

pub struct ProjectManager {
    store: Arc<dyn ProjectStore>,
    settings: ProjectsSettings,
    image_builder: ImageBuilder,
}

impl ProjectManager {
    pub fn new(store: Arc<dyn ProjectStore>, settings: ProjectsSettings) -> Self {
        let image_builder = ImageBuilder::new(settings.max_concurrent_image_builds);
        Self {
            store,
            settings,
            image_builder,
        }
    }

    /// Registers a project: clones (or, on a repeat registration, fetches)
    /// its repository, checks out a default workspace to inspect, detects
    /// which image strategy applies, and builds that image if one is
    /// needed - all before the project row is ever considered ready.
    ///
    /// Idempotent: registering the same slug again re-clones nothing
    /// (`clone_or_fetch_project` fetches instead), reuses the existing
    /// default workspace, and only rebuilds the image if its source hash
    /// actually changed (`ImageBuilder::build_if_needed`'s own cache).
    pub async fn register(&self, spec: RegisterProjectSpec) -> Result<Project, ProjectError> {
        let local_path = self
            .settings
            .root
            .join(spec.munibot_user_id.to_string())
            .join(&spec.slug);

        clone_or_fetch_project(&local_path, &spec.clone_url, spec.token.as_deref()).await?;

        let default_workspace = self
            .checkout_workspace(
                &local_path,
                &spec.default_branch,
                &spec.default_branch,
                WorkspaceOwner::Scratch,
            )
            .await?;

        let (strategy, _skip_reasons) = detect_image_strategy(&default_workspace);
        let image_status = self
            .image_builder
            .build_if_needed(&default_workspace, &spec.slug, strategy)
            .await;

        let project = self
            .store
            .upsert_project(&NewProject {
                slug: spec.slug,
                display_name: spec.display_name,
                repo: spec.repo,
                clone_url: spec.clone_url,
                default_branch: spec.default_branch,
                local_path,
                image_strategy: strategy,
                network_policy: spec.network_policy,
                trigger: spec.trigger,
            })
            .await?;

        // upsert_project doesn't carry image status - a fresh
        // registration always starts Unbuilt there, so the build result
        // above is applied as a second, explicit step rather than
        // smuggled into the insert itself
        if image_status != ImageStatus::Unbuilt {
            return self
                .store
                .update_image_status(project.id, strategy, image_status)
                .await;
        }

        Ok(project)
    }

    /// Resolves the workspace `owner` should use, creating it (on
    /// `branch`) if it doesn't already exist, or touching and reusing it
    /// if it does - the persistent-workspace half of the milestone's own
    /// architecture: a workspace survives a run, so a resumed pipeline or
    /// a re-entered chat session always finds its own work still there.
    pub async fn workspace_for(
        &self,
        project: &Project,
        branch: &str,
        owner: WorkspaceOwner,
    ) -> Result<Workspace, ProjectError> {
        let name = owner.workspace_name();

        if let Some(existing) = self.store.get_workspace_by_name(project.id, &name).await? {
            self.store.touch_workspace(existing.id).await?;
            return Ok(existing);
        }

        let path = self
            .checkout_workspace(&project.local_path, branch, &project.default_branch, owner)
            .await?;
        let (pipeline_id, conversation_id) = owner.ids();

        self.store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name,
                branch: branch.to_string(),
                path,
                pipeline_id,
                conversation_id,
            })
            .await
    }

    /// Creates (or reuses) the on-disk worktree `owner`'s workspace needs,
    /// without touching `ai_workspaces` at all - the row-level bookkeeping
    /// is [`Self::register`] and [`Self::workspace_for`]'s own job, since
    /// they need it at different points (before a project row exists at
    /// all, and after one already does).
    async fn checkout_workspace(
        &self,
        project_root: &std::path::Path,
        branch: &str,
        base_branch: &str,
        owner: WorkspaceOwner,
    ) -> Result<PathBuf, ProjectError> {
        let bare_repo = bare_repo_path(project_root);
        let path = project_root.join("worktrees").join(owner.workspace_name());

        if !path.is_dir() {
            create_worktree(&bare_repo, &path, branch, base_branch).await?;
        }

        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ImageStrategy, InMemoryProjectStore};

    /// A local bare repository with one commit on `main`, real git
    /// plumbing, zero network - matching every other `crate::project`
    /// test module's own strategy.
    struct TestUpstream {
        bare_path: PathBuf,
    }

    impl TestUpstream {
        async fn new(files: &[(&str, &str)]) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let bare_path =
                std::env::temp_dir().join(format!("munibot_ai_manager_upstream_{unique}"));
            let work_dir = std::env::temp_dir().join(format!("munibot_ai_manager_work_{unique}"));
            std::fs::create_dir_all(&work_dir).unwrap();

            run(&[
                "init",
                "--bare",
                "--initial-branch",
                "main",
                &bare_path.display().to_string(),
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
            for (name, content) in files {
                let path = work_dir.join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(path, content).unwrap();
            }
            run(&["-C", &work_dir_str, "add", "."]).await;
            run(&["-C", &work_dir_str, "commit", "-m", "initial commit"]).await;
            run(&[
                "-C",
                &work_dir_str,
                "push",
                &bare_path.display().to_string(),
                "main",
            ])
            .await;
            std::fs::remove_dir_all(&work_dir).ok();

            Self { bare_path }
        }

        fn url(&self) -> String {
            format!("file://{}", self.bare_path.display())
        }
    }

    impl Drop for TestUpstream {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.bare_path).ok();
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

    fn manager_with_root(root: PathBuf) -> (ProjectManager, Arc<InMemoryProjectStore>) {
        let store = Arc::new(InMemoryProjectStore::new());
        let settings = ProjectsSettings {
            root,
            ..ProjectsSettings::default()
        };
        (ProjectManager::new(store.clone(), settings), store)
    }

    fn spec(upstream: &TestUpstream, munibot_user_id: i64, slug: &str) -> RegisterProjectSpec {
        RegisterProjectSpec {
            munibot_user_id,
            slug: slug.to_string(),
            display_name: "Cocoa".to_string(),
            repo: RepoRef::new(munibot_vcs::Forge::GitHub, "muni-corn", "cocoa"),
            clone_url: upstream.url(),
            default_branch: "main".to_string(),
            network_policy: SandboxNetworkConfig::None,
            trigger: None,
            token: None,
        }
    }

    #[tokio::test]
    async fn test_register_clones_and_creates_a_project_row_resolved_to_base() {
        let upstream = TestUpstream::new(&[("README.md", "hello")]).await;
        let root = std::env::temp_dir().join(format!(
            "munibot_ai_manager_root_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (manager, _store) = manager_with_root(root.clone());

        let project = manager
            .register(spec(&upstream, 1, "cocoa"))
            .await
            .expect("should register");

        assert_eq!(project.slug, "cocoa");
        assert_eq!(project.image_strategy, ImageStrategy::Base);
        assert!(project.local_path.join("repo.git").is_dir());
        assert!(
            project
                .local_path
                .join("worktrees/default/README.md")
                .is_file()
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_registering_the_same_slug_twice_is_idempotent() {
        let upstream = TestUpstream::new(&[("README.md", "hello")]).await;
        let root = std::env::temp_dir().join(format!(
            "munibot_ai_manager_root_idempotent_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (manager, store) = manager_with_root(root.clone());

        let first = manager
            .register(spec(&upstream, 1, "cocoa"))
            .await
            .expect("first registration should succeed");
        let second = manager
            .register(spec(&upstream, 1, "cocoa"))
            .await
            .expect("second registration should succeed");

        assert_eq!(first.id, second.id);
        assert_eq!(store.list_projects().await.unwrap().len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_workspace_for_creates_a_worktree_named_after_the_pipeline() {
        let upstream = TestUpstream::new(&[("README.md", "hello")]).await;
        let root = std::env::temp_dir().join(format!(
            "munibot_ai_manager_root_workspace_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (manager, _store) = manager_with_root(root.clone());
        let project = manager
            .register(spec(&upstream, 1, "cocoa"))
            .await
            .expect("should register");

        let workspace = manager
            .workspace_for(&project, "munibot/issue-17", WorkspaceOwner::Pipeline(17))
            .await
            .expect("should create a workspace");

        assert_eq!(workspace.name, "pipeline-17");
        assert_eq!(workspace.branch, "munibot/issue-17");
        assert_eq!(workspace.pipeline_id, Some(17));
        assert!(workspace.path.join("README.md").is_file());

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_workspace_for_reuses_an_existing_workspace_for_the_same_owner() {
        let upstream = TestUpstream::new(&[("README.md", "hello")]).await;
        let root = std::env::temp_dir().join(format!(
            "munibot_ai_manager_root_reuse_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (manager, _store) = manager_with_root(root.clone());
        let project = manager
            .register(spec(&upstream, 1, "cocoa"))
            .await
            .expect("should register");

        let first = manager
            .workspace_for(&project, "munibot/issue-17", WorkspaceOwner::Pipeline(17))
            .await
            .expect("should create");
        let second = manager
            .workspace_for(&project, "munibot/issue-17", WorkspaceOwner::Pipeline(17))
            .await
            .expect("should reuse");

        assert_eq!(first.id, second.id);

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_register_detects_and_builds_a_containerfile_strategy() {
        let upstream = TestUpstream::new(&[
            (".munibot/Containerfile", "FROM alpine:latest\n"),
            ("README.md", "hello"),
        ])
        .await;
        let root = std::env::temp_dir().join(format!(
            "munibot_ai_manager_root_containerfile_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (manager, _store) = manager_with_root(root.clone());

        let project = manager
            .register(spec(&upstream, 1, "cocoa-with-containerfile"))
            .await
            .expect("should register");

        assert_eq!(project.image_strategy, ImageStrategy::Containerfile);
        let ImageStatus::Built { tag, .. } = &project.image_status else {
            panic!("expected a built image, got {:?}", project.image_status);
        };

        let _ = tokio::process::Command::new("podman")
            .args(["rmi", "-f", tag])
            .output()
            .await;
        std::fs::remove_dir_all(&root).ok();
    }
}
