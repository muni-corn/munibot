use std::collections::HashSet;

use crate::types::{AiError, ModelRef};

/// Maps a provider name to the environment variable that configures it.
///
/// `ollama` is deliberately absent: it needs no key at all, and is always
/// treated as available.
const PROVIDER_KEY_VARS: &[(&str, &str)] = &[
    ("anthropic", "ANTHROPIC_API_KEY"),
    ("openai", "OPENAI_API_KEY"),
    ("openrouter", "OPENROUTER_API_KEY"),
];

/// Which providers have credentials configured, checked once at startup.
///
/// Complements [`crate::provider::ProviderResolver`]: where the resolver lazily
/// builds and caches a provider per distinct model on first use, this registry
/// answers the cheaper, earlier question of which providers could work at all -
/// so a persona referencing an unconfigured provider fails at startup with a
/// named environment variable, rather than on its first turn in front of a
/// user.
pub struct ProviderRegistry {
    available: HashSet<String>,
}

impl ProviderRegistry {
    /// Builds a registry from an explicit set of available provider names,
    /// useful for tests and for callers with another source of truth for
    /// availability. `ollama` is always included, since it needs no key.
    pub fn from_available(names: impl IntoIterator<Item = String>) -> Self {
        let mut available: HashSet<String> = names.into_iter().collect();
        available.insert("ollama".to_string());
        Self { available }
    }

    /// Checks environment variables for each supported provider and logs what
    /// it finds.
    ///
    /// A separate, explicit constructor from [`Self::from_available`] on
    /// purpose: reading real process environment variables in a test would
    /// mean mutating global state that every other test shares, which is
    /// exactly the kind of hazard that produces flaky parallel test runs.
    pub fn from_env() -> Self {
        let available = PROVIDER_KEY_VARS
            .iter()
            .filter(|(_, key_var)| std::env::var(key_var).is_ok())
            .map(|(provider, _)| provider.to_string());

        let registry = Self::from_available(available);

        for provider in &registry.available {
            tracing::info!(provider = %provider, "ai provider available");
        }
        for (provider, key_var) in PROVIDER_KEY_VARS {
            if !registry.is_available(provider) {
                tracing::debug!(provider = %provider, %key_var, "ai provider not configured");
            }
        }

        registry
    }

    /// Returns `true` if a provider has credentials configured, or needs none.
    pub fn is_available(&self, provider: &str) -> bool {
        self.available.contains(provider)
    }

    /// Validates that a model's provider is available, failing with a message
    /// naming the missing environment variable when it is not.
    pub fn check(&self, model: &ModelRef) -> Result<(), AiError> {
        if self.is_available(model.provider()) {
            return Ok(());
        }

        let hint = PROVIDER_KEY_VARS
            .iter()
            .find(|(provider, _)| *provider == model.provider())
            .map(|(_, key_var)| *key_var);

        Err(AiError::Config(match hint {
            Some(key_var) => format!(
                "the {} provider needs {key_var} to use {model} :<",
                model.provider()
            ),
            None => format!(
                "unknown provider {:?} :< supported providers are anthropic, openai, openrouter, \
                 and ollama",
                model.provider()
            ),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ollama_is_always_available() {
        let registry = ProviderRegistry::from_available([]);
        assert!(
            registry.is_available("ollama"),
            "ollama needs no key and should always be available"
        );
    }

    #[test]
    fn test_explicitly_available_provider_is_available() {
        let registry = ProviderRegistry::from_available(["anthropic".to_string()]);
        assert!(registry.is_available("anthropic"));
    }

    #[test]
    fn test_unlisted_provider_is_not_available() {
        let registry = ProviderRegistry::from_available(["anthropic".to_string()]);
        assert!(!registry.is_available("openai"));
    }

    #[test]
    fn test_check_passes_for_an_available_provider() {
        let registry = ProviderRegistry::from_available(["anthropic".to_string()]);
        let model = ModelRef::new("anthropic", "claude-opus-5");
        assert!(registry.check(&model).is_ok());
    }

    #[test]
    fn test_check_fails_and_names_the_missing_variable() {
        let registry = ProviderRegistry::from_available([]);
        let model = ModelRef::new("anthropic", "claude-opus-5");

        let error = registry
            .check(&model)
            .expect_err("should fail without a key");
        let message = error.to_string();

        assert!(
            message.contains("ANTHROPIC_API_KEY"),
            "the error should name the exact variable to set, got {message:?}"
        );
    }

    #[test]
    fn test_check_fails_for_an_unknown_provider_without_a_variable_hint() {
        let registry = ProviderRegistry::from_available([]);
        let model = ModelRef::new("bogus", "some-model");

        let error = registry
            .check(&model)
            .expect_err("should fail for an unknown provider");
        let message = error.to_string();

        assert!(
            message.contains("anthropic") && message.contains("openai"),
            "an unknown provider should still list what is supported, got {message:?}"
        );
    }

    #[test]
    fn test_check_passes_for_ollama_with_nothing_configured() {
        let registry = ProviderRegistry::from_available([]);
        let model = ModelRef::new("ollama", "llama3");
        assert!(
            registry.check(&model).is_ok(),
            "ollama should never need a key to pass this check"
        );
    }
}
