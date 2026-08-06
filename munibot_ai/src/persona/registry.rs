use std::{collections::HashMap, path::Path};

use crate::{
    persona::{AiConfig, Persona, PersonaConfig, PersonaId, PromptTemplate},
    provider::ProviderRegistry,
    types::{AiError, ModelParams},
};

/// The four personas built into the binary, embedded so a container deployment
/// or a config-less `EXA_API_KEY`-free development boot both have a working
/// default without any extra files.
///
/// This is deliberately a short, hardcoded list rather than a directory scan:
/// adding a persona means adding a prompt file and a line here, which is one
/// commit's worth of change, not an open-ended plugin surface.
fn embedded_prompt(filename: &str) -> Option<&'static str> {
    match filename {
        "companion.md" => Some(include_str!("../../prompts/companion.md")),
        "writer.md" => Some(include_str!("../../prompts/writer.md")),
        "researcher.md" => Some(include_str!("../../prompts/researcher.md")),
        "coder.md" => Some(include_str!("../../prompts/coder.md")),
        _ => None,
    }
}

/// Every configured persona, resolved and ready to run.
///
/// Built once at startup by [`PersonaRegistry::load`] and never mutated
/// afterward - a persona misconfiguration is a startup failure, not something
/// discovered mid-conversation.
#[derive(Debug)]
pub struct PersonaRegistry {
    personas: HashMap<PersonaId, Persona>,
    default_persona: Option<PersonaId>,
}

impl PersonaRegistry {
    /// Resolves every persona in `config`, checking each one's model against
    /// `providers` and reading its prompt from `config.prompt_dir` (falling
    /// back to the embedded default when absent or when no override
    /// directory is configured).
    ///
    /// Collects every problem across every persona before returning, rather
    /// than stopping at the first: an operator fixing a config file wants
    /// the whole list of what is wrong in one pass, not one error per
    /// restart.
    pub fn load(config: &AiConfig, providers: &ProviderRegistry) -> Result<Self, AiError> {
        let mut personas = HashMap::new();
        let mut problems = Vec::new();

        for (id, persona_config) in &config.personas {
            match Self::resolve_one(id, persona_config, config.prompt_dir.as_deref(), providers) {
                Ok(persona) => {
                    personas.insert(id.clone(), persona);
                }
                Err(error) => problems.push(format!("persona {id}: {error}")),
            }
        }

        if let Some(default_id) = &config.default_persona
            && !personas.contains_key(default_id)
        {
            problems.push(format!(
                "default_persona {default_id:?} is not one of the configured personas"
            ));
        }

        if !problems.is_empty() {
            return Err(AiError::Config(problems.join("; ")));
        }

        Ok(Self {
            personas,
            default_persona: config.default_persona.clone(),
        })
    }

    fn resolve_one(
        id: &PersonaId,
        config: &PersonaConfig,
        prompt_dir: Option<&Path>,
        providers: &ProviderRegistry,
    ) -> Result<Persona, AiError> {
        providers.check(&config.model)?;
        let prompt_source = Self::resolve_prompt(&config.prompt, prompt_dir)?;

        Ok(Persona {
            id: id.clone(),
            display_name: config.display_name.clone().unwrap_or_else(|| id.0.clone()),
            description: config.description.clone(),
            model: config.model.clone(),
            params: ModelParams {
                temperature: config.temperature,
                ..ModelParams::default()
            },
            system_prompt: PromptTemplate::new(prompt_source),
            tools: config.tools.clone(),
            budget: config.budget.resolve(),
            // chat personas never carry a handoff requirement; only the eventual pipeline roles
            // (milestone 4) set this, and nothing constructs those through this config path
            handoff: None,
            memory: config.memory,
            sandbox: config.sandbox,
        })
    }

