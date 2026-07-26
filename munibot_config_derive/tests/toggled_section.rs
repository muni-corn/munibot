//! Compiles and exercises `#[derive(Configurable)]` for a `Toggled<T>`
//! field, verifying it becomes a toggled `Section` and behaves the same as
//! the hand-written equivalent in `munibot_config/tests/end_to_end.rs`.

use munibot_config::{ChannelRef, ConfigNode, ConfigPatch, ConfigValue, Configurable, Toggled};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct LoggingSettings {
    #[config(label = "log channel")]
    channel: Option<ChannelRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
#[config(id = "logging", label = "server logging", scope = DiscordGuild)]
struct LoggingConfig {
    #[config(label = "server logging", description = "posts events to a channel")]
    logging: Toggled<LoggingSettings>,
}

#[test]
fn test_toggled_field_becomes_a_section_with_a_bool_toggle() {
    let schema = LoggingConfig::config_schema();
    let [
        ConfigNode::Section {
            id,
            label,
            description,
            toggle,
            children,
        },
    ] = schema.root.as_slice()
    else {
        panic!("expected exactly one Section node");
    };

    assert_eq!(id, "logging");
    assert_eq!(label, "server logging");
    assert_eq!(description.as_deref(), Some("posts events to a channel"));
    assert_eq!(toggle.as_ref().map(|f| f.id.as_str()), Some("enabled"));
    assert_eq!(children.len(), 1);
}

#[test]
fn test_default_state_has_logging_disabled_and_no_channel() {
    let values = LoggingConfig::default().to_values();
    assert_eq!(
        values.get(&"logging.enabled".parse().unwrap()),
        Some(&ConfigValue::Bool(false))
    );
    assert_eq!(
        values.get(&"logging.channel".parse().unwrap()),
        Some(&ConfigValue::Unset)
    );
}

#[test]
fn test_enabling_and_setting_a_channel_via_one_patch() {
    let patch = ConfigPatch::new()
        .with("logging.enabled".parse().unwrap(), ConfigValue::Bool(true))
        .with(
            "logging.channel".parse().unwrap(),
            ConfigValue::Ref("6437".to_owned()),
        );

    let updated = LoggingConfig::default()
        .with_patch(&patch)
        .expect("patch should apply");

    assert!(updated.logging.enabled);
    assert_eq!(updated.logging.settings.channel, Some(ChannelRef(6437)));
}

#[test]
fn test_disabling_preserves_the_configured_channel() {
    let configured = LoggingConfig {
        logging: Toggled::new(true, LoggingSettings {
            channel: Some(ChannelRef(6437)),
        }),
    };

    let patch =
        ConfigPatch::new().with("logging.enabled".parse().unwrap(), ConfigValue::Bool(false));
    let updated = configured.with_patch(&patch).expect("patch should apply");

    assert!(!updated.logging.enabled);
    assert_eq!(updated.logging.settings.channel, Some(ChannelRef(6437)));
}
