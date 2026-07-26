//! Compiles and exercises `#[derive(Configurable)]` for the leaf-field-only
//! case this commit implements, using the actual munibot_config trait (not
//! hand-written impls, unlike phase 1's end_to_end.rs) to make sure the
//! generated code really compiles and behaves, not just that macro
//! expansion doesn't error.

use munibot_config::{ChannelRef, ConfigPatch, ConfigValue, Configurable, ScopeKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
#[config(id = "logging", label = "server logging", description = "posts events", scope = DiscordGuild)]
struct LoggingConfig {
    #[config(label = "enable logging")]
    enabled: bool,

    #[config(label = "log channel")]
    channel: Option<ChannelRef>,
}

// a nested type needs no container attributes at all.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct Nested {
    name: String,
}

#[test]
fn test_generated_descriptor_matches_container_attrs() {
    let schema = LoggingConfig::config_schema();
    assert_eq!(schema.descriptor.id, "logging");
    assert_eq!(schema.descriptor.label, "server logging");
    assert_eq!(
        schema.descriptor.description.as_deref(),
        Some("posts events")
    );
    assert_eq!(schema.descriptor.scopes, vec![ScopeKind::DiscordGuild]);
}

#[test]
fn test_generated_config_fields_has_one_node_per_field() {
    let schema = LoggingConfig::config_schema();
    assert_eq!(schema.root.len(), 2);
}

#[test]
fn test_field_label_falls_back_to_the_humanized_field_name() {
    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
    struct X {
        log_channel: Option<ChannelRef>,
    }
    let fields = X::config_fields();
    let [munibot_config::ConfigNode::Field(field)] = fields.as_slice() else {
        panic!("expected one field node");
    };
    assert_eq!(field.label, "log channel");
}

#[test]
fn test_nested_type_has_no_descriptor_call_panics() {
    let result = std::panic::catch_unwind(Nested::descriptor);
    assert!(
        result.is_err(),
        "descriptor() should panic on a nested type"
    );
}

#[test]
fn test_to_values_reflects_current_state() {
    let config = LoggingConfig {
        enabled: true,
        channel: Some(ChannelRef(6437)),
    };
    let values = config.to_values();
    assert_eq!(
        values.get(&"enabled".parse().unwrap()),
        Some(&ConfigValue::Bool(true))
    );
    assert_eq!(
        values.get(&"channel".parse().unwrap()),
        Some(&ConfigValue::Ref("6437".to_owned()))
    );
}

#[test]
fn test_with_patch_applies_changes_and_keeps_the_rest() {
    let config = LoggingConfig::default();
    let patch = ConfigPatch::new()
        .with("enabled".parse().unwrap(), ConfigValue::Bool(true))
        .with(
            "channel".parse().unwrap(),
            ConfigValue::Ref("6437".to_owned()),
        );

    let updated = config.with_patch(&patch).expect("patch should apply");
    assert!(updated.enabled);
    assert_eq!(updated.channel, Some(ChannelRef(6437)));
}

#[test]
fn test_with_patch_rejects_an_unparsable_channel() {
    let config = LoggingConfig::default();
    let patch = ConfigPatch::new().with(
        "channel".parse().unwrap(),
        ConfigValue::Ref("not a number".to_owned()),
    );

    let errors = config.with_patch(&patch).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, "channel".parse().unwrap());
}
