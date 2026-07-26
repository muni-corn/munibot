//! The derive-powered counterpart to `munibot_config/tests/end_to_end.rs`
//! (phase 1's hand-written prototype): the same `LoggingConfig` and
//! `AutoDeleteConfig` shapes from docs/plans/gui-configuration.md section
//! 6.3, built entirely from `#[derive(Configurable)]` and
//! `#[derive(ConfigChoice)]` plus `#[config(...)]` attributes -- no
//! hand-written trait impls anywhere in this file. This is the proof the
//! whole design in docs/plans/gui-configuration.md actually holds up.

use std::{collections::BTreeMap, time::Duration};

use munibot_config::{
    ChannelRef, ConfigChoice, ConfigNode, ConfigPatch, ConfigValue, Configurable, ScopeKind,
    SettingDuration, Toggled,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, ConfigChoice)]
enum CleanupMode {
    Always,
    #[default]
    AfterSilence,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct LoggingSettings {
    #[config(label = "log channel")]
    channel: Option<ChannelRef>,

    // never sent to the gui or touched by a patch -- e.g. an internal
    // bookkeeping field that happens to live on the same struct.
    //
    // #[serde(default)] is required here, not just nice to have:
    // with_patch rebuilds json purely from config_fields(), which never
    // emits a node for a skipped field, so the rebuilt json is always
    // missing this key. without a serde default, deserializing back into
    // Self would fail with "missing field `internal_note`" on every patch.
    #[config(skip)]
    #[serde(default)]
    internal_note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
#[config(id = "logging", label = "server logging", description = "posts events to a channel", scope = DiscordGuild)]
struct LoggingConfig {
    #[config(label = "server logging")]
    logging: Toggled<LoggingSettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct ChannelAutoDelete {
    #[config(label = "message lifetime", min = 3600, max = 604800)]
    lifetime: SettingDuration,
    #[config(label = "cleanup mode")]
    mode: CleanupMode,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
#[config(id = "autodelete", label = "autodelete", scope = DiscordGuild)]
struct AutoDeleteConfig {
    #[config(label = "autodelete timers", add_label = "add a channel")]
    timers: BTreeMap<ChannelRef, ChannelAutoDelete>,
}

fn path(s: &str) -> munibot_config::ConfigPath {
    s.parse().expect("valid path")
}

// -- schemas --

#[test]
fn test_logging_config_schema_matches_the_plans_shape() {
    let schema = LoggingConfig::config_schema();
    assert_eq!(schema.descriptor.id, "logging");
    assert_eq!(schema.descriptor.scopes, vec![ScopeKind::DiscordGuild]);

    let [
        ConfigNode::Section {
            toggle, children, ..
        },
    ] = schema.root.as_slice()
    else {
        panic!("expected one Section");
    };
    assert!(toggle.is_some());
    // channel is present, internal_note is skipped.
    assert_eq!(children.len(), 1);
}

#[test]
fn test_autodelete_config_schema_matches_the_plans_shape() {
    let schema = AutoDeleteConfig::config_schema();
    assert_eq!(schema.descriptor.id, "autodelete");

    let [ConfigNode::Collection { item, .. }] = schema.root.as_slice() else {
        panic!("expected one Collection");
    };
    assert_eq!(item.len(), 2);
}

// -- logging: skip, toggle-preserves-state --

#[test]
fn test_skipped_field_never_appears_in_values() {
    let config = LoggingConfig {
        logging: Toggled::new(true, LoggingSettings {
            channel: Some(ChannelRef(6437)),
            internal_note: "do not send this".to_owned(),
        }),
    };
    let values = config.to_values();
    assert!(!values.contains_key(&path("logging.internal_note")));
}

#[test]
fn test_full_logging_lifecycle_enable_set_disable_preserves_channel() {
    let enable_and_set = ConfigPatch::new()
        .with(path("logging.enabled"), ConfigValue::Bool(true))
        .with(path("logging.channel"), ConfigValue::Ref("6437".to_owned()));
    let enabled = LoggingConfig::default()
        .with_patch(&enable_and_set)
        .expect("patch should apply");
    assert!(enabled.logging.enabled);
    assert_eq!(enabled.logging.settings.channel, Some(ChannelRef(6437)));

    let disable = ConfigPatch::new().with(path("logging.enabled"), ConfigValue::Bool(false));
    let disabled = enabled.with_patch(&disable).expect("patch should apply");
    assert!(!disabled.logging.enabled);
    assert_eq!(
        disabled.logging.settings.channel,
        Some(ChannelRef(6437)),
        "disabling must not lose the configured channel"
    );
}

// -- autodelete: full collection lifecycle --

#[test]
fn test_full_autodelete_lifecycle_add_edit_remove() {
    let add_two = ConfigPatch::new()
        .with(path("timers[6437].lifetime"), ConfigValue::Duration(3600))
        .with(
            path("timers[6437].mode"),
            ConfigValue::Choice("after_silence".to_owned()),
        )
        .with(path("timers[9001].lifetime"), ConfigValue::Duration(7200))
        .with(
            path("timers[9001].mode"),
            ConfigValue::Choice("always".to_owned()),
        );
    let config = AutoDeleteConfig::default()
        .with_patch(&add_two)
        .expect("patch should apply");
    assert_eq!(config.timers.len(), 2);

    let edit_one =
        ConfigPatch::new().with(path("timers[6437].lifetime"), ConfigValue::Duration(86400));
    let config = config.with_patch(&edit_one).expect("patch should apply");
    assert_eq!(
        config.timers[&ChannelRef(6437)].lifetime,
        SettingDuration::new(Duration::from_secs(86400))
    );
    assert_eq!(
        config.timers[&ChannelRef(9001)].mode,
        CleanupMode::Always,
        "untouched sibling"
    );

    let remove_one = ConfigPatch::new().with(path("timers[9001]"), ConfigValue::Unset);
    let config = config.with_patch(&remove_one).expect("patch should apply");
    assert_eq!(config.timers.len(), 1);
    assert!(!config.timers.contains_key(&ChannelRef(9001)));
}

// -- error aggregation across shapes --

#[test]
fn test_a_patch_with_a_coercion_error_and_a_constraint_violation_reports_both() {
    let patch = ConfigPatch::new()
        // structurally invalid: not a parseable snowflake.
        .with(
            path("timers[6437].lifetime"),
            ConfigValue::Text("not a duration".to_owned()),
        )
        // structurally valid, but violates the field's own min bound.
        .with(path("timers[9001].lifetime"), ConfigValue::Duration(1));

    let errors = AutoDeleteConfig::default().with_patch(&patch).unwrap_err();
    assert_eq!(errors.len(), 2);
}

// -- the outer struct's own serde impl still works normally --

#[test]
fn test_the_whole_config_struct_still_roundtrips_through_json_directly() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete {
        lifetime: SettingDuration::new(Duration::from_secs(3600)),
        mode: CleanupMode::Always,
    });
    let config = AutoDeleteConfig { timers };

    let json = serde_json::to_string(&config).expect("serialize");
    let parsed: AutoDeleteConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, config);
}
