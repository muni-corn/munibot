//! `PipelineLauncher`: the factory that actually starts an autonomous
//! pipeline run - `GitHubForge` (authenticating as the project's own
//! GitHub App installation), `DieselPipelineStore`, `HarnessDispatcher`,
//! `ProjectSandboxLifecycle`, `PipelineRegistry::try_start`, and a spawned
//! `Executor::run_with_interaction`, composed for real.
//!
//! Every one of those types was built and tested in an earlier commit;
//! this module is almost entirely composition - see
//! `docs/plans/ai/milestone-7-projects.md` phase 26, commits 232-233.
//!
//! Also implements `super::webhooks::PipelineDispatch` over
//! [`PipelineLauncher::launch`] (commit 233) - what makes a matched
//! webhook trigger actually start a run, once `super::webhooks`'s own
//! trigger source is backed by the project store rather than the
//! hardcoded empty list it still is today (commit 234).

use std::sync::Arc;

use async_trait::async_trait;
use munibot_ai::{
    audit::ToolAuditor,
    persona::{AiConfig, PersonaRegistry},
    pipeline::{
        Executor, GitHubCommentAdapter, HarnessDispatcher, PipelineEvent, PipelineId,
        PipelineRegistry, PipelineStore, PipelineStoreError, ProjectSandboxLifecycle,
        SandboxLifecycle, resolve_branch_name,
    },
    project::{ProjectManager, bare_repo_path, existing_branches},
    provider::{ProviderRegistry, ProviderResolver},
    sandbox::config::SandboxConfig,
    service::ProviderSource,
    tools::ToolRegistry,
    usage::UsageRecorder,
};
use munibot_github::GitHubForge;
use munibot_vcs::{ForgeEvent, IssueRef, IssueSource, VcsError};
use octocrab::models::{AppId, InstallationId};
use tracing::Instrument;

use super::webhooks::PipelineDispatch;

