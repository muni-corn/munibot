// ai.rs: builds the fully-wired `Ai` service for the running server.
//
// `Ai::new` alone returns a bare service with every optional capability
// (memory tools, memory store, usage recording, tool auditing, conversation
// compaction, rate limiting, spend caps) left off - each is opt-in via its
// own `with_*` builder, precisely so existing constructor call sites and
// tests never had to change as those capabilities were added. This module
// is the one place that actually opts every one of them in for the real,
// running server.

use std::sync::Arc;

use munibot_ai::{
    Ai,
    abuse::{AbuseDetector, DieselAbuseStore},
    audit::DieselToolAuditor,
    crisis::{CrisisClassifier, CrisisPersona},
    limits::{DieselRateLimitStore, DieselSpendCapStore, RateLimiter, SpendCapEnforcer},
    memory::{
        CompactionPersona, CompactionSettings, DieselMemoryOptIn, DieselMemoryStore,
        DieselSessionStore, GatedMemoryStore, MemoryStore, SessionStore, Summariser,
        TitleGenerator, TitlePersona, register_memory_tools,
    },
    moderation::OpenAiModerator,
    persona::AiConfig,
    pipeline::{ConcurrencyConfig, DieselPipelineStore, PipelineRegistry},
    project::{DieselProjectStore, ProjectManager, ProjectsSettings},
    provider::ProviderResolver,
    safety::DieselSafetyEventAuditor,
    tools::{DelegablePersona, DelegateTool, Delegator, DelegatorCell, ToolRegistry},
    types::ModelRef,
    usage::DieselUsageRecorder,
};
use munibot_core::db::DbPool;
use munibot_gui::server::pipeline_launcher::PipelineLauncher;
use tracing::{info, warn};

/// Builds the pipeline launcher, or `None` when `GITHUB_APP_ID` isn't
/// set - a deployment with no GitHub App configured at all is one where
/// the autonomous pipeline is simply off, the same `Option` convention
/// `ai` itself and `OpenAiModerator::from_env` both already establish.
///
/// A malformed `GITHUB_APP_ID` (not a number) or an `AiConfig` that
/// fails to resolve a `PersonaRegistry` from (a pipeline role's persona
/// misconfigured the same way any chat persona could be) both log and
/// return `None` rather than failing the whole server's startup over a
/// feature that was always optional.
fn build_pipeline_launcher(
    config: &AiConfig,
    pool: DbPool,
    project_manager: Arc<ProjectManager>,
    registry: Arc<PipelineRegistry>,
) -> Option<Arc<PipelineLauncher>> {
    let Ok(app_id) = std::env::var("GITHUB_APP_ID") else {
        info!("GITHUB_APP_ID isn't set; the autonomous pipeline is disabled");
        return None;
    };
    let Ok(app_id) = app_id.parse::<u64>() else {
        warn!("GITHUB_APP_ID isn't a number; the autonomous pipeline is disabled");
        return None;
    };
    let Ok(private_key) = std::env::var("GITHUB_APP_PRIVATE_KEY") else {
        warn!(
            "GITHUB_APP_ID is set but GITHUB_APP_PRIVATE_KEY isn't; the autonomous pipeline is \
             disabled"
        );
        return None;
    };
    let bot_login =
        std::env::var("GITHUB_BOT_LOGIN").unwrap_or_else(|_| "munibot[bot]".to_string());

    let launcher = match PipelineLauncher::new(
        app_id,
        private_key,
        bot_login,
        Arc::new(DieselPipelineStore::new(pool.clone())),
        project_manager,
        config,
        registry,
    ) {
        Ok(launcher) => launcher,
        Err(error) => {
            warn!(%error, "couldn't build the pipeline launcher; the autonomous pipeline is disabled");
            return None;
        }
    };

    Some(Arc::new(
        launcher
            .with_auditor(Arc::new(DieselToolAuditor::new(pool.clone())))
            .with_usage_recorder(Arc::new(DieselUsageRecorder::new(pool))),
    ))
}

