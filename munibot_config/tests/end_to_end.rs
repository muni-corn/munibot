//! End-to-end coverage combining every piece built in phase 1: a toggled
//! section, a collection of keyed items, a choice, a duration, and a
//! discord reference, wired together the same way the real `LoggingConfig`
//! and `AutoDeleteConfig` (docs/plans/gui-configuration.md section 6.3)
//! will be in phase 5/9.
//!
//! The types below stand in for what `#[derive(Configurable)]` and
//! `#[derive(ConfigChoice)]` (phase 2) will generate. Writing this by hand
//! is itself a check that the design holds together before committing to
//! proc-macro complexity.

use std::collections::BTreeMap;

use munibot_config::{
    ChannelRef, ChoiceOption, ConfigChoice, ConfigNode, ConfigPatch, ConfigPath, ConfigValue,
    Configurable, Field, FieldKind, KeyKind, ProviderDescriptor, RefKind, ScopeKind,
    SettingDuration, Toggled,
};
use serde::{Deserialize, Serialize, Serializer, de::Deserializer};

// -- CleanupMode: stands in for AutoDeleteMode with #[derive(ConfigChoice)] --

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum CleanupMode {
    Always,
    #[default]
    AfterSilence,
}

impl ConfigChoice for CleanupMode {
    fn choice_options() -> Vec<ChoiceOption> {
        vec![
            ChoiceOption {
                id: "always".to_owned(),
                label: "always".to_owned(),
            },
            ChoiceOption {
                id: "after_silence".to_owned(),
                label: "after silence".to_owned(),
            },
        ]
    }

    fn choice_id(&self) -> String {
        match self {
            Self::Always => "always".to_owned(),
            Self::AfterSilence => "after_silence".to_owned(),
        }
    }

    fn from_choice_id(id: &str) -> Option<Self> {
        match id {
            "always" => Some(Self::Always),
            "after_silence" => Some(Self::AfterSilence),
            _ => None,
        }
    }
}

// a real #[derive(ConfigChoice)] should generate this Serialize/Deserialize
// pair too (delegating to choice_id/from_choice_id) rather than requiring a
// separately-derived serde impl with matching rename attributes -- see
// docs/notes/config-schema-implementation-notes.md. writing it by hand
// here is what that generated code should look like.
impl Serialize for CleanupMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.choice_id())
    }
}

impl<'de> Deserialize<'de> for CleanupMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::from_choice_id(&id)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown cleanup mode `{id}`")))
    }
}

// -- ChannelAutoDelete: a Collection item, stands in for the per-channel --
// -- settings inside AutoDeleteConfig::timers                          --

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct ChannelAutoDelete {
    lifetime: SettingDuration,
    mode: CleanupMode,
}

impl Configurable for ChannelAutoDelete {
    fn config_fields() -> Vec<ConfigNode> {
        vec![
            ConfigNode::Field(Field::new(
                "lifetime",
                "message lifetime",
                FieldKind::Duration {
                    min: None,
                    max: None,
                },
            )),
            ConfigNode::Field(
                Field::new(
                    "mode",
                    "cleanup mode",
                    FieldKind::Choice {
                        options: CleanupMode::choice_options(),
                    },
                )
                // a required (non-Option) field needs a real default here,
                // not ConfigValue::Unset -- otherwise unflatten falls back
                // to null for anything that never explicitly set this
                // field (e.g. a newly-added collection item), and CleanupMode
                // isn't nullable.
                .default(ConfigValue::Choice(CleanupMode::default().choice_id())),
            ),
        ]
    }
    // no descriptor(): this type is only ever used as Collection item
    // content, never registered directly with a ConfigurationProvider.
}

// -- AutoDeleteConfig: a provider root with one Collection field --

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct AutoDeleteConfig {
    timers: BTreeMap<ChannelRef, ChannelAutoDelete>,
}

impl Configurable for AutoDeleteConfig {
    fn config_fields() -> Vec<ConfigNode> {
        vec![ConfigNode::Collection {
            id: "timers".to_owned(),
            label: "autodelete timers".to_owned(),
            key: KeyKind::DiscordChannel,
            add_label: "add a channel".to_owned(),
            item: ChannelAutoDelete::config_fields(),
        }]
    }

    fn descriptor() -> ProviderDescriptor {
        ProviderDescriptor {
            id: "autodelete".to_owned(),
            label: "autodelete".to_owned(),
            description: None,
            icon: None,
            scopes: vec![ScopeKind::DiscordGuild],
        }
    }
}

