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
/// Builds the `GitHubForge` a pipeline run authenticates as, given the
/// installation id its own project was registered under.
///
/// A trait, rather than naming `GitHubForge::new` directly in
/// [`PipelineLauncher`], so a test can substitute one that points at a
/// mocked HTTP server (`GitHubForge::from_parts`) instead of ever
/// reaching the real GitHub API - the same seam `AgentDispatcher` and
/// `ProviderSource` already are elsewhere in this stack.
pub trait ForgeBuilder: Send + Sync {
    fn build(
        &self,
        installation_id: InstallationId,
    ) -> Result<GitHubForge, munibot_github::GitHubError>;
}

/// The real implementation: [`GitHubForge::new`], authenticating as a
/// real GitHub App installation.
struct RealForgeBuilder {
    app_id: AppId,
    private_key: String,
}

impl ForgeBuilder for RealForgeBuilder {
    fn build(
        &self,
        installation_id: InstallationId,
    ) -> Result<GitHubForge, munibot_github::GitHubError> {
        GitHubForge::new(self.app_id, &self.private_key, installation_id)
    }
}

pub struct PipelineLauncher {
    forge_builder: Arc<dyn ForgeBuilder>,
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
            forge_builder: Arc::new(RealForgeBuilder {
                app_id: AppId(app_id),
                private_key,
            }),
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

    /// Overrides how a `GitHubForge` gets built - what a test substitutes
    /// a mock-server-backed [`ForgeBuilder`] through, instead of
    /// [`Self::new`]'s real, GitHub-authenticating one.
    #[cfg(test)]
    pub(crate) fn with_forge_builder(mut self, builder: Arc<dyn ForgeBuilder>) -> Self {
        self.forge_builder = builder;
        self
    }

    /// Overrides the persona registry and provider source a pipeline
    /// role's own turn resolves against - what a test substitutes a
    /// `MockProvider` through, so a launch can be proven end to end
    /// without ever calling a real model or spending real money.
    #[cfg(test)]
    pub(crate) fn with_test_providers(
        mut self,
        personas: Arc<PersonaRegistry>,
        providers: Arc<dyn ProviderSource>,
    ) -> Self {
        self.personas = personas;
        self.providers = providers;
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
        Ok(self
            .forge_builder
            .build(InstallationId(installation_id.unsigned_abs()))?)
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
        persona::{
            AbuseConfig, BudgetConfig, MemoryPolicy, PersonaConfig, PersonaId, RateLimitConfig,
            SandboxPolicy, SpendCapConfig,
        },
        pipeline::{
            ConcurrencyConfig, InMemoryPipelineStore, IssueAnalysis, IssueClassification,
            RecommendedAction, ReproductionStatus,
        },
        project::{
            ImageStrategy, InMemoryProjectStore, NewProject, ProjectStore, ProjectsSettings,
        },
        provider::{MockProvider, Provider},
        sandbox::config::{SandboxNetworkConfig, SandboxSettings},
        tools::ToolSelection,
        types::{AiError, ModelRef},
    };
    use munibot_vcs::{Forge, RepoRef};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::server::webhooks::{self, WebhookConfig};

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

    // --- the full launch seam: a signed webhook -> project resolution
    // -> an executor start a concurrency slot can be aborted from ---

    /// Always returns the same fake token - real enough for
    /// `GitHubForge::from_parts`'s own token cache, which nothing in
    /// these tests ever actually inspects.
    struct FixedTokenMinter;

    #[async_trait]
    impl munibot_github::TokenMinter for FixedTokenMinter {
        async fn mint(
            &self,
            _installation_id: InstallationId,
        ) -> Result<String, munibot_github::GitHubError> {
            Ok("fake-installation-token".to_string())
        }
    }

    /// Points `GitHubForge` at a mocked HTTP server instead of the real
    /// GitHub API - see `ForgeBuilder`'s own doc comment.
    struct MockForgeBuilder {
        uri: String,
    }

    impl ForgeBuilder for MockForgeBuilder {
        fn build(
            &self,
            installation_id: InstallationId,
        ) -> Result<GitHubForge, munibot_github::GitHubError> {
            let client = octocrab::Octocrab::builder()
                .base_uri(&self.uri)
                .unwrap()
                .build()
                .unwrap();
            Ok(GitHubForge::from_parts(
                installation_id,
                client,
                munibot_github::InstallationTokenCache::new(FixedTokenMinter),
            ))
        }
    }

