use std::{collections::HashMap, path::Path};

use crate::{
    persona::{AiConfig, Persona, PersonaConfig, PersonaId, PromptTemplate},
    provider::ProviderRegistry,
    types::{AiError, ModelParams},
};

/// Every prompt built into the binary, embedded so a container deployment or
/// a config-less, `EXA_API_KEY`-free development boot both have a working
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
        "software-architect.md" => Some(include_str!("../../prompts/software-architect.md")),
        "issue-analyst.md" => Some(include_str!("../../prompts/issue-analyst.md")),
        "code-reviewer.md" => Some(include_str!("../../prompts/code-reviewer.md")),
        "test-reviewer.md" => Some(include_str!("../../prompts/test-reviewer.md")),
        "architecture-reviewer.md" => Some(include_str!("../../prompts/architecture-reviewer.md")),
        "project-manager.md" => Some(include_str!("../../prompts/project-manager.md")),
        "critic.md" => Some(include_str!("../../prompts/critic.md")),
        "codebase-researcher.md" => Some(include_str!("../../prompts/codebase-researcher.md")),
        "builder.md" => Some(include_str!("../../prompts/builder.md")),
        "test-engineer.md" => Some(include_str!("../../prompts/test-engineer.md")),
        "final-code-reviewer.md" => Some(include_str!("../../prompts/final-code-reviewer.md")),
        "commit-crafter.md" => Some(include_str!("../../prompts/commit-crafter.md")),
        "pr-author.md" => Some(include_str!("../../prompts/pr-author.md")),
        _ => None,
    }
}

