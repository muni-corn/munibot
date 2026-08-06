use std::{collections::HashMap, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    harness::Budget,
    limits::{RateLimitPolicy, ScopePolicies, SpendCapPolicies, SpendCapPolicy},
    persona::{MemoryPolicy, PersonaId, SandboxPolicy},
    tools::ToolSelection,
    types::{AiError, Cost, ModelRef},
};

/// The TOML-deserializable shape of one persona's budget overrides.
///
/// Distinct from [`Budget`] itself: this is a config-ergonomic shape (a plain
/// dollar amount, a humantime-style duration string) that
/// [`BudgetConfig::resolve`] converts into a real `Budget`, falling back to
/// `Budget::default()` for anything left unset.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BudgetConfig {
    #[serde(default)]
    pub max_iterations: Option<usize>,
    #[serde(default)]
    pub max_input_tokens: Option<u64>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default, with = "humantime_serde::option")]
    pub max_wall_clock: Option<std::time::Duration>,
    pub max_cost_usd: Option<f64>,
    #[serde(default)]
    pub max_tool_retries: Option<usize>,
}

impl BudgetConfig {
    /// Resolves to a real [`Budget`], falling back to [`Budget::default`] for
    /// every field left unset here.
    pub fn resolve(&self) -> Budget {
        let default = Budget::default();
        Budget {
            max_iterations: self.max_iterations.or(default.max_iterations),
            max_input_tokens: self.max_input_tokens.or(default.max_input_tokens),
            max_output_tokens: self.max_output_tokens.or(default.max_output_tokens),
            max_wall_clock: self.max_wall_clock.or(default.max_wall_clock),
            max_cost: self
                .max_cost_usd
                .map(Cost::from_dollars)
                .or(default.max_cost),
            max_tool_retries: self.max_tool_retries.or(default.max_tool_retries),
        }
    }
}

/// The TOML-deserializable shape of one scope kind's rate limit, before
/// [`Self::resolve`] fills in [`RateLimitPolicy::default`] for anything left
/// unset - the same ergonomic-config-then-resolve shape [`BudgetConfig`]
/// already uses.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RateLimitPolicyConfig {
    #[serde(default)]
    pub max_requests: Option<u32>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_concurrent_turns: Option<u32>,
    #[serde(default, with = "humantime_serde::option")]
    pub window: Option<Duration>,
}

impl RateLimitPolicyConfig {
    /// Resolves to a real [`RateLimitPolicy`], falling back to
    /// [`RateLimitPolicy::default`] for `window` when unset.
    pub fn resolve(&self) -> RateLimitPolicy {
        let default = RateLimitPolicy::default();
        RateLimitPolicy {
            max_requests: self.max_requests,
            max_tokens: self.max_tokens,
            max_concurrent_turns: self.max_concurrent_turns,
            window: self.window.unwrap_or(default.window),
        }
    }
}

/// The TOML-deserializable shape of `[ai.rate_limits]`: every scope kind's
/// own policy, all optional - an operator configures only the scopes they
/// actually want limited.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub user: RateLimitPolicyConfig,
    #[serde(default)]
    pub guild: RateLimitPolicyConfig,
    #[serde(default)]
    pub global: RateLimitPolicyConfig,
}

impl RateLimitConfig {
    /// Resolves to real [`ScopePolicies`], for
    /// [`crate::limits::RateLimiter::new`].
    pub fn resolve(&self) -> ScopePolicies {
        ScopePolicies {
            user: self.user.resolve(),
            guild: self.guild.resolve(),
            global: self.global.resolve(),
        }
    }
}

/// The TOML-deserializable shape of one scope kind's spend cap, before
/// [`Self::resolve`] fills in [`SpendCapPolicy::default`] for anything left
/// unset - the same ergonomic-config-then-resolve shape [`BudgetConfig`]
/// already uses. `max_usd` rather than raw micros, for the same reason
/// [`BudgetConfig::max_cost_usd`] is a dollar amount rather than micros too.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SpendCapPolicyConfig {
    #[serde(default)]
    pub max_usd: Option<f64>,
    #[serde(default)]
    pub period: Option<String>,
    #[serde(default, with = "humantime_serde::option")]
    pub duration: Option<Duration>,
}

