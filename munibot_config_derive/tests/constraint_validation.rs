//! Exercises the `validate()` generated for fields with
//! `#[config(min/max/max_len = ...)]`, and that nested `Section`/
//! `Collection` validation errors get prefixed with the right path.

use std::{collections::BTreeMap, time::Duration};

use munibot_config::{
    ChannelRef, ConfigPatch, ConfigValue, Configurable, SettingDuration, Toggled,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct ChannelAutoDelete {
    #[config(label = "message lifetime", min = 3600, max = 604800)]
    lifetime: SettingDuration,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
struct LoggingSettings {
    #[config(label = "note", max_len = 5)]
    note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Configurable)]
#[config(id = "logging", label = "server logging", scope = DiscordGuild)]
struct LoggingConfig {
    #[config(label = "server logging")]
    logging: Toggled<LoggingSettings>,

    #[config(label = "timers")]
    timers: BTreeMap<ChannelRef, ChannelAutoDelete>,
}

#[test]
fn test_a_value_within_bounds_validates() {
    let config = ChannelAutoDelete {
        lifetime: SettingDuration::new(Duration::from_secs(86400)),
    };
    assert!(config.validate().is_ok());
}

#[test]
fn test_a_value_below_min_fails_validation() {
    let config = ChannelAutoDelete {
        lifetime: SettingDuration::new(Duration::from_secs(60)),
    };
    let errors = config.validate().unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, "lifetime".parse().unwrap());
}

#[test]
fn test_a_value_above_max_fails_validation() {
    let config = ChannelAutoDelete {
        lifetime: SettingDuration::new(Duration::from_secs(999999)),
    };
    assert!(config.validate().is_err());
}

#[test]
fn test_with_patch_rejects_a_patch_that_violates_a_bound() {
    let config = ChannelAutoDelete::default();
    let patch = ConfigPatch::new().with("lifetime".parse().unwrap(), ConfigValue::Duration(1));

    let errors = config.with_patch(&patch).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, "lifetime".parse().unwrap());
}

#[test]
fn test_max_len_rejects_a_too_long_string() {
    let config = LoggingSettings {
        note: "way too long".to_owned(),
    };
    assert!(config.validate().is_err());
}

#[test]
fn test_section_validation_errors_are_prefixed_with_the_field_path() {
    let config = LoggingConfig {
        logging: Toggled::new(true, LoggingSettings {
            note: "way too long".to_owned(),
        }),
        timers: BTreeMap::new(),
    };

    let errors = config.validate().unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, "logging.note".parse().unwrap());
}

#[test]
fn test_collection_validation_errors_are_prefixed_with_the_field_and_key() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete {
        lifetime: SettingDuration::new(Duration::from_secs(60)),
    });

    let config = LoggingConfig {
        logging: Toggled::default(),
        timers,
    };

    let errors = config.validate().unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, "timers[6437].lifetime".parse().unwrap());
}

#[test]
fn test_a_fully_valid_config_has_no_errors() {
    let mut timers = BTreeMap::new();
    timers.insert(ChannelRef(6437), ChannelAutoDelete {
        lifetime: SettingDuration::new(Duration::from_secs(86400)),
    });

    let config = LoggingConfig {
        logging: Toggled::new(true, LoggingSettings {
            note: "ok".to_owned(),
        }),
        timers,
    };

    assert_eq!(config.validate(), Ok(()));
}
