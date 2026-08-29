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
    persona::AiConfig,
    provider::ProviderResolver,
    tools::{DelegablePersona, DelegateTool, Delegator, DelegatorCell, ToolRegistry},
    types::ModelRef,
    usage::DieselUsageRecorder,
};
use munibot_core::db::DbPool;
use tracing::{info, warn};

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

/// Builds the AI service, or `None` when `ai.enabled` is `false` in config.
///
/// `pool` backs every diesel-based piece: conversation persistence, the
/// memory tools and store, usage recording, tool auditing, rate limit
/// windows, spend caps, and (once a provider for the default persona's
/// model resolves) conversation compaction. The `delegate` tool is
/// registered here too, its `DelegatorCell` completed once `ai` itself
/// exists (see that type's own doc comment for why it can't just hold an
/// `Arc<Ai>` from the start).
pub async fn build(config: &AiConfig, pool: DbPool) -> anyhow::Result<Option<Arc<Ai>>> {
    if !config.enabled {
        info!("ai.enabled is false; skipping ai setup");
        return Ok(None);
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
        .with_abuse_detector(Arc::new(abuse_detector));

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

    let ai = Arc::new(ai);
    delegator_cell.set(Arc::downgrade(&ai) as std::sync::Weak<dyn Delegator>);

    Ok(Some(ai))
}
