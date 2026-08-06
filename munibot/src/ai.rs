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
    memory::{
        CompactionPersona, CompactionSettings, DieselMemoryOptIn, DieselMemoryStore,
        DieselSessionStore, GatedMemoryStore, MemoryStore, SessionStore, Summariser,
        register_memory_tools,
    },
    persona::AiConfig,
    provider::ProviderResolver,
    tools::ToolRegistry,
    usage::DieselUsageRecorder,
};
use munibot_core::db::DbPool;
use tracing::{info, warn};

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

    // compaction needs a real, working provider - reuses the default
    // persona's own model rather than adding a separate config knob for it,
    // since an operator who already trusts that model for real conversation
    // has no reason to want a different one just for summarising it
    let compaction_model = config
        .default_persona
        .as_ref()
        .and_then(|id| config.personas.get(id))
        .map(|persona| persona.model.clone());
    match compaction_model {
        Some(model) => match ProviderResolver::new().resolve(&model) {
            Ok(provider) => {
                let summariser = Summariser::new(provider, CompactionPersona::embedded(model));
                ai = ai.with_summariser(summariser, CompactionSettings::default());
            }
            Err(error) => {
                warn!(
                    %error,
                    "couldn't resolve a provider for conversation compaction; long conversations \
                     won't compact themselves"
                );
            }
        },
        None => {
            warn!("ai.default_persona isn't set; conversations won't compact themselves");
        }
    }

    Ok(Some(Arc::new(ai)))
}