/// The model the default persona is configured with, if one resolves to one.
///
/// Both conversation compaction and crisis classification fall back to this
/// rather than adding their own config knobs for it: an operator who already
/// trusts a model for real conversation has no reason to want a different
/// one just to summarise it or screen it.
///
/// Mirrors `PersonaRegistry::load`'s own two fallbacks, since this runs
/// against the raw `AiConfig` rather than the resolved registry: `companion`
/// when `default_persona` isn't set at all, and `ai.default_model` when the
/// resolved persona (explicitly configured or an embedded default) doesn't
/// name its own model.
fn default_persona_model(config: &AiConfig) -> Option<ModelRef> {
    let default_persona = config
        .default_persona
        .clone()
        .unwrap_or_else(|| munibot_ai::persona::PersonaId::new("companion"));

    config
        .personas
        .get(&default_persona)
        .and_then(|persona| persona.model.clone())
        .or_else(|| config.default_model.clone())
}

/// Every persona configured with `delegable = true`, for the `delegate`
/// tool's own input schema - computed from config directly, since a
/// resolved `PersonaRegistry` only exists once `Ai::new` runs, and the tool
/// has to be registered before that.
fn delegable_personas(config: &AiConfig) -> Vec<DelegablePersona> {
    config
        .personas
        .iter()
        .filter(|(_, persona)| persona.delegable)
        .map(|(id, persona)| DelegablePersona {
            id: id.clone(),
            description: persona.description.clone(),
        })
        .collect()
}

