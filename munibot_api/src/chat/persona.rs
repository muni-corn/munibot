use serde::{Deserialize, Serialize};

/// A persona available to chat with, as shown in a picker.
///
/// Deliberately slim, the same philosophy as [`crate::guilds::GuildSummary`]:
/// a picker and the future catalogue page both need only enough to list and
/// describe a persona, not its model, budget, or prompt.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PersonaSummary {
    pub id: String,
    pub display_name: String,
    pub description: String,
}

#[cfg(feature = "server")]
impl From<&munibot_ai::persona::Persona> for PersonaSummary {
    fn from(persona: &munibot_ai::persona::Persona) -> Self {
        Self {
            id: persona.id.to_string(),
            display_name: persona.display_name.clone(),
            description: persona.description.clone(),
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
        }
    }

    #[test]
    fn test_from_persona_carries_id_name_and_description() {
        let summary = PersonaSummary::from(&persona());
        assert_eq!(summary.id, "companion");
        assert_eq!(summary.display_name, "Companion");
        assert_eq!(summary.description, "warm, playful conversation");
    }
}