impl SpendCapPolicyConfig {
    /// Resolves to a real [`SpendCapPolicy`], falling back to
    /// [`SpendCapPolicy::default`] for `period` and `duration` when unset.
    pub fn resolve(&self) -> SpendCapPolicy {
        let default = SpendCapPolicy::default();
        SpendCapPolicy {
            limit_micros: self.max_usd.map(|usd| Cost::from_dollars(usd).0),
            period: self.period.clone().unwrap_or(default.period),
            duration: self.duration.unwrap_or(default.duration),
        }
    }
}

/// The TOML-deserializable shape of `[ai.spend_caps]`: per-user and global
/// only, no guild - matching [`SpendCapPolicies`]'s own doc comment for why.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SpendCapConfig {
    #[serde(default)]
    pub user: SpendCapPolicyConfig,
    #[serde(default)]
    pub global: SpendCapPolicyConfig,
}

impl SpendCapConfig {
    /// Resolves to real [`SpendCapPolicies`], for
    /// [`crate::limits::SpendCapEnforcer::new`].
    pub fn resolve(&self) -> SpendCapPolicies {
        SpendCapPolicies {
            user: self.user.resolve(),
            global: self.global.resolve(),
        }
    }
}

/// The TOML-deserializable shape of one persona, before its prompt file has
/// been read or its model checked against a configured provider.
///
/// The persona registry (a later commit) is what turns this into a fully
/// resolved [`crate::persona::Persona`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PersonaConfig {
    pub model: ModelRef,
    /// A filename resolved against [`AiConfig::prompt_dir`], or an embedded
    /// default when unset.
    pub prompt: String,
    /// A human-readable name, for a future settings surface. Falls back to the
    /// persona's own id string when unset, so this is never required to
    /// write a working config.
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub tools: ToolSelection,
    #[serde(default)]
    pub budget: BudgetConfig,
    #[serde(default)]
    pub memory: MemoryPolicy,
    #[serde(default)]
    pub sandbox: SandboxPolicy,
}

/// The `[ai]` section of munibot's configuration file.
///
/// Deliberately **not** a field of `munibot_core::Config`: this crate depends
/// on `munibot_core`, so the reverse would be a dependency cycle.
/// [`AiConfig::load_from_file`] instead performs its own, independent
/// deserialization pass over the same file, reading only the `[ai]` table and
/// ignoring every other section - the same file, two separate parses. This
/// deliberately avoids repeating the generic, pluggable per-crate configuration
/// mechanism documented (and abandoned, at roughly 7,400 lines) in
/// `docs/notes/gui-configuration-research.md`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiConfig {
    /// Defaults to `false`: a config file predating AI support, or one that
    /// never mentions `[ai]` at all, must boot with the feature off rather
    /// than silently on.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub default_persona: Option<PersonaId>,
    #[serde(default)]
    pub prompt_dir: Option<PathBuf>,
    #[serde(default)]
    pub personas: HashMap<PersonaId, PersonaConfig>,
    /// Real, region-appropriate crisis resources (a hotline, a text line, a
    /// website), surfaced verbatim by the crisis response path on a positive
    /// signal from a [`crate::crisis::CrisisClassifier`]. Never a model's own
    /// invention - see that module and `Ai`'s own crisis-handling code for
    /// why this must always be reviewed, real contact information rather
    /// than anything generated.
    #[serde(default)]
    pub crisis_resources: Vec<CrisisResourceConfig>,
    /// Request, token, and concurrency limits per scope (user, guild, and
    /// global), checked before a turn's provider call. Every scope is
    /// unlimited by default, matching the behaviour before rate limiting
    /// existed at all.
    #[serde(default)]
    pub rate_limits: RateLimitConfig,
    /// Spend caps per user and globally, checked alongside rate limits.
    /// Uncapped by default, matching the behaviour before spend caps
    /// existed at all.
    #[serde(default)]
    pub spend_caps: SpendCapConfig,
}

/// One crisis resource an operator has configured: a hotline, a text line, a
/// website, or similar. `contact` is free text (a phone number, a "text HOME
/// to 741741" instruction, a URL) rather than a typed phone/url field, since
/// how someone actually reaches a given resource varies too much to force
/// into one shape.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CrisisResourceConfig {
    pub name: String,
    pub contact: String,
}

