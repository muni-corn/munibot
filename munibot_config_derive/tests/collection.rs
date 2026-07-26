//! Compiles and exercises `#[derive(Configurable)]` for a `BTreeMap<K, V>`
//! field, verifying it becomes a `Collection` and behaves the same as the
//! hand-written `AutoDeleteConfig` in `munibot_config/tests/end_to_end.rs`.

use std::{collections::BTreeMap, time::Duration};

use munibot_config::{
    ChannelRef, ConfigNode, ConfigPatch, ConfigValue, Configurable, KeyKind, SettingDuration,
};
use munibot_config_derive::{ConfigChoice, Configurable};
use serde::{Deserialize, Serialize};

// now the real derive, upgraded from the hand-rolled prototype this test
// used before ConfigChoice existed -- proves the two derives compose.
#[derive(Clone, Copy, Debug, Default, PartialEq, ConfigChoice)]
enum CleanupMode {
    Always,
    #[default]
    AfterSilence,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct ChannelAutoDelete {
    #[config(label = "message lifetime")]
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

#[test]
fn test_btreemap_field_becomes_a_collection() {
    let schema = AutoDeleteConfig::config_schema();
    let [
        ConfigNode::Collection {
            id,
            label,
            key,
            add_label,
            item,
        },
    ] = schema.root.as_slice()
    else {
        panic!("expected exactly one Collection node");
    };

    assert_eq!(id, "timers");
    assert_eq!(label, "autodelete timers");
    assert_eq!(*key, KeyKind::DiscordChannel);
    assert_eq!(add_label, "add a channel");
    assert_eq!(item.len(), 2);
}

#[test]
fn test_adding_a_timer_needs_only_one_leaf_field_patched() {
    let patch = ConfigPatch::new().with(
        "timers[6437].lifetime".parse().unwrap(),
        ConfigValue::Duration(3600),
    );

    let updated = AutoDeleteConfig::default()
        .with_patch(&patch)
        .expect("patch should apply");

    let timer = updated
        .timers
        .get(&ChannelRef(6437))
        .expect("timer should exist");
    assert_eq!(
        timer.lifetime,
        SettingDuration::new(Duration::from_secs(3600))
    );
    assert_eq!(timer.mode, CleanupMode::AfterSilence, "schema default");
}

#[test]
fn test_editing_one_timer_leaves_its_sibling_alone() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete {
        lifetime: SettingDuration::new(Duration::from_secs(3600)),
        mode: CleanupMode::AfterSilence,
    });
    timers.insert(ChannelRef(9001), ChannelAutoDelete {
        lifetime: SettingDuration::new(Duration::from_secs(60)),
        mode: CleanupMode::Always,
    });

    let patch = ConfigPatch::new().with(
        "timers[6437].lifetime".parse().unwrap(),
        ConfigValue::Duration(7200),
    );
    let updated = AutoDeleteConfig { timers }
        .with_patch(&patch)
        .expect("patch should apply");

    assert_eq!(
        updated.timers[&ChannelRef(6437)].lifetime,
        SettingDuration::new(Duration::from_secs(7200))
    );
    assert_eq!(updated.timers[&ChannelRef(9001)].mode, CleanupMode::Always);
}

#[test]
fn test_removing_a_timer_via_unset_on_its_bare_key_path() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete::default());
    timers.insert(ChannelRef(9001), ChannelAutoDelete::default());

    let patch = ConfigPatch::new().with("timers[9001]".parse().unwrap(), ConfigValue::Unset);
    let updated = AutoDeleteConfig { timers }
        .with_patch(&patch)
        .expect("patch should apply");

    assert_eq!(updated.timers.len(), 1);
    assert!(updated.timers.contains_key(&ChannelRef(6437)));
}