// -- LoggingSettings/LoggingConfig: a toggled Section, stands in for --
// -- LoggingConfig                                                  --

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct LoggingSettings {
    channel: Option<ChannelRef>,
}

impl Configurable for LoggingSettings {
    fn config_fields() -> Vec<ConfigNode> {
        vec![ConfigNode::Field(Field::new(
            "channel",
            "log channel",
            FieldKind::Ref {
                kind: RefKind::DiscordChannel,
            },
        ))]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct LoggingConfig {
    logging: Toggled<LoggingSettings>,
}

impl Configurable for LoggingConfig {
    fn config_fields() -> Vec<ConfigNode> {
        vec![ConfigNode::Section {
            id: "logging".to_owned(),
            label: "server logging".to_owned(),
            description: None,
            toggle: Some(Field::new("enabled", "enable logging", FieldKind::Bool)),
            children: LoggingSettings::config_fields(),
        }]
    }

    fn descriptor() -> ProviderDescriptor {
        ProviderDescriptor {
            id: "logging".to_owned(),
            label: "server logging".to_owned(),
            description: None,
            icon: None,
            scopes: vec![ScopeKind::DiscordGuild],
        }
    }
}

fn path(s: &str) -> ConfigPath {
    s.parse().expect("valid path")
}

// -- schemas --

#[test]
fn test_logging_config_schema_matches_its_field_shape() {
    let schema = LoggingConfig::config_schema();
    assert_eq!(schema.descriptor.id, "logging");
    let [
        ConfigNode::Section {
            id,
            toggle,
            children,
            ..
        },
    ] = schema.root.as_slice()
    else {
        panic!("expected exactly one Section");
    };
    assert_eq!(id, "logging");
    assert_eq!(toggle.as_ref().map(|f| f.id.as_str()), Some("enabled"));
    assert_eq!(children.len(), 1);
}

#[test]
fn test_autodelete_config_schema_matches_its_field_shape() {
    let schema = AutoDeleteConfig::config_schema();
    assert_eq!(schema.descriptor.id, "autodelete");
    let [ConfigNode::Collection { id, key, item, .. }] = schema.root.as_slice() else {
        panic!("expected exactly one Collection");
    };
    assert_eq!(id, "timers");
    assert_eq!(*key, KeyKind::DiscordChannel);
    assert_eq!(item.len(), 2);
}

// -- logging: toggled section --

#[test]
fn test_logging_default_state_has_nothing_configured() {
    let values = LoggingConfig::default().to_values();
    assert_eq!(
        values.get(&path("logging.enabled")),
        Some(&ConfigValue::Bool(false))
    );
    assert_eq!(
        values.get(&path("logging.channel")),
        Some(&ConfigValue::Unset)
    );
}

#[test]
fn test_enabling_logging_and_setting_a_channel_via_one_patch() {
    let patch = ConfigPatch::new()
        .with(path("logging.enabled"), ConfigValue::Bool(true))
        .with(path("logging.channel"), ConfigValue::Ref("6437".to_owned()));

    let updated = LoggingConfig::default()
        .with_patch(&patch)
        .expect("patch should apply");

    assert!(updated.logging.enabled);
    assert_eq!(updated.logging.settings.channel, Some(ChannelRef(6437)));
}

#[test]
fn test_disabling_logging_preserves_the_configured_channel() {
    let configured = LoggingConfig {
        logging: Toggled::new(true, LoggingSettings {
            channel: Some(ChannelRef(6437)),
        }),
    };

    let patch = ConfigPatch::new().with(path("logging.enabled"), ConfigValue::Bool(false));
    let updated = configured.with_patch(&patch).expect("patch should apply");

    // this is the whole point of the toggle: turning logging off doesn't
    // forget which channel was configured, unlike today's admin.rs where
    // "off" means deleting the row.
    assert!(!updated.logging.enabled);
    assert_eq!(updated.logging.settings.channel, Some(ChannelRef(6437)));
}

#[test]
fn test_with_patch_rejects_an_unparsable_channel_and_names_its_path() {
    let patch = ConfigPatch::new().with(
        path("logging.channel"),
        ConfigValue::Ref("not a snowflake".to_owned()),
    );

    let errors = LoggingConfig::default().with_patch(&patch).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, path("logging.channel"));
}

// -- autodelete: collection of keyed items --

#[test]
fn test_adding_a_timer_needs_no_explicit_key_creation_step() {
    // patching a single leaf field under a key that doesn't exist yet is
    // enough to create that collection item -- its other fields fall back
    // to their schema defaults.
    let patch = ConfigPatch::new().with(path("timers[6437].lifetime"), ConfigValue::Duration(3600));

    let updated = AutoDeleteConfig::default()
        .with_patch(&patch)
        .expect("patch should apply");

    let timer = updated
        .timers
        .get(&ChannelRef(6437))
        .expect("timer should exist");
    assert_eq!(
        timer.lifetime,
        SettingDuration::new(std::time::Duration::from_secs(3600))
    );
    assert_eq!(timer.mode, CleanupMode::AfterSilence); // schema default
}

#[test]
fn test_adding_two_timers_in_one_patch() {
    let patch = ConfigPatch::new()
        .with(path("timers[6437].lifetime"), ConfigValue::Duration(3600))
        .with(
            path("timers[6437].mode"),
            ConfigValue::Choice("after_silence".to_owned()),
        )
        .with(path("timers[9001].lifetime"), ConfigValue::Duration(60))
        .with(
            path("timers[9001].mode"),
            ConfigValue::Choice("always".to_owned()),
        );

    let updated = AutoDeleteConfig::default()
        .with_patch(&patch)
        .expect("patch should apply");

    assert_eq!(updated.timers.len(), 2);
    assert_eq!(updated.timers[&ChannelRef(9001)].mode, CleanupMode::Always);
}

#[test]
fn test_editing_one_timer_leaves_its_own_other_field_and_its_sibling_alone() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete {
        lifetime: SettingDuration::new(std::time::Duration::from_secs(3600)),
        mode: CleanupMode::AfterSilence,
    });
    timers.insert(ChannelRef(9001), ChannelAutoDelete {
        lifetime: SettingDuration::new(std::time::Duration::from_secs(60)),
        mode: CleanupMode::Always,
    });
    let config = AutoDeleteConfig { timers };

    // only touch 6437's lifetime.
    let patch = ConfigPatch::new().with(path("timers[6437].lifetime"), ConfigValue::Duration(7200));
    let updated = config.with_patch(&patch).expect("patch should apply");

    let edited = &updated.timers[&ChannelRef(6437)];
    assert_eq!(
        edited.lifetime,
        SettingDuration::new(std::time::Duration::from_secs(7200))
    );
    assert_eq!(
        edited.mode,
        CleanupMode::AfterSilence,
        "untouched sibling field"
    );

    let untouched = &updated.timers[&ChannelRef(9001)];
    assert_eq!(
        untouched.lifetime,
        SettingDuration::new(std::time::Duration::from_secs(60))
    );
    assert_eq!(untouched.mode, CleanupMode::Always);
}