    /// A [`ProviderSource`] that always returns the same fixed provider -
    /// what lets a pipeline role's turn run against a real `HarnessDispatcher`
    /// without ever calling a real model.
    struct FixedProviderSource(Arc<dyn Provider>);

    impl ProviderSource for FixedProviderSource {
        fn resolve(&self, _model: &ModelRef) -> Result<Arc<dyn Provider>, AiError> {
            Ok(self.0.clone())
        }
    }

    /// A real, offline-resolved `PersonaRegistry` with one persona at
    /// `issue-analyst`'s own conventional id and embedded prompt -
    /// enough for `HarnessDispatcher` to resolve `AgentRole::IssueAnalyst`
    /// against it exactly as it does in production.
    fn issue_analyst_personas() -> Arc<PersonaRegistry> {
        let mut config = empty_ai_config();
        config
            .personas
            .insert(PersonaId::new("issue-analyst"), PersonaConfig {
                model: Some(ModelRef::new("anthropic", "claude-opus-5")),
                prompt: "issue-analyst.md".to_string(),
                display_name: Some("Issue Analyst".to_string()),
                description: "triages an incoming issue".to_string(),
                temperature: None,
                tools: ToolSelection::none(),
                budget: BudgetConfig::default(),
                memory: MemoryPolicy::None,
                sandbox: SandboxPolicy::default(),
                delegable: false,
                moderation_fail_closed: None,
            });
        let providers =
            munibot_ai::provider::ProviderRegistry::from_available(["anthropic".to_string()]);
        Arc::new(PersonaRegistry::load(&config, &providers).expect("should resolve"))
    }

