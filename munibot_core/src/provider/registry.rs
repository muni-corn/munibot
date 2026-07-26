use std::sync::Arc;

use munibot_config::ProviderDescriptor;

use crate::provider::{dyn_provider::DynConfigProvider, typed::ConfigurationProvider};

/// The set of configuration providers munibot knows about, built once at
/// startup (see `munibot/src/bot.rs`) next to the existing discord handler
/// and command-provider vectors, then shared with the gui server as an
/// axum `Extension` so it can dispatch a request to whichever provider a
/// url names without knowing the full set of providers at compile time.
#[derive(Default)]
pub struct ConfigRegistry {
    providers: Vec<Arc<dyn DynConfigProvider>>,
}

impl ConfigRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a provider, returning `&mut Self` so registrations can be
    /// chained. Does not check for a duplicate id -- getting that right is
    /// the caller's responsibility, the same as munibot's existing handler
    /// and command-provider registration.
    pub fn register<P: ConfigurationProvider>(&mut self, provider: P) -> &mut Self {
        self.providers.push(Arc::new(provider));
        self
    }

    /// Every registered provider, in registration order.
    pub fn providers(&self) -> &[Arc<dyn DynConfigProvider>] {
        &self.providers
    }

    /// The registered provider whose `Configurable::descriptor().id`
    /// matches `id`, or `None` if there isn't one.
    pub fn get(&self, id: &str) -> Option<&Arc<dyn DynConfigProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.descriptor().id == id)
    }

    /// Every registered provider's descriptor, in registration order --
    /// what the gui lists before the user has picked a provider to
    /// configure.
    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.providers
            .iter()
            .map(|provider| provider.descriptor())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use munibot_config::{ConfigNode, Configurable, ProviderDescriptor, ScopeKind};
    use serde::{Deserialize, Serialize};

    use super::ConfigRegistry;
    use crate::provider::{
        context::{ConfigContext, ConfigError},
        typed::ConfigurationProvider,
    };

    macro_rules! test_provider {
        ($provider_ty:ident, $config_ty:ident, $id:literal) => {
            #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
            struct $config_ty;

            impl Configurable for $config_ty {
                fn config_fields() -> Vec<ConfigNode> {
                    Vec::new()
                }

                fn descriptor() -> ProviderDescriptor {
                    ProviderDescriptor {
                        id: $id.to_owned(),
                        label: $id.to_owned(),
                        description: None,
                        icon: None,
                        scopes: vec![ScopeKind::DiscordGuild],
                    }
                }
            }

            struct $provider_ty;

            #[async_trait]
            impl ConfigurationProvider for $provider_ty {
                type Config = $config_ty;

                async fn read(&self, _ctx: &ConfigContext) -> Result<Self::Config, ConfigError> {
                    Ok($config_ty)
                }

                async fn write(
                    &self,
                    _ctx: &ConfigContext,
                    _previous: &Self::Config,
                    _desired: &Self::Config,
                ) -> Result<(), ConfigError> {
                    Ok(())
                }
            }
        };
    }

    test_provider!(LoggingProvider, LoggingConfig, "logging");
    test_provider!(AutoDeleteProvider, AutoDeleteConfig, "autodelete");

    #[test]
    fn test_empty_registry_has_no_providers() {
        let registry = ConfigRegistry::new();
        assert!(registry.providers().is_empty());
        assert!(registry.descriptors().is_empty());
    }

    #[test]
    fn test_register_adds_a_provider() {
        let mut registry = ConfigRegistry::new();
        registry.register(LoggingProvider);
        assert_eq!(registry.providers().len(), 1);
    }

    #[test]
    fn test_register_returns_self_for_chaining() {
        let mut registry = ConfigRegistry::new();
        registry
            .register(LoggingProvider)
            .register(AutoDeleteProvider);
        assert_eq!(registry.providers().len(), 2);
    }

    #[test]
    fn test_get_finds_a_provider_by_id() {
        let mut registry = ConfigRegistry::new();
        registry
            .register(LoggingProvider)
            .register(AutoDeleteProvider);

        let found = registry.get("autodelete").expect("should find autodelete");
        assert_eq!(found.descriptor().id, "autodelete");
    }

    #[test]
    fn test_get_returns_none_for_an_unknown_id() {
        let mut registry = ConfigRegistry::new();
        registry.register(LoggingProvider);
        assert!(registry.get("bogus").is_none());
    }

    #[test]
    fn test_descriptors_preserves_registration_order() {
        let mut registry = ConfigRegistry::new();
        registry
            .register(LoggingProvider)
            .register(AutoDeleteProvider);

        let ids: Vec<_> = registry.descriptors().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, vec!["logging".to_owned(), "autodelete".to_owned()]);
    }
}
