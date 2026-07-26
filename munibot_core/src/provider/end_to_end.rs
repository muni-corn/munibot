//! Ties `ConfigRegistry`, `DynConfigProvider`'s blanket impl, and
//! `collection_diff` together in one realistic scenario: a
//! `LoggingConfig`-shaped provider (a `Toggled` section) and an
//! `AutoDeleteConfig`-shaped provider (a `BTreeMap` collection, diffed in
//! `write()` the way a real database-backed provider would), both
//! registered and driven entirely through `ConfigRegistry`, never calling
//! either provider directly.
//!
//! Uses the real `#[derive(Configurable)]` (see this crate's
//! `[dev-dependencies]`) rather than hand-written `Configurable` impls, so this
//! is as close as munibot_core can get, without a real database, to how
//! `munibot_discord`'s providers will actually be written.

use std::{collections::BTreeMap, sync::atomic::Ordering};

use async_trait::async_trait;
use munibot_config::{
    ChannelRef, ConfigPatch, ConfigValue, Configurable, SettingDuration, Toggled,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::provider::{
    collection::collection_diff,
    context::{ConfigContext, ConfigError},
    dyn_provider::DynConfigProvider,
    registry::ConfigRegistry,
    typed::{ConfigurationProvider, test_support::unconnected_db_pool},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct LoggingSettings {
    #[config(label = "log channel")]
    channel: Option<ChannelRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
#[config(id = "logging", label = "server logging", scope = DiscordGuild)]
struct LoggingConfig {
    #[config(label = "server logging")]
    logging: Toggled<LoggingSettings>,
}

#[derive(Default)]
struct LoggingProvider {
    store: Mutex<LoggingConfig>,
}

#[async_trait]
impl ConfigurationProvider for LoggingProvider {
    type Config = LoggingConfig;

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
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct ChannelAutoDelete {
    #[config(label = "message lifetime")]
    lifetime: SettingDuration,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
#[config(id = "autodelete", label = "autodelete", scope = DiscordGuild)]
struct AutoDeleteConfig {
    #[config(label = "autodelete timers")]
    timers: BTreeMap<ChannelRef, ChannelAutoDelete>,
}

/// Records what `collection_diff` found on the most recent `write()`, so
/// tests can assert a real write() implementation classifies changes the
/// way `collection_diff`'s own unit tests already say it should.
#[derive(Default)]
struct AutoDeleteProvider {
    store: Mutex<AutoDeleteConfig>,
    last_added: std::sync::atomic::AtomicUsize,
    last_changed: std::sync::atomic::AtomicUsize,
    last_removed: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ConfigurationProvider for AutoDeleteProvider {
    type Config = AutoDeleteConfig;

    async fn read(&self, _ctx: &ConfigContext) -> Result<Self::Config, ConfigError> {
        Ok(self.store.lock().await.clone())
    }

    async fn write(
        &self,
        _ctx: &ConfigContext,
        previous: &Self::Config,
        desired: &Self::Config,
    ) -> Result<(), ConfigError> {
        let delta = collection_diff(&previous.timers, &desired.timers);
        self.last_added.store(delta.added.len(), Ordering::SeqCst);
        self.last_changed
            .store(delta.changed.len(), Ordering::SeqCst);
        self.last_removed
            .store(delta.removed.len(), Ordering::SeqCst);

        *self.store.lock().await = desired.clone();
        Ok(())
    }
}

fn test_context() -> ConfigContext {
    ConfigContext {
        db: unconnected_db_pool(),
        scope: munibot_config::ConfigScope::DiscordGuild {
            guild_id: "1".to_owned(),
        },
        actor: None,
    }
}

fn build_registry() -> ConfigRegistry {
    let mut registry = ConfigRegistry::new();
    registry
        .register(LoggingProvider::default())
        .register(AutoDeleteProvider::default());
    registry
}

#[tokio::test]
async fn test_unknown_provider_id_is_not_in_the_registry() {
    let registry = build_registry();
    assert!(registry.get("bogus").is_none());
}

#[tokio::test]
async fn test_logging_full_lifecycle_through_the_registry() {
    let registry = build_registry();
    let provider = registry
        .get("logging")
        .expect("logging should be registered");
    let ctx = test_context();

    let enable_and_set = ConfigPatch::new()
        .with("logging.enabled".parse().unwrap(), ConfigValue::Bool(true))
        .with(
            "logging.channel".parse().unwrap(),
            ConfigValue::Ref("6437".to_owned()),
        );
    let values = provider
        .apply_patch(&ctx, &enable_and_set)
        .await
        .expect("patch should apply");
    assert_eq!(
        values.get(&"logging.enabled".parse().unwrap()),
        Some(&ConfigValue::Bool(true))
    );

    // reload independently, through load_values, to confirm the write
    // actually persisted into the provider's own store rather than just
    // being echoed back from apply_patch's return value.
    let reloaded = provider.load_values(&ctx).await.expect("should not error");
    assert_eq!(reloaded, values);

    // disabling must not lose the configured channel -- the whole point
    // of Toggled<T>.
    let disable =
        ConfigPatch::new().with("logging.enabled".parse().unwrap(), ConfigValue::Bool(false));
    let after_disable = provider
        .apply_patch(&ctx, &disable)
        .await
        .expect("patch should apply");
    assert_eq!(
        after_disable.get(&"logging.enabled".parse().unwrap()),
        Some(&ConfigValue::Bool(false))
    );
    assert_eq!(
        after_disable.get(&"logging.channel".parse().unwrap()),
        Some(&ConfigValue::Ref("6437".to_owned()))
    );
}

#[tokio::test]
async fn test_a_rejected_patch_leaves_the_registrys_provider_unchanged() {
    let registry = build_registry();
    let provider = registry
        .get("logging")
        .expect("logging should be registered");
    let ctx = test_context();

    let bad_patch = ConfigPatch::new().with(
        "logging.channel".parse().unwrap(),
        ConfigValue::Ref("not a snowflake".to_owned()),
    );
    let error = provider.apply_patch(&ctx, &bad_patch).await.unwrap_err();
    assert!(matches!(error, ConfigError::Invalid(_)));

    let values = provider.load_values(&ctx).await.expect("should not error");
    assert_eq!(
        values.get(&"logging.channel".parse().unwrap()),
        Some(&ConfigValue::Unset),
        "the rejected patch must not have changed anything"
    );
}

#[tokio::test]
async fn test_autodelete_write_classifies_changes_via_collection_diff() {
    let registry = build_registry();
    let provider = registry
        .get("autodelete")
        .expect("autodelete should be registered");
    let ctx = test_context();

    // two brand new timers: both should be classified as "added".
    let add_two = ConfigPatch::new()
        .with(
            "timers[6437].lifetime".parse().unwrap(),
            ConfigValue::Duration(3600),
        )
        .with(
            "timers[9001].lifetime".parse().unwrap(),
            ConfigValue::Duration(60),
        );
    provider
        .apply_patch(&ctx, &add_two)
        .await
        .expect("patch should apply");

    // DynConfigProvider is deliberately type-erased, so there's no way to
    // reach AutoDeleteProvider's own added/changed/removed counters
    // through this Arc<dyn DynConfigProvider> -- only that the write
    // succeeded and the values it produced are right. The counters
    // themselves are exercised directly in the next test.
    let values = provider.load_values(&ctx).await.expect("should not error");
    assert_eq!(
        values.get(&"timers[6437].lifetime".parse().unwrap()),
        Some(&ConfigValue::Duration(3600))
    );
    assert_eq!(
        values.get(&"timers[9001].lifetime".parse().unwrap()),
        Some(&ConfigValue::Duration(60))
    );
}

#[tokio::test]
async fn test_autodelete_provider_write_records_added_changed_and_removed_counts() {
    // exercised directly (not through the type-erased registry) so the
    // counters collection_diff feeds are actually observable.
    let provider = AutoDeleteProvider::default();
    let ctx = test_context();

    let add_two = ConfigPatch::new()
        .with(
            "timers[6437].lifetime".parse().unwrap(),
            ConfigValue::Duration(3600),
        )
        .with(
            "timers[9001].lifetime".parse().unwrap(),
            ConfigValue::Duration(60),
        );
    provider
        .apply_patch(&ctx, &add_two)
        .await
        .expect("patch should apply");
    assert_eq!(provider.last_added.load(Ordering::SeqCst), 2);
    assert_eq!(provider.last_changed.load(Ordering::SeqCst), 0);
    assert_eq!(provider.last_removed.load(Ordering::SeqCst), 0);

    // edit one, remove the other.
    let edit_and_remove = ConfigPatch::new()
        .with(
            "timers[6437].lifetime".parse().unwrap(),
            ConfigValue::Duration(7200),
        )
        .with("timers[9001]".parse().unwrap(), ConfigValue::Unset);
    provider
        .apply_patch(&ctx, &edit_and_remove)
        .await
        .expect("patch should apply");
    assert_eq!(provider.last_added.load(Ordering::SeqCst), 0);
    assert_eq!(provider.last_changed.load(Ordering::SeqCst), 1);
    assert_eq!(provider.last_removed.load(Ordering::SeqCst), 1);
}
