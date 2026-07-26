use async_trait::async_trait;
use munibot_config::{
    ConfigPatch, ConfigSchema, ConfigValues, Configurable, ProviderDescriptor, RefKind,
};

use crate::provider::{
    context::{ConfigContext, ConfigError},
    typed::{ConfigurationProvider, RefOption},
};

/// Object-safe facade over a `ConfigurationProvider`, letting a
/// `ConfigRegistry` hold providers with different `Self::Config` types
/// behind one `dyn` trait object.
///
/// Never implement this by hand -- the blanket impl below builds it
/// entirely out of `ConfigurationProvider` and `Configurable`, and is the
/// only intended implementation.
#[async_trait]
pub trait DynConfigProvider: Send + Sync + 'static {
    fn descriptor(&self) -> ProviderDescriptor;
    fn schema(&self) -> ConfigSchema;
    async fn load_values(&self, ctx: &ConfigContext) -> Result<ConfigValues, ConfigError>;
    async fn apply_patch(
        &self,
        ctx: &ConfigContext,
        patch: &ConfigPatch,
    ) -> Result<ConfigValues, ConfigError>;
    async fn ref_options(
        &self,
        ctx: &ConfigContext,
        kind: RefKind,
    ) -> Result<Vec<RefOption>, ConfigError>;
}

#[async_trait]
impl<P: ConfigurationProvider> DynConfigProvider for P {
    fn descriptor(&self) -> ProviderDescriptor {
        P::Config::descriptor()
    }

    fn schema(&self) -> ConfigSchema {
        P::Config::config_schema()
    }

    async fn load_values(&self, ctx: &ConfigContext) -> Result<ConfigValues, ConfigError> {
        Ok(self.read(ctx).await?.to_values())
    }

    /// Reads the current value, applies `patch` to it (merging, coercing,
    /// and validating -- see `Configurable::with_patch`), persists the
    /// result, and runs `on_changed` before returning the new values.
    async fn apply_patch(
        &self,
        ctx: &ConfigContext,
        patch: &ConfigPatch,
    ) -> Result<ConfigValues, ConfigError> {
        let previous = self.read(ctx).await?;
        let desired = previous.with_patch(patch)?;
        self.write(ctx, &previous, &desired).await?;
        self.on_changed(ctx, &desired).await?;
        Ok(desired.to_values())
    }

    async fn ref_options(
        &self,
        ctx: &ConfigContext,
        kind: RefKind,
    ) -> Result<Vec<RefOption>, ConfigError> {
        ConfigurationProvider::ref_options(self, ctx, kind).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use munibot_config::{
        ConfigNode, ConfigPatch, ConfigScope, ConfigValue, Configurable, Field, FieldKind,
        ProviderDescriptor, ScopeKind,
    };
    use serde::{Deserialize, Serialize};
    use tokio::sync::Mutex;

    use super::DynConfigProvider;
    use crate::provider::{
        context::{ConfigContext, ConfigError},
        typed::{ConfigurationProvider, test_support::unconnected_db_pool},
    };

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    struct TestConfig {
        enabled: bool,
    }

    impl Configurable for TestConfig {
        fn config_fields() -> Vec<ConfigNode> {
            vec![ConfigNode::Field(
                Field::new("enabled", "enabled", FieldKind::Bool).default(ConfigValue::Bool(false)),
            )]
        }

        fn descriptor() -> ProviderDescriptor {
            ProviderDescriptor {
                id: "test".to_owned(),
                label: "test provider".to_owned(),
                description: None,
                icon: None,
                scopes: vec![ScopeKind::DiscordGuild],
            }
        }
    }

    /// A provider backed by an in-memory store instead of a real database,
    /// so its ConfigurationProvider impl (and, through it,
    /// DynConfigProvider's blanket impl) can be tested without one.
    /// Counts writes and on_changed calls so tests can assert on them.
    #[derive(Default)]
    struct TestProvider {
        store: Mutex<TestConfig>,
        write_count: AtomicUsize,
        on_changed_count: AtomicUsize,
    }

    #[async_trait]
    impl ConfigurationProvider for TestProvider {
        type Config = TestConfig;

        async fn read(&self, _ctx: &ConfigContext) -> Result<Self::Config, ConfigError> {
            Ok(self.store.lock().await.clone())
        }

        async fn write(
            &self,
            _ctx: &ConfigContext,
            _previous: &Self::Config,
            desired: &Self::Config,
        ) -> Result<(), ConfigError> {
            *self.store.lock().await = desired.clone();
            self.write_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn on_changed(
            &self,
            _ctx: &ConfigContext,
            _config: &Self::Config,
        ) -> Result<(), ConfigError> {
            self.on_changed_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn test_context() -> ConfigContext {
        ConfigContext {
            db: unconnected_db_pool(),
            scope: ConfigScope::DiscordGuild {
                guild_id: "1".to_owned(),
            },
            actor: None,
        }
    }

    #[test]
    fn test_descriptor_delegates_to_the_configs_own_descriptor() {
        let provider = TestProvider::default();
        assert_eq!(DynConfigProvider::descriptor(&provider).id, "test");
    }

    #[test]
    fn test_schema_delegates_to_the_configs_own_schema() {
        let provider = TestProvider::default();
        let schema = provider.schema();
        assert_eq!(schema.descriptor.id, "test");
        assert_eq!(schema.root.len(), 1);
    }

    #[tokio::test]
    async fn test_load_values_reflects_the_providers_current_state() {
        let provider = TestProvider::default();
        let values = provider
            .load_values(&test_context())
            .await
            .expect("should not error");
        assert_eq!(
            values.get(&"enabled".parse().unwrap()),
            Some(&ConfigValue::Bool(false))
        );
    }

    #[tokio::test]
    async fn test_apply_patch_writes_and_runs_on_changed() {
        let provider = TestProvider::default();
        let patch = ConfigPatch::new().with("enabled".parse().unwrap(), ConfigValue::Bool(true));

        let values = provider
            .apply_patch(&test_context(), &patch)
            .await
            .expect("patch should apply");

        assert_eq!(
            values.get(&"enabled".parse().unwrap()),
            Some(&ConfigValue::Bool(true))
        );
        assert_eq!(provider.write_count.load(Ordering::SeqCst), 1);
        assert_eq!(provider.on_changed_count.load(Ordering::SeqCst), 1);
        assert!(provider.store.lock().await.enabled);
    }

    #[tokio::test]
    async fn test_apply_patch_propagates_a_bad_patch_as_config_error_invalid() {
        let provider = TestProvider::default();
        // "enabled" is a Bool field; Int is the wrong ConfigValue variant.
        let patch = ConfigPatch::new().with("enabled".parse().unwrap(), ConfigValue::Int(1));

        let error = provider
            .apply_patch(&test_context(), &patch)
            .await
            .expect_err("patch should be rejected");

        assert!(matches!(error, ConfigError::Invalid(_)));
        // a rejected patch must not have written anything.
        assert_eq!(provider.write_count.load(Ordering::SeqCst), 0);
        assert_eq!(provider.on_changed_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_ref_options_delegates_to_the_typed_trait_and_defaults_empty() {
        let provider = TestProvider::default();
        let options = DynConfigProvider::ref_options(
            &provider,
            &test_context(),
            munibot_config::RefKind::DiscordChannel,
        )
        .await
        .expect("should not error");
        assert!(options.is_empty());
    }
}
