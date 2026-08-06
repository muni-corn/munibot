use serde::{Deserialize, Serialize};

use crate::{
    harness::{Budget, HandoffSchema},
    persona::PromptTemplate,
    tools::ToolSelection,
    types::{ModelParams, ModelRef},
};

/// A persona's stable identifier, used in configuration, commands, and
/// eventually the database.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PersonaId(pub String);

impl PersonaId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for PersonaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// How much conversation history a persona's system prompt is given access to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPolicy {
    /// No memory beyond the current turn's own history.
    #[default]
    None,
    /// The current conversation's history and summary, once compaction exists.
    Conversation,
    /// Conversation history plus the invoking user's opted-in long-term
    /// memories.
    User,
}

/// Whether a persona may use the sandboxed filesystem and shell tools.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxPolicy {
    /// No sandbox is ever provisioned for this persona.
    #[default]
    Forbidden,
    /// A sandbox is provisioned lazily, on the first sandboxed tool call.
    Optional,
    /// A sandbox is provisioned before the turn begins.
    Required,
}

/// A model, a prompt, a tool allowlist, a budget, and everything else that
/// distinguishes one persona from another - the companion, the researcher, and
/// every pipeline agent role alike are all this same type.
#[derive(Clone, Debug)]
pub struct Persona {
    pub id: PersonaId,
    pub display_name: String,
    /// Shown to the router so it can choose between personas.
    pub description: String,
    pub model: ModelRef,
    pub params: ModelParams,
    pub system_prompt: PromptTemplate,
    pub tools: ToolSelection,
    pub budget: Budget,
    /// Structured terminal output. Chat personas leave this `None`; pipeline
    /// roles set it.
    pub handoff: Option<HandoffSchema>,
    pub memory: MemoryPolicy,
    pub sandbox: SandboxPolicy,
    /// Whether munibot may bring this persona in mid-conversation via the
    /// `delegate` tool. Defaults to `false` (see [`PersonaConfig::delegable`]),
    /// so an orchestration-only role is excluded by construction rather than
    /// by remembering to exclude it in every tool schema that lists
    /// candidates.
    pub delegable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persona() -> Persona {
        Persona {
            id: PersonaId::new("companion"),
            display_name: "Companion".to_string(),
            description: "warm, playful conversation".to_string(),
            model: ModelRef::new("anthropic", "claude-opus-5"),
            params: ModelParams::default(),
            system_prompt: PromptTemplate::new("be kind"),
            tools: ToolSelection::none(),
            budget: Budget::default(),
            handoff: None,
            memory: MemoryPolicy::default(),
            sandbox: SandboxPolicy::default(),
            delegable: false,
        }
    }

    #[test]
    fn test_persona_id_displays_its_inner_string() {
        assert_eq!(PersonaId::new("researcher").to_string(), "researcher");
    }

    #[test]
    fn test_persona_id_serializes_as_a_bare_string() {
        let encoded = serde_json::to_value(PersonaId::new("companion")).expect("should serialize");
        assert_eq!(encoded, serde_json::json!("companion"));
    }

    #[test]
    fn test_memory_policy_defaults_to_none() {
        assert_eq!(MemoryPolicy::default(), MemoryPolicy::None);
    }

    #[test]
    fn test_sandbox_policy_defaults_to_forbidden() {
        assert_eq!(SandboxPolicy::default(), SandboxPolicy::Forbidden);
    }

    #[test]
    fn test_memory_policy_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&MemoryPolicy::User).expect("should serialize");
        assert_eq!(encoded, "\"user\"");
    }

    #[test]
    fn test_sandbox_policy_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&SandboxPolicy::Required).expect("should serialize");
        assert_eq!(encoded, "\"required\"");
    }

    #[test]
    fn test_a_chat_persona_has_no_handoff() {
        // documenting the distinction the overview draws between chat personas and
        // pipeline roles: this is the only field that tells them apart at the
        // type level
        let chat_persona = persona();
        assert!(chat_persona.handoff.is_none());
    }

    #[test]
    fn test_persona_fields_are_reachable() {
        let persona = persona();
        assert_eq!(persona.id, PersonaId::new("companion"));
        assert_eq!(persona.model, ModelRef::new("anthropic", "claude-opus-5"));
        assert_eq!(persona.memory, MemoryPolicy::None);
        assert_eq!(persona.sandbox, SandboxPolicy::Forbidden);
    }

    #[test]
    fn test_a_persona_is_not_delegable_by_default() {
        // orchestration-only roles are excluded by construction, not by
        // remembering to exclude them everywhere a persona list is built
        let persona = persona();
        assert!(!persona.delegable);
    }
}