    /// Reads a persona's prompt, preferring `prompt_dir` when it contains a
    /// matching file and falling back to the embedded default otherwise.
    fn resolve_prompt(filename: &str, prompt_dir: Option<&Path>) -> Result<String, AiError> {
        if let Some(dir) = prompt_dir {
            let path = dir.join(filename);
            if path.exists() {
                return std::fs::read_to_string(&path).map_err(|error| {
                    AiError::Config(format!("couldn't read prompt file {path:?} :< {error}"))
                });
            }
        }

        embedded_prompt(filename)
            .map(str::to_string)
            .ok_or_else(|| {
                AiError::Config(format!(
                    "no prompt file named {filename:?} found ({} and no embedded default by that \
                     name)",
                    match prompt_dir {
                        Some(dir) => format!("checked {dir:?}"),
                        None => "no prompt_dir is configured".to_string(),
                    }
                ))
            })
    }

    /// Looks a resolved persona up by id.
    pub fn get(&self, id: &PersonaId) -> Option<&Persona> {
        self.personas.get(id)
    }

    /// The configured default persona, if `default_persona` was set and
    /// resolved successfully.
    pub fn default_persona(&self) -> Option<&Persona> {
        self.default_persona.as_ref().and_then(|id| self.get(id))
    }

    /// Every resolved persona's id, for listing what is available.
    pub fn ids(&self) -> impl Iterator<Item = &PersonaId> {
        self.personas.keys()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{
        persona::{BudgetConfig, MemoryPolicy, SandboxPolicy},
        types::ModelRef,
    };

    fn persona_config(model: &str, prompt: &str) -> PersonaConfig {
        let (provider, model_name) = model.split_once(':').unwrap();
        PersonaConfig {
            model: ModelRef::new(provider, model_name),
            prompt: prompt.to_string(),
            display_name: None,
            description: "a test persona".to_string(),
            temperature: None,
            tools: crate::tools::ToolSelection::none(),
            budget: BudgetConfig::default(),
            memory: MemoryPolicy::default(),
            sandbox: SandboxPolicy::default(),
        }
    }

    fn ai_config(personas: Vec<(&str, PersonaConfig)>) -> AiConfig {
        AiConfig {
            enabled: true,
            default_persona: None,
            prompt_dir: None,
            crisis_resources: Vec::new(),
            rate_limits: crate::persona::config::RateLimitConfig::default(),
            personas: personas
                .into_iter()
                .map(|(id, config)| (PersonaId::new(id), config))
                .collect::<HashMap<_, _>>(),
        }
    }

    fn providers_with(available: &[&str]) -> ProviderRegistry {
        ProviderRegistry::from_available(available.iter().map(|s| s.to_string()))
    }

    #[test]
    fn test_load_resolves_a_persona_with_an_embedded_prompt() {
        let config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");

        let persona = registry
            .get(&PersonaId::new("companion"))
            .expect("should be present");
        assert_eq!(persona.model, ModelRef::new("anthropic", "claude-opus-5"));
        assert!(
            persona
                .system_prompt
                .required_variables()
                .contains(&"user_name".to_string())
        );
    }

    #[test]
    fn test_load_fails_when_the_model_provider_is_unconfigured() {
        let config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        let providers = providers_with(&[]); // no anthropic key

        let error = PersonaRegistry::load(&config, &providers).expect_err("should fail");
        let message = error.to_string();
        assert!(
            message.contains("companion"),
            "the error should name the broken persona: {message:?}"
        );
    }

    #[test]
    fn test_load_fails_when_the_prompt_file_does_not_exist() {
        let config = ai_config(vec![(
            "ghost",
            persona_config("anthropic:claude-opus-5", "does-not-exist.md"),
        )]);
        let providers = providers_with(&["anthropic"]);

        let error = PersonaRegistry::load(&config, &providers).expect_err("should fail");
        assert!(error.to_string().contains("ghost"));
    }

    #[test]
    fn test_load_reports_every_broken_persona_at_once() {
        let config = ai_config(vec![
            (
                "no_provider",
                persona_config("openrouter:whatever", "companion.md"),
            ),
            (
                "no_prompt",
                persona_config("anthropic:claude-opus-5", "missing.md"),
            ),
        ]);
        let providers = providers_with(&["anthropic"]);

        let error = PersonaRegistry::load(&config, &providers).expect_err("both should fail");
        let message = error.to_string();

        assert!(message.contains("no_provider"), "got {message:?}");
        assert!(message.contains("no_prompt"), "got {message:?}");
    }

    #[test]
    fn test_load_fails_when_default_persona_is_not_configured() {
        let mut config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        config.default_persona = Some(PersonaId::new("nonexistent"));
        let providers = providers_with(&["anthropic"]);

        let error = PersonaRegistry::load(&config, &providers).expect_err("should fail");
        assert!(error.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_default_persona_accessor_returns_the_resolved_persona() {
        let mut config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        config.default_persona = Some(PersonaId::new("companion"));
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert_eq!(
            registry.default_persona().unwrap().id,
            PersonaId::new("companion")
        );
    }

    #[test]
    fn test_no_default_persona_configured_yields_none() {
        let config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert!(registry.default_persona().is_none());
    }

    #[test]
    fn test_display_name_falls_back_to_the_persona_id() {
        let config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert_eq!(
            registry
                .get(&PersonaId::new("companion"))
                .unwrap()
                .display_name,
            "companion"
        );
    }

    #[test]
    fn test_explicit_display_name_overrides_the_fallback() {
        let mut persona = persona_config("anthropic:claude-opus-5", "companion.md");
        persona.display_name = Some("The Companion".to_string());
        let config = ai_config(vec![("companion", persona)]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert_eq!(
            registry
                .get(&PersonaId::new("companion"))
                .unwrap()
                .display_name,
            "The Companion"
        );
    }

    #[test]
    fn test_temperature_from_config_becomes_model_params() {
        let mut persona = persona_config("anthropic:claude-opus-5", "companion.md");
        persona.temperature = Some(0.9);
        let config = ai_config(vec![("companion", persona)]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert_eq!(
            registry
                .get(&PersonaId::new("companion"))
                .unwrap()
                .params
                .temperature,
            Some(0.9)
        );
    }

    #[test]
    fn test_budget_config_is_resolved_into_a_real_budget() {
        let mut persona = persona_config("anthropic:claude-opus-5", "companion.md");
        persona.budget = BudgetConfig {
            max_iterations: Some(30),
            ..BudgetConfig::default()
        };
        let config = ai_config(vec![("companion", persona)]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert_eq!(
            registry
                .get(&PersonaId::new("companion"))
                .unwrap()
                .budget
                .max_iterations,
            Some(30)
        );
    }

    #[test]
    fn test_chat_personas_never_carry_a_handoff() {
        let config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert!(
            registry
                .get(&PersonaId::new("companion"))
                .unwrap()
                .handoff
                .is_none()
        );
    }

    #[test]
    fn test_ids_lists_every_resolved_persona() {
        let config = ai_config(vec![
            (
                "companion",
                persona_config("anthropic:claude-opus-5", "companion.md"),
            ),
            (
                "coder",
                persona_config("anthropic:claude-opus-5", "coder.md"),
            ),
        ]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        let mut ids: Vec<_> = registry.ids().map(|id| id.0.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["coder".to_string(), "companion".to_string()]);
    }

    #[test]
    fn test_prompt_dir_override_takes_precedence_over_the_embedded_default() {
        let dir = std::env::temp_dir().join(format!(
            "munibot_ai_registry_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("companion.md"),
            "a custom override prompt with no variables",
        )
        .unwrap();

        let mut config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        config.prompt_dir = Some(dir.clone());
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        let rendered = registry
            .get(&PersonaId::new("companion"))
            .unwrap()
            .system_prompt
            .render(&HashMap::new())
            .expect("the override has no variables to satisfy");

        assert_eq!(rendered, "a custom override prompt with no variables");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_prompt_dir_falls_back_to_embedded_when_the_file_is_absent() {
        let dir = std::env::temp_dir().join(format!(
            "munibot_ai_registry_test_fallback_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // deliberately do not write companion.md into this directory

        let mut config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        config.prompt_dir = Some(dir.clone());
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect(
            "an empty override directory should fall back to the embedded prompt, not fail",
        );
        assert!(
            registry
                .get(&PersonaId::new("companion"))
                .unwrap()
                .system_prompt
                .required_variables()
                .contains(&"user_name".to_string()),
            "the embedded companion prompt should have been used"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
