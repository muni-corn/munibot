use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use rig_core::{
    client::{CompletionClient, ProviderClient},
    providers::{anthropic, ollama, openai, openrouter},
};

use crate::{
    provider::{Provider, rig::adapter::RigProvider},
    types::{AiError, ModelRef},
};

/// Resolves a [`ModelRef`] to a working [`Provider`], constructing and caching
/// it on first use.
///
/// One match arm per supported provider - adding one is a one-arm change. This
/// is necessary rather than a runtime lookup because rig's `CompletionModel`
/// cannot be boxed generically; see `docs/notes/ai-preflight-findings.md`.
///
/// Cached by the **full** `provider:model` string, not just the provider name.
/// A cache keyed on provider name alone would return the wrong model's provider
/// on a second, differently-modelled request to the same provider -
/// `anthropic:claude-haiku-4` would silently receive whatever instance was
/// first built for `anthropic:claude-opus-5`. The cost is rebuilding the
/// underlying rig client (cheap: it parses an env var and builds a
/// `reqwest::Client`, no network round trip) once per distinct model rather
/// than once per provider, which is an acceptable trade for correctness given
/// personas resolve their model once at startup, not per request.
#[derive(Default)]
pub struct ProviderResolver {
    cache: RwLock<HashMap<String, Arc<dyn Provider>>>,
}

impl ProviderResolver {
    /// Builds an empty resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves a model reference, building and caching the provider on first
    /// use.
    pub fn resolve(&self, model: &ModelRef) -> Result<Arc<dyn Provider>, AiError> {
        let key = model.to_string();

        if let Some(provider) = self.cache.read().unwrap().get(&key) {
            return Ok(Arc::clone(provider));
        }

        let provider = build_provider(model.provider(), model.model())?;

        // another thread may have built the same provider concurrently; keep whichever
        // won the race rather than paying for a second lock upgrade to detect
        // and discard the loser
        let mut cache = self.cache.write().unwrap();
        Ok(Arc::clone(cache.entry(key).or_insert(provider)))
    }
}

fn build_provider(provider: &str, model: &str) -> Result<Arc<dyn Provider>, AiError> {
    match provider {
        "anthropic" => {
            let client =
                anthropic::Client::from_env().map_err(|error| config_error(provider, error))?;
            Ok(Arc::new(RigProvider::new(
                provider,
                client.completion_model(model),
            )))
        }
        "openai" => {
            let client =
                openai::Client::from_env().map_err(|error| config_error(provider, error))?;
            Ok(Arc::new(RigProvider::new(
                provider,
                client.completion_model(model),
            )))
        }
        "openrouter" => {
            let client =
                openrouter::Client::from_env().map_err(|error| config_error(provider, error))?;
            Ok(Arc::new(RigProvider::new(
                provider,
                client.completion_model(model),
            )))
        }
        // ollama needs no key; from_env() defaults to a local unauthenticated server and never
        // fails on a missing OLLAMA_API_KEY
        "ollama" => {
            let client =
                ollama::Client::from_env().map_err(|error| config_error(provider, error))?;
            Ok(Arc::new(RigProvider::new(
                provider,
                client.completion_model(model),
            )))
        }
        other => Err(AiError::Config(format!(
            "unknown provider {other:?} :< supported providers are anthropic, openai, openrouter, \
             and ollama"
        ))),
    }
}

fn config_error(provider: &str, error: rig_core::client::ProviderClientError) -> AiError {
    AiError::Config(format!(
        "couldn't set up the {provider} provider :< {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Arc<dyn Provider>` is not `Debug`, so `expect_err` cannot format the
    /// `Ok` case - a plain match reaches the error without requiring one.
    fn resolve_err(resolver: &ProviderResolver, model: &ModelRef) -> AiError {
        match resolver.resolve(model) {
            Err(error) => error,
            Ok(_) => panic!("expected {model} to be rejected, but it resolved"),
        }
    }

    #[test]
    fn test_unknown_provider_is_rejected() {
        let resolver = ProviderResolver::new();
        let error = resolve_err(&resolver, &ModelRef::new("bogus", "some-model"));

        assert!(
            matches!(error, AiError::Config(_)),
            "an unknown provider should be a config error, not a provider error"
        );
    }

    #[test]
    fn test_unknown_provider_error_names_the_supported_ones() {
        let resolver = ProviderResolver::new();
        let error = resolve_err(&resolver, &ModelRef::new("bogus", "some-model"));

        let message = error.to_string();
        assert!(
            message.contains("anthropic") && message.contains("openai"),
            "the error should help a reader fix the config, not just say 'no': {message:?}"
        );
    }

    #[test]
    fn test_ollama_resolves_without_any_key() {
        // ollama needs no credentials, which makes it the one provider we can construct
        // deterministically in a test regardless of the ambient environment's api keys
        let resolver = ProviderResolver::new();
        let provider = resolver
            .resolve(&ModelRef::new("ollama", "llama3"))
            .expect("ollama should resolve with no key configured");

        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn test_resolving_the_same_model_reference_twice_returns_the_cached_instance() {
        let resolver = ProviderResolver::new();
        let model = ModelRef::new("ollama", "llama3");

        let first = resolver.resolve(&model).expect("should resolve");
        let second = resolver.resolve(&model).expect("should resolve");

        assert!(
            Arc::ptr_eq(&first, &second),
            "resolving the same model reference twice should return the cached instance, not \
             rebuild"
        );
    }

    #[test]
    fn test_resolving_a_different_model_on_the_same_provider_does_not_share_the_cached_instance() {
        // the correctness property this whole cache design exists for: two distinct
        // models on the same provider must never collapse onto one cached
        // provider instance
        let resolver = ProviderResolver::new();

        let small = resolver
            .resolve(&ModelRef::new("ollama", "llama3"))
            .expect("should resolve");
        let large = resolver
            .resolve(&ModelRef::new("ollama", "llama3:70b"))
            .expect("should resolve");

        assert!(
            !Arc::ptr_eq(&small, &large),
            "different models on the same provider must not share a cached provider instance"
        );
    }
}
