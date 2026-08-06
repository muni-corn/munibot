use serde::{Deserialize, Serialize};

/// A persona available to chat with, as shown in the picker and the
/// catalogue page.
///
/// Deliberately slim, the same philosophy as [`crate::guilds::GuildSummary`]:
/// just enough to list, describe, and distinguish a persona, not its full
/// budget or prompt.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PersonaSummary {
    pub id: String,
    pub display_name: String,
    pub description: String,
    /// A display string (`"provider:model"`), for the catalogue page - see
    /// `munibot_ai::types::ModelRef`'s own `Display` impl, which this
    /// mirrors rather than depends on directly (this type has to compile
    /// for wasm, and that one is behind a server-only dependency).
    pub model: String,
    /// Whether this persona remembers things about the person it's talking
    /// to, across separate conversations - `MemoryPolicy::User` specifically,
    /// not just recalling the current conversation, which every persona
    /// does regardless.
    pub remembers_you: bool,
}

#[cfg(feature = "server")]
impl From<&munibot_ai::persona::Persona> for PersonaSummary {
    fn from(persona: &munibot_ai::persona::Persona) -> Self {
        Self {
            id: persona.id.to_string(),
            display_name: persona.display_name.clone(),
            description: persona.description.clone(),
            model: persona.model.to_string(),
            remembers_you: persona.memory == munibot_ai::persona::MemoryPolicy::User,
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use munibot_ai::{
        persona::{Persona, PersonaId},
        types::ModelRef,
    };

    use super::*;

    fn persona() -> Persona {
        Persona {
            id: PersonaId::new("companion"),
            display_name: "Companion".to_string(),
            description: "warm, playful conversation".to_string(),
            model: ModelRef::new("anthropic", "claude-opus-5"),
            params: Default::default(),
            system_prompt: munibot_ai::persona::PromptTemplate::new("be kind"),
            tools: Default::default(),
            budget: Default::default(),
            handoff: None,
            memory: Default::default(),
            sandbox: Default::default(),
            delegable: false,
        }
    }

    #[test]
    fn test_from_persona_carries_id_name_and_description() {
        let summary = PersonaSummary::from(&persona());
        assert_eq!(summary.id, "companion");
        assert_eq!(summary.display_name, "Companion");
        assert_eq!(summary.description, "warm, playful conversation");
    }

    #[test]
    fn test_from_persona_formats_the_model_as_provider_colon_model() {
        let summary = PersonaSummary::from(&persona());
        assert_eq!(summary.model, "anthropic:claude-opus-5");
    }

    #[test]
    fn test_remembers_you_reflects_the_user_memory_policy_specifically() {
        let mut with_user_memory = persona();
        with_user_memory.memory = munibot_ai::persona::MemoryPolicy::User;
        assert!(PersonaSummary::from(&with_user_memory).remembers_you);

        let mut with_conversation_memory = persona();
        with_conversation_memory.memory = munibot_ai::persona::MemoryPolicy::Conversation;
        assert!(
            !PersonaSummary::from(&with_conversation_memory).remembers_you,
            "recalling the current conversation isn't the same as remembering across conversations"
        );

        assert!(
            !PersonaSummary::from(&persona()).remembers_you,
            "the default is MemoryPolicy::None"
        );
    }
}