/// Builds the AI service and the project manager every chat conversation
/// and pipeline run shares, or `None` for the service half when
/// `ai.enabled` is `false` in config.
///
/// `pool` backs every diesel-based piece: conversation persistence, the
/// memory tools and store, usage recording, tool auditing, rate limit
/// windows, spend caps, and (once a provider for the default persona's
/// model resolves) conversation compaction. The `delegate` tool is
/// registered here too, its `DelegatorCell` completed once `ai` itself
/// exists (see that type's own doc comment for why it can't just hold an
/// `Arc<Ai>` from the start).
///
/// The project manager and pipeline registry are both built
/// unconditionally, before `ai` -- the same "always-on infrastructure"
/// precedent this module's own caller already sets for the project
/// manager -- specifically so the project manager can be wired into
/// `ai` via `with_project_manager` while `ai` was still owned, not yet
/// behind the `Arc` a chat-bound conversation's `coder` turns would
/// otherwise never be able to resolve a real project workspace through.
/// Both are returned alongside `ai` so the caller can also inject them
/// as axum `Extension`s for the project and pipeline-monitor server
/// functions.
///
/// The pipeline launcher, in contrast, really is optional: `None`
/// whenever `GITHUB_APP_ID` isn't configured, the same convention `ai`
/// itself follows for `ai.enabled` - see [`build_pipeline_launcher`].
pub async fn build(
    config: &AiConfig,
    pool: DbPool,
    projects_settings: ProjectsSettings,
) -> anyhow::Result<(
    Option<Arc<Ai>>,
    Arc<ProjectManager>,
    Arc<PipelineRegistry>,
    Option<Arc<PipelineLauncher>>,
)> {
    let project_manager = Arc::new(ProjectManager::new(
        Arc::new(DieselProjectStore::new(pool.clone())),
        projects_settings,
    ));
    let pipeline_registry = Arc::new(PipelineRegistry::new(ConcurrencyConfig::default()));
    let pipeline_launcher = build_pipeline_launcher(
        config,
        pool.clone(),
        project_manager.clone(),
        pipeline_registry.clone(),
    );

    if !config.enabled {
        info!("ai.enabled is false; skipping ai setup");
        return Ok((None, project_manager, pipeline_registry, pipeline_launcher));
    }

    let mut tools = ToolRegistry::from_env();
    register_memory_tools(&mut tools, pool.clone());

    // completed below, once ai actually exists - see DelegatorCell's own
    // doc comment for why the delegate tool cannot just hold an Arc<Ai>
    let delegator_cell = Arc::new(DelegatorCell::new());
    tools.register(Arc::new(DelegateTool::new(
        delegator_cell.clone(),
        delegable_personas(config),
        config.max_delegation_depth,
    )));

    let sessions: Arc<dyn SessionStore> = Arc::new(DieselSessionStore::new(pool.clone()));

    let mut ai = Ai::new(config, Arc::new(tools), sessions)?;

    let memory_store: Arc<dyn MemoryStore> = Arc::new(GatedMemoryStore::new(
        DieselMemoryStore::new(pool.clone()),
        DieselMemoryOptIn::new(pool.clone()),
    ));
    let rate_limiter = RateLimiter::new(
        Arc::new(DieselRateLimitStore::new(pool.clone())),
        config.rate_limits.resolve(),
    );
    let spend_cap_enforcer = SpendCapEnforcer::new(
        Arc::new(DieselSpendCapStore::new(pool.clone())),
        config.spend_caps.resolve(),
    );
    let (cooldown_policy, detection_thresholds) = config.abuse.resolve();
    let abuse_detector = AbuseDetector::with_thresholds(
        Arc::new(DieselAbuseStore::new(pool.clone())),
        cooldown_policy,
        detection_thresholds,
    );
    ai = ai
        .with_memory_store(memory_store)
        .with_usage_recorder(Arc::new(DieselUsageRecorder::new(pool.clone())))
        .with_tool_auditor(Arc::new(DieselToolAuditor::new(pool.clone())))
        .with_rate_limiter(Arc::new(rate_limiter))
        .with_spend_cap_enforcer(Arc::new(spend_cap_enforcer))
        .with_abuse_detector(Arc::new(abuse_detector))
        .with_safety_auditor(Arc::new(DieselSafetyEventAuditor::new(pool.clone())));

    // opt-in: moderation only ever runs where a provider actually has an
    // endpoint for it (today, only OpenAI) - see OpenAiModerator's own doc
    // comment. Without OPENAI_API_KEY set at all, munibot boots exactly as
    // it did before moderation existed, the same as every other with_*
    // capability above.
    match OpenAiModerator::from_env() {
        Some(moderator) => ai = ai.with_moderator(Arc::new(moderator)),
        None => {
            info!("OPENAI_API_KEY isn't set; provider moderation is disabled for every persona")
        }
    }

    let providers = ProviderResolver::new();
    match default_persona_model(config) {
        Some(model) => match providers.resolve(&model) {
            Ok(provider) => {
                // compaction, crisis classification, and title generation all reuse
                // this same resolved provider and model - see
                // default_persona_model's own doc comment. a genuinely cheaper,
                // dedicated model for these is worth adding once there is a real
                // cost signal to justify a new config knob for it
                let summariser =
                    Summariser::new(provider.clone(), CompactionPersona::embedded(model.clone()));
                ai = ai.with_summariser(summariser, CompactionSettings::default());

                let classifier =
                    CrisisClassifier::new(provider.clone(), CrisisPersona::embedded(model.clone()));
                ai = ai.with_crisis_classifier(classifier);

                let title_generator = TitleGenerator::new(provider, TitlePersona::embedded(model));
                ai = ai.with_title_generator(title_generator);
            }
            Err(error) => {
                warn!(
                    %error,
                    "couldn't resolve a provider for the default persona's model; \
                     conversations won't compact themselves, inbound messages won't be \
                     screened for crisis signals, and conversations won't be named \
                     automatically"
                );
            }
        },
        None => {
            warn!(
                "ai.default_persona isn't set; conversations won't compact themselves, inbound \
                 messages won't be screened for crisis signals, and conversations won't be named \
                 automatically"
            );
        }
    }

    ai = ai.with_project_manager(project_manager.clone());

    let ai = Arc::new(ai);
    delegator_cell.set(Arc::downgrade(&ai) as std::sync::Weak<dyn Delegator>);

    Ok((
        Some(ai),
        project_manager,
        pipeline_registry,
        pipeline_launcher,
    ))
}