#[test]
fn test_removing_a_timer_via_unset_on_its_bare_key_path() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete::default());
    timers.insert(ChannelRef(9001), ChannelAutoDelete::default());
    let config = AutoDeleteConfig { timers };

    let patch = ConfigPatch::new().with(path("timers[9001]"), ConfigValue::Unset);
    let updated = config.with_patch(&patch).expect("patch should apply");

    assert_eq!(updated.timers.len(), 1);
    assert!(updated.timers.contains_key(&ChannelRef(6437)));
    assert!(!updated.timers.contains_key(&ChannelRef(9001)));
}

#[test]
fn test_removing_the_only_timer_leaves_an_empty_collection() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete::default());
    let config = AutoDeleteConfig { timers };

    let patch = ConfigPatch::new().with(path("timers[6437]"), ConfigValue::Unset);
    let updated = config.with_patch(&patch).expect("patch should apply");

    assert!(updated.timers.is_empty());
}

#[test]
fn test_a_patch_touching_two_bad_timers_reports_both_errors() {
    let patch = ConfigPatch::new()
        .with(
            path("timers[6437].lifetime"),
            ConfigValue::Text("not a duration".to_owned()),
        )
        .with(
            path("timers[9001].mode"),
            ConfigValue::Choice("bogus".to_owned()),
        );

    let errors = AutoDeleteConfig::default().with_patch(&patch).unwrap_err();
    assert_eq!(errors.len(), 2);
}

// -- full round trips --

#[test]
fn test_autodelete_state_survives_a_to_values_with_patch_round_trip_unchanged() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete {
        lifetime: SettingDuration::new(std::time::Duration::from_secs(3600)),
        mode: CleanupMode::Always,
    });
    let config = AutoDeleteConfig { timers };

    // applying an empty patch should be a no-op.
    let round_tripped = config
        .with_patch(&ConfigPatch::new())
        .expect("empty patch should apply");
    assert_eq!(round_tripped, config);
}