/// Why [`PipelineLauncher::launch`] couldn't start a run.
#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    /// No registered project matches the event's own repository at all -
    /// only reachable if a caller invokes this without having resolved a
    /// trigger against the project store first, which
    /// `super::webhooks::PipelineDispatch`'s own implementation (commit
    /// 233) never does.
    #[error("no registered project matches {0} :<")]
    ProjectNotRegistered(munibot_vcs::RepoRef),
    /// The matched project was registered manually, with no GitHub App
    /// installation behind it - see `Project::installation_id`'s own
    /// doc comment for why nothing can authenticate as it to comment on
    /// an issue or open a pull request.
    #[error(
        "project {0:?} was registered manually and has no github app installation to run a \
         pipeline as :<"
    )]
    NoInstallation(String),
    #[error(transparent)]
    Forge(#[from] munibot_github::GitHubError),
    #[error(transparent)]
    Vcs(#[from] VcsError),
    #[error(transparent)]
    Store(#[from] PipelineStoreError),
    #[error(transparent)]
    Project(#[from] munibot_ai::project::ProjectError),
}

/// Where [`PipelineLauncher::launch`] left a newly created run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOutcome {
    /// A concurrency slot was available; the run is executing in a
    /// spawned task right now.
    Started(PipelineId),
    /// No slot was available under `[ai.pipeline].max_concurrent` (or
    /// whatever `ConcurrencyConfig` the registry was built with) for
    /// this repository - queued for `PipelineRegistry::dequeue` to pick
    /// up once one frees.
    Queued(PipelineId),
}

impl LaunchOutcome {
    pub fn pipeline_id(&self) -> PipelineId {
        match self {
            Self::Started(id) | Self::Queued(id) => *id,
        }
    }
}

/// Starts (or queues) an autonomous pipeline run for one issue.
///
/// [`Self::build_executor`] is the private half a resume-at-boot caller
/// (commit 236) will also need, wrapped in a small function that first
/// looks up which project and branch an already-running pipeline id
/// belongs to - `resume_all`'s own `build_executor` factory has no issue
/// event to derive that from directly, unlike [`Self::launch`].
pub struct PipelineLauncher {
    forge_app_id: AppId,
    forge_private_key: String,
    bot_login: String,
    pipeline_store: Arc<dyn PipelineStore>,
    project_manager: Arc<ProjectManager>,
    personas: Arc<PersonaRegistry>,
    providers: Arc<dyn ProviderSource>,
    base_tools: Arc<ToolRegistry>,
    sandbox_config: SandboxConfig,
    registry: Arc<PipelineRegistry>,
    tool_auditor: Option<Arc<dyn ToolAuditor>>,
    usage_recorder: Option<Arc<dyn UsageRecorder>>,
}

impl PipelineLauncher {
    /// Builds a launcher from the same [`AiConfig`] the chat service
    /// itself resolves personas and a sandbox image from - a pipeline
    /// role's persona (`issue-analyst`, `builder`, ...) is configured
    /// the same way any chat persona is, under the same `[ai.personas]`
    /// table.
    pub fn new(
        app_id: u64,
        private_key: String,
        bot_login: String,
        pipeline_store: Arc<dyn PipelineStore>,
        project_manager: Arc<ProjectManager>,
        ai_config: &AiConfig,
        registry: Arc<PipelineRegistry>,
    ) -> anyhow::Result<Self> {
        let provider_registry = ProviderRegistry::from_env();
        let personas = Arc::new(PersonaRegistry::load(ai_config, &provider_registry)?);

        Ok(Self {
            forge_app_id: AppId(app_id),
            forge_private_key: private_key,
            bot_login,
            pipeline_store,
            project_manager,
            personas,
            providers: Arc::new(ProviderResolver::new()),
            base_tools: Arc::new(ToolRegistry::from_env()),
            sandbox_config: ai_config.sandbox.resolve_with_tool_agent(),
            registry,
            tool_auditor: None,
            usage_recorder: None,
        })
    }

    /// Audits every tool call every pipeline turn makes, the same as a
    /// chat turn's own `Ai::with_tool_auditor`.
    pub fn with_auditor(mut self, auditor: Arc<dyn ToolAuditor>) -> Self {
        self.tool_auditor = Some(auditor);
        self
    }

    /// Records a [`munibot_ai::usage::UsageRecord`] after every pipeline
    /// turn, the same as a chat turn's own `Ai::with_usage_recorder`.
    pub fn with_usage_recorder(mut self, recorder: Arc<dyn UsageRecorder>) -> Self {
        self.usage_recorder = Some(recorder);
        self
    }

    /// The project registered for `repo`, if any.
    ///
    /// A linear scan over every registered project: the expected count is
    /// small (one process serves one operator's own projects), and this
    /// avoids a dedicated `get_project_by_repo` query for what is, so
    /// far, this launcher's only caller.
    async fn project_for(
        &self,
        repo: &munibot_vcs::RepoRef,
    ) -> Result<munibot_ai::project::Project, LauncherError> {
        self.project_manager
            .list_projects()
            .await?
            .into_iter()
            .find(|project| &project.repo == repo)
            .ok_or_else(|| LauncherError::ProjectNotRegistered(repo.clone()))
    }

    /// Builds the `GitHubForge` that authenticates as `project`'s own
    /// installation - never munibot's own default identity, if it even
    /// has one: a pipeline run always acts as the specific installation
    /// that registered the project it works against.
    fn forge_for(
        &self,
        project: &munibot_ai::project::Project,
    ) -> Result<GitHubForge, LauncherError> {
        let installation_id = project
            .installation_id
            .ok_or_else(|| LauncherError::NoInstallation(project.slug.clone()))?;
        Ok(GitHubForge::new(
            self.forge_app_id,
            &self.forge_private_key,
            InstallationId(installation_id.unsigned_abs()),
        )?)
    }

    /// Builds the sandbox lifecycle and the `Executor` over it for
    /// `pipeline_id`, wired against `project`'s own workspace on
    /// `branch` - shared as one `Arc` between the two, since
    /// `PipelineRegistry::abort_pipeline` tears down through the exact
    /// same lifecycle instance the executor's own run loop provisions
    /// through, never a second, independent one that never actually
    /// provisioned anything.
    fn build_executor(
        &self,
        project: munibot_ai::project::Project,
        branch: String,
        pipeline_id: PipelineId,
    ) -> (Executor, Arc<dyn SandboxLifecycle>) {
        let mut dispatcher = HarnessDispatcher::new(
            self.providers.clone(),
            self.personas.clone(),
            self.base_tools.clone(),
        );
        if let Some(auditor) = &self.tool_auditor {
            dispatcher = dispatcher.with_auditor(auditor.clone());
        }
        if let Some(recorder) = &self.usage_recorder {
            dispatcher = dispatcher.with_usage_recorder(recorder.clone());
        }

        let sandbox: Arc<dyn SandboxLifecycle> = Arc::new(ProjectSandboxLifecycle::new(
            self.project_manager.clone(),
            project,
            branch,
            pipeline_id,
            self.sandbox_config.clone(),
            self.base_tools.clone(),
        ));

        let executor = Executor::new(
            self.pipeline_store.clone(),
            Arc::new(dispatcher),
            sandbox.clone(),
        );
        (executor, sandbox)
    }

    /// Builds a fresh `Executor` for an already-running pipeline id -
    /// what [`Self::resume_all`]'s own call into
    /// `munibot_ai::pipeline::resume_all` needs as its factory. Looks up
    /// which issue and project the pipeline belongs to from its own
    /// event log and the project store, rather than a fresh trigger
    /// event, since resuming has none.
    ///
    /// The branch decision only matters the very first time a
    /// pipeline's own workspace is created -
    /// `ProjectManager::workspace_for` reuses the existing workspace's
    /// own stored branch on every later call regardless of what's
    /// passed here (see that function's own early return once a
    /// workspace by that name already exists) - so recomputing it fresh
    /// on every resume is harmless, never a second, different branch a
    /// resumed run suddenly finds itself on.
    async fn executor_for(&self, pipeline_id: PipelineId) -> Result<Executor, LauncherError> {
        let issue = self.pipeline_store.issue_for(pipeline_id).await?;
        let project = self.project_for(&issue.repo).await?;
        let forge = self.forge_for(&project)?;

        let fetched = forge.fetch_issue(&issue).await?;
        let bare_repo = bare_repo_path(&project.local_path);
        let branches = existing_branches(&bare_repo).await?;
        let branch = resolve_branch_name(issue.number, &fetched.title, None, &branches);

        let (executor, _sandbox) = self.build_executor(project, branch, pipeline_id);
        Ok(executor)
    }

    /// Resumes every non-terminal pipeline this process's own store
    /// knows about, at boot, before the server starts accepting
    /// webhooks - what makes "restarting the process is a valid way to
    /// regain control of a pipeline" (`docs/ai-operations.md`) actually
    /// true rather than aspirational.
    ///
    /// Deliberately does not touch `self.registry` at all, matching
    /// `munibot_ai::pipeline::resume_all`'s own choice to call
    /// `Executor::run` directly rather than `run_with_interaction` - a
    /// resumed run already `AwaitingUserInput` has nothing new to
    /// resolve yet, and every other non-terminal state simply continues
    /// forward until it reaches one that does or finishes outright.
    /// Every result is logged; a resume failure for one pipeline never
    /// stops another from resuming.
    pub async fn resume_all(self: &Arc<Self>) {
        let launcher = self.clone();
        let results =
            munibot_ai::pipeline::resume_all(self.pipeline_store.clone(), move |pipeline_id| {
                let launcher = launcher.clone();
                async move {
                    launcher
                        .executor_for(pipeline_id)
                        .await
                        .map_err(|error| error.to_string())
                }
            })
            .await;

        for (pipeline_id, result) in results {
            match result {
                Ok(outcome) => {
                    tracing::info!(pipeline_id = pipeline_id.0, ?outcome, "resumed a pipeline")
                }
                Err(error) => {
                    tracing::error!(%error, pipeline_id = pipeline_id.0, "couldn't resume a pipeline")
                }
            }
        }
    }

    /// Starts a fresh pipeline run for `issue`: resolves the registered
    /// project and its installation, decides a branch name from the
    /// issue's own title, creates the `ai_pipelines` row, and either
    /// runs it in a spawned task (a concurrency slot was available) or
    /// queues it (one was not).
    ///
    /// The spawned task is instrumented with its own span
    /// (`.instrument`, never `.entered()` - see `docs/tracing.md` on why
    /// a guard that isn't `Send` cannot be held across the `.await`
    /// points a whole pipeline run is full of) and frees its
    /// concurrency slot (`PipelineRegistry::finish`) unconditionally,
    /// on every exit path, success or failure.
    #[tracing::instrument(skip_all, fields(repo = %issue.repo, issue = issue.number))]
    pub async fn launch(&self, issue: IssueRef) -> Result<LaunchOutcome, LauncherError> {
        let project = self.project_for(&issue.repo).await?;
        let forge = self.forge_for(&project)?;

        let fetched = forge.fetch_issue(&issue).await?;
        let bare_repo = bare_repo_path(&project.local_path);
        let branches = existing_branches(&bare_repo).await?;
        let branch = resolve_branch_name(issue.number, &fetched.title, None, &branches);

        let pipeline_id = self.pipeline_store.create_pipeline(&issue).await?;
        self.pipeline_store
            .append_event(pipeline_id, PipelineEvent::Triggered {
                issue: issue.clone(),
            })
            .await?;

        let (executor, sandbox) = self.build_executor(project, branch, pipeline_id);

        let Some(cancellation) = self.registry.try_start(pipeline_id, &issue.repo, sandbox) else {
            self.registry.enqueue(pipeline_id);
            return Ok(LaunchOutcome::Queued(pipeline_id));
        };

        let executor = executor.with_cancellation(cancellation);
        let registry = self.registry.clone();
        let adapter =
            GitHubCommentAdapter::new(Arc::new(forge), issue.clone(), self.bot_login.clone());
        let span = tracing::info_span!("pipeline_run", pipeline_id = pipeline_id.0);

        tokio::spawn(
            async move {
                if let Err(error) =
                    executor.run_with_interaction(pipeline_id, &adapter).await
                {
                    tracing::error!(%error, pipeline_id = pipeline_id.0, "pipeline run ended in error");
                }
                registry.finish(pipeline_id);
            }
            .instrument(span),
        );

        Ok(LaunchOutcome::Started(pipeline_id))
    }
}

#[async_trait]
impl PipelineDispatch for PipelineLauncher {
    /// Best-effort, matching this trait's own `dispatch` signature (no
    /// `Result` to propagate to `handle_webhook`, which has already
    /// committed to returning `202` regardless): a failure here is
    /// logged, never surfaced to GitHub as a delivery failure it would
    /// only retry identically.
    async fn dispatch(&self, event: ForgeEvent) {
        let issue = event.issue().clone();
        match self.launch(issue.clone()).await {
            Ok(LaunchOutcome::Started(id)) => {
                tracing::info!(pipeline_id = id.0, issue = %issue, "started a pipeline run");
            }
            Ok(LaunchOutcome::Queued(id)) => {
                tracing::info!(
                    pipeline_id = id.0,
                    issue = %issue,
                    "queued a pipeline run - no concurrency slot free right now"
                );
            }
            Err(error) => {
                tracing::error!(%error, issue = %issue, "couldn't start a pipeline run for a matched trigger");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use munibot_ai::{
        persona::{AbuseConfig, RateLimitConfig, SpendCapConfig},
        pipeline::{ConcurrencyConfig, InMemoryPipelineStore},
        project::{InMemoryProjectStore, NewProject, ProjectStore, ProjectsSettings},
        sandbox::config::SandboxSettings,
    };
    use munibot_vcs::{Forge, RepoRef};

    use super::*;

    fn empty_ai_config() -> AiConfig {
        AiConfig {
            enabled: true,
            default_persona: None,
            default_model: None,
            prompt_dir: None,
            personas: std::collections::HashMap::new(),
            crisis_resources: Vec::new(),
            rate_limits: RateLimitConfig::default(),
            spend_caps: SpendCapConfig::default(),
            abuse: AbuseConfig::default(),
            sandbox: SandboxSettings::default(),
            projects: ProjectsSettings::default(),
            max_delegation_depth: 2,
        }
    }

    async fn launcher_with_projects(store: Arc<InMemoryProjectStore>) -> PipelineLauncher {
        launcher_with(store, Arc::new(InMemoryPipelineStore::new())).await
    }

    async fn launcher_with(
        project_store: Arc<InMemoryProjectStore>,
        pipeline_store: Arc<InMemoryPipelineStore>,
    ) -> PipelineLauncher {
        let project_manager = Arc::new(ProjectManager::new(
            project_store,
            ProjectsSettings::default(),
        ));
        PipelineLauncher::new(
            123456,
            "not a real pem, never reached by either test below".to_string(),
            "munibot[bot]".to_string(),
            pipeline_store,
            project_manager,
            &empty_ai_config(),
            Arc::new(PipelineRegistry::new(ConcurrencyConfig::default())),
        )
        .expect("should build with no personas configured")
    }

    fn cocoa_repo() -> RepoRef {
        RepoRef::new(Forge::GitHub, "muni-corn", "cocoa")
    }

    /// A `NewProject` with no on-disk clone at all - fine for these two
    /// tests, which never reach `ProjectSandboxLifecycle::provision` or
    /// anything else that would need one.
    fn unregistered_project_spec(installation_id: Option<i64>) -> NewProject {
        NewProject {
            slug: "cocoa".to_string(),
            display_name: "Cocoa".to_string(),
            repo: cocoa_repo(),
            clone_url: "https://example.invalid/cocoa.git".to_string(),
            default_branch: "main".to_string(),
            local_path: std::path::PathBuf::from("/tmp/does-not-matter"),
            image_strategy: munibot_ai::project::ImageStrategy::Base,
            network_policy: munibot_ai::sandbox::config::SandboxNetworkConfig::None,
            trigger: None,
            installation_id,
        }
    }

    #[tokio::test]
    async fn test_project_for_refuses_a_repository_with_no_registered_project() {
        let store = Arc::new(InMemoryProjectStore::new());
        let launcher = launcher_with_projects(store).await;

        let error = launcher
            .project_for(&cocoa_repo())
            .await
            .expect_err("an unregistered repository should be refused");
        assert!(matches!(error, LauncherError::ProjectNotRegistered(repo) if repo == cocoa_repo()));
    }

    #[tokio::test]
    async fn test_forge_for_refuses_a_manually_registered_project() {
        let store = Arc::new(InMemoryProjectStore::new());
        store
            .upsert_project(&unregistered_project_spec(None))
            .await
            .expect("upsert failed");
        let launcher = launcher_with_projects(store.clone()).await;

        let project = launcher
            .project_for(&cocoa_repo())
            .await
            .expect("should find the project just registered");
        let Err(error) = launcher.forge_for(&project) else {
            panic!("a project with no installation id should be refused");
        };
        assert!(matches!(error, LauncherError::NoInstallation(slug) if slug == "cocoa"));
    }

    #[tokio::test]
    async fn test_forge_for_builds_a_forge_for_a_project_with_a_real_installation() {
        let store = Arc::new(InMemoryProjectStore::new());
        store
            .upsert_project(&unregistered_project_spec(Some(4242)))
            .await
            .expect("upsert failed");
        let launcher = launcher_with_projects(store.clone()).await;

        let project = launcher
            .project_for(&cocoa_repo())
            .await
            .expect("should find the project just registered");

        // GitHubForge::new does no network I/O of its own (see
        // OctocrabTokenMinter::installation_client's own doc comment) -
        // it only fails here if the private key itself doesn't parse as
        // a PEM-encoded RSA key, which this deliberately is not
        let Err(error) = launcher.forge_for(&project) else {
            panic!("a placeholder, non-PEM private key should fail to parse");
        };
        assert!(matches!(error, LauncherError::Forge(_)));
    }

    /// `PipelineDispatch::dispatch` has no `Result` to propagate a
    /// failure through at all - `handle_webhook` has already committed
    /// to a `202` regardless - so the only thing to prove here is that
    /// it never panics even when nothing is registered for the matched
    /// repository, logging the failure and returning instead.
    #[tokio::test]
    async fn test_dispatch_never_panics_for_an_unregistered_repository() {
        let store = Arc::new(InMemoryProjectStore::new());
        let launcher = launcher_with_projects(store).await;

        launcher
            .dispatch(munibot_vcs::ForgeEvent::IssueOpened {
                issue: munibot_vcs::IssueRef::new(cocoa_repo(), 1),
            })
            .await;
    }

    #[tokio::test]
    async fn test_executor_for_refuses_a_pipeline_whose_project_is_no_longer_registered() {
        let project_store = Arc::new(InMemoryProjectStore::new());
        let pipeline_store = Arc::new(InMemoryPipelineStore::new());
        let issue = munibot_vcs::IssueRef::new(cocoa_repo(), 1);
        let pipeline_id = pipeline_store.create_pipeline(&issue).await.unwrap();
        pipeline_store
            .append_event(pipeline_id, PipelineEvent::Triggered { issue })
            .await
            .unwrap();

        let launcher = launcher_with(project_store, pipeline_store).await;

        let Err(error) = launcher.executor_for(pipeline_id).await else {
            panic!("no project is registered for this pipeline's own repository");
        };
        assert!(matches!(error, LauncherError::ProjectNotRegistered(repo) if repo == cocoa_repo()));
    }

    /// `resume_all` has no `Result` to propagate a per-pipeline failure
    /// through either - every result is logged instead (see
    /// `PipelineLauncher::resume_all`'s own doc comment) - so, the same
    /// as `dispatch`, the only thing left to prove here is that it never
    /// panics.
    #[tokio::test]
    async fn test_resume_all_never_panics_when_a_pipelines_project_is_gone() {
        let project_store = Arc::new(InMemoryProjectStore::new());
        let pipeline_store = Arc::new(InMemoryPipelineStore::new());
        let issue = munibot_vcs::IssueRef::new(cocoa_repo(), 1);
        let pipeline_id = pipeline_store.create_pipeline(&issue).await.unwrap();
        pipeline_store
            .append_event(pipeline_id, PipelineEvent::Triggered { issue })
            .await
            .unwrap();

        let launcher = Arc::new(launcher_with(project_store, pipeline_store).await);
        launcher.resume_all().await;
    }
}