impl AiConfig {
    /// Loads the `[ai]` section from the same configuration file
    /// `munibot_core::Config` reads.
    ///
    /// A missing file, or a file with no `[ai]` table at all, yields
    /// [`AiConfig::default`] (`enabled: false`) rather than an error - an
    /// existing deployment upgrading to a build with AI support should boot
    /// completely unchanged until someone actually configures it.
    pub fn load_from_file(path: impl AsRef<std::path::Path>) -> Result<Self, AiError> {
        #[derive(Deserialize)]
        struct ConfigFile {
            #[serde(default)]
            ai: AiConfig,
        }

        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(path)
            .map_err(|error| AiError::Config(format!("couldn't read {path:?} :< {error}")))?;

        let parsed: ConfigFile = toml::from_str(&contents)
            .map_err(|error| AiError::Config(format!("couldn't parse {path:?} :< {error}")))?;

        Ok(parsed.ai)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_ai_config_is_disabled_with_no_personas() {
        let config = AiConfig::default();
        assert!(!config.enabled);
        assert!(config.personas.is_empty());
        assert_eq!(config.default_persona, None);
    }

    #[test]
    fn test_missing_config_file_yields_the_default() {
        let config = AiConfig::load_from_file("/nonexistent/path/does/not/exist.toml").unwrap();
        assert_eq!(config, AiConfig::default());
    }

    #[test]
    fn test_a_config_file_with_no_ai_section_yields_the_default() {
        let dir = tempdir();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[discord]\ninvite_link = \"https://example.com\"\n").unwrap();

        let config = AiConfig::load_from_file(&path).unwrap();
        assert_eq!(
            config,
            AiConfig::default(),
            "an ai-less config file should not error"
        );
    }

    #[test]
    fn test_loading_a_real_ai_section() {
        let dir = tempdir();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
            [discord]
            invite_link = "https://example.com"

            [ai]
            enabled = true
            default_persona = "companion"

            [ai.personas.companion]
            model = "anthropic:claude-opus-5"
            prompt = "companion.md"
            description = "warm, playful conversation"
            tools = ["tier0", "web_search"]

            [ai.personas.researcher]
            model = "anthropic:claude-opus-5"
            prompt = "researcher.md"
            tools = ["tier0", "tier1"]
            budget = { max_iterations = 30, max_cost_usd = 2.0 }
            "#,
        )
        .unwrap();

        let config = AiConfig::load_from_file(&path).unwrap();

        assert!(config.enabled);
        assert_eq!(config.default_persona, Some(PersonaId::new("companion")));
        assert_eq!(config.personas.len(), 2);

        let companion = &config.personas[&PersonaId::new("companion")];
        assert_eq!(companion.model, ModelRef::new("anthropic", "claude-opus-5"));
        assert_eq!(companion.prompt, "companion.md");