    /// An empty bare repository - enough for `existing_branches` (a
    /// plain `git for-each-ref`) to run without error. This scenario's
    /// own pipeline never advances past `Triaging`, so nothing ever
    /// actually checks out a workspace from it.
    async fn bare_repo_fixture() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "munibot_gui_pipeline_launch_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bare = root.join("repo.git");
        tokio::process::Command::new("git")
            .args([
                "init",
                "--bare",
                "--initial-branch",
                "main",
                &bare.display().to_string(),
            ])
            .status()
            .await
            .unwrap();
        root
    }

    /// A minimal, real `models::issues::Issue` payload - every field
    /// that type actually requires, per octocrab's own struct
    /// definition, nothing more.
    fn issue_response_body() -> serde_json::Value {
        let author = serde_json::json!({
            "login": "someone", "id": 1, "node_id": "abc",
            "avatar_url": "https://example.com/a", "gravatar_id": "",
            "url": "https://api.github.com/users/someone",
            "html_url": "https://github.com/someone",
            "followers_url": "https://api.github.com/users/someone/followers",
            "following_url": "https://api.github.com/users/someone/following{/other_user}",
            "gists_url": "https://api.github.com/users/someone/gists{/gist_id}",
            "starred_url": "https://api.github.com/users/someone/starred{/owner}{/repo}",
            "subscriptions_url": "https://api.github.com/users/someone/subscriptions",
            "organizations_url": "https://api.github.com/users/someone/orgs",
            "repos_url": "https://api.github.com/users/someone/repos",
            "events_url": "https://api.github.com/users/someone/events{/privacy}",
            "received_events_url": "https://api.github.com/users/someone/received_events",
            "type": "User", "site_admin": false
        });
        serde_json::json!({
            "id": 1,
            "node_id": "abc",
            "url": "https://api.github.com/repos/muni-corn/cocoa/issues/17",
            "repository_url": "https://api.github.com/repos/muni-corn/cocoa",
            "labels_url": "https://api.github.com/repos/muni-corn/cocoa/issues/17/labels{/name}",
            "comments_url": "https://api.github.com/repos/muni-corn/cocoa/issues/17/comments",
            "events_url": "https://api.github.com/repos/muni-corn/cocoa/issues/17/events",
            "html_url": "https://github.com/muni-corn/cocoa/issues/17",
            "number": 17,
            "state": "open",
            "title": "the app crashes on startup",
            "body": "steps to reproduce...",
            "user": author,
            "labels": [],
            "assignees": [],
            "locked": false,
            "comments": 0,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        })
    }

    const WEBHOOK_SECRET: &str = "it's a secret to everybody";
    const ISSUE_OPENED_PAYLOAD: &str = r#"{
        "action": "opened",
        "issue": { "number": 17, "title": "the app crashes on startup", "body": "steps to reproduce..." },
        "repository": { "name": "cocoa", "owner": { "login": "muni-corn" } },
        "sender": { "login": "someone" }
    }"#;

    fn signed_webhook_body(secret: &str, body: &str) -> (axum::body::Bytes, String) {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body.as_bytes());
        let signature = format!(
            "sha256={}",
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        (axum::body::Bytes::from(body.to_string()), signature)
    }

    fn webhook_request(
        event_type: &str,
        body: axum::body::Bytes,
        signature: &str,
    ) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .uri("/webhooks/github")
            .header("X-GitHub-Event", event_type)
            .header("X-Hub-Signature-256", signature)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body))
            .unwrap()
    }

    /// The seam this whole milestone exists to build, proven end to
    /// end: a signed webhook payload, verified and normalized, resolves
    /// to a registered project via `trigger_for`, reaches
    /// `PipelineLauncher::dispatch` -> `launch`, authenticates as that
    /// project's own (mocked) GitHub App installation, creates the
    /// `ai_pipelines` row, and runs a real turn against `MockProvider` -
    /// no real model called, no money spent - all the way to `Complete`.
    #[tokio::test]
    async fn test_a_signed_webhook_delivery_creates_and_runs_a_pipeline() {
        use tower::ServiceExt;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/muni-corn/cocoa/issues/17"))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_response_body()))
            .mount(&server)
            .await;

        let root = bare_repo_fixture().await;
        let project_store = Arc::new(InMemoryProjectStore::new());
        project_store
            .upsert_project(&NewProject {
                slug: "cocoa".to_string(),
                display_name: "Cocoa".to_string(),
                repo: RepoRef::new(Forge::GitHub, "muni-corn", "cocoa"),
                clone_url: "https://example.invalid/cocoa.git".to_string(),
                default_branch: "main".to_string(),
                local_path: root.clone(),
                image_strategy: ImageStrategy::Base,
                network_policy: SandboxNetworkConfig::None,
                trigger: Some(munibot_vcs::TriggerMode::AllIssues),
                installation_id: Some(999),
            })
            .await
            .unwrap();

        let pipeline_store = Arc::new(InMemoryPipelineStore::new());
        let project_manager = Arc::new(ProjectManager::new(
            project_store,
            ProjectsSettings::default(),
        ));
        let registry = Arc::new(PipelineRegistry::new(ConcurrencyConfig::default()));

        let provider: Arc<dyn Provider> = Arc::new(
            MockProvider::new().respond_tool_use(
                "c1",
                "handoff",
                serde_json::to_value(IssueAnalysis {
                    classification: IssueClassification::NotActionable,
                    reproduction_status: ReproductionStatus::NotApplicable,
                    summary: "not actionable".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::Skip,
                    relevant_files: vec![],
                })
                .unwrap(),
            ),
        );

        let launcher = PipelineLauncher::new(
            123456,
            "unused - the forge builder is overridden below".to_string(),
            "munibot[bot]".to_string(),
            pipeline_store.clone(),
            project_manager.clone(),
            &empty_ai_config(),
            registry,
        )
        .expect("should build with a real persona configured")
        .with_forge_builder(Arc::new(MockForgeBuilder { uri: server.uri() }))
        .with_test_providers(
            issue_analyst_personas(),
            Arc::new(FixedProviderSource(provider)),
        );

        let webhook_config = Arc::new(WebhookConfig {
            webhook_secret: Some(WEBHOOK_SECRET.to_string()),
            bot_login: "munibot[bot]".to_string(),
            project_manager,
            dispatch: Some(Arc::new(launcher) as Arc<dyn PipelineDispatch>),
        });
        let app = webhooks::router().layer(axum::Extension(webhook_config));

        let (body, signature) = signed_webhook_body(WEBHOOK_SECRET, ISSUE_OPENED_PAYLOAD);
        let response = app
            .oneshot(webhook_request("issues", body, &signature))
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::ACCEPTED);

        // handle_webhook spawns process_delivery, which itself calls
        // dispatch -> launch asynchronously - poll for the pipeline row
        // it creates, then for the spawned run to actually finish
        let mut pipeline_id = None;
        for _ in 0..200 {
            let ids = pipeline_store.all_pipeline_ids().await.unwrap();
            if let Some(id) = ids.first() {
                pipeline_id = Some(*id);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let pipeline_id = pipeline_id.expect(
            "the signed webhook delivery should have resolved the project and created a pipeline \
             row",
        );

        let mut final_state = None;
        for _ in 0..200 {
            let state = pipeline_store.replay(pipeline_id).await.unwrap();
            if state.is_terminal() {
                final_state = Some(state);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            final_state,
            Some(munibot_ai::pipeline::PipelineState::Complete),
            "a NotActionable/Skip analysis should resolve the whole run to Complete"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// `launch` reserves a concurrency slot synchronously, before ever
    /// spawning the task that actually runs a turn - so, called
    /// directly (never through the spawned `process_delivery` a real
    /// webhook delivery goes through), the registry entry is guaranteed
    /// to exist the instant this returns, deterministically, regardless
    /// of how quickly the spawned run itself later completes.
    /// `PipelineRegistry::abort_pipeline` removes that same entry
    /// synchronously too - see that function's own first line - so this
    /// needs no polling and races nothing.
    #[tokio::test]
    async fn test_launch_reserves_a_concurrency_slot_that_abort_releases() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/muni-corn/cocoa/issues/17"))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_response_body()))
            .mount(&server)
            .await;

        let root = bare_repo_fixture().await;
        let project_store = Arc::new(InMemoryProjectStore::new());
        let project = project_store
            .upsert_project(&NewProject {
                slug: "cocoa".to_string(),
                display_name: "Cocoa".to_string(),
                repo: RepoRef::new(Forge::GitHub, "muni-corn", "cocoa"),
                clone_url: "https://example.invalid/cocoa.git".to_string(),
                default_branch: "main".to_string(),
                local_path: root.clone(),
                image_strategy: ImageStrategy::Base,
                network_policy: SandboxNetworkConfig::None,
                trigger: Some(munibot_vcs::TriggerMode::AllIssues),
                installation_id: Some(999),
            })
            .await
            .unwrap();

        let pipeline_store = Arc::new(InMemoryPipelineStore::new());
        let project_manager = Arc::new(ProjectManager::new(
            project_store,
            ProjectsSettings::default(),
        ));
        let registry = Arc::new(PipelineRegistry::new(ConcurrencyConfig::default()));

        let provider: Arc<dyn Provider> = Arc::new(
            MockProvider::new().respond_tool_use(
                "c1",
                "handoff",
                serde_json::to_value(IssueAnalysis {
                    classification: IssueClassification::NotActionable,
                    reproduction_status: ReproductionStatus::NotApplicable,
                    summary: "not actionable".to_string(),
                    reproduction_details: String::new(),
                    recommended_action: RecommendedAction::Skip,
                    relevant_files: vec![],
                })
                .unwrap(),
            ),
        );

        let launcher = PipelineLauncher::new(
            123456,
            "unused - the forge builder is overridden below".to_string(),
            "munibot[bot]".to_string(),
            pipeline_store,
            project_manager,
            &empty_ai_config(),
            registry.clone(),
        )
        .expect("should build with a real persona configured")
        .with_forge_builder(Arc::new(MockForgeBuilder { uri: server.uri() }))
        .with_test_providers(
            issue_analyst_personas(),
            Arc::new(FixedProviderSource(provider)),
        );

        let issue = munibot_vcs::IssueRef::new(project.repo.clone(), 17);
        let outcome = launcher.launch(issue).await.expect("should launch");
        let LaunchOutcome::Started(pipeline_id) = outcome else {
            panic!("a fresh registry always has a free concurrency slot, got {outcome:?}");
        };

        assert!(
            registry.is_running(pipeline_id),
            "try_start reserves the slot synchronously, before launch() ever returns"
        );

        assert!(
            registry.abort_pipeline(pipeline_id).await,
            "aborting a pipeline still holding its slot should report it was actually running"
        );
        assert!(!registry.is_running(pipeline_id));

        std::fs::remove_dir_all(&root).ok();
    }
}
