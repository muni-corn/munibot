// ai.rs: builds the fully-wired `Ai` service for the running server.
//
// `Ai::new` alone returns a bare service with every optional capability
// (memory tools, memory store, usage recording, tool auditing, conversation
// compaction) left off - each is opt-in via its own `with_*` builder,
// precisely so existing constructor call sites and tests never had to
// change as those capabilities were added. This module is the one place
// that actually opts every one of them in for the real, running server.

use std::sync::Arc;

use munibot_ai::{
    Ai,
    audit::DieselToolAuditor,
    crisis::{CrisisClassifier, CrisisPersona},
    memory::{
        CompactionPersona, CompactionSettings, DieselMemoryOptIn, DieselMemoryStore,
        DieselSessionStore, GatedMemoryStore, MemoryStore, SessionStore, Summariser,
        register_memory_tools,
    },
    persona::AiConfig,
    provider::ProviderResolver,
    tools::ToolRegistry,
    types::ModelRef,
    usage::DieselUsageRecorder,
};
use munibot_core::db::DbPool;
use tracing::{info, warn};

/// The model the default persona is configured with, if one is set.
///
/// Both conversation compaction and crisis classification fall back to this
/// rather than adding their own config knobs for it: an operator who already
/// trusts a model for real conversation has no reason to want a different
/// one just to summarise it or screen it.
fn default_persona_model(config: &AiConfig) -> Option<ModelRef> {
    config
        .default_persona
        .as_ref()
        .and_then(|id| config.personas.get(id))
        .map(|persona| persona.model.clone())
}

/// Builds the AI service, or `None` when `ai.enabled` is `false` in config.
///
/// `pool` backs every diesel-based piece: conversation persistence, the
/// memory tools and store, usage recording, tool auditing, and (once a
/// provider for the default persona's model resolves) conversation
/// compaction.
pub async fn build(config: &AiConfig, pool: DbPool) -> anyhow::Result<Option<Arc<Ai>>> {
    if !config.enabled {
        info!("ai.enabled is false; skipping ai setup");
        return Ok(None);
    }

    let mut tools = ToolRegistry::from_env();
    register_memory_tools(&mut tools, pool.clone());

    let sessions: Arc<dyn SessionStore> = Arc::new(DieselSessionStore::new(pool.clone()));

    let mut ai = Ai::new(config, Arc::new(tools), sessions)?;

    let memory_store: Arc<dyn MemoryStore> = Arc::new(GatedMemoryStore::new(
        DieselMemoryStore::new(pool.clone()),
        DieselMemoryOptIn::new(pool.clone()),
    ));
    ai = ai
        .with_memory_store(memory_store)
        .with_usage_recorder(Arc::new(DieselUsageRecorder::new(pool.clone())))
        .with_tool_auditor(Arc::new(DieselToolAuditor::new(pool.clone())));

    let providers = ProviderResolver::new();
    match default_persona_model(config) {
        Some(model) => match providers.resolve(&model) {
            Ok(provider) => {
                let summariser =
                    Summariser::new(provider.clone(), CompactionPersona::embedded(model.clone()));
                ai = ai.with_summariser(summariser, CompactionSettings::default());

                // reuses the same resolved provider and model as compaction above,
                // for the same reason - see default_persona_model's own doc comment.
                // a genuinely cheaper, dedicated model for this is worth adding once
                // there is a real cost signal to justify a new config knob for it
                let classifier = CrisisClassifier::new(provider, CrisisPersona::embedded(model));
                ai = ai.with_crisis_classifier(classifier);
            }
            Err(error) => {
                warn!(
                    %error,
                    "couldn't resolve a provider for the default persona's model; \
                     conversations won't compact themselves and inbound messages won't be \
                     screened for crisis signals"
                );
            }
        },
        None => {
            warn!(
                "ai.default_persona isn't set; conversations won't compact themselves and inbound \
                 messages won't be screened for crisis signals"
            );
        }
    }

    Ok(Some(Arc::new(ai)))
}