/// The full built-in persona roster - every embedded prompt above, paired
/// with a sensible model-less [`PersonaConfig`] (see
/// [`AiConfig::default_model`]), tool selection, and delegation policy.
///
/// [`PersonaRegistry::load`] merges this underneath whatever an operator
/// configures: an id an operator's own config also defines is replaced by
/// their entry entirely (no field-by-field merge - simplest to reason
/// about), and anything they don't mention falls back to this roster
/// unchanged. This is what lets `[ai] enabled = true` plus a single
/// `default_model` produce a fully working companion and engineering team
/// with no per-persona configuration at all.
fn embedded_personas() -> HashMap<PersonaId, PersonaConfig> {
    use crate::{
        persona::{BudgetConfig, MemoryPolicy},
        tools::ToolSelection,
    };

    /// A persona config with every field at its own sensible default,
    /// before the handful of overrides each embedded persona actually needs.
    fn base(
        prompt: &str,
        description: &str,
        tools: ToolSelection,
        delegable: bool,
    ) -> PersonaConfig {
        PersonaConfig {
            model: None,
            prompt: prompt.to_string(),
            display_name: None,
            description: description.to_string(),
            temperature: None,
            tools,
            budget: BudgetConfig::default(),
            memory: MemoryPolicy::None,
            sandbox: crate::persona::SandboxPolicy::default(),
            delegable,
        }
    }

    fn budget(max_iterations: usize, max_cost_usd: f64) -> BudgetConfig {
        BudgetConfig {
            max_iterations: Some(max_iterations),
            max_cost_usd: Some(max_cost_usd),
            ..BudgetConfig::default()
        }
    }

    HashMap::from([
        (PersonaId::new("companion"), PersonaConfig {
            temperature: Some(1.0),
            memory: MemoryPolicy::User,
            ..base(
                "companion.md",
                "warm, playful conversation and emotional support",
                ToolSelection::named(["tier0", "web_search", "web_fetch", "delegate"]),
                false,
            )
        }),
        (PersonaId::new("researcher"), PersonaConfig {
            budget: budget(30, 2.0),
            ..base(
                "researcher.md",
                "multi-step research with citations",
                ToolSelection::named(["tier0", "tier1"]),
                true,
            )
        }),
        (PersonaId::new("coder"), PersonaConfig {
            // lazy in spirit - see provision_if_needed's own note on why
            // Optional is currently provisioned exactly as eagerly as
            // Required, tracked in docs/notes/sandbox-verification-gaps.md
            sandbox: crate::persona::SandboxPolicy::Optional,
            ..base(
                "coder.md",
                "explains, reviews, debugs, and now runs and verifies code in a sandbox",
                ToolSelection::named(["tier0", "tier3"]),
                false,
            )
        }),
        (
            PersonaId::new("writer"),
            base(
                "writer.md",
                "creative and long-form writing help",
                ToolSelection::none(),
                false,
            ),
        ),
        (PersonaId::new("software-architect"), PersonaConfig {
            budget: budget(15, 1.0),
            ..base(
                "software-architect.md",
                "turns a request into a detailed, buildable plan",
                ToolSelection::named(["tier0"]),
                true,
            )
        }),
        (
            PersonaId::new("issue-analyst"),
            base(
                "issue-analyst.md",
                "triages an issue and works out what it needs before anyone builds it",
                ToolSelection::named(["tier0", "tier1"]),
                true,
            ),
        ),
        (
            PersonaId::new("code-reviewer"),
            base(
                "code-reviewer.md",
                "reviews a diff or pasted code against the project's standards",
                ToolSelection::named(["tier0"]),
                true,
            ),
        ),
        (
            PersonaId::new("test-reviewer"),
            base(
                "test-reviewer.md",
                "reviews pasted tests as a specification, before anything is built against them",
                ToolSelection::named(["tier0"]),
                true,
            ),
        ),
        (
            PersonaId::new("architecture-reviewer"),
            base(
                "architecture-reviewer.md",
                "critiques a plan against completeness, ordering, and instruction quality",
                ToolSelection::named(["tier0"]),
                true,
            ),
        ),
        (
            PersonaId::new("project-manager"),
            base(
                "project-manager.md",
                "given a plan and what's done, decides what to work on next",
                ToolSelection::named(["tier0"]),
                true,
            ),
        ),
        (
            PersonaId::new("critic"),
            base(
                "critic.md",
                "critiques a drawing, design, or screenshot: what works, what doesn't, what to \
                 try next",
                ToolSelection::none(),
                true,
            ),
        ),
        (PersonaId::new("codebase-researcher"), PersonaConfig {
            sandbox: crate::persona::SandboxPolicy::Required,
            budget: budget(20, 1.5),
            ..base(
                "codebase-researcher.md",
                "explores a checked-out repository and summarizes what an implementation plan \
                 needs to know",
                ToolSelection::named(["tier0", "read", "grep", "glob"]),
                true,
            )
        }),
        (PersonaId::new("builder"), PersonaConfig {
            sandbox: crate::persona::SandboxPolicy::Required,
            budget: budget(40, 3.0),
            ..base(
                "builder.md",
                "implements one subtask in a checked-out repository",
                ToolSelection::named(["tier0", "tier3"]),
                true,
            )
        }),
        (PersonaId::new("test-engineer"), PersonaConfig {
            sandbox: crate::persona::SandboxPolicy::Required,
            budget: budget(30, 2.0),
            ..base(
                "test-engineer.md",
                "writes tests for a subtask before the implementation exists, and runs them to \
                 confirm they fail for the right reason",
                ToolSelection::named(["tier0", "tier3"]),
                true,
            )
        }),
        (PersonaId::new("final-code-reviewer"), PersonaConfig {
            sandbox: crate::persona::SandboxPolicy::Required,
            budget: budget(25, 1.5),
            ..base(
                "final-code-reviewer.md",
                "reviews every change across every subtask holistically, against the original plan",
                ToolSelection::named(["tier0", "read", "grep", "glob", "bash"]),
                true,
            )
        }),
        (PersonaId::new("commit-crafter"), PersonaConfig {
            sandbox: crate::persona::SandboxPolicy::Required,
            budget: budget(10, 0.5),
            ..base(
                "commit-crafter.md",
                "turns approved changes into a clean, atomic git commit",
                ToolSelection::named(["tier0", "bash", "read", "grep", "glob"]),
                true,
            )
        }),
        (PersonaId::new("pr-author"), PersonaConfig {
            sandbox: crate::persona::SandboxPolicy::Required,
            budget: budget(10, 0.5),
            ..base(
                "pr-author.md",
                "writes a pull request title and body from the real diff and commit history",
                ToolSelection::named(["tier0", "read", "grep", "glob", "bash"]),
                true,
            )
        }),
    ])
}

