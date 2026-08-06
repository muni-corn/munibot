use std::{collections::HashMap, sync::Arc};

use munibot_ai::{Ai, persona::PersonaId};
use poise::serenity_prelude::ChannelId;
use tokio::sync::Mutex;

/// In-memory persona pins per Discord channel, for `/persona`'s pinning half.
///
/// Becomes a real `ai_conversations.pinned_persona` column read by the router
/// in milestone 2 phase 11; this exists only so the feature has somewhere to
/// live before that lands, and is lost on restart like the rest of this
/// milestone's conversation state.
#[derive(Clone, Default)]
pub struct PinnedPersonas {
    pinned: Arc<Mutex<HashMap<ChannelId, PersonaId>>>,
}

impl PinnedPersonas {
    pub fn new() -> Self {
        Self::default()
    }

    /// The channel's pinned persona, if one has been set.
    pub async fn get(&self, channel_id: ChannelId) -> Option<PersonaId> {
        self.pinned.lock().await.get(&channel_id).cloned()
    }

    /// Pins a channel to a persona, replacing any existing pin.
    pub async fn set(&self, channel_id: ChannelId, persona_id: PersonaId) {
        self.pinned.lock().await.insert(channel_id, persona_id);
    }

    /// Removes a channel's pin, if it had one. Returns whether one was
    /// actually removed.
    pub async fn unset(&self, channel_id: ChannelId) -> bool {
        self.pinned.lock().await.remove(&channel_id).is_some()
    }

    /// The persona that should answer in `channel_id` right now: the pinned
    /// one if set, falling back to `ai`'s configured default.
    pub async fn effective(&self, channel_id: ChannelId, ai: &Ai) -> Option<PersonaId> {
        match self.get(channel_id).await {
            Some(pinned) => Some(pinned),
            None => ai.default_persona_id().cloned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use munibot_ai::{
        persona::{
            AiConfig, BudgetConfig, MemoryPolicy, PersonaConfig, PersonaRegistry, SandboxPolicy,
        },
        provider::{MockProvider, ProviderRegistry},
        tools::ToolSelection,
        types::ModelRef,
    };

    use super::*;

    fn channel(id: u64) -> ChannelId {
        ChannelId::new(id)
    }

    /// A minimal, offline-resolved `Ai` with one persona, "companion", set as
    /// the default - enough to exercise `effective`'s fallback without any
    /// real credentials or network access.
    fn ai_with_default_companion() -> Ai {
        let mut personas = HashMap::new();
        personas.insert(PersonaId::new("companion"), PersonaConfig {
            model: ModelRef::new("anthropic", "claude-opus-5"),
            prompt: "companion.md".to_string(),
            display_name: None,
            description: String::new(),
            temperature: None,
            tools: ToolSelection::none(),
            budget: BudgetConfig::default(),
            memory: MemoryPolicy::default(),
            sandbox: SandboxPolicy::default(),
        });
        let config = AiConfig {
            enabled: true,
            default_persona: Some(PersonaId::new("companion")),
            prompt_dir: None,
            crisis_resources: Vec::new(),
            rate_limits: munibot_ai::persona::RateLimitConfig::default(),
            spend_caps: munibot_ai::persona::SpendCapConfig::default(),
            personas,
        };
        let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");

        Ai::from_parts(
            registry,
            Arc::new(munibot_ai::tools::ToolRegistry::new()),
            Arc::new(munibot_ai::memory::InMemorySessionStore::new()),
            Arc::new(FixedProvider),
        )
    }

    /// A [`munibot_ai::service::ProviderSource`] stub - `effective` never
    /// actually resolves a provider, so this is never called, but `Ai`
    /// requires one to be constructed.
    struct FixedProvider;

    impl munibot_ai::service::ProviderSource for FixedProvider {
        fn resolve(
            &self,
            _model: &ModelRef,
        ) -> Result<Arc<dyn munibot_ai::provider::Provider>, munibot_ai::AiError> {
            Ok(Arc::new(MockProvider::new()))
        }
    }

    #[tokio::test]
    async fn test_effective_falls_back_to_ais_default_persona_when_unpinned() {
        let pins = PinnedPersonas::new();
        let ai = ai_with_default_companion();

        assert_eq!(
            pins.effective(channel(1), &ai).await,
            Some(PersonaId::new("companion"))
        );
    }

    #[tokio::test]
    async fn test_effective_prefers_the_pin_over_the_default() {
        let pins = PinnedPersonas::new();
        let ai = ai_with_default_companion();
        pins.set(channel(1), PersonaId::new("researcher")).await;

        assert_eq!(
            pins.effective(channel(1), &ai).await,
            Some(PersonaId::new("researcher"))
        );
    }

    #[tokio::test]
    async fn test_an_unpinned_channel_has_no_pin() {
        let pins = PinnedPersonas::new();
        assert_eq!(pins.get(channel(1)).await, None);
    }

    #[tokio::test]
    async fn test_setting_a_pin_makes_it_retrievable() {
        let pins = PinnedPersonas::new();
        pins.set(channel(1), PersonaId::new("researcher")).await;
        assert_eq!(
            pins.get(channel(1)).await,
            Some(PersonaId::new("researcher"))
        );
    }

    #[tokio::test]
    async fn test_setting_a_pin_again_replaces_it() {
        let pins = PinnedPersonas::new();
        pins.set(channel(1), PersonaId::new("researcher")).await;
        pins.set(channel(1), PersonaId::new("coder")).await;
        assert_eq!(pins.get(channel(1)).await, Some(PersonaId::new("coder")));
    }

    #[tokio::test]
    async fn test_pins_do_not_cross_channels() {
        let pins = PinnedPersonas::new();
        pins.set(channel(1), PersonaId::new("researcher")).await;
        assert_eq!(pins.get(channel(2)).await, None);
    }

    #[tokio::test]
    async fn test_unset_removes_an_existing_pin_and_reports_it_was_removed() {
        let pins = PinnedPersonas::new();
        pins.set(channel(1), PersonaId::new("researcher")).await;

        assert!(pins.unset(channel(1)).await);
        assert_eq!(pins.get(channel(1)).await, None);
    }

    #[tokio::test]
    async fn test_unset_on_an_unpinned_channel_reports_nothing_was_removed() {
        let pins = PinnedPersonas::new();
        assert!(!pins.unset(channel(1)).await);
    }
}