        let researcher = &config.personas[&PersonaId::new("researcher")];
        assert_eq!(researcher.budget.max_iterations, Some(30));
        assert_eq!(researcher.budget.max_cost_usd, Some(2.0));
    }

    #[test]
    fn test_sections_other_than_ai_are_ignored() {
        // proves the independent-pass design: fields only munibot_core::Config knows
        // about must not cause a parse failure here
        let dir = tempdir();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
            [twitch]
            twitch_user = "muni__bot"
            initial_channels = ["a", "b"]
            "#,
        )
        .unwrap();

        let config = AiConfig::load_from_file(&path).unwrap();
        assert_eq!(config, AiConfig::default());
    }

    #[test]
    fn test_budget_config_resolve_falls_back_to_defaults_for_every_unset_field() {
        let resolved = BudgetConfig::default().resolve();
        assert_eq!(resolved, Budget::default());
    }

    #[test]
    fn test_budget_config_resolve_overrides_only_what_is_set() {
        let config = BudgetConfig {
            max_iterations: Some(30),
            ..BudgetConfig::default()
        };
        let resolved = config.resolve();

        assert_eq!(
            resolved.max_iterations,
            Some(30),
            "the override should take effect"
        );
        assert_eq!(
            resolved.max_cost,
            Budget::default().max_cost,
            "an unset field should keep the default"
        );
    }

    #[test]
    fn test_budget_config_max_cost_usd_converts_to_cost() {
        let config = BudgetConfig {
            max_cost_usd: Some(2.0),
            ..BudgetConfig::default()
        };
        assert_eq!(config.resolve().max_cost, Some(Cost::from_dollars(2.0)));
    }

    #[test]
    fn test_default_rate_limit_config_resolves_unlimited() {
        let policies = RateLimitConfig::default().resolve();
        assert!(policies.user.is_unlimited());
        assert!(policies.guild.is_unlimited());
        assert!(policies.global.is_unlimited());
    }

    #[test]
    fn test_rate_limit_policy_config_resolve_overrides_only_what_is_set() {
        let config = RateLimitPolicyConfig {
            max_requests: Some(20),
            ..RateLimitPolicyConfig::default()
        };
        let resolved = config.resolve();

        assert_eq!(resolved.max_requests, Some(20));
        assert_eq!(
            resolved.window,
            crate::limits::RateLimitPolicy::default().window,
            "an unset window should keep the default"
        );
    }

    #[test]
    fn test_rate_limit_policy_config_window_parses_as_humantime() {
        let config: RateLimitPolicyConfig = toml::from_str("window = \"1m\"").unwrap();
        assert_eq!(config.resolve().window, Duration::from_secs(60));
    }

    #[test]
    fn test_loading_a_real_rate_limits_section() {
        let dir = tempdir();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
            [ai]
            enabled = true

            [ai.rate_limits.user]
            max_requests = 20
            window = "1m"
            max_tokens = 50000
            max_concurrent_turns = 2

            [ai.rate_limits.global]
            max_requests = 200
            window = "1m"
            "#,
        )
        .unwrap();

        let config = AiConfig::load_from_file(&path).unwrap();
        let policies = config.rate_limits.resolve();

        assert_eq!(policies.user.max_requests, Some(20));
        assert_eq!(policies.user.max_tokens, Some(50000));
        assert_eq!(policies.user.max_concurrent_turns, Some(2));
        assert_eq!(policies.global.max_requests, Some(200));
        assert!(
            policies.guild.is_unlimited(),
            "an unconfigured scope should stay unlimited"
        );
    }

    #[test]
    fn test_default_spend_cap_config_resolves_uncapped() {
        let policies = SpendCapConfig::default().resolve();
        assert_eq!(policies.user.limit_micros, None);
        assert_eq!(policies.global.limit_micros, None);
    }

    #[test]
    fn test_spend_cap_policy_config_max_usd_converts_to_micros() {
        let config = SpendCapPolicyConfig {
            max_usd: Some(5.0),
            ..SpendCapPolicyConfig::default()
        };
        assert_eq!(
            config.resolve().limit_micros,
            Some(Cost::from_dollars(5.0).0)
        );
    }

    #[test]
    fn test_spend_cap_policy_config_resolve_falls_back_to_defaults_for_period_and_duration() {
        let resolved = SpendCapPolicyConfig::default().resolve();
        assert_eq!(resolved.period, SpendCapPolicy::default().period);
        assert_eq!(resolved.duration, SpendCapPolicy::default().duration);
    }

    #[test]
    fn test_loading_a_real_spend_caps_section() {
        let dir = tempdir();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
            [ai]
            enabled = true

            [ai.spend_caps.user]
            max_usd = 5.0
            period = "monthly"

            [ai.spend_caps.global]
            max_usd = 500.0
            period = "monthly"
            "#,
        )
        .unwrap();

        let config = AiConfig::load_from_file(&path).unwrap();
        let policies = config.spend_caps.resolve();

        assert_eq!(policies.user.limit_micros, Some(Cost::from_dollars(5.0).0));
        assert_eq!(policies.user.period, "monthly");
        assert_eq!(
            policies.global.limit_micros,
            Some(Cost::from_dollars(500.0).0)
        );
    }

    #[test]
    fn test_persona_config_requires_model_and_prompt() {
        let result: Result<PersonaConfig, _> =
            toml::from_str("description = \"missing required fields\"");
        assert!(
            result.is_err(),
            "model and prompt should be required, not defaulted"
        );
    }

    /// A minimal RAII temp directory, since this crate has no existing
    /// test-fixture dependency for one and the need here is only ever a
    /// single scratch file.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tempdir() -> TempDir {
        let path = std::env::temp_dir().join(format!("munibot_ai_test_{}", uuid_like()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    /// Just enough entropy to avoid collisions between concurrently running
    /// tests, without pulling in a uuid dependency for one test helper.
    fn uuid_like() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{nanos}_{count}")
    }
}