/// `config.personas`, with every embedded default (see
/// [`embedded_personas`]) an operator didn't already define for themselves
/// filled in underneath it.
fn merged_personas(config: &AiConfig) -> HashMap<PersonaId, PersonaConfig> {
    let mut merged = embedded_personas();
    for (id, persona_config) in &config.personas {
        merged.insert(id.clone(), persona_config.clone());
    }
    merged
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

        for (id, persona_config) in &merged_personas(config) {
            let result = Self::resolve_one(
                id,
                persona_config,
                config.prompt_dir.as_deref(),
                config.default_model.as_ref(),
                providers,
            );
            match result {
                Ok(persona) => {
                    personas.insert(id.clone(), persona);
                }
                // an operator explicitly configured this one, so a failure here is a
                // real problem with their config, not something to quietly paper over
                Err(error) if config.personas.contains_key(id) => {
                    problems.push(format!("persona {id}: {error}"));
                }
                // this id was never mentioned in config at all - it only exists
                // because embedded_personas() supplies it, and an operator who set
                // no default_model (or no credentials for the provider it needs)
                // never asked for it to work at all. skip it rather than failing
                // startup over a convenience nobody opted into.
                Err(error) => {
                    tracing::warn!(
                        persona = %id,
                        %error,
                        "an embedded default persona couldn't be resolved; skipping it"
                    );
                }
            }
        }

        // falls back to the embedded companion when nothing was configured
        // and companion actually resolved - "no configuration at all" should
        // still produce someone to talk to, not silently no default
        let default_persona = config.default_persona.clone().or_else(|| {
            let companion = PersonaId::new("companion");
            personas.contains_key(&companion).then_some(companion)
        });

        if let Some(default_id) = &default_persona
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
            default_persona,
        })
    }

    fn resolve_one(
        id: &PersonaId,
        config: &PersonaConfig,
        prompt_dir: Option<&Path>,
        default_model: Option<&crate::types::ModelRef>,
        providers: &ProviderRegistry,
    ) -> Result<Persona, AiError> {
        let model = config
            .model
            .clone()
            .or_else(|| default_model.cloned())
            .ok_or_else(|| {
                AiError::Config(
                    "has no model configured, and ai.default_model isn't set either".to_string(),
                )
            })?;
        providers.check(&model)?;
        let prompt_source = Self::resolve_prompt(&config.prompt, prompt_dir)?;

        Ok(Persona {
            id: id.clone(),
            display_name: config.display_name.clone().unwrap_or_else(|| id.0.clone()),
            description: config.description.clone(),
            model,
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
            delegable: config.delegable,
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
            model: Some(ModelRef::new(provider, model_name)),
            prompt: prompt.to_string(),
            display_name: None,
            description: "a test persona".to_string(),
            temperature: None,
            tools: crate::tools::ToolSelection::none(),
            budget: BudgetConfig::default(),
            memory: MemoryPolicy::default(),
            sandbox: SandboxPolicy::default(),
            delegable: false,
        }
    }

    fn ai_config(personas: Vec<(&str, PersonaConfig)>) -> AiConfig {
        AiConfig {
            enabled: true,
            default_persona: None,
            default_model: None,
            prompt_dir: None,
            crisis_resources: Vec::new(),
            rate_limits: crate::persona::config::RateLimitConfig::default(),
            spend_caps: crate::persona::config::SpendCapConfig::default(),
            abuse: crate::persona::config::AbuseConfig::default(),
            max_delegation_depth: 2,
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
    fn test_no_default_persona_configured_falls_back_to_companion() {
        // "no configuration at all" should still produce someone to talk
        // to - see PersonaRegistry::load's own doc comment on this fallback
        let config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert_eq!(
            registry.default_persona().map(|persona| &persona.id),
            Some(&PersonaId::new("companion"))
        );
    }

    #[test]
    fn test_no_default_persona_and_no_companion_yields_none() {
        // the fallback only ever applies when companion actually resolved -
        // with no persona configured and no default_model to resolve the
        // embedded companion either, there is genuinely nothing to fall
        // back to
        let config = ai_config(vec![]);
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
    fn test_delegable_defaults_to_false_when_resolved() {
        let config = ai_config(vec![(
            "companion",
            persona_config("anthropic:claude-opus-5", "companion.md"),
        )]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert!(
            !registry
                .get(&PersonaId::new("companion"))
                .unwrap()
                .delegable
        );
    }

    #[test]
    fn test_delegable_is_carried_through_from_config() {
        let mut config = persona_config("anthropic:claude-opus-5", "researcher.md");
        config.delegable = true;
        let config = ai_config(vec![("researcher", config)]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");
        assert!(
            registry
                .get(&PersonaId::new("researcher"))
                .unwrap()
                .delegable
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

    #[test]
    fn test_an_empty_config_with_no_default_model_resolves_no_personas_but_does_not_error() {
        // every embedded default persona has no model of its own - with no
        // default_model to fall back to, none of them can resolve, but that
        // is a convenience nobody opted into, not a config error
        let config = ai_config(vec![]);
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers)
            .expect("an unresolvable embedded default should never fail startup");
        assert_eq!(registry.ids().count(), 0);
    }

    #[test]
    fn test_a_default_model_alone_resolves_the_whole_embedded_roster() {
        // "no configuration at all" beyond enabling ai and naming a model -
        // every embedded persona (companion and the engineering team alike)
        // should resolve using it
        let mut config = ai_config(vec![]);
        config.default_model = Some(ModelRef::new("anthropic", "claude-opus-5"));
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");

        for id in [
            "companion",
            "researcher",
            "coder",
            "writer",
            "software-architect",
            "issue-analyst",
            "code-reviewer",
            "test-reviewer",
            "architecture-reviewer",
            "project-manager",
            "codebase-researcher",
            "builder",
            "test-engineer",
            "final-code-reviewer",
            "commit-crafter",
            "pr-author",
        ] {
            assert!(
                registry.get(&PersonaId::new(id)).is_some(),
                "{id} should have resolved from the embedded roster using default_model"
            );
        }
    }

    #[test]
    fn test_an_operators_own_persona_entirely_overrides_the_embedded_default() {
        let mut config = ai_config(vec![("companion", PersonaConfig {
            description: "a totally different companion".to_string(),
            delegable: true,
            ..persona_config("anthropic:claude-opus-5", "companion.md")
        })]);
        config.default_model = Some(ModelRef::new("anthropic", "claude-opus-5"));
        let providers = providers_with(&["anthropic"]);

        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");

        let companion = registry.get(&PersonaId::new("companion")).unwrap();
        assert_eq!(companion.description, "a totally different companion");
        assert!(
            companion.delegable,
            "the operator's own entry should win entirely, not merge field by field with the \
             embedded default (which is not delegable)"
        );
    }

    #[test]
    fn test_embedded_personas_are_delegable_except_companion_coder_and_writer() {
        let personas = super::embedded_personas();
        for id in ["companion", "coder", "writer"] {
            assert!(
                !personas[&PersonaId::new(id)].delegable,
                "{id} should not be delegable by default"
            );
        }
        for id in [
            "researcher",
            "software-architect",
            "issue-analyst",
            "code-reviewer",
            "test-reviewer",
            "architecture-reviewer",
            "project-manager",
            "codebase-researcher",
            "builder",
            "test-engineer",
            "final-code-reviewer",
            "commit-crafter",
            "pr-author",
        ] {
            assert!(
                personas[&PersonaId::new(id)].delegable,
                "{id} should be delegable by default"
            );
        }
    }

    #[test]
    fn test_the_hands_on_team_all_require_a_sandbox() {
        let personas = super::embedded_personas();
        for id in [
            "codebase-researcher",
            "builder",
            "test-engineer",
            "final-code-reviewer",
            "commit-crafter",
            "pr-author",
        ] {
            assert_eq!(
                personas[&PersonaId::new(id)].sandbox,
                crate::persona::SandboxPolicy::Required,
                "{id} needs a checked-out repository, so its sandbox policy must be Required"
            );
        }
    }

    #[test]
    fn test_the_codebase_researcher_cannot_modify_files() {
        // the read-only tool selection is the actual enforcement of "do
        // not modify any files" - not just the prompt's own instruction
        let personas = super::embedded_personas();
        let researcher = &personas[&PersonaId::new("codebase-researcher")];
        for destructive_tool in ["write", "edit", "bash"] {
            assert!(
                !researcher
                    .tools
                    .covers(destructive_tool, crate::tools::RiskTier::Sandbox),
                "the codebase researcher should not be authorized for {destructive_tool:?}"
            );
        }
    }

    #[test]
    fn test_the_final_code_reviewer_cannot_modify_files() {
        let personas = super::embedded_personas();
        let reviewer = &personas[&PersonaId::new("final-code-reviewer")];
        for destructive_tool in ["write", "edit"] {
            assert!(
                !reviewer
                    .tools
                    .covers(destructive_tool, crate::tools::RiskTier::Sandbox),
                "the final code reviewer should not be authorized for {destructive_tool:?}"
            );
        }
    }

    #[test]
    fn test_embedded_personas_all_defer_to_default_model() {
        for (id, config) in super::embedded_personas() {
            assert!(
                config.model.is_none(),
                "{id} should have no model of its own, deferring to ai.default_model"
            );
        }
    }
}
